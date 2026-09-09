package top.wxx9248.photosync.net

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.math.BigInteger
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.cert.X509Certificate
import java.util.Date
import javax.security.auth.x500.X500Principal

/**
 * This phone's own key, and the certificate it presents.
 *
 * `STACK.md` §3.6 keeps the private key in the Android Keystore, where nothing — including
 * this application — can read it out. The keystore generates the self-signed certificate as
 * part of creating the key, so no certificate builder is needed and no library has to be
 * trusted with the key material.
 *
 * The certificate's fields do not matter. Neither end validates a name, an issuer or a date:
 * §5.2 pins the subject public key and nothing else, which is what makes a certificate that
 * expired or names the wrong host still exactly as trustworthy as the day it was made.
 */
class Identity private constructor(
    private val entry: KeyStore.PrivateKeyEntry,
) {
    val certificate: X509Certificate = entry.certificate as X509Certificate

    val privateKey: PrivateKey = entry.privateKey

    /** The pin the desktop stores for this phone: the subject public key, as hexadecimal. */
    fun publicKeyPin(): String = pinOf(certificate)

    companion object {
        private const val KEYSTORE = "AndroidKeyStore"

        /** The one name this phone's key is stored and asked for under. */
        const val ALIAS: String = "photo-sync-device"

        /** Loads this phone's key, making one the first time. */
        fun loadOrCreate(): Identity {
            val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
            (store.getEntry(ALIAS, null) as? KeyStore.PrivateKeyEntry)?.let {
                return Identity(it)
            }

            val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, KEYSTORE)
            generator.initialize(
                KeyGenParameterSpec.Builder(
                    ALIAS,
                    KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY,
                )
                    .setDigests(KeyProperties.DIGEST_SHA256)
                    .setCertificateSubject(X500Principal("CN=photo-sync"))
                    .setCertificateSerialNumber(BigInteger.ONE)
                    // Dates nothing checks, because §5.2 pins the key rather than trusting a
                    // certificate. A phone whose clock is wrong still syncs.
                    .setCertificateNotBefore(Date(0))
                    .setCertificateNotAfter(Date(Long.MAX_VALUE))
                    .setKeySize(256)
                    .build()
            )
            generator.generateKeyPair()

            val made = store.getEntry(ALIAS, null) as? KeyStore.PrivateKeyEntry
                ?: error("the keystore made a key it will not hand back")
            return Identity(made)
        }

        /**
         * The pin of a certificate: its subject public key, hashed.
         *
         * Both ends compute this the same way over the same bytes — `SubjectPublicKeyInfo` as
         * the certificate encodes it — so a pin written down by one is recognised by the
         * other. A phone that reinstalls gets a new key and is a new phone, which is the
         * operational rule `STACK.md` §5.4 spells out.
         */
        fun pinOf(certificate: X509Certificate): String =
            java.security.MessageDigest.getInstance("SHA-256")
                .digest(certificate.publicKey.encoded)
                .joinToString("") { "%02x".format(it) }
    }
}
