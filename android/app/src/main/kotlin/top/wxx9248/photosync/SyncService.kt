package top.wxx9248.photosync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import top.wxx9248.photosync.net.Identity
import top.wxx9248.photosync.net.Sync
import top.wxx9248.photosync.pairing.FilePairingStorage
import top.wxx9248.photosync.session.Pairing

/**
 * Runs one session, and keeps the process alive while it does.
 *
 * `SPEC.md` §3.3 calls this the keep-alive stack, and the foreground service is its floor: an
 * ordinary background process on the target devices is killed within minutes. The two locks
 * beside it are [Awake]. Everything above them — the battery-optimisation exemption, the
 * brand-specific autostart settings — is asked for during onboarding and cannot be enforced
 * from here.
 *
 * The notification is not decoration. It is what the platform requires in exchange for staying
 * alive, and it is also the only honest place to tell somebody their phone is busy.
 */
class SyncService : LifecycleService() {
    private val awake: Awake by lazy { Awake(this) }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        startForeground(
            NOTIFICATION,
            notification(getString(R.string.notification_working)),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )
        awake.hold()

        // §3.3 asks the notification for live progress. It is the only thing a person can see
        // while the screen is off and the application is not in front of them, so it follows
        // the session rather than being written once at the start.
        //
        // Paced, because the platform will not take them as fast as a transfer produces them:
        // a session of three and a half thousand photographs posted several a second and the
        // system logged `Package enqueue rate is 5.01. Shedding`, dropping them on the floor.
        // A slow collector on a state flow sees only the newest value --- that is what makes
        // one conflated --- so waiting here drops what it overtook rather than queueing it,
        // and the last state of a session is still the one that ends up on screen.
        lifecycleScope.launch {
            Transfers.stage.collect { stage ->
                say(wording(stage))
                delay(NOTIFICATION_MILLIS)
            }
        }

        // Tied to the service, so a session ends when the service does rather than outliving
        // the notification that justifies it. Rule K9. On the I/O threads, because a session
        // reads and hashes every photograph it sends and `lifecycleScope` is the main one.
        lifecycleScope.launch(Dispatchers.IO) {
            runSession()
            stopSelf(startId)
        }

        // A session that dies with the process is re-run from the handshake by §6, so there is
        // nothing to redeliver: starting again from a stale intent would re-send a catalog
        // that has since gone stale.
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        awake.release()
        super.onDestroy()
    }

    private suspend fun runSession() {
        val paired = Pairing(FilePairingStorage(pairingFile(this))).known()
        if (paired == null) {
            // Nothing to sync with. §5.2 makes pairing a deliberate act in front of a person,
            // so this is not something a service starts on its own.
            Transfers.idle()
            return
        }

        Transfers.working()
        val outcome = Sync(applicationContext, Identity.loadOrCreate(), deviceName())
            .run(paired, Transfers::ask, Transfers::remove, Transfers::progressed)

        if (outcome == null) Transfers.noDesktop() else Transfers.finished(outcome)
    }

    private fun deviceName(): String = android.os.Build.MODEL

    private fun say(line: Wording) {
        val text = getString(line.text, *line.numbers.toTypedArray())
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION, notification(text))
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

        /**
         * How often the notification may be rewritten, in milliseconds.
         *
         * Half a second is under the rate the platform sheds at and over the rate a person
         * can read at, so what it costs is nothing anybody was going to see.
         */
        private const val NOTIFICATION_MILLIS: Long = 500

        /**
         * The only thing the phone keeps between sessions. §3.5.
         *
         * In the application's own storage, which is removed when the application is, because
         * a pairing that outlived the install would be a key nothing holds any more.
         */
        fun pairingFile(context: Context): File = File(context.filesDir, "paired")

        fun start(context: Context) {
            context.startForegroundService(Intent(context, SyncService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, SyncService::class.java))
        }
    }
}
