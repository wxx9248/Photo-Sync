package top.wxx9248.photosync

import android.content.Context
import android.net.wifi.WifiManager
import android.os.PowerManager

/**
 * The two locks `SPEC.md` §3.3 names beside the foreground service.
 *
 * A foreground service is a promise the system will not kill the process. It is not a promise
 * that the processor stays awake or that the radio keeps its full throughput: with the screen
 * off Android is free to suspend the one and to put the other into a power-saving mode that
 * parks packets for tens of milliseconds at a time. Neither ends a transfer outright, which is
 * what makes them easy to miss --- a session watched on a phone in deep idle finished without
 * either lock. It finished on that phone, on that network, that evening.
 *
 * Both are released together, because a partial wake lock left held is the worst thing an
 * application can do to a battery.
 */
internal class Awake(context: Context) {
    private val cpu = (context.getSystemService(Context.POWER_SERVICE) as PowerManager)
        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, TAG)
        // The service acquires once per start command and a second start must not leave a
        // count behind that the single release at the end cannot undo.
        .apply { setReferenceCounted(false) }

    // The Wi-Fi service is one of the ones that must be taken from the application context:
    // holding it from a service context leaks the service for as long as the lock lives.
    private val radio = (
        context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        )
        .createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, TAG)
        .apply { setReferenceCounted(false) }

    /** Whether both locks are being held. */
    val held: Boolean get() = cpu.isHeld && radio.isHeld

    fun hold() {
        cpu.acquire(LIMIT_MILLIS)
        radio.acquire()
    }

    fun release() {
        if (cpu.isHeld) cpu.release()
        if (radio.isHeld) radio.release()
    }

    private companion object {
        const val TAG = "PhotoSync:transfer"

        /**
         * A ceiling on the wake lock, in case a session ends in a way that skips the release.
         *
         * Eight hours is far longer than any transfer this application can start --- twelve
         * hundred photographs took seven minutes --- and short enough that a phone left in a
         * drawer recovers by morning rather than by flat battery.
         */
        const val LIMIT_MILLIS = 8L * 60 * 60 * 1000
    }
}
