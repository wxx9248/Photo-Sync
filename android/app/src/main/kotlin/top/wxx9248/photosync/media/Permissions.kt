package top.wxx9248.photosync.media

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.MediaStore
import android.provider.Settings
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
     * Whether photographs can be removed without the system asking again each time.
     *
     * Step two of §3.1, and skippable: §8 falls back to the system's own confirmation. What it
     * is not is optional to *ask* for --- without it every delete of a photograph this
     * application did not take is refused, which is the whole of what a person pressed the
     * button for.
     */
    fun canManageMedia(context: Context): Boolean = MediaStore.canManageMedia(context)

    /**
     * The settings screen that grants it.
     *
     * Special access is never granted by a dialog: the platform insists a person goes and
     * turns it on, so this is the only thing an application can do about it.
     */
    fun manageMediaRequest(context: Context): Intent =
        Intent(
            Settings.ACTION_REQUEST_MANAGE_MEDIA,
            Uri.fromParts("package", context.packageName, null),
        )

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
