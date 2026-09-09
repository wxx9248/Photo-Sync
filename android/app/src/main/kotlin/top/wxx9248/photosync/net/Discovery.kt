package top.wxx9248.photosync.net

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import java.net.InetAddress
import java.net.InetSocketAddress
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Finding the desktop on the local network. `SPEC.md` §5.1.
 *
 * The phone auto-connects to the desktop it is paired with, and the start order of the two
 * applications does not matter: the desktop advertises for as long as it runs, so a phone
 * that wakes up later hears it. A desktop that is not found is not an error, it is the
 * "is the computer on?" screen, so this reports absence rather than throwing.
 */
internal class Discovery(context: Context) {
    private val manager =
        context.getSystemService(Context.NSD_SERVICE) as NsdManager

    /**
     * Waits for a desktop to answer, or gives up.
     *
     * The first service of the right type wins. A house with two desktops advertising is not
     * something §5.1 handles: the phone is paired with one key, and connecting to the wrong
     * one fails the pinning check rather than doing anything worse.
     */
    suspend fun find(timeout: Duration = LONG_ENOUGH): InetSocketAddress? =
        withTimeoutOrNull(timeout) {
            suspendCancellableCoroutine { waiting ->
                val looking = FirstDesktop(waiting)
                manager.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, looking)
                waiting.invokeOnCancellation { looking.answer(null) }
            }
        }

    /**
     * One attempt: the first desktop that resolves, and nothing after it.
     *
     * NSD calls back on its own threads and keeps the radio looking until it is told to stop,
     * so the first thread to reach an answer is the one that stops it. Every way out of a
     * search goes through [answer]: an address, a discovery that would not start, and the
     * cancellation the timeout causes.
     */
    private inner class FirstDesktop(
        private val waiting: CancellableContinuation<InetSocketAddress?>,
    ) : NsdManager.DiscoveryListener, NsdManager.ResolveListener {
        private val answered = AtomicBoolean(false)

        fun answer(address: InetSocketAddress?) {
            if (!answered.compareAndSet(false, true)) return
            runCatching { manager.stopServiceDiscovery(this) }
            waiting.resume(address)
        }

        override fun onDiscoveryStarted(serviceType: String?) = Unit

        override fun onDiscoveryStopped(serviceType: String?) = Unit

        override fun onStopDiscoveryFailed(serviceType: String?, errorCode: Int) = Unit

        override fun onServiceLost(service: NsdServiceInfo?) = Unit

        override fun onStartDiscoveryFailed(serviceType: String?, errorCode: Int) = answer(null)

        override fun onServiceFound(service: NsdServiceInfo?) {
            manager.resolveService(service ?: return, this)
        }

        // A service that will not resolve is one desktop of possibly several. The search
        // carries on until something resolves or the timeout ends it.
        override fun onResolveFailed(info: NsdServiceInfo?, errorCode: Int) = Unit

        override fun onServiceResolved(info: NsdServiceInfo?) {
            val resolved = info ?: return
            val host = resolved.address ?: return
            answer(InetSocketAddress(host, resolved.port))
        }
    }

    companion object {
        /**
         * Where a resolved service actually is.
         *
         * `hostAddresses` is the one to use and it is not on every phone this application
         * supports: it arrived in Android 14, and on Android 13 only with the seventh
         * Tiramisu extension. `SPEC.md` §3.1 starts at Android 12, so the older call is what
         * the floor of that range has. Both return the same address.
         */
        private val NsdServiceInfo.address: InetAddress?
            get() = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                hostAddresses.firstOrNull()
            } else {
                @Suppress("DEPRECATION")
                host
            }

        /** What the desktop advertises. Fixed by §5.1 and matched exactly. */
        const val SERVICE_TYPE: String = "_photosync._tcp"

        /**
         * How long a phone looks before deciding the computer is off.
         *
         * Long enough for a desktop that is awake to answer over a busy home network, short
         * enough that somebody who pressed Start gets told something.
         */
        val LONG_ENOUGH: Duration = 10.seconds
    }
}
