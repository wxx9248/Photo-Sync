package top.wxx9248.photosync.session

/**
 * A desktop, played well enough to put the phone through §6 from end to end.
 *
 * This is not a second implementation of the desktop and is not trying to be: the real one is
 * in Rust and is checked against its own model. What this is for is the other half of the
 * conversation, so that the phone's state machine can be driven through every branch of §6 in
 * a plain JVM test — including the ones a real desktop only reaches after a power loss.
 */
class FakeDesktop(
    private val library: FakeLibrary,
    /** Files the desktop already holds part of, and how much of each. §7.6. */
    private val partials: Map<String, Long> = emptyMap(),
    /** Files it already has whole, which the diff will not ask for. §6.2. */
    private val imported: Set<String> = emptySet(),
    private val commitInProgress: Boolean = false,
    private val reject: RejectReason? = null,
    /** Files the desktop will refuse, and how. */
    private val refuse: Map<String, UploadOutcome> = emptyMap(),
    /** Whether a photograph offered for deletion was committed by this very session. */
    private val earlier: Set<String> = emptySet(),
) {
    /** Everything the phone actually sent, in order. */
    val heard = mutableListOf<Outbound>()

    private val received = linkedMapOf<FileId, MutableList<Byte>>()
    private val paths = linkedMapOf<FileId, DevicePath>()
    private var nextFile = 1L
    private var sent = 0L
    private var skipped = 0L
    private var deleted = 0L
    private var kept = 0L

    /** What the desktop holds whole, once a stream has been verified. */
    val stored = linkedMapOf<DevicePath, ByteArray>()

    fun answer(said: Outbound): Inbound? {
        heard.add(said)
        reject?.let { return Inbound.Rejected(it) }

        return when (said) {
            is Outbound.Handshake -> Inbound.HandshakeAccepted("Kitchen iMac", commitInProgress)
            is Outbound.SubmitCatalog -> {
                Inbound.CatalogAcknowledged.also { plan(said.catalog) }
            }
            Outbound.RequestDiff -> Inbound.Diff(planned, summaryOf(said))
            is Outbound.OpenUpload -> {
                paths[said.file] = said.path
                received[said.file] = received[said.file] ?: mutableListOf()
                null
            }
            is Outbound.SendChunk -> {
                received.getValue(said.file).addAll(said.data.toList())
                null
            }
            is Outbound.CloseUpload -> close(said)
            Outbound.Finish -> Inbound.Candidates(nominate())
            is Outbound.ReportDeletions -> {
                said.outcomes.forEach {
                    when (it.result) {
                        DeletionResult.DELETED -> deleted += 1
                        else -> kept += 1
                    }
                }
                Inbound.SessionFinished(sent, skipped, 0, deleted, kept, 0)
            }
        }
    }

    private var planned: List<ToSend> = emptyList()
    private var catalogued: Catalog = Catalog(emptyList())

    private fun plan(catalog: Catalog) {
        catalogued = catalog
        planned = catalog.entries
            .filterNot { imported.contains(it.path.value) }
            .map { entry ->
                val file = FileId(nextFile++)
                val already = partials[entry.path.value] ?: 0
                if (already > 0) {
                    // The prefix the desktop is resuming from, as it would have on disk.
                    received[file] =
                        library.read(entry.path, 0, already.toInt()).toMutableList()
                }
                ToSend(file, entry.path, already)
            }
    }

    private fun summaryOf(said: Outbound): DiffSummary = DiffSummary(
        toSendCount = planned.size.toLong(),
        toSendBytes = planned.sumOf { one -> catalogued[one.path]?.size ?: 0 },
        alreadyImported = imported.size.toLong(),
        alreadyStaged = 0,
    ).also { require(said == Outbound.RequestDiff) }

    private fun close(said: Outbound.CloseUpload): Inbound {
        val path = paths.getValue(said.file)
        refuse[path.value]?.let {
            skipped += 1
            return Inbound.UploadAnswered(said.file, it)
        }

        val bytes = received.getValue(said.file).toByteArray()
        // The digest covers the whole photograph, prefix included, so a resumed transfer is
        // verified exactly as a fresh one is.
        return if (digestOf(bytes) == said.digest) {
            stored[path] = bytes
            sent += 1
            Inbound.UploadAnswered(said.file, UploadOutcome.VERIFIED)
        } else {
            skipped += 1
            Inbound.UploadAnswered(said.file, UploadOutcome.HASH_MISMATCH)
        }
    }

    /** §8: only what the vault actually holds, out of this session's own catalog. */
    private fun nominate(): List<DeletionCandidate> =
        catalogued.entries.mapNotNull { entry ->
            val held = stored[entry.path] ?: return@mapNotNull null
            DeletionCandidate(
                path = entry.path,
                size = entry.size,
                mtime = entry.mtime,
                expected = digestOf(held),
                origin = if (earlier.contains(entry.path.value)) {
                    CandidateOrigin.EARLIER
                } else {
                    CandidateOrigin.THIS_TRANSFER
                },
            )
        }
}

/** Runs a whole session between the two, and hands back how it ended. */
fun play(session: Session, desktop: FakeDesktop, library: FakeLibrary): Outcome? {
    var guard = 0
    while (session.outcome == null) {
        // §8 stops and asks before anything is deleted. These tests are about a person who
        // says yes; the one about saying no drives the session by hand.
        if (session.asking != null) {
            session.freeUp()
            continue
        }
        // What a phone does with the list §8 cleared: remove those photographs, and say which
        // actually went.
        val freeing = session.freeing
        if (freeing != null) {
            session.freed(library.remove(freeing))
            continue
        }
        val said = session.next() ?: break
        desktop.answer(said)?.let { session.receive(it) }
        guard += 1
        check(guard < 100_000) { "the session never finished" }
    }
    return session.outcome
}
