package top.wxx9248.photosync

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import top.wxx9248.photosync.session.verification.Covers

/**
 * The two locks `SPEC.md` §3.3 names beside the foreground service.
 *
 * Only a real device has a power manager and a Wi-Fi service to take them from, which is why
 * this is an instrumented test. What it checks is that both are actually held --- a lock a
 * phone silently refuses reads exactly like one nobody asked for, and the difference only
 * shows up hours later as a transfer that stalled with the screen off.
 */
@RunWith(AndroidJUnit4::class)
class AwakeTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    @Covers("R-ALIVE-001")
    fun `holding takes both locks and releasing gives both back`() {
        val awake = Awake(context)

        assertFalse("nothing is held before anything asks", awake.held)

        awake.hold()
        assertTrue("the processor and the radio are both held", awake.held)

        awake.release()
        assertFalse("a transfer that ended holds nothing", awake.held)
    }

    @Test
    @Covers("R-ALIVE-001")
    fun `a second start does not leave a lock behind`() {
        val awake = Awake(context)

        awake.hold()
        awake.hold()
        awake.release()

        assertFalse("one release undoes any number of starts", awake.held)
    }
}
