package top.wxx9248.photosync.media

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat

/**
 * What the application has to be granted before it can do anything at all. `SPEC.md` §3.1.
 *
 * Only the first step of that onboarding is here, because it is the only one without which
 * nothing works: a phone that cannot read its own camera roll has nothing to offer. Media
 * management, the battery exemption and the per-brand keep-alive steps are all skippable, and
 * §8 and §3.3 say what the application does when they are declined.
 */
internal object Permissions {
    /**
     * The ones this platform actually uses.
     *
     * `READ_MEDIA_IMAGES` and `READ_MEDIA_VIDEO` arrived in Android 13. Below that the whole
     * library is one permission, which is why `minSdk` 31 needs both spellings.
     */
    val needed: List<String> = buildList {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            add(Manifest.permission.READ_MEDIA_IMAGES)
            add(Manifest.permission.READ_MEDIA_VIDEO)
            add(Manifest.permission.POST_NOTIFICATIONS)
        } else {
            @Suppress("DEPRECATION")
            add(Manifest.permission.READ_EXTERNAL_STORAGE)
        }
    }

    /** The ones still to ask for, which is empty when there is nothing to ask. */
    fun missing(context: Context): List<String> = needed.filterNot { granted(context, it) }

    private fun granted(context: Context, permission: String): Boolean =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED

    /**
     * Whether the camera roll can be read.
     *
     * Notifications are asked for in the same breath and are not part of this: a person who
     * refuses them gets a quieter transfer, not a broken one.
     */
    fun canReadTheLibrary(context: Context): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            granted(context, Manifest.permission.READ_MEDIA_IMAGES) ||
                granted(context, Manifest.permission.READ_MEDIA_VIDEO)
        } else {
            @Suppress("DEPRECATION")
            granted(context, Manifest.permission.READ_EXTERNAL_STORAGE)
        }
}
