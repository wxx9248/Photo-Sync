package top.wxx9248.photosync.media

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import top.wxx9248.photosync.R
import top.wxx9248.photosync.session.verification.Covers

/**
 * Which keep-alive steps a phone is shown. `SPEC.md` §3.1 step 4.
 *
 * The steps name menus that exist only in one manufacturer's system software, so the question
 * is whether that software is on this phone --- not whose name is on the back of it. The two
 * disagree on any handset running a community build of Android, which is where this was found.
 */
class KeepAliveTest {
    @Test
    @Covers("R-ALIVE-002")
    fun `a phone with none of the power managers is shown no steps`() {
        assertNull(KeepAlive.brandSteps { false })
    }

    @Test
    @Covers("R-ALIVE-002")
    fun `a phone running MIUI is shown the steps for it`() {
        assertEquals(
            R.string.keepalive_xiaomi,
            KeepAlive.brandSteps { name -> name == "com.miui.securitycenter" },
        )
    }

    @Test
    @Covers("R-ALIVE-002")
    fun `each system has its own steps`() {
        val systems = mapOf(
            "com.oneplus.security" to R.string.keepalive_oneplus,
            "com.coloros.safecenter" to R.string.keepalive_oppo,
            "com.iqoo.secure" to R.string.keepalive_vivo,
            "com.samsung.android.lool" to R.string.keepalive_samsung,
        )

        for ((installed, steps) in systems) {
            assertEquals(steps, KeepAlive.brandSteps { name -> name == installed }, installed)
        }
    }

    @Test
    @Covers("R-ALIVE-002")
    fun `a OnePlus carrying Oppo's package is shown the OnePlus steps`() {
        val onePlus = setOf("com.oneplus.security", "com.oplus.safecenter")

        assertEquals(R.string.keepalive_oneplus, KeepAlive.brandSteps { it in onePlus })
    }
}
