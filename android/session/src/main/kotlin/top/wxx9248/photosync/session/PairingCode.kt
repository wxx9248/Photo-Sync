package top.wxx9248.photosync.session

import java.security.MessageDigest

/**
 * The six digits shown on both screens when a phone and a desktop first meet. `STACK.md` §4.6.
 *
 * The code is a digest over *both* public keys, which is what makes it worth reading aloud: an
 * attacker standing in the middle necessarily holds a different key on each side, so the two
 * screens show different numbers and a person sees it. A code over one key would be the same
 * on both screens whether or not anybody was in between.
 *
 * The derivation is written out in `STACK.md` because two implementations have to arrive at
 * the same answer, and `verification/vectors/pairing.tsv` is where they are made to prove it.
 * The label keeps this digest apart from every other one the project takes, and the lengths
 * sit inside it so no pair of keys can be rearranged into another pair with the same code.
 */
object PairingCode {
    private const val DOMAIN = "photo-sync pairing v1"
    private const val DIGITS = 6

    /**
     * The code for a desktop key and a phone key, in that order.
     *
     * Both are `SubjectPublicKeyInfo` exactly as the certificate encodes it, so neither end
     * has to agree about anything but which of them is the desktop.
     */
    fun of(desktopKey: ByteArray, phoneKey: ByteArray): String {
        val sha = MessageDigest.getInstance("SHA-256")
        sha.update(DOMAIN.toByteArray(Charsets.US_ASCII))
        for (key in listOf(desktopKey, phoneKey)) {
            sha.update(bigEndian(key.size))
            sha.update(key)
        }

        val digest = sha.digest()
        val leading = (digest[0].toLong() and 0xff shl 24) or
            (digest[1].toLong() and 0xff shl 16) or
            (digest[2].toLong() and 0xff shl 8) or
            (digest[3].toLong() and 0xff)
        var modulus = 1L
        repeat(DIGITS) { modulus *= 10 }
        return (leading % modulus).toString().padStart(DIGITS, '0')
    }

    private fun bigEndian(value: Int): ByteArray = byteArrayOf(
        (value ushr 24).toByte(),
        (value ushr 16).toByte(),
        (value ushr 8).toByte(),
        value.toByte(),
    )
}
