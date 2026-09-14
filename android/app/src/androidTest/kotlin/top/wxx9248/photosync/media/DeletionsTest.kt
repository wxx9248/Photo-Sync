package top.wxx9248.photosync.media

import android.provider.MediaStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.FileInputStream
import org.junit.After
import org.junit.Before
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test
import org.junit.runner.RunWith
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.verification.Covers

/**
 * How a phone removes a photograph it did not take. `SPEC.md` §8.
 *
 * This is the half of deletion no plain JVM can see, and it is the half the specification is
 * most particular about: a direct `delete` is refused for anything this application did not
 * create --- which is every photograph a camera took --- and the only legal route is a request
 * the system puts to a person, silently when `MANAGE_MEDIA` is granted.
 *
 * So the photographs here are made through a shell rather than by this application, which is
 * what makes them somebody else's. A test that inserted its own would be testing the one case
 * the application never meets.
 *
 * One trap, found by falling into it: when the silent case fails, what is left on the phone is
 * the system's own dialog, waiting for an answer nobody is there to give. It survives the run
 * and blocks the next one, which then fails for a reason that has nothing to do with the code.
 * A device whose screen is not clear is not a device this suite can speak for.
 */
@RunWith(AndroidJUnit4::class)
class DeletionsTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val resolver = instrumentation.targetContext.contentResolver
    private val library = MediaStoreLibrary(resolver)
    private val made = mutableListOf<DevicePath>()

    /** Runs a command as the shell, which is how a photograph gets an owner that is not us. */
    private fun shell(command: String): String =
        instrumentation.uiAutomation.executeShellCommand(command).use { pipe ->
            FileInputStream(pipe.fileDescriptor).use { it.readBytes().decodeToString() }
        }

    /**
     * The permissions a person grants in §3.1, given to the test's own copy of the
     * application.
     *
     * The instrumented build carries its own application id so a run cannot uninstall the one
     * somebody is using, and a different application id is a different application: nothing a
     * person granted the real one reaches here.
     */
    @Before
    fun grant() {
        val application = instrumentation.targetContext.packageName
        for (permission in MEDIA_PERMISSIONS) {
            shell("pm grant $application $permission")
        }
    }

    private fun photographs(count: Int): List<DevicePath> {
        val names = (0 until count).map { at -> "deletions_test_$at.jpg" }
        for (name in names) {
            // Bytes enough to be a file; MediaStore only has to agree it is there.
            shell("dd if=/dev/urandom of=/sdcard/DCIM/Camera/$name bs=1024 count=4")
        }
        shell("content call --uri content://media --method scan_volume --arg external_primary")
        val paths = names.map { DevicePath("DCIM/Camera/$it") }
        made.addAll(paths)

        // A scan says it has started rather than that it has finished, so this waits for the
        // rows rather than assuming them.
        val deadline = System.currentTimeMillis() + PATIENCE_MILLIS
        while (library.uris(paths).size < count && System.currentTimeMillis() < deadline) {
            Thread.sleep(POLL_MILLIS)
        }
        return paths
    }

    @After
    fun tidy() {
        made.forEach { path -> shell("rm -f /sdcard/${path.value}") }
        shell("content call --uri content://media --method scan_volume --arg external_primary")
        made.clear()
    }

    @Test
    @Covers("R-DELETE-009")
    fun `a photograph this application did not take cannot simply be deleted`() {
        val paths = photographs(1)
        val uri = requireNotNull(library.uris(paths).firstOrNull()) { "the file was not indexed" }

        // The mechanism §8 exists for. A camera's photograph belongs to the camera, and this
        // is what the platform says about deleting it directly --- whatever the permissions.
        val refused = runCatching { resolver.delete(uri, null, null) }

        assertNotEquals(
            "the platform allowed a direct delete, which §8 is written around it refusing",
            Result.success(1),
            refused,
        )
    }

    @Test
    @Covers("R-DELETE-009")
    fun `with media management granted the request takes them without asking anybody`() {
        val paths = photographs(3)
        val items = library.uris(paths)
        assertEquals("the files were not indexed", 3, items.size)

        // The grant §3.1 step 2 asks for, given here the way a person gives it in Settings.
        // Without it the system puts a dialog on screen and nothing below happens unattended.
        shell("appops set ${instrumentation.targetContext.packageName} MANAGE_MEDIA allow")

        Deletions.request(resolver, items).send()

        // Nothing touches the screen between the request going out and the photographs being
        // gone. That is the whole of what "silent" means, and a dialog waiting for somebody
        // would spend this patience and fail here.
        val left = untilGone(paths)
        assertEquals("these were still on the phone: $left", emptySet<DevicePath>(), left)
    }

    /** What is still there, after giving the platform a while to act. */
    private fun untilGone(paths: List<DevicePath>): Set<DevicePath> {
        val deadline = System.currentTimeMillis() + PATIENCE_MILLIS
        var left = paths.toSet()
        while (System.currentTimeMillis() < deadline) {
            left = paths.toSet() - library.gone(paths)
            if (left.isEmpty()) {
                return left
            }
            Thread.sleep(POLL_MILLIS)
        }
        return left
    }

    private companion object {
        val MEDIA_PERMISSIONS = listOf(
            "android.permission.READ_MEDIA_IMAGES",
            "android.permission.READ_MEDIA_VIDEO",
        )

        /** Long enough for the platform to do it, short enough that a dialog fails the test. */
        const val PATIENCE_MILLIS = 20_000L
        const val POLL_MILLIS = 250L
    }
}
