package top.wxx9248.photosync.media

import android.content.ContentResolver
import android.content.ContentUris
import android.net.Uri
import android.provider.MediaStore
import java.security.MessageDigest
import top.wxx9248.photosync.session.Catalog
import top.wxx9248.photosync.session.CatalogEntry
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.Library
import top.wxx9248.photosync.session.Sha256
import top.wxx9248.photosync.session.Timestamp

/**
 * The phone's photographs, read through MediaStore. `SPEC.md` §3.2.
 *
 * This is the whole of the platform surface the catalog needs, kept in one class so the
 * session state machine never sees an Android type — rule K13. What it enumerates is narrow
 * on purpose: the `DCIM/Camera` bucket, images and videos, and nothing that is still being
 * written or already in the trash.
 */
class MediaStoreLibrary(private val resolver: ContentResolver) : Library {
    /**
     * Everything on offer, read once.
     *
     * §3.2 enumerates at the start of a session and holds the result for the whole of it, so
     * this is called once and its answer frozen into a [Catalog]. Photographs taken during a
     * transfer belong to the next session.
     */
    fun catalog(): Catalog {
        val entries = mutableListOf<CatalogEntry>()
        for (collection in collections()) {
            resolver.query(collection, COLUMNS, SELECTION, SELECTION_ARGUMENTS, null)?.use { rows ->
                val name = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
                val relative = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.RELATIVE_PATH)
                val size = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
                val modified = rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
                while (rows.moveToNext()) {
                    entries.add(
                        CatalogEntry(
                            path = DevicePath(rows.getString(relative) + rows.getString(name)),
                            size = rows.getLong(size),
                            mtime = Timestamp(rows.getLong(modified)),
                        )
                    )
                }
            }
        }
        return Catalog(entries.sortedBy { it.path.value })
    }

    override fun describe(path: DevicePath): CatalogEntry? =
        find(path)?.let { (_, entry) -> entry }

    override fun read(path: DevicePath, offset: Long, length: Int): ByteArray {
        val uri = find(path)?.first ?: return ByteArray(0)
        return resolver.openInputStream(uri)?.use { stream ->
            stream.skip(offset)
            val buffer = ByteArray(length)
            var filled = 0
            while (filled < length) {
                val read = stream.read(buffer, filled, length - filled)
                if (read <= 0) break
                filled += read
            }
            buffer.copyOf(filled)
        } ?: ByteArray(0)
    }

    /**
     * The sha256 of the whole file.
     *
     * §3.2 promises no upfront hashing, so nothing calls this until a photograph is actually
     * being sent or a deletion candidate has to be proved. It reads in blocks because a video
     * does not fit in memory.
     */
    override fun digest(path: DevicePath): Sha256 {
        val uri = find(path)?.first ?: return Sha256("0".repeat(64))
        val sha = MessageDigest.getInstance("SHA-256")
        resolver.openInputStream(uri)?.use { stream ->
            val buffer = ByteArray(1 shl 16)
            while (true) {
                val read = stream.read(buffer)
                if (read <= 0) break
                sha.update(buffer, 0, read)
            }
        }
        return Sha256(sha.digest().joinToString("") { "%02x".format(it) })
    }

    /**
     * Removes one photograph.
     *
     * A direct delete only succeeds where the application owns the item or holds
     * `MANAGE_MEDIA`; everything else goes through the system's own request, which is what
     * [deleteRequest] builds. Either way §8's gates have already been passed before this is
     * reached: nothing here decides whether a photograph may go.
     */
    override fun delete(path: DevicePath): Boolean {
        val uri = find(path)?.first ?: return false
        return runCatching { resolver.delete(uri, null, null) > 0 }.getOrDefault(false)
    }

    /** The identifier and current facts of one path, or null if it is not there any more. */
    fun find(path: DevicePath): Pair<Uri, CatalogEntry>? {
        for (collection in collections()) {
            val where = "$SELECTION AND ${MediaStore.MediaColumns.RELATIVE_PATH} || " +
                "${MediaStore.MediaColumns.DISPLAY_NAME} = ?"
            val arguments = SELECTION_ARGUMENTS + path.value
            resolver.query(collection, COLUMNS + BaseId, where, arguments, null)?.use { rows ->
                if (rows.moveToFirst()) {
                    val id = rows.getLong(rows.getColumnIndexOrThrow(MediaStore.MediaColumns._ID))
                    val name =
                        rows.getString(rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME))
                    val relative =
                        rows.getString(rows.getColumnIndexOrThrow(MediaStore.MediaColumns.RELATIVE_PATH))
                    return ContentUris.withAppendedId(collection, id) to CatalogEntry(
                        path = DevicePath(relative + name),
                        size = rows.getLong(rows.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)),
                        mtime = Timestamp(
                            rows.getLong(rows.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED))
                        ),
                    )
                }
            }
        }
        return null
    }

    private fun collections(): List<Uri> = listOf(
        MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL),
        MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL),
    )

    private companion object {
        const val BaseId = MediaStore.MediaColumns._ID

        val COLUMNS = arrayOf(
            MediaStore.MediaColumns.DISPLAY_NAME,
            MediaStore.MediaColumns.RELATIVE_PATH,
            MediaStore.MediaColumns.SIZE,
            MediaStore.MediaColumns.DATE_MODIFIED,
        )

        /**
         * The `DCIM/Camera` bucket only, and nothing half-written. A video still recording has
         * `IS_PENDING` set and a size that means nothing yet; §3.2 excludes it rather than
         * catalog a file that will not match itself a second later.
         */
        const val SELECTION =
            "${MediaStore.MediaColumns.RELATIVE_PATH} = ? AND " +
                "${MediaStore.MediaColumns.IS_PENDING} = 0 AND " +
                "${MediaStore.MediaColumns.IS_TRASHED} = 0"

        val SELECTION_ARGUMENTS = arrayOf("DCIM/Camera/")
    }
}
