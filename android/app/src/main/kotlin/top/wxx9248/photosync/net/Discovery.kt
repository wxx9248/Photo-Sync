package top.wxx9248.photosync.net

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import java.net.InetSocketAddress
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

/**
 * Finding the desktop on the local network. `SPEC.md` §5.1.
 *
 * The phone auto-connects to the desktop it is paired with, and the start order of the two
 * applications does not matter: the desktop advertises for as long as it runs, so a phone
 * that wakes up later hears it. A desktop that is not found is not an error, it is the
 * "is the computer on?" screen, so this reports absence rather than throwing.
 */
class Discovery(context: Context) {
    private val manager =
        context.getSystemService(Context.NSD_SERVICE) as NsdManager

    /**
     * Waits for a desktop to answer, or gives up.
     *
     * The first service of the right type wins. A house with two desktops advertising is not
     * something §5.1 handles: the phone is paired with one key, and connecting to the wrong
     * one fails the pinning check rather than doing anything worse.
     */
    suspend fun find(timeoutMillis: Long = 10_000): InetSocketAddress? =
        suspendCancellableCoroutine { waiting ->
            var answered = false

            val listener = object : NsdManager.DiscoveryListener {
                override fun onDiscoveryStarted(serviceType: String?) = Unit
                override fun onDiscoveryStopped(serviceType: String?) = Unit
                override fun onStartDiscoveryFailed(serviceType: String?, errorCode: Int) {
                    if (!answered) {
                        answered = true
                        waiting.resume(null)
                    }
                }

                override fun onStopDiscoveryFailed(serviceType: String?, errorCode: Int) = Unit
                override fun onServiceLost(service: NsdServiceInfo?) = Unit

                override fun onServiceFound(service: NsdServiceInfo?) {
                    val found = service ?: return
                    manager.resolveService(
                        found,
                        object : NsdManager.ResolveListener {
                            override fun onResolveFailed(info: NsdServiceInfo?, errorCode: Int) = Unit

                            override fun onServiceResolved(info: NsdServiceInfo?) {
                                val resolved = info ?: return
                                val host = resolved.hostAddresses.firstOrNull() ?: return
                                if (!answered) {
                                    answered = true
                                    waiting.resume(InetSocketAddress(host, resolved.port))
                                }
                            }
                        },
                    )
                }
            }

            manager.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
            waiting.invokeOnCancellation {
                runCatching { manager.stopServiceDiscovery(listener) }
            }
        }

    companion object {
        /** What the desktop advertises. Fixed by §5.1 and matched exactly. */
        const val SERVICE_TYPE: String = "_photosync._tcp"
    }
}
