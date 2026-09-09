package top.wxx9248.photosync.session

/**
 * The one desktop this phone is paired with. `SPEC.md` §5.2.
 *
 * One, exactly: a phone that remembered several would have to be asked which, and the whole
 * point of §5.2 is that after the first connection nobody is asked anything again. Pairing
 * with a second desktop replaces the first rather than joining it, and that is a decision a
 * person makes knowingly — which is why replacing is a different call from remembering.
 *
 * A changed key for a desktop already known is never waved through. §5.2 calls that the
 * re-pair flow: the phone refuses and says so, because a key that changed is either a
 * reinstall or somebody standing in the middle, and only a person can tell those apart.
 */
class Pairing(private val storage: PairingStorage) {
    /** The desktop this phone knows, if it knows one. */
    fun known(): PairedDesktop? = storage.read()

    /**
     * Remembers a desktop, provided that does not silently change one already known.
     */
    fun remember(desktop: PairedDesktop): PairingResult {
        val known = storage.read()
        return when {
            known == null -> {
                storage.write(desktop)
                PairingResult.Paired
            }
            known.publicKey == desktop.publicKey -> PairingResult.AlreadyPaired
            else -> PairingResult.KeyChanged(known)
        }
    }

    /**
     * Replaces whatever was known, which is what a person confirming a re-pair has decided.
     */
    fun replace(desktop: PairedDesktop) {
        storage.write(desktop)
    }

    /** Forgets the desktop entirely, so the next pairing starts from nothing. */
    fun forget() {
        storage.clear()
    }
}

/** A desktop, as the phone remembers it between sessions. */
data class PairedDesktop(
    val name: String,
    /** The subject public key the phone pins. Nothing else about the certificate matters. */
    val publicKey: String,
)

sealed interface PairingResult {
    data object Paired : PairingResult
    data object AlreadyPaired : PairingResult

    /** The desktop answering is not the one remembered. A person has to decide. */
    data class KeyChanged(val known: PairedDesktop) : PairingResult
}

/** Where the pairing is kept. The Android app writes it to disk; a test keeps it in memory. */
interface PairingStorage {
    fun read(): PairedDesktop?
    fun write(desktop: PairedDesktop)
    fun clear()
}
