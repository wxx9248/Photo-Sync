package top.wxx9248.photosync.media

import android.content.ContentValues
import android.net.Uri
import android.provider.MediaStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.verification.Covers

/**
 * The platform surface of `SPEC.md` §3.2, against a real MediaStore.
 *
 * Nothing here can be checked on a plain JVM: what is being tested is what MediaStore does,
 * not what this code does with the answer. `docs/ROADMAP.md` M8 runs these on a device the
 * build creates itself, so a person does not have to plug a phone in for them.
 */
@RunWith(AndroidJUnit4::class)
class MediaStoreLibraryTest {
    private val resolver =
        InstrumentationRegistry.getInstrumentation().targetContext.contentResolver

    private val written = mutableListOf<Uri>()

    @After
    fun tidy() {
        written.forEach { resolver.delete(it, null, null) }
        written.clear()
    }

    /** Puts an image where the test wants it, and hands back what identifies it. */
    private fun insert(name: String, relative: String, pending: Boolean, bytes: ByteArray): Uri {
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, name)
            put(MediaStore.MediaColumns.RELATIVE_PATH, relative)
            put(MediaStore.MediaColumns.MIME_TYPE, "image/jpeg")
            put(MediaStore.MediaColumns.IS_PENDING, if (pending) 1 else 0)
        }
        val collection = MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = requireNotNull(resolver.insert(collection, values)) { "cannot write $name" }
        written.add(uri)
        resolver.openOutputStream(uri)?.use { it.write(bytes) }
        return uri
    }

    private fun finish(uri: Uri) {
        resolver.update(
            uri,
            ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) },
            null,
            null,
        )
    }

    @Test
    @Covers("R-CATALOG-001")
    fun the_camera_bucket_is_what_is_offered_and_nothing_else() {
        val photograph = "photo-sync-test-${System.nanoTime()}.jpg"
        val elsewhere = "photo-sync-other-${System.nanoTime()}.jpg"
        finish(insert(photograph, "DCIM/Camera/", pending = false, bytes = ByteArray(32)))
        finish(insert(elsewhere, "Pictures/Screenshots/", pending = false, bytes = ByteArray(32)))

        val catalog = MediaStoreLibrary(resolver).catalog()
        val paths = catalog.entries.map { it.path.value }

        assertTrue("the camera photograph is missing", paths.contains("DCIM/Camera/$photograph"))
        assertTrue(
            "something outside DCIM/Camera was offered",
            paths.none { it.startsWith("Pictures/") },
        )
    }

    @Test
    @Covers("R-CATALOG-002")
    fun a_file_still_being_written_is_not_offered() {
        val recording = "photo-sync-pending-${System.nanoTime()}.jpg"
        val uri = insert(recording, "DCIM/Camera/", pending = true, bytes = ByteArray(8))

        val before = MediaStoreLibrary(resolver).catalog().entries.map { it.path.value }
        assertTrue(
            "a half-written file was offered",
            before.none { it.endsWith(recording) },
        )

        // Once it is finished it belongs in the catalog like anything else.
        finish(uri)
        val after = MediaStoreLibrary(resolver).catalog().entries.map { it.path.value }
        assertTrue("a finished file was not offered", after.any { it.endsWith(recording) })
    }

    @Test
    fun a_photograph_reads_back_the_bytes_it_was_given() {
        val name = "photo-sync-bytes-${System.nanoTime()}.jpg"
        val content = "the bytes of one photograph".toByteArray()
        finish(insert(name, "DCIM/Camera/", pending = false, bytes = content))

        val library = MediaStoreLibrary(resolver)
        val path = DevicePath("DCIM/Camera/$name")
        val described = library.describe(path)

        assertNotNull("the photograph cannot be found again", described)
        assertEquals(content.size.toLong(), described?.size)
        assertTrue(library.read(path, 0, content.size).contentEquals(content))
        // The whole-file digest is what §7.6 has the phone state, prefix included.
        assertEquals(64, library.digest(path).hex.length)
    }

    @Test
    fun a_photograph_that_is_gone_describes_as_nothing() {
        assertNull(MediaStoreLibrary(resolver).describe(DevicePath("DCIM/Camera/never-here.jpg")))
    }
}
