package top.wxx9248.photosync.net

import java.net.Socket
import java.security.Principal
import java.security.PrivateKey
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.SSLEngine
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509TrustManager

/**
 * Who the phone will talk to, and what it presents when asked.
 *
 * `SPEC.md` §5.2 replaces the whole certificate-authority idea with one comparison: the
 * desktop is the key the phone wrote down when a person confirmed a six-digit code, and
 * nothing else. There is no name to check, no chain to walk and no expiry to honour, and
 * saying that out loud in a trust manager is safer than leaving a default in place that
 * would accept a public authority's certificate for the same host.
 */
internal class PinnedDesktop(private val expectedPin: String) : X509TrustManager {
    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        val presented = chain?.firstOrNull()
            ?: throw CertificateException("the desktop presented no certificate")
        val pin = Identity.pinOf(presented)
        if (pin != expectedPin) {
            // Either the desktop was reinstalled or somebody is standing in the middle, and
            // only a person can tell those apart. §5.2 calls this the re-pair flow.
            throw CertificateException("this is not the desktop this phone is paired with")
        }
    }

    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        throw CertificateException("a phone is never a server")
    }

    // Deliberately empty: an empty list of accepted issuers is what says "no authority is
    // trusted here", which is the point.
    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}

/** Presents this phone's keystore-resident key, which is how the desktop recognises it. */
internal class DeviceKeyManager(private val identity: Identity) : X509ExtendedKeyManager() {
    override fun getClientAliases(keyType: String?, issuers: Array<out Principal>?) =
        arrayOf(Identity.ALIAS)

    override fun chooseClientAlias(
        keyType: Array<out String>?,
        issuers: Array<out Principal>?,
        socket: Socket?,
    ) = Identity.ALIAS

    override fun chooseEngineClientAlias(
        keyType: Array<out String>?,
        issuers: Array<out Principal>?,
        engine: SSLEngine?,
    ) = Identity.ALIAS

    override fun getServerAliases(keyType: String?, issuers: Array<out Principal>?) = null

    override fun chooseServerAlias(
        keyType: String?,
        issuers: Array<out Principal>?,
        socket: Socket?,
    ) = null

    override fun getCertificateChain(alias: String?) = arrayOf(identity.certificate)

    /**
     * The private key itself never leaves the keystore; this hands back the handle to it, and
     * the platform does the signing behind that handle.
     */
    override fun getPrivateKey(alias: String?): PrivateKey = identity.privateKey
}
