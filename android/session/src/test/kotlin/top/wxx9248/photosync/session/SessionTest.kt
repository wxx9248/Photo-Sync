package top.wxx9248.photosync.session

import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue
import top.wxx9248.photosync.session.verification.Covers

private const val MTIME = 1_756_000_000L
private val PHOTO = "the bytes of one photograph".toByteArray()

private fun phone(library: FakeLibrary) =
    Session(DeviceId("phone-a"), "Kitchen phone", library.catalog(), library)

private fun oneLibrary(): FakeLibrary =
    FakeLibrary(mapOf("DCIM/Camera/IMG_0001.jpg" to (MTIME to PHOTO)))

/**
 * What the session says next, when a test has already established there is something to say.
 *
 * A session with nothing left to say is a failure of the step before, and saying so here
 * names it. Rule 9.5 holds tests to K5: `!!` reports it as a null pointer instead.
 */
private fun saying(session: Session): Outbound =
    assertNotNull(session.next(), "the session had nothing to say")

/**
 * Drives a session as far as §8's prompt and no further.
 *
 * Stopping there is the point: what these tests are about is what the phone has and has not
 * done at the moment a person is asked.
 */
private fun playUntilAsked(session: Session, desktop: FakeDesktop): FreeUp {
    var guard = 0
    while (session.asking == null && session.outcome == null) {
        val said = session.next() ?: break
        desktop.answer(said)?.let { session.receive(it) }
        guard += 1
        check(guard < 100_000) { "the session never reached the prompt" }
    }
    return assertNotNull(session.asking, "the session never asked before deleting")
}

/** What the phone told the desktop about the photographs it was offered. */
private fun reported(desktop: FakeDesktop): List<DeletionOutcome> =
    desktop.heard.filterIsInstance<Outbound.ReportDeletions>().flatMap { it.outcomes }

class SessionTest {
    @Test
    @Covers("R-DELETE-005")
    fun `nothing is deleted until somebody has been asked`() {
        val library = oneLibrary()
        val session = phone(library)

        val asked = playUntilAsked(session, FakeDesktop(library))

        assertTrue(
            library.contains("DCIM/Camera/IMG_0001.jpg"),
            "a photograph went before anybody was asked",
        )
        assertEquals(1, asked.fromThisTransfer)
        assertEquals(0, asked.fromEarlier)
        assertEquals(PHOTO.size.toLong(), asked.bytes)
        assertNull(session.next(), "the session carried on with a person still deciding")
    }

    @Test
    @Covers("R-DELETE-005")
    fun `a person who says no keeps every photograph`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library)
        val session = phone(library)
        playUntilAsked(session, desktop)

        session.keepThem()
        val outcome = play(session, desktop, library)

        val finished = assertIs<Outcome.Finished>(outcome)
        assertEquals(0, finished.deleted)
        assertEquals(1, finished.kept)
        assertTrue(
            library.contains("DCIM/Camera/IMG_0001.jpg"),
            "the phone deleted a photograph it had been told to keep",
        )
        // Told, not left to be inferred from silence: §8 reports a result per file.
        assertEquals(
            listOf(DeletionResult.KEPT_USER),
            reported(desktop).map { it.result },
        )
    }

    @Test
    fun `a person who turns the system down has kept the photograph, not failed to delete it`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library)
        val session = phone(library)
        playUntilAsked(session, desktop)

        session.freeUp()
        val cleared = assertNotNull(session.freeing)

        // The platform put the request to them and they said no. The wire calls that
        // KEPT_USER, and a household reading "failed" would think something went wrong.
        session.freed(gone = emptySet(), declined = cleared.toSet())
        play(session, desktop, library)

        assertEquals(listOf(DeletionResult.KEPT_USER), reported(desktop).map { it.result })
        assertEquals(1, assertIs<Outcome.Finished>(session.outcome).kept)
    }

    // No `@Covers`: R-DELETE-009 is about the mechanism --- `createDeleteRequest`, silent when
    // media management is granted --- and that is the platform's behaviour rather than this
    // module's. What is checked here is what the session does with the answer.
    @Test
    fun `a photograph the platform would not remove is reported as failed`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library)
        val session = phone(library)
        playUntilAsked(session, desktop)

        session.freeUp()
        val cleared = assertNotNull(session.freeing, "§8's gates cleared nothing to remove")
        assertEquals(listOf(DevicePath("DCIM/Camera/IMG_0001.jpg")), cleared)

        // The person agreed and the gates agreed; the platform refused. That is a failure
        // rather than a photograph somebody chose to keep, and the desktop is told which.
        session.freed(emptySet())
        play(session, desktop, library)

        assertEquals(listOf(DeletionResult.FAILED), reported(desktop).map { it.result })
        assertTrue(library.contains("DCIM/Camera/IMG_0001.jpg"))
    }

    @Test
    @Covers("R-DELETE-005")
    fun `saying yes is permission to check rather than permission to delete`() {
        // The photograph changed after the desktop nominated it, so §8's gate keeps it even
        // though a person agreed to free up space.
        val library = oneLibrary()
        val desktop = FakeDesktop(library)
        val session = phone(library)
        playUntilAsked(session, desktop)
        library.put("DCIM/Camera/IMG_0001.jpg", MTIME + 1, "something else entirely".toByteArray())

        session.freeUp()
        val outcome = play(session, desktop, library)

        assertEquals(0, assertIs<Outcome.Finished>(outcome).deleted)
        assertEquals(
            listOf(DeletionResult.KEPT_CHANGED),
            reported(desktop).map { it.result },
        )
    }

    @Test
    fun `one photograph crosses and the phone lets go of it`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library)

        val outcome = play(phone(library), desktop, library)

        val finished = assertIs<Outcome.Finished>(outcome)
        assertEquals(1, finished.sent)
        assertEquals(1, finished.deleted)
        assertContentEquals(PHOTO, desktop.stored.values.single())
        assertTrue(library.paths.isEmpty(), "the phone kept a photograph it was told was safe")
    }

    @Test
    @Covers("R-SESSION-002")
    fun `a phone with nothing to send finishes without being asked twice`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library, imported = setOf("DCIM/Camera/IMG_0001.jpg"))

        val outcome = play(phone(library), desktop, library)

        assertIs<Outcome.Finished>(outcome)
        // Straight from the diff to the finish signal: no upload, and nothing waiting on a
        // person to press anything.
        assertTrue(desktop.heard.none { it is Outbound.OpenUpload })
        assertTrue(desktop.heard.any { it == Outbound.Finish })
    }

    @Test
    @Covers("R-SESSION-006", "R-STAGE-008")
    fun `a resumed transfer starts where the desktop says and hashes the whole photograph`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library, partials = mapOf("DCIM/Camera/IMG_0001.jpg" to 10L))

        val outcome = play(phone(library), desktop, library)

        assertIs<Outcome.Finished>(outcome)
        val opened = desktop.heard.filterIsInstance<Outbound.OpenUpload>().single()
        assertEquals(10, opened.offset, "the phone chose its own offset")

        val chunks = desktop.heard.filterIsInstance<Outbound.SendChunk>()
        assertEquals(10, chunks.first().offset)
        assertContentEquals(PHOTO.copyOfRange(10, PHOTO.size), chunks.first().data)

        // The digest covers everything, including the ten bytes it did not send. A resumed
        // transfer proves exactly as much as a fresh one.
        val closed = desktop.heard.filterIsInstance<Outbound.CloseUpload>().single()
        assertEquals(digestOf(PHOTO), closed.digest)
        assertContentEquals(PHOTO, desktop.stored.values.single())
    }

    @Test
    @Covers("R-CATALOG-004")
    fun `the catalog is what it was at the start, whatever the camera does next`() {
        val library = oneLibrary()
        val session = phone(library)
        val desktop = FakeDesktop(library)

        // The camera takes another photograph while the session is running. §3.2 froze the
        // list at the start, so this one belongs to the next session.
        library.put("DCIM/Camera/IMG_0002.jpg", MTIME + 60, "a later photograph".toByteArray())

        play(session, desktop, library)

        val offered = desktop.heard.filterIsInstance<Outbound.SubmitCatalog>().single()
        assertEquals(1, offered.catalog.entries.size)
        assertEquals(setOf(DevicePath("DCIM/Camera/IMG_0001.jpg")), desktop.stored.keys)
    }

    @Test
    @Covers("R-CATALOG-005")
    fun `nothing is hashed before the desktop asks for the file`() {
        val library = CountingLibrary(oneLibrary())
        val session = Session(DeviceId("phone-a"), "Kitchen phone", library.catalog(), library)
        val desktop = FakeDesktop(library.underlying)

        // Up to and including the diff, §3.2 promises no hashing: a library of fifty thousand
        // photographs would otherwise be read end to end before anything moved.
        session.receive(Inbound.HandshakeAccepted("Kitchen iMac", false))
        desktop.answer(saying(session))
        session.receive(Inbound.CatalogAcknowledged)
        desktop.answer(saying(session))
        assertEquals(0, library.digests, "the phone hashed something before it was asked to")

        play(session, desktop, library.underlying)
        assertEquals(1, library.digests, "a photograph was hashed more than once")
    }

    @Test
    @Covers("R-PAIR-004")
    fun `the handshake says the version, the device and the name`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library)

        play(phone(library), desktop, library)

        val said = assertIs<Outbound.Handshake>(desktop.heard.first())
        assertEquals(Session.PROTOCOL_VERSION, said.protocolVersion)
        assertEquals(DeviceId("phone-a"), said.device)
        assertEquals("Kitchen phone", said.name)
    }

    @Test
    fun `a phone told to wait for a running commit stops rather than starting a transfer`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library, commitInProgress = true)

        val outcome = play(phone(library), desktop, library)

        assertEquals(Outcome.Refused(RejectReason.COMMIT_IN_PROGRESS), outcome)
        assertTrue(desktop.heard.none { it is Outbound.OpenUpload })
        assertTrue(library.contains("DCIM/Camera/IMG_0001.jpg"))
    }

    @Test
    fun `a session the desktop turns away leaves every photograph alone`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library, reject = RejectReason.NO_SPACE)

        val outcome = play(phone(library), desktop, library)

        assertEquals(Outcome.Refused(RejectReason.NO_SPACE), outcome)
        assertTrue(library.contains("DCIM/Camera/IMG_0001.jpg"))
    }

    @Test
    fun `a photograph the desktop could not store is not offered for deletion`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(
            library,
            refuse = mapOf("DCIM/Camera/IMG_0001.jpg" to UploadOutcome.WRITE_FAILED),
        )

        val outcome = play(phone(library), desktop, library)

        val finished = assertIs<Outcome.Finished>(outcome)
        assertEquals(0, finished.deleted)
        assertTrue(library.contains("DCIM/Camera/IMG_0001.jpg"))
    }
}

/** Counts what the session asks of storage, so a promise about *not* reading can be checked. */
private class CountingLibrary(val underlying: FakeLibrary) : Library by underlying {
    var digests = 0
        private set

    fun catalog(): Catalog = underlying.catalog()

    override fun digest(path: DevicePath): Sha256 {
        digests += 1
        return underlying.digest(path)
    }
}

/**
 * Several photographs on their way at once. `SPEC.md` §6.4.
 *
 * These are the tests that need a desktop which does not answer straight away. A real one
 * answers when it has written, fsynced and verified the file, which is long after the last
 * chunk arrived --- and it is exactly that gap several streams are for. A fake that answers
 * the moment a stream closes can never have two files in flight, so it would show one at a
 * time whatever the state machine did.
 */
class ConcurrentTransferTest {
    /**
     * A session that opens four streams, said here rather than read from the constant.
     *
     * A test that asserts `Session.LANES` files are in flight passes whatever that number is,
     * including one --- which is the behaviour these tests exist to rule out. The number the
     * application ships with is checked once, on its own, below.
     */
    private fun phone(library: FakeLibrary, lanes: Int = 4) =
        Session(DeviceId("phone-a"), "Kitchen phone", library.catalog(), library, lanes = lanes)

    private fun library(count: Int): FakeLibrary = FakeLibrary(
        (1..count).associate { at ->
            "DCIM/Camera/IMG_%04d.jpg".format(at) to (MTIME + at to "photograph $at".toByteArray())
        }
    )

    /** Plays until the session has nothing left to say without an answer coming back. */
    private fun untilItWaits(session: Session, desktop: FakeDesktop) {
        var guard = 0
        while (true) {
            val said = session.next() ?: return
            desktop.answer(said)?.let { session.receive(it) }
            guard += 1
            check(guard < 100_000) { "the session never stopped to wait" }
        }
    }

    @Test
    @Covers("R-XFER-006")
    fun `four photographs are in flight before any of them is answered`() {
        val library = library(10)
        val desktop = FakeDesktop(library, defers = true)
        val session = phone(library)

        untilItWaits(session, desktop)

        assertEquals(
            4,
            desktop.waiting.size,
            "the phone had ${desktop.waiting.size} files in flight, not four",
        )
    }

    @Test
    @Covers("R-XFER-006")
    fun `a photograph answered frees its lane for the next one`() {
        val library = library(10)
        val desktop = FakeDesktop(library, defers = true)
        val session = phone(library)

        untilItWaits(session, desktop)
        val first = desktop.waiting.first()
        session.receive(desktop.settle(first))
        untilItWaits(session, desktop)

        assertEquals(4, desktop.waiting.size, "the freed lane was not filled")
        assertTrue(first !in desktop.waiting, "the answered file was opened again")
    }

    @Test
    @Covers("R-XFER-006")
    fun `answers that come back out of order are matched to their own files`() {
        val library = library(4)
        val desktop = FakeDesktop(library, defers = true)
        val session = phone(library)

        untilItWaits(session, desktop)
        // Last one first, which is what a desktop does when a small file lands behind a large
        // one: whichever it finishes verifying first is the one it answers for.
        desktop.waiting.reversed().forEach { file -> session.receive(desktop.settle(file)) }

        assertEquals(4, session.progress.sent)
        assertEquals(Outbound.Finish, session.next(), "the session did not move on to §6.6")
    }

    @Test
    @Covers("R-XFER-006", "R-XFER-001")
    fun `interleaved streams each arrive as the photograph they were`() {
        val library = library(6)
        val desktop = FakeDesktop(library, defers = true)
        val session = phone(library)

        // Every file is opened, fed and closed with the others' chunks between its own.
        untilItWaits(session, desktop)
        while (desktop.waiting.isNotEmpty()) {
            session.receive(desktop.settle(desktop.waiting.first()))
            untilItWaits(session, desktop)
        }

        assertEquals(6, desktop.stored.size, "the desktop holds ${desktop.stored.keys}")
        desktop.stored.forEach { (path, bytes) ->
            assertContentEquals(
                library.read(path, 0, bytes.size),
                bytes,
                "$path arrived as somebody else's photograph",
            )
        }
    }

    @Test
    @Covers("R-XFER-006")
    fun `progress counts the photographs the desktop has taken, not the ones sent at`() {
        val library = library(8)
        val desktop = FakeDesktop(library, defers = true)
        val session = phone(library)

        untilItWaits(session, desktop)

        assertEquals(0, session.progress.sent, "progress counted files nobody has taken yet")
        assertEquals(8, session.progress.total)

        session.receive(desktop.settle(desktop.waiting.first()))
        assertEquals(1, session.progress.sent)
    }

    @Test
    @Covers("R-XFER-006")
    fun `one lane is the old one-at-a-time order`() {
        val library = library(5)
        val desktop = FakeDesktop(library, defers = true)

        untilItWaits(phone(library, lanes = 1), desktop)

        assertEquals(1, desktop.waiting.size, "a session given one lane opened more than one")
    }

    /**
     * `STACK.md` §4.5 fixes the number at four, and `docs/VERIFICATION.md` §5 asks for the
     * transport's fixed numbers to be asserted rather than assumed. The driver opens one
     * connection per lane, so this number and that pool are the same decision.
     */
    @Test
    @Covers("R-XFER-006")
    fun `the phone ships with four streams`() {
        assertEquals(4, Session.LANES)
    }
}
