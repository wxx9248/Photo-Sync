package top.wxx9248.photosync.session

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNull
import top.wxx9248.photosync.session.verification.Covers

private class InMemoryStorage : PairingStorage {
    private var held: PairedDesktop? = null
    override fun read(): PairedDesktop? = held
    override fun write(desktop: PairedDesktop) { held = desktop }
    override fun clear() { held = null }
}

class PairingTest {
    private val kitchen = PairedDesktop("Kitchen iMac", "key-one")
    private val other = PairedDesktop("Somebody else", "key-two")

    @Test
    @Covers("R-PAIR-002")
    fun `a phone remembers one desktop and pairing with another does not add a second`() {
        val pairing = Pairing(InMemoryStorage())
        assertEquals(PairingResult.Paired, pairing.remember(kitchen))
        assertEquals(kitchen, pairing.known())

        // Not joined, not silently swapped: a person is asked.
        val answer = pairing.remember(other)
        assertIs<PairingResult.KeyChanged>(answer)
        assertEquals(kitchen, answer.known)
        assertEquals(kitchen, pairing.known(), "a second desktop replaced the first quietly")
    }

    @Test
    @Covers("R-PAIR-003")
    fun `a desktop whose key changed is never trusted without being replaced on purpose`() {
        val pairing = Pairing(InMemoryStorage())
        pairing.remember(kitchen)

        val reinstalled = PairedDesktop("Kitchen iMac", "key-after-reinstall")
        assertIs<PairingResult.KeyChanged>(pairing.remember(reinstalled))

        // Only an explicit replacement, which is what a person confirming the re-pair does.
        pairing.replace(reinstalled)
        assertEquals(reinstalled, pairing.known())
    }

    @Test
    fun `pairing with the desktop already known changes nothing`() {
        val pairing = Pairing(InMemoryStorage())
        pairing.remember(kitchen)
        assertEquals(PairingResult.AlreadyPaired, pairing.remember(kitchen))
        assertEquals(kitchen, pairing.known())
    }

    @Test
    fun `forgetting leaves the phone ready to pair from nothing`() {
        val pairing = Pairing(InMemoryStorage())
        pairing.remember(kitchen)
        pairing.forget()
        assertNull(pairing.known())
        assertEquals(PairingResult.Paired, pairing.remember(other))
    }
}
