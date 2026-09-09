package top.wxx9248.photosync.pairing

import java.io.File
import kotlin.test.AfterTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import top.wxx9248.photosync.session.PairedDesktop
import top.wxx9248.photosync.session.verification.Covers

/**
 * The one thing the phone keeps between sessions.
 *
 * `SPEC.md` §3.5 keeps nothing else: no progress, no catalog, nothing that could disagree with
 * the desktop later. That makes this file the whole of the phone's memory, and a phone that
 * cannot read back what it wrote asks for the pairing code again on every start.
 */
class FilePairingStorageTest {
    private val directory: File = File.createTempFile("photo-sync-pairing", "").let { made ->
        made.delete()
        made.mkdirs()
        made
    }

    private val storage = FilePairingStorage(File(directory, "paired"))

    @AfterTest
    fun clearUp() {
        directory.deleteRecursively()
    }

    @Test
    @Covers("R-PAIR-002")
    fun `a desktop written down is the desktop read back`() {
        val desktop = PairedDesktop(name = "Kitchen iMac", publicKey = "ab".repeat(32))

        storage.write(desktop)

        assertEquals(desktop, storage.read())
    }

    @Test
    fun `a phone that has never paired holds nothing`() {
        assertNull(storage.read())
    }

    @Test
    @Covers("R-PAIR-002")
    fun `pairing again replaces the desktop rather than adding one`() {
        storage.write(PairedDesktop(name = "Kitchen iMac", publicKey = "ab".repeat(32)))
        val second = PairedDesktop(name = "Study desktop", publicKey = "cd".repeat(32))

        storage.write(second)

        assertEquals(second, storage.read())
    }

    @Test
    fun `a half-written record is treated as no record`() {
        File(directory, "paired").writeText("Kitchen iMac\n")

        // A name with no key cannot be pinned against, and trusting the next desktop to answer
        // is exactly what §5.2 rules out.
        assertNull(storage.read())
    }

    @Test
    fun `unpairing leaves nothing behind`() {
        storage.write(PairedDesktop(name = "Kitchen iMac", publicKey = "ab".repeat(32)))

        storage.clear()

        assertNull(storage.read())
    }
}
