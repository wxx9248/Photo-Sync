package top.wxx9248.photosync

import kotlin.test.Test
import kotlin.test.assertEquals
import top.wxx9248.photosync.session.Outcome
import top.wxx9248.photosync.session.Progress
import top.wxx9248.photosync.session.verification.Covers

/**
 * What the persistent notification says. `SPEC.md` §3.3.
 *
 * With the screen off this line is the whole of the interface, and the specification asks it
 * for live progress rather than for a sentence written once when the service started.
 */
class WordingTest {
    @Test
    @Covers("R-ALIVE-001")
    fun `a transfer under way says how far it has got`() {
        val line = wording(Transfers.Stage.Working(Progress(sent = 40, total = 1200)))

        assertEquals(R.string.transfer_progress, line.text)
        assertEquals(listOf(40, 1200), line.numbers)
    }

    @Test
    @Covers("R-ALIVE-001")
    fun `a transfer counts up as it goes`() {
        val first = wording(Transfers.Stage.Working(Progress(sent = 1, total = 3)))
        val later = wording(Transfers.Stage.Working(Progress(sent = 2, total = 3)))

        assertEquals(listOf(1, 3), first.numbers)
        assertEquals(listOf(2, 3), later.numbers)
    }

    @Test
    @Covers("R-ALIVE-001")
    fun `a session with no total yet claims no numbers`() {
        for (stage in listOf(
            Transfers.Stage.Working(),
            Transfers.Stage.Idle,
            Transfers.Stage.Finished(
                Outcome.Finished(
                    sent = 1,
                    skipped = 0,
                    failed = 0,
                    deleted = 0,
                    kept = 0,
                    bytesFreed = 0,
                )
            ),
        )) {
            assertEquals(R.string.notification_working, wording(stage).text, stage.toString())
            assertEquals(emptyList(), wording(stage).numbers, stage.toString())
        }
    }
}
