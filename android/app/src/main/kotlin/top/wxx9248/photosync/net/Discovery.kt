package top.wxx9248.photosync.net

import android.content.Context
import android.util.Log
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume
import kotlin.time.Duration
import kotlin.time.Duration.Companion.seconds
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
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
    suspend fun find(timeout: Duration = LONG_ENOUGH): InetSocketAddress? {
        val offered = withTimeoutOrNull(timeout) {
            suspendCancellableCoroutine { waiting ->
                val looking = FirstDesktop(waiting)
                Log.i(TAG, "looking for $SERVICE_TYPE for $timeout")
                manager.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, looking)
                waiting.invokeOnCancellation { looking.answer(null) }
            }
        }.orEmpty()

        if (offered.isEmpty()) {
            Log.i(TAG, "no desktop answered")
            return null
        }

        val reachable = reachable(offered)
        Log.i(
            TAG,
            if (reachable == null) {
                "a desktop answered on $offered and none of it could be reached"
            } else {
                "found a desktop at $reachable"
            },
        )
        return reachable
    }

    /**
     * The first of those addresses that will actually take a connection.
     *
     * A desktop advertises every address the machine holds, and a phone can rarely use all of
     * them: the two can share an address family the network will not route between them, or a
     * firewall may answer on one stack and not the other. Taking the first and giving up is
     * how a session fails against a desktop that is plainly there --- which is what happened
     * the first time this met a real network, on the IPv6 address of a dual-stack machine.
     */
    private suspend fun reachable(offered: List<InetSocketAddress>): InetSocketAddress? =
        withContext(Dispatchers.IO) {
            offered.firstOrNull { address ->
                try {
                    Socket().use { it.connect(address, REACH_MILLIS) }
                    true
                } catch (unreachable: Exception) {
                    Log.i(TAG, "$address did not answer: ${unreachable.message}")
                    false
                }
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
        private val waiting: CancellableContinuation<List<InetSocketAddress>?>,
    ) : NsdManager.DiscoveryListener {
        private val answered = AtomicBoolean(false)

        fun answer(address: List<InetSocketAddress>?) {
            if (!answered.compareAndSet(false, true)) return
            runCatching { manager.stopServiceDiscovery(this) }
            waiting.resume(address)
        }

        override fun onDiscoveryStarted(serviceType: String?) = Unit

        override fun onDiscoveryStopped(serviceType: String?) = Unit

        override fun onStopDiscoveryFailed(serviceType: String?, errorCode: Int) = Unit

        override fun onServiceLost(service: NsdServiceInfo?) = Unit

        override fun onStartDiscoveryFailed(serviceType: String?, errorCode: Int) {
            Log.w(TAG, "discovery would not start: $errorCode")
            answer(null)
        }

        override fun onServiceFound(service: NsdServiceInfo?) {
            Log.i(TAG, "found ${service?.serviceName}")
            manager.resolveService(service ?: return, resolver())
        }

        /**
         * A listener per service, because NsdManager refuses one that is already resolving.
         *
         * A home network answers more than once --- two desktops, or one announcing itself
         * again --- and reusing a single listener throws `IllegalArgumentException` on the
         * system's own thread, which takes the application down with it.
         */
        private fun resolver() = object : NsdManager.ResolveListener {
            // A service that will not resolve is one desktop of possibly several. The search
            // carries on until something resolves or the timeout ends it.
            override fun onResolveFailed(info: NsdServiceInfo?, errorCode: Int) = Unit

            override fun onServiceResolved(info: NsdServiceInfo?) {
                val resolved = info ?: return
                val addresses = resolved.addresses.map { InetSocketAddress(it, resolved.port) }
                if (addresses.isNotEmpty()) {
                    answer(addresses)
                }
            }
        }
    }

    companion object {
        /**
         * Where a resolved service actually is.
         *
         * `hostAddresses` is the one to use and it is not on every phone this application
         * supports: it arrived in Android 14, and on Android 13 only with the seventh
         * Tiramisu extension. `SPEC.md` §3.1 starts at Android 12, so the older call is what
         * the floor of that range has. The older one knows of a single address, which is the
         * best it can do.
         */
        private val NsdServiceInfo.addresses: List<InetAddress>
            get() = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                hostAddresses
            } else {
                @Suppress("DEPRECATION")
                listOfNotNull(host)
            }

        /**
         * How long one address is given to answer.
         *
         * Short: an address on the same network answers in milliseconds, and an address that
         * cannot be routed to is usually refused rather than left hanging. What this bounds is
         * the unusual case where it is neither.
         */
        private const val REACH_MILLIS = 2_000

        private const val TAG = "PhotoSyncDiscovery"

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
