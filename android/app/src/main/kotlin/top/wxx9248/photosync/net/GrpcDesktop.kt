package top.wxx9248.photosync.net

import com.google.protobuf.ByteString
import io.grpc.ManagedChannel
import io.grpc.okhttp.OkHttpChannelBuilder
import java.net.InetSocketAddress
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import kotlin.coroutines.coroutineContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.SendChannel
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.consumeAsFlow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.plus
import kotlinx.coroutines.selects.select
import photosync.v1.PhotoSyncGrpcKt
// The generated messages, under the name the desktop's own adapter gives them.
import photosync.v1.Sync as Wire
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
internal class GrpcDesktop(
    private val channel: ManagedChannel,
    parent: CoroutineScope,
) : AutoCloseable {
    private val stub = PhotoSyncGrpcKt.PhotoSyncCoroutineStub(channel)

    /**
     * Where an upload runs while the calls that feed it come and go.
     *
     * A file crosses over many [say] calls, so its request cannot live inside one of them. The
     * scope is a child of the caller's, so ending a session ends the upload, and it is a
     * supervisor, so a file the desktop refused fails on its own rather than taking the
     * session down with it.
     */
    private val uploads = parent + SupervisorJob(parent.coroutineContext[Job])

    /** The file crossing right now: what it is being fed, and what the desktop will answer. */
    private var uploading: Uploading? = null

    private class Uploading(
        val pieces: SendChannel<Wire.FileChunk>,
        val answer: Deferred<Wire.UploadResult>,
    )

    /**
     * Says one thing, and hands back the answer if that call has one.
     *
     * Not every message is a call: the pieces of a file all belong to one `UploadFile`
     * request, so opening starts it, chunks travel along it, and the digest closes it.
     */
    suspend fun say(said: Outbound): Inbound? = when (said) {
        is Outbound.Handshake -> handshake(said)
        is Outbound.SubmitCatalog -> submit(said.catalog)
        Outbound.RequestDiff -> diff()
        is Outbound.OpenUpload -> open(said)
        is Outbound.SendChunk -> {
            uploading?.let { feed(it, fileChunk { data = ByteString.copyFrom(said.data) }) }
            null
        }
        is Outbound.CloseUpload -> upload(said)
        Outbound.Finish -> finish()
        is Outbound.ReportDeletions -> report(said.outcomes)
    }

    private suspend fun handshake(said: Outbound.Handshake): Inbound =
        answering({ Inbound.Rejected(reasonOf(it)) }) {
            val answered = stub.handshake(
                handshakeRequest {
                    protocolVersion = said.protocolVersion
                    deviceId = said.device.value
                    deviceName = said.name
                }
            )
            Inbound.HandshakeAccepted(answered.desktopName, answered.commitInProgress)
        }

    private suspend fun submit(catalog: Catalog): Inbound =
        answering({ Inbound.Rejected(reasonOf(it)) }) {
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
            Inbound.CatalogAcknowledged
        }

    private suspend fun diff(): Inbound = answering({ Inbound.Rejected(reasonOf(it)) }) {
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
        Inbound.Diff(wanted, summary)
    }

    /**
     * Opens the request a file travels in, and sends its header.
     *
     * What the header states is what the phone holds now, which is why §6.4 lets the desktop
     * notice a photograph that changed and skip it.
     */
    private suspend fun open(said: Outbound.OpenUpload): Inbound? {
        val pieces = Channel<Wire.FileChunk>(Channel.RENDEZVOUS)
        val sending = Uploading(pieces, uploads.async { stub.uploadFile(pieces.consumeAsFlow()) })
        uploading = sending
        feed(
            sending,
            fileChunk {
                header = fileHeader {
                    fileId = said.file.value.toString()
                    path = said.path.value
                    size = said.size
                    mtime = said.mtime.seconds
                    offset = said.offset
                }
            },
        )
        return null
    }

    /**
     * Hands one message to the upload in flight, and waits for it to actually go out.
     *
     * The channel has no buffer, and that is the point: the phone holds the one piece it is
     * sending rather than the whole file, so a video larger than the phone's memory still
     * crosses. `STACK.md` §5.3 says chunking bounds memory on both ends, and this is the
     * phone's end of it.
     */
    private suspend fun feed(sending: Uploading, piece: Wire.FileChunk) {
        try {
            // Whichever happens first: the piece goes out, or the call ends without it. A
            // desktop that disappears mid-file leaves nothing to hand the piece to, and
            // waiting on that alone is waiting for ever --- which is how a dropped connection
            // became a phone that said "Working…" and never stopped.
            select {
                sending.pieces.onSend(piece) {}
                sending.answer.onAwait {}
            }
        } catch (ended: Exception) {
            // The desktop ended the call: it refused the file, or the connection went. A
            // channel whose collector has gone reports that the same way a cancelled caller
            // is reported, so the caller is asked which of the two happened. Either way the
            // answer to give is already waiting in `answer`, and the digest asks for it.
            coroutineContext.ensureActive()
        }
    }

    /** Closes the request with the digest, and reads what became of the file. */
    private suspend fun upload(said: Outbound.CloseUpload): Inbound {
        val sending = uploading ?: return Inbound.UploadAnswered(said.file, UploadOutcome.WRITE_FAILED)
        uploading = null

        feed(sending, fileChunk { trailer = fileTrailer { sha256 = hexToBytes(said.digest) } })
        sending.pieces.close()
        return answering({ Inbound.UploadAnswered(said.file, UploadOutcome.WRITE_FAILED) }) {
            Inbound.UploadAnswered(said.file, outcomeOf(sending.answer.await().statusValue))
        }
    }

    private suspend fun finish(): Inbound = answering({ Inbound.Rejected(reasonOf(it)) }) {
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
        Inbound.Candidates(candidates)
    }

    private suspend fun report(outcomes: List<DeletionOutcome>): Inbound =
        answering({ Inbound.Rejected(reasonOf(it)) }) {
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
            )
        }

    override fun close() {
        uploads.cancel()
        channel.shutdownNow()
    }

    /**
     * Runs one call, turning a failure into an answer the session can act on.
     *
     * `runCatching` is the wrong tool here. It also catches the `CancellationException` a
     * session that was told to stop throws, and reporting that as a refusal would leave the
     * coroutine running after its scope ended. Rule K8: the caller is asked whether it was
     * cancelled, and only what is left is the desktop or the network.
     */
    private suspend fun <T> answering(whenItFails: (Throwable) -> T, call: suspend () -> T): T =
        try {
            call()
        } catch (failure: Exception) {
            coroutineContext.ensureActive()
            whenItFails(failure)
        }

    /**
     * What the desktop's answer means, or that there was not one.
     *
     * The catch-all used to be "not paired", which made every dropped connection look like a
     * phone the desktop had forgotten. A connection that did not survive is [UNREACHABLE],
     * and §6 rejoins those instead of reporting them.
     */
    private fun reasonOf(failure: Throwable): RejectReason =
        when (io.grpc.Status.fromThrowable(failure).code) {
            io.grpc.Status.Code.UNAUTHENTICATED -> RejectReason.NOT_PAIRED
            io.grpc.Status.Code.ABORTED -> RejectReason.COMMIT_IN_PROGRESS
            io.grpc.Status.Code.RESOURCE_EXHAUSTED -> RejectReason.NO_SPACE
            io.grpc.Status.Code.FAILED_PRECONDITION -> RejectReason.PROTOCOL_VERSION
            else -> RejectReason.UNREACHABLE
        }

    private fun hexToBytes(digest: Sha256): ByteString =
        ByteString.copyFrom(digest.hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray())

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

        /**
         * How often a quiet connection is prodded, and how long an answer is waited for.
         *
         * Short enough that a desktop which went away is noticed while somebody is still
         * holding the phone, long enough not to be chatter on a home network.
         */
        const val KEEPALIVE_SECONDS: Long = 15

        const val KEEPALIVE_TIMEOUT_SECONDS: Long = 10

        private const val ORIGIN_EARLIER = 2

        /**
         * Opens a channel to a desktop this phone is paired with.
         *
         * Both directions are pinned: the phone checks the desktop against the key it wrote
         * down, and presents its own so the desktop can do the same. Nothing else about
         * either certificate is examined. §5.2.
         */
        fun connect(
            address: InetSocketAddress,
            identity: Identity,
            desktopPin: String,
            scope: CoroutineScope,
        ): GrpcDesktop {
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
                // A desktop that goes away mid-transfer takes its socket with it and says
                // nothing. Without these the phone waits on that socket for as long as the
                // operating system lets it, which is minutes, and §6's rejoin never starts
                // because the session it would rejoin has not ended.
                .keepAliveTime(KEEPALIVE_SECONDS, TimeUnit.SECONDS)
                .keepAliveTimeout(KEEPALIVE_TIMEOUT_SECONDS, TimeUnit.SECONDS)
                // The name is not checked — the key is — but a channel needs one to put in
                // the SNI extension, and "photo-sync" is what the desktop's own certificate
                // says.
                .overrideAuthority("photo-sync")
                .build()
            return GrpcDesktop(channel, scope)
        }
    }
}
