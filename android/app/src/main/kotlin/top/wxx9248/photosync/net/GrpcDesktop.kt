package top.wxx9248.photosync.net

import io.grpc.ManagedChannel
import io.grpc.okhttp.OkHttpChannelBuilder
import java.net.InetSocketAddress
import javax.net.ssl.SSLContext
import kotlinx.coroutines.flow.flow
import photosync.v1.CatalogChunkKt
import photosync.v1.PhotoSyncGrpcKt
import photosync.v1.catalogChunk
import photosync.v1.catalogEntry
import photosync.v1.deletionOutcome
import photosync.v1.deletionReport
import photosync.v1.diffRequest
import photosync.v1.fileChunk
import photosync.v1.fileHeader
import photosync.v1.fileTrailer
import photosync.v1.finishRequest
import photosync.v1.handshakeRequest
import top.wxx9248.photosync.session.CandidateOrigin
import top.wxx9248.photosync.session.Catalog
import top.wxx9248.photosync.session.DeletionCandidate
import top.wxx9248.photosync.session.DeletionOutcome
import top.wxx9248.photosync.session.DeletionResult
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.DiffSummary
import top.wxx9248.photosync.session.FileId
import top.wxx9248.photosync.session.Inbound
import top.wxx9248.photosync.session.Outbound
import top.wxx9248.photosync.session.RejectReason
import top.wxx9248.photosync.session.Sha256
import top.wxx9248.photosync.session.Timestamp
import top.wxx9248.photosync.session.ToSend
import top.wxx9248.photosync.session.UploadOutcome

/**
 * The wire, on this end.
 *
 * The session state machine says what to send and reads what comes back; this is the only
 * thing that knows either is gRPC. It is written as a translation and nothing else — no
 * decisions, no retries, no state of its own beyond the file being streamed — because a
 * decision made here is a decision the JVM tests cannot see.
 *
 * One file at a time crosses here. §6.4 allows several concurrent streams and the desktop is
 * built for them; opening more is a change to this class alone, and it is worth measuring on
 * a real network before it is made.
 */
class GrpcDesktop(
    private val channel: ManagedChannel,
    private val identity: Identity,
) : AutoCloseable {
    private val stub = PhotoSyncGrpcKt.PhotoSyncCoroutineStub(channel)

    /** Bytes of the file being sent, gathered until the phone closes the upload. */
    private var streaming: Streaming? = null

    private class Streaming(
        val file: FileId,
        val path: DevicePath,
        val size: Long,
        val mtime: Long,
        val offset: Long,
        val body: MutableList<ByteArray> = mutableListOf(),
    )

    /**
     * Says one thing, and hands back the answer if that call has one.
     *
     * Not every message is a call: the chunks of a file accumulate here and travel when the
     * digest arrives, because `UploadFile` is one streaming call rather than one per chunk.
     */
    suspend fun say(said: Outbound): Inbound? = when (said) {
        is Outbound.Handshake -> handshake(said)
        is Outbound.SubmitCatalog -> submit(said.catalog)
        Outbound.RequestDiff -> diff()
        is Outbound.OpenUpload -> {
            streaming = Streaming(said.file, said.path, said.size, said.mtime.seconds, said.offset)
            null
        }
        is Outbound.SendChunk -> {
            streaming?.body?.add(said.data)
            null
        }
        is Outbound.CloseUpload -> upload(said)
        Outbound.Finish -> finish()
        is Outbound.ReportDeletions -> report(said.outcomes)
    }

    private suspend fun handshake(said: Outbound.Handshake): Inbound = runCatching {
        val answered = stub.handshake(
            handshakeRequest {
                protocolVersion = said.protocolVersion
                deviceId = said.device.value
                deviceName = said.name
            }
        )
        Inbound.HandshakeAccepted(answered.desktopName, answered.commitInProgress) as Inbound
    }.getOrElse { Inbound.Rejected(reasonOf(it)) }

    private suspend fun submit(catalog: Catalog): Inbound = runCatching {
        val chunks = catalog.entries.chunked(ENTRIES_PER_MESSAGE)
        stub.submitCatalog(
            flow {
                if (chunks.isEmpty()) {
                    emit(catalogChunk { last = true; totalBytes = 0 })
                    return@flow
                }
                chunks.forEachIndexed { at, batch ->
                    emit(
                        catalogChunk {
                            entries.addAll(
                                batch.map { one ->
                                    catalogEntry {
                                        path = one.path.value
                                        size = one.size
                                        mtime = one.mtime.seconds
                                    }
                                }
                            )
                            last = at == chunks.lastIndex
                            if (last) totalBytes = catalog.totalBytes
                        }
                    )
                }
            }
        )
        Inbound.CatalogAcknowledged as Inbound
    }.getOrElse { Inbound.Rejected(reasonOf(it)) }

    private suspend fun diff(): Inbound = runCatching {
        val wanted = mutableListOf<ToSend>()
        var summary = DiffSummary(0, 0, 0, 0)
        stub.getDiff(diffRequest {}).collect { chunk ->
            chunk.toSendList.forEach { one ->
                wanted.add(
                    ToSend(
                        file = FileId(one.fileId.toLongOrNull() ?: 0),
                        path = DevicePath(one.path),
                        resumeOffset = one.resumeOffset,
                    )
                )
            }
            if (chunk.last && chunk.hasSummary()) {
                summary = DiffSummary(
                    toSendCount = chunk.summary.toSendCount,
                    toSendBytes = chunk.summary.toSendBytes,
                    alreadyImported = chunk.summary.alreadyImported,
                    alreadyStaged = chunk.summary.alreadyStaged,
                )
            }
        }
        Inbound.Diff(wanted, summary) as Inbound
    }.getOrElse { Inbound.Rejected(reasonOf(it)) }

    private suspend fun upload(said: Outbound.CloseUpload): Inbound {
        val sending = streaming ?: return Inbound.UploadAnswered(said.file, UploadOutcome.WRITE_FAILED)
        streaming = null

        return runCatching {
            val answered = stub.uploadFile(
                flow {
                    emit(
                        fileChunk {
                            header = fileHeader {
                                fileId = sending.file.value.toString()
                                path = sending.path.value
                                size = sending.size
                                mtime = sending.mtime
                                offset = sending.offset
                            }
                        }
                    )
                    sending.body.forEach { piece ->
                        emit(fileChunk { data = com.google.protobuf.ByteString.copyFrom(piece) })
                    }
                    emit(
                        fileChunk {
                            trailer = fileTrailer {
                                sha256 = com.google.protobuf.ByteString.copyFrom(
                                    said.digest.hex.chunked(2)
                                        .map { it.toInt(16).toByte() }
                                        .toByteArray()
                                )
                            }
                        }
                    )
                }
            )
            Inbound.UploadAnswered(said.file, outcomeOf(answered.statusValue)) as Inbound
        }.getOrElse { Inbound.UploadAnswered(said.file, UploadOutcome.WRITE_FAILED) }
    }

    private suspend fun finish(): Inbound = runCatching {
        val candidates = mutableListOf<DeletionCandidate>()
        stub.finish(finishRequest {}).collect { chunk ->
            chunk.candidatesList.forEach { one ->
                candidates.add(
                    DeletionCandidate(
                        path = DevicePath(one.path),
                        size = one.size,
                        mtime = Timestamp(one.mtime),
                        expected = Sha256(
                            one.expectedSha256.toByteArray().joinToString("") { "%02x".format(it) }
                        ),
                        origin = if (one.originValue == ORIGIN_EARLIER) {
                            CandidateOrigin.EARLIER
                        } else {
                            CandidateOrigin.THIS_TRANSFER
                        },
                    )
                )
            }
        }
        Inbound.Candidates(candidates) as Inbound
    }.getOrElse { Inbound.Rejected(reasonOf(it)) }

    private suspend fun report(outcomes: List<DeletionOutcome>): Inbound = runCatching {
        val answered = stub.reportDeletions(
            flow {
                emit(
                    deletionReport {
                        this.outcomes.addAll(
                            outcomes.map { one ->
                                deletionOutcome {
                                    path = one.path.value
                                    resultValue = resultOf(one.result)
                                }
                            }
                        )
                        last = true
                    }
                )
            }
        )
        Inbound.SessionFinished(
            sent = answered.sent,
            skipped = answered.skipped,
            failed = answered.failed,
            deleted = answered.deleted,
            kept = answered.kept,
            bytesFreed = answered.bytesFreed,
        ) as Inbound
    }.getOrElse { Inbound.Rejected(reasonOf(it)) }

    override fun close() {
        channel.shutdownNow()
    }

    private fun reasonOf(failure: Throwable): RejectReason {
        val status = io.grpc.Status.fromThrowable(failure)
        return when (status.code) {
            io.grpc.Status.Code.RESOURCE_EXHAUSTED -> RejectReason.NO_SPACE
            io.grpc.Status.Code.UNAVAILABLE -> RejectReason.COMMIT_IN_PROGRESS
            io.grpc.Status.Code.FAILED_PRECONDITION -> RejectReason.PROTOCOL_VERSION
            else -> RejectReason.NOT_PAIRED
        }
    }

    private fun outcomeOf(status: Int): UploadOutcome = when (status) {
        1 -> UploadOutcome.VERIFIED
        2 -> UploadOutcome.HASH_MISMATCH
        3 -> UploadOutcome.CHANGED_ON_PHONE
        else -> UploadOutcome.WRITE_FAILED
    }

    private fun resultOf(result: DeletionResult): Int = when (result) {
        DeletionResult.DELETED -> 1
        DeletionResult.KEPT_CHANGED -> 2
        DeletionResult.KEPT_USER -> 3
        DeletionResult.FAILED -> 4
    }

    companion object {
        /** As many entries as the desktop puts in one message. `STACK.md` §5.3. */
        const val ENTRIES_PER_MESSAGE: Int = 1000

        private const val ORIGIN_EARLIER = 2

        /**
         * Opens a channel to a desktop this phone is paired with.
         *
         * Both directions are pinned: the phone checks the desktop against the key it wrote
         * down, and presents its own so the desktop can do the same. Nothing else about
         * either certificate is examined. §5.2.
         */
        fun connect(address: InetSocketAddress, identity: Identity, desktopPin: String): GrpcDesktop {
            val context = SSLContext.getInstance("TLSv1.3").apply {
                init(
                    arrayOf(DeviceKeyManager(identity)),
                    arrayOf(PinnedDesktop(desktopPin)),
                    null,
                )
            }
            val channel = OkHttpChannelBuilder.forAddress(address.hostString, address.port)
                .useTransportSecurity()
                .sslSocketFactory(context.socketFactory)
                // The name is not checked — the key is — but a channel needs one to put in
                // the SNI extension, and "photo-sync" is what the desktop's own certificate
                // says.
                .overrideAuthority("photo-sync")
                .build()
            return GrpcDesktop(channel, identity)
        }
    }
}
