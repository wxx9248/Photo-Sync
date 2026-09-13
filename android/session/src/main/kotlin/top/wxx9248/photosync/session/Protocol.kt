package top.wxx9248.photosync.session

/**
 * What the phone says, and what it hears back.
 *
 * The phone drives every exchange in `SPEC.md` §6 and the desktop only answers, so these two
 * sets are not symmetrical: one is a set of requests and the other a set of replies. Both are
 * sealed, so a case added later is a compile error here rather than a silent fallthrough.
 */
sealed interface Outbound {
    /** First message on every connection, including a reconnection. §6 step 1. */
    data class Handshake(val device: DeviceId, val name: String, val protocolVersion: Int) : Outbound

    data class SubmitCatalog(val catalog: Catalog) : Outbound

    data object RequestDiff : Outbound

    /**
     * Opens one file's stream. What is stated here is what the phone holds *now*, which is how
     * §6.4 notices a photograph that changed since the catalog was frozen.
     */
    data class OpenUpload(
        val file: FileId,
        val path: DevicePath,
        val size: Long,
        val mtime: Timestamp,
        val offset: Long,
    ) : Outbound

    data class SendChunk(val file: FileId, val offset: Long, val data: ByteArray) : Outbound {
        // A data class over a ByteArray needs these written out; the generated ones compare
        // the reference, which would make two identical chunks unequal.
        override fun equals(other: Any?): Boolean =
            other is SendChunk && file == other.file && offset == other.offset &&
                data.contentEquals(other.data)

        override fun hashCode(): Int =
            (file.value.hashCode() * 31 + offset.hashCode()) * 31 + data.contentHashCode()
    }

    /** The digest covers the whole photograph, prefix included. §7.6. */
    data class CloseUpload(val file: FileId, val digest: Sha256) : Outbound

    data object Finish : Outbound

    data class ReportDeletions(val outcomes: List<DeletionOutcome>) : Outbound
}

/** One file the desktop wants, and where to start it. */
data class ToSend(val file: FileId, val path: DevicePath, val resumeOffset: Long)

/** How the desktop split the catalog. §6.2. */
data class DiffSummary(
    val toSendCount: Long,
    val toSendBytes: Long,
    val alreadyImported: Long,
    val alreadyStaged: Long,
)

/** Which gate a candidate faces before the phone lets go of it. §8. */
enum class CandidateOrigin {
    /** Committed by this session's batch: size and mtime carry the digest guarantee. */
    THIS_TRANSFER,

    /** Matched against an older index row: the phone re-hashes the file. */
    EARLIER,
}

data class DeletionCandidate(
    val path: DevicePath,
    val size: Long,
    val mtime: Timestamp,
    val expected: Sha256,
    val origin: CandidateOrigin,
)

enum class DeletionResult { DELETED, KEPT_CHANGED, KEPT_USER, FAILED }

data class DeletionOutcome(val path: DevicePath, val result: DeletionResult)

/**
 * Why a session ended without finishing, in terms a person can be shown.
 *
 * All but the last are the desktop's answers. [UNREACHABLE] is the phone's own: the desktop
 * never said anything, because the connection did not survive long enough for it to. §6 treats
 * that as something to rejoin rather than something to report.
 */
enum class RejectReason { NO_SPACE, COMMIT_IN_PROGRESS, PROTOCOL_VERSION, NOT_PAIRED, UNREACHABLE }

enum class UploadOutcome { VERIFIED, HASH_MISMATCH, CHANGED_ON_PHONE, WRITE_FAILED }

sealed interface Inbound {
    data class HandshakeAccepted(val desktopName: String, val commitInProgress: Boolean) : Inbound

    data object CatalogAcknowledged : Inbound

    data class Diff(val toSend: List<ToSend>, val summary: DiffSummary) : Inbound

    data class UploadAnswered(val file: FileId, val outcome: UploadOutcome) : Inbound

    data class Candidates(val candidates: List<DeletionCandidate>) : Inbound

    data class SessionFinished(
        val sent: Long,
        val skipped: Long,
        val failed: Long,
        val deleted: Long,
        val kept: Long,
        val bytesFreed: Long,
    ) : Inbound

    data class Rejected(val reason: RejectReason) : Inbound
}
