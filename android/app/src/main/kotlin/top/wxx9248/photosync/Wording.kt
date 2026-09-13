package top.wxx9248.photosync

/**
 * What the notification says, worked out apart from the notification itself.
 *
 * `SPEC.md` §3.3 asks the persistent notification for live progress, and the part of that
 * worth checking is the choice --- which sentence, and with which numbers --- rather than the
 * platform call that displays it. Keeping the choice here lets a plain JVM test make it.
 */
internal data class Wording(val text: Int, val numbers: List<Int> = emptyList())

/**
 * The line for a stage.
 *
 * A count only appears once the desktop has said what it wants. Before that there is no total
 * to be part of the way through, and a notification claiming "0 of 0" would be a lie told
 * every time somebody glances at their phone.
 */
internal fun wording(stage: Transfers.Stage): Wording {
    val progress = (stage as? Transfers.Stage.Working)?.progress
        ?: return Wording(R.string.notification_working)
    return Wording(R.string.transfer_progress, listOf(progress.sent, progress.total))
}
