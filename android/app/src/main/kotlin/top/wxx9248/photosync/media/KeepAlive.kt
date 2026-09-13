package top.wxx9248.photosync.media

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
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
     * What this phone's system software needs beyond the exemption, or null when it needs
     * nothing.
     *
     * The same steps as the table in `docs/INSTALL.md`, which is where somebody looks them up
     * afterwards, and that table is keyed the way `SPEC.md` §3.1 names them: by the system ---
     * MIUI, ColorOS, One UI --- rather than by whose name is on the back of the phone. The two
     * come apart. A Xiaomi handset running a community build of Android still reports a
     * `Build.MANUFACTURER` of "Xiaomi" and has no Autostart screen to send anybody to, and
     * steps for a menu that is not there are worse than none: they tell a person their
     * transfer will stop unless they change a setting they cannot find.
     *
     * So the question asked here is whether the manufacturer's own power manager is installed,
     * which is the application those steps walk through.
     */
    fun brandSteps(context: Context): Int? = brandSteps { name -> installed(context, name) }

    /**
     * The same decision, against any answer about what is installed.
     *
     * Separated so a plain JVM test can make it: the mapping is the part that goes wrong, and
     * it needs no phone to check.
     */
    internal fun brandSteps(installed: (String) -> Boolean): Int? =
        SYSTEMS.firstOrNull { system -> system.second.any(installed) }?.first

    private fun installed(context: Context, name: String): Boolean =
        try {
            val packages = context.packageManager
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                packages.getPackageInfo(name, PackageManager.PackageInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                packages.getPackageInfo(name, 0)
            }
            true
        } catch (_: PackageManager.NameNotFoundException) {
            false
        }

    /**
     * Each system, and the packages that say it is this one.
     *
     * Every entry is declared in the manifest's `queries` block, without which Android 11 and
     * later answer "not installed" for all of them and nobody is ever shown any steps.
     *
     * OnePlus comes before Oppo because a recent OxygenOS ships Oppo's own package alongside
     * its own, and the OnePlus menus are the ones on that phone.
     */
    private val SYSTEMS: List<Pair<Int, List<String>>> = listOf(
        R.string.keepalive_xiaomi to listOf("com.miui.securitycenter", "com.miui.powerkeeper"),
        R.string.keepalive_oneplus to listOf("com.oneplus.security"),
        R.string.keepalive_oppo to listOf(
            "com.coloros.safecenter",
            "com.oplus.safecenter",
            "com.coloros.oppoguardelf",
        ),
        R.string.keepalive_vivo to listOf(
            "com.iqoo.secure",
            "com.vivo.permissionmanager",
            "com.vivo.abe",
        ),
        R.string.keepalive_samsung to listOf("com.samsung.android.lool"),
    )
}
