package top.wxx9248.photosync.media

import android.app.PendingIntent
import android.content.ContentResolver
import android.net.Uri
import android.provider.MediaStore

/**
 * Asking the system to delete photographs. `SPEC.md` §8.
 *
 * With `MANAGE_MEDIA` granted this is silent, which is the point: the person has already
 * confirmed once, in this application, against a list the desktop proved it holds. Without it
 * the system shows its own confirmation, and the request is sent in batches because a binder
 * transaction has a size limit that a few thousand identifiers would exceed.
 *
 * `createTrashRequest` was rejected as the primary mechanism: it asks for the same consent and
 * leaves the items occupying storage for weeks, which is the opposite of what a person pressing
 * "free up 18.2 GB" asked for.
 */
internal object Deletions {
    /** How many identifiers travel in one request. */
    const val BATCH: Int = 500

    /**
     * Builds the requests that would delete these photographs, in batches.
     *
     * Each has to be launched and its answer waited for; nothing is deleted by building one.
     */
    fun requests(resolver: ContentResolver, items: List<Uri>): List<PendingIntent> =
        items.chunked(BATCH).map { batch ->
            MediaStore.createDeleteRequest(resolver, batch)
        }
}
