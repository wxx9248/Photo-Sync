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
        val outcome = play(session, desktop)

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
        val outcome = play(session, desktop)

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

        val outcome = play(phone(library), desktop)

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

        val outcome = play(phone(library), desktop)

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

        val outcome = play(phone(library), desktop)

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

        play(session, desktop)

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

        play(session, desktop)
        assertEquals(1, library.digests, "a photograph was hashed more than once")
    }

    @Test
    @Covers("R-PAIR-004")
    fun `the handshake says the version, the device and the name`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library)

        play(phone(library), desktop)

        val said = assertIs<Outbound.Handshake>(desktop.heard.first())
        assertEquals(Session.PROTOCOL_VERSION, said.protocolVersion)
        assertEquals(DeviceId("phone-a"), said.device)
        assertEquals("Kitchen phone", said.name)
    }

    @Test
    fun `a phone told to wait for a running commit stops rather than starting a transfer`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library, commitInProgress = true)

        val outcome = play(phone(library), desktop)

        assertEquals(Outcome.Refused(RejectReason.COMMIT_IN_PROGRESS), outcome)
        assertTrue(desktop.heard.none { it is Outbound.OpenUpload })
        assertTrue(library.contains("DCIM/Camera/IMG_0001.jpg"))
    }

    @Test
    fun `a session the desktop turns away leaves every photograph alone`() {
        val library = oneLibrary()
        val desktop = FakeDesktop(library, reject = RejectReason.NO_SPACE)

        val outcome = play(phone(library), desktop)

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

        val outcome = play(phone(library), desktop)

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
