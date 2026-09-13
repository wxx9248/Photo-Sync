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
    /**
     * How many photographs may be in flight at once. §6.4.
     *
     * `STACK.md` §4.5 gives the driver four connections, one upload each, and this is the
     * state machine's half of that number: it decides how many files it will have open, and
     * the driver decides what carries them. A test passes 1 to watch the old behaviour, or 2
     * to watch two streams interleave without needing four.
     */
    private val lanes: Int = LANES,
) {
    /** How much of a file travels in one message. `STACK.md` §5.3. */
    private val chunkBytes = 512 * 1024

    private var phase: Phase = Phase.Handshaking
    private var wanted: List<ToSend> = emptyList()

    /**
     * The files being sent right now, in the order they were opened.
     *
     * Keyed by the identifier the desktop gave the file, because that is what its answers are
     * keyed by: with several streams open the next answer is not necessarily about the file
     * whose bytes went out last.
     */
    private val flying = LinkedHashMap<FileId, Lane>()

    /** How many of the wanted files have had a stream opened, and how many are finished. */
    private var started = 0
    private var answered = 0

    /** Whose turn it is among the open streams, so none of them is starved. */
    private var turn = 0

    /**
     * One file on its way: where it is up to, how large it was when it opened, and whether
     * its digest has gone.
     *
     * The size is taken once, when the stream opens, and is the size the header stated. Asking
     * the library again for every chunk is a query to another process each time --- a third of
     * a session, measured on a real phone --- and it cannot answer a different question than
     * the header already asked: §6.4 has the desktop compare the header with the catalog, and
     * a photograph that changes after that is caught by its digest.
     */
    private class Lane(val at: Int, val size: Long, var offset: Long, var closed: Boolean = false)

    /**
     * How far through the photographs the desktop asked for. §3.4 shows this while it runs.
     *
     * Counted in files rather than bytes: what a person watching wants to know is whether it
     * is moving and roughly how much is left, and a count of files says both without the
     * phone having to know how large the remainder is.
     *
     * What is counted is files the desktop has answered for, not files started. With several
     * streams open, more are on their way than this says --- and a number that went up before
     * the desktop had the file would be claiming something that is not true yet.
     */
    val progress: Progress
        get() = Progress(sent = answered, total = wanted.size)

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
        Phase.Sending -> sending()
        Phase.Finishing -> Outbound.Finish
        is Phase.Deleting -> Outbound.ReportDeletions(here.outcomes)
        // Nothing to say while a person is being asked, or while the platform is being asked
        // to remove what they agreed to. §8 stops at both.
        is Phase.Asking, is Phase.Freeing -> null
        Phase.Done -> null
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
     * kept and reported. What survives is handed to the driver rather than removed here --- on
     * a phone, removing a photograph is a request the platform puts to a person.
     */
    fun freeUp() {
        val here = phase as? Phase.Asking ?: return
        val verdicts = here.candidates.map { it to decide(it, library) }
        phase = Phase.Freeing(
            approved = verdicts.filter { (_, verdict) -> verdict == DeletionResult.DELETED }
                .map { (candidate, _) -> candidate.path },
            kept = verdicts.filterNot { (_, verdict) -> verdict == DeletionResult.DELETED }
                .map { (candidate, verdict) -> DeletionOutcome(candidate.path, verdict) },
        )
    }

    /**
     * The photographs §8's gates have cleared, waiting on whoever can remove them.
     *
     * Null when nothing is waiting. An empty list is not the same thing: it means the gates
     * cleared none of them, and the driver still has to say so before the session can report.
     */
    val freeing: List<DevicePath>?
        get() = (phase as? Phase.Freeing)?.approved

    /**
     * What actually went, and what a person turned down on the way.
     *
     * The wire calls a declined request `KEPT_USER` --- "the user declined, or the platform
     * delete request was not granted" --- and that is a different thing from a delete that was
     * allowed and did not happen. A household that says no to the system's own dialog should
     * read "kept", not "failed".
     */
    fun freed(gone: Set<DevicePath>, declined: Set<DevicePath> = emptySet()) {
        val here = phase as? Phase.Freeing ?: return
        val removed = here.approved.map { path ->
            DeletionOutcome(
                path,
                when {
                    gone.contains(path) -> DeletionResult.DELETED
                    declined.contains(path) -> DeletionResult.KEPT_USER
                    else -> DeletionResult.FAILED
                },
            )
        }
        phase = Phase.Deleting(removed + here.kept)
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
                started = 0
                answered = 0
                turn = 0
                flying.clear()
                phase = if (wanted.isEmpty()) Phase.Finishing else Phase.Sending
            }
            is Inbound.UploadAnswered -> {
                // By identifier, not by position: with several streams open the answer that
                // arrives is about whichever file the desktop finished first, which is not
                // necessarily the one whose bytes went out last.
                if (flying.remove(inbound.file) != null) {
                    answered += 1
                }
                phase = if (answered >= wanted.size) Phase.Finishing else Phase.Sending
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

    /**
     * The next thing to say about whichever file it is that file's turn.
     *
     * Null while every open stream is waiting on the desktop: there is more to send, but
     * nothing to say until an answer comes back. The driver waits for one rather than
     * treating this as the end.
     */
    private fun sending(): Outbound? {
        if (flying.size < lanes && started < wanted.size) {
            return open(wanted[started])
        }

        val busy = flying.entries.filterNot { (_, lane) -> lane.closed }
        if (busy.isEmpty()) {
            return null
        }

        // Round-robin, so a large photograph cannot hold up the ones beside it: several
        // streams are only worth opening if they all make progress.
        val chosen = busy[turn % busy.size]
        turn += 1
        return piece(chosen.key, chosen.value)
    }

    /**
     * Opens a stream for one file.
     *
     * What the header states is what the phone holds now, not what the catalog froze: §6.4 is
     * written so the desktop can notice the difference and skip the file.
     */
    private fun open(one: ToSend): Outbound {
        val whole = catalog[one.path]
        val held = library.describe(one.path)
        val size = held?.size ?: whole?.size ?: 0
        flying[one.file] = Lane(at = started, size = size, offset = one.resumeOffset)
        started += 1
        return Outbound.OpenUpload(
            file = one.file,
            path = one.path,
            size = size,
            mtime = held?.mtime ?: whole?.mtime ?: Timestamp(0),
            offset = one.resumeOffset,
        )
    }

    /** The next chunk of one open stream, or the digest that ends it. */
    private fun piece(file: FileId, lane: Lane): Outbound {
        val one = wanted[lane.at]
        val size = lane.size
        if (lane.offset >= size) {
            lane.closed = true
            return Outbound.CloseUpload(file, library.digest(one.path))
        }

        val take = minOf(chunkBytes.toLong(), size - lane.offset).toInt()
        val data = library.read(one.path, lane.offset, take)
        val at = lane.offset
        lane.offset += data.size
        return Outbound.SendChunk(file, at, data)
    }

    private sealed interface Phase {
        data object Handshaking : Phase
        data object Cataloguing : Phase
        data object Diffing : Phase
        data object Sending : Phase
        data class Asking(val candidates: List<DeletionCandidate>) : Phase
        data class Freeing(
            val approved: List<DevicePath>,
            val kept: List<DeletionOutcome>,
        ) : Phase
        data object Finishing : Phase
        data class Deleting(val outcomes: List<DeletionOutcome>) : Phase
        data object Done : Phase
    }

    companion object {
        /** The version this phone speaks, stated in the handshake. `STACK.md` §5.4. */
        const val PROTOCOL_VERSION: Int = 1

        /**
         * How many photographs cross at once. `STACK.md` §4.5.
         *
         * Four, because that is how many connections the driver opens and each carries one
         * upload. A fifth stream would have to share a connection with another, which is the
         * arrangement §3.6 of that document rejects: one lost segment would stall every file
         * behind it.
         */
        const val LANES: Int = 4
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

/** How far a transfer has got. */
data class Progress(val sent: Int, val total: Int)

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
