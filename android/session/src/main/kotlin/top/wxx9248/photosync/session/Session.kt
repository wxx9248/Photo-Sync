package top.wxx9248.photosync.session

/**
 * The phone's half of `SPEC.md` §6, as a state machine with nothing underneath it.
 *
 * The phone drives every exchange and the desktop only answers, so this is written the way
 * that reads: ask it what to say next, give it what came back, ask again. It owns no socket,
 * no coroutine and no Android type, which is what lets the whole protocol be played out in a
 * plain JVM test against a fake desktop — the fast half of the harness `docs/VERIFICATION.md`
 * describes.
 *
 * A session that is interrupted is not resumed from here. §6 rejoins by running this again
 * from the handshake with the *same* frozen catalog, and the desktop's re-diff is what makes
 * that cheap: it can only shrink the work.
 */
class Session(
    private val device: DeviceId,
    private val name: String,
    private val catalog: Catalog,
    private val library: Library,
    private val protocolVersion: Int = PROTOCOL_VERSION,
) {
    /** How much of a file travels in one message. `STACK.md` §5.3. */
    private val chunkBytes = 512 * 1024

    private var phase: Phase = Phase.Handshaking
    private var wanted: List<ToSend> = emptyList()
    private var sending: Int = 0
    private var offset: Long = 0

    /** What became of the session, once there is an answer. */
    var outcome: Outcome? = null
        private set

    /**
     * What to say next, or null while the desktop owes an answer or there is nothing left.
     */
    fun next(): Outbound? = when (val here = phase) {
        Phase.Handshaking -> Outbound.Handshake(device, name, protocolVersion)
        Phase.Cataloguing -> Outbound.SubmitCatalog(catalog)
        Phase.Diffing -> Outbound.RequestDiff
        is Phase.Sending -> sending(here)
        Phase.Finishing -> Outbound.Finish
        is Phase.Deleting -> Outbound.ReportDeletions(here.outcomes)
        // Nothing to say while a person is being asked. §8 stops here until they answer.
        is Phase.Asking -> null
        Phase.Waiting, Phase.Done -> null
    }

    /**
     * What a person is being asked, or null when nothing is waiting on them.
     *
     * §8 allows exactly one prompt, and this is what it says: how much can be freed, split
     * between what crossed just now and what an earlier session had already stored.
     */
    val asking: FreeUp?
        get() = (phase as? Phase.Asking)?.let { here ->
            FreeUp(
                fromThisTransfer = here.candidates.count {
                    it.origin == CandidateOrigin.THIS_TRANSFER
                },
                fromEarlier = here.candidates.count { it.origin == CandidateOrigin.EARLIER },
                bytes = here.candidates.sumOf { it.size },
            )
        }

    /**
     * The person said yes, so §8's gates decide the rest.
     *
     * Saying yes is permission to check, not permission to delete: every candidate still has
     * to prove itself against the copy the desktop claims to hold, and anything that cannot is
     * kept and reported.
     */
    fun freeUp() {
        val here = phase as? Phase.Asking ?: return
        phase = Phase.Deleting(carryOut(here.candidates))
    }

    /**
     * The person said no, so every photograph stays and the desktop is told why.
     *
     * The desktop is told rather than left to infer it from silence: §8 has the phone report
     * per-file results, and "the person declined" is a result like any other.
     */
    fun keepThem() {
        val here = phase as? Phase.Asking ?: return
        phase = Phase.Deleting(
            here.candidates.map { DeletionOutcome(it.path, DeletionResult.KEPT_USER) }
        )
    }

    /** Takes what the desktop said, and moves on. */
    fun receive(inbound: Inbound) {
        when (inbound) {
            is Inbound.Rejected -> {
                outcome = Outcome.Refused(inbound.reason)
                phase = Phase.Done
            }
            is Inbound.HandshakeAccepted -> {
                if (inbound.commitInProgress) {
                    // §7.3 holds a phone whose previous batch is still being filed. Waiting is
                    // short, and joining a running batch is not on offer.
                    outcome = Outcome.Refused(RejectReason.COMMIT_IN_PROGRESS)
                    phase = Phase.Done
                } else {
                    phase = Phase.Cataloguing
                }
            }
            Inbound.CatalogAcknowledged -> phase = Phase.Diffing
            is Inbound.Diff -> {
                wanted = inbound.toSend
                sending = 0
                phase = if (wanted.isEmpty()) Phase.Finishing else openNext()
            }
            is Inbound.UploadAnswered -> {
                sending += 1
                phase = if (sending < wanted.size) openNext() else Phase.Finishing
            }
            // Nothing is deleted here. §8 puts one prompt between the desktop's list and the
            // phone acting on it, and `asking` is what that prompt is built from.
            is Inbound.Candidates -> phase = Phase.Asking(inbound.candidates)
            is Inbound.SessionFinished -> {
                outcome = Outcome.Finished(
                    sent = inbound.sent,
                    skipped = inbound.skipped,
                    failed = inbound.failed,
                    deleted = inbound.deleted,
                    kept = inbound.kept,
                    bytesFreed = inbound.bytesFreed,
                )
                phase = Phase.Done
            }
        }
    }

    /** The messages of one file, in order, as far as the desktop has let it get. */
    private fun sending(here: Phase.Sending): Outbound {
        val one = wanted[sending]
        val whole = catalog[one.path]
        val held = library.describe(one.path)

        // What the header states is what the phone holds now, not what the catalog froze:
        // §6.4 is written so the desktop can notice the difference and skip the file.
        if (here.opened.not()) {
            phase = Phase.Sending(opened = true)
            offset = one.resumeOffset
            return Outbound.OpenUpload(
                file = one.file,
                path = one.path,
                size = held?.size ?: whole?.size ?: 0,
                mtime = held?.mtime ?: whole?.mtime ?: Timestamp(0),
                offset = one.resumeOffset,
            )
        }

        val size = held?.size ?: 0
        if (offset >= size) {
            phase = Phase.Waiting
            return Outbound.CloseUpload(one.file, library.digest(one.path))
        }

        val take = minOf(chunkBytes.toLong(), size - offset).toInt()
        val data = library.read(one.path, offset, take)
        val at = offset
        offset += data.size
        return Outbound.SendChunk(one.file, at, data)
    }

    private fun openNext(): Phase = Phase.Sending(opened = false)

    /** Runs §8's own checks and then does what they allow, one photograph at a time. */
    private fun carryOut(candidates: List<DeletionCandidate>): List<DeletionOutcome> =
        candidates.map { candidate ->
            val verdict = decide(candidate, library)
            val result = if (verdict == DeletionResult.DELETED) {
                if (library.delete(candidate.path)) {
                    DeletionResult.DELETED
                } else {
                    DeletionResult.FAILED
                }
            } else {
                verdict
            }
            DeletionOutcome(candidate.path, result)
        }

    private sealed interface Phase {
        data object Handshaking : Phase
        data object Cataloguing : Phase
        data object Diffing : Phase
        data class Sending(val opened: Boolean) : Phase
        data object Waiting : Phase
        data class Asking(val candidates: List<DeletionCandidate>) : Phase
        data object Finishing : Phase
        data class Deleting(val outcomes: List<DeletionOutcome>) : Phase
        data object Done : Phase
    }

    companion object {
        /** The version this phone speaks, stated in the handshake. `STACK.md` §5.4. */
        const val PROTOCOL_VERSION: Int = 1
    }
}

/**
 * The one question §8 allows the phone to ask.
 *
 * The split is what makes the number believable: "from this transfer" is what crossed minutes
 * ago, "from earlier" is what an older session stored and the phone is about to re-prove.
 */
data class FreeUp(
    val fromThisTransfer: Int,
    val fromEarlier: Int,
    val bytes: Long,
)

/** How a session ended. */
sealed interface Outcome {
    data class Finished(
        val sent: Long,
        val skipped: Long,
        val failed: Long,
        val deleted: Long,
        val kept: Long,
        val bytesFreed: Long,
    ) : Outcome

    data class Refused(val reason: RejectReason) : Outcome
}
