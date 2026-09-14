package top.wxx9248.photosync.net

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Temporary. Does mDNS resolution on this phone need the multicast lock, or not?
 *
 * Uses NsdManager directly, not this application's discovery, so the only thing that differs
 * between the two halves is whether the lock is held.
 */
@RunWith(AndroidJUnit4::class)
class MulticastExperiment {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun resolved(holdingLock: Boolean, seconds: Long): Int {
        val wifi =
            context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        val lock = wifi.createMulticastLock("experiment").apply { setReferenceCounted(false) }
        val manager = context.getSystemService(Context.NSD_SERVICE) as NsdManager
        val found = AtomicInteger()
        val first = CountDownLatch(1)

        val listener = object : NsdManager.DiscoveryListener {
            override fun onStartDiscoveryFailed(type: String?, code: Int) = first.countDown()
            override fun onStopDiscoveryFailed(type: String?, code: Int) {}
            override fun onDiscoveryStarted(type: String?) {}
            override fun onDiscoveryStopped(type: String?) {}
            override fun onServiceFound(service: NsdServiceInfo?) {
                found.incrementAndGet()
                first.countDown()
            }
            override fun onServiceLost(service: NsdServiceInfo?) {}
        }

        if (holdingLock) lock.acquire()
        try {
            manager.discoverServices("_photosync._tcp", NsdManager.PROTOCOL_DNS_SD, listener)
            first.await(seconds, TimeUnit.SECONDS)
            // A moment more, in case a second answer is on its way.
            Thread.sleep(500)
        } finally {
            runCatching { manager.stopServiceDiscovery(listener) }
            if (holdingLock) runCatching { lock.release() }
        }
        // The system keeps looking for a moment after being told to stop.
        Thread.sleep(1500)
        return found.get()
    }

    @Test
    fun does_the_lock_decide_whether_mdns_resolves() {
        for (round in 1..4) {
            val without = resolved(holdingLock = false, seconds = 6)
            val with = resolved(holdingLock = true, seconds = 6)
            println("ROUND $round  without-lock=$without  with-lock=$with")
        }
        // And the other order, in case one primes the other.
        for (round in 5..6) {
            val with = resolved(holdingLock = true, seconds = 6)
            val without = resolved(holdingLock = false, seconds = 6)
            println("ROUND $round  with-lock=$with  without-lock=$without")
        }
    }
}
