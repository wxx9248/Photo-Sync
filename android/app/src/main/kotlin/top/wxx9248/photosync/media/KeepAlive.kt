package top.wxx9248.photosync.media

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import top.wxx9248.photosync.R

/**
 * Staying alive long enough to finish. `SPEC.md` §3.1 steps 3 and 4, and §3.3.
 *
 * Android's own exemption is the part that can be asked for in a dialog. Everything after it is
 * a setting buried in a manufacturer's own menus, which no application may change on a person's
 * behalf: the target phones of §3.1 kill an ordinary background process within minutes of the
 * screen going off, and the only thing that can be done about it is to say where the setting is
 * and take somebody there.
 */
internal object KeepAlive {
    /** Whether Android itself will leave this application alone. */
    fun exempt(context: Context): Boolean {
        val power = context.getSystemService(Context.POWER_SERVICE) as PowerManager
        return power.isIgnoringBatteryOptimizations(context.packageName)
    }

    /**
     * The dialog that asks for the exemption.
     *
     * `SPEC.md` §3.1 asks for this by name. It is a prompt the Play Store restricts, which is
     * one of the reasons `STACK.md` ships this through F-Droid.
     */
    @android.annotation.SuppressLint("BatteryLife")
    fun exemptionRequest(context: Context): Intent =
        Intent(
            Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
            Uri.fromParts("package", context.packageName, null),
        )

    /** This application's page in the system settings, which is where the rest of them live. */
    fun settings(context: Context): Intent =
        Intent(
            Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
            Uri.fromParts("package", context.packageName, null),
        )

    /**
     * What this phone's manufacturer needs beyond the exemption, or null when it needs nothing.
     *
     * The same steps as the table in `docs/INSTALL.md`, which is where somebody looks them up
     * afterwards. Stock Android is not in the list because the exemption is enough there.
     */
    fun brandSteps(): Int? = when (Build.MANUFACTURER.lowercase()) {
        "xiaomi", "redmi", "poco" -> R.string.keepalive_xiaomi
        "oppo", "realme" -> R.string.keepalive_oppo
        "oneplus" -> R.string.keepalive_oneplus
        "vivo", "iqoo" -> R.string.keepalive_vivo
        "samsung" -> R.string.keepalive_samsung
        else -> null
    }
}
