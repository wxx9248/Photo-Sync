package top.wxx9248.photosync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder

/**
 * Keeps the transfer alive while the screen is off and the application is in the background.
 *
 * `SPEC.md` §3.3 calls this the keep-alive stack, and the foreground service is its floor: an
 * ordinary background process on the target devices is killed within minutes. Everything above
 * it — the battery-optimisation exemption, the brand-specific autostart settings — is asked for
 * during onboarding and cannot be enforced from here.
 *
 * The notification is not decoration. It is what the platform requires in exchange for staying
 * alive, and it is also the only honest place to tell somebody their phone is busy.
 */
class SyncService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForeground(
            NOTIFICATION,
            notification(getString(R.string.notification_working)),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )
        // A session that dies with the process is re-run from the handshake by §6, so there is
        // nothing to redeliver: starting again from a stale intent would re-send a catalog
        // that has since gone stale.
        return START_NOT_STICKY
    }

    private fun notification(text: String): Notification {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL,
                getString(R.string.notification_channel),
                NotificationManager.IMPORTANCE_LOW,
            )
        )
        return Notification.Builder(this, CHANNEL)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setOngoing(true)
            .build()
    }

    companion object {
        private const val CHANNEL = "sync"
        private const val NOTIFICATION = 1

        fun start(context: Context) {
            context.startForegroundService(Intent(context, SyncService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, SyncService::class.java))
        }
    }
}
