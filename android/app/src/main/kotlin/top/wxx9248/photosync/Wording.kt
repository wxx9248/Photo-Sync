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
internal fun wording(stage: Transfers.Stage): Wording = when (stage) {
    is Transfers.Stage.Working -> stage.progress?.let { progress ->
        Wording(R.string.transfer_progress, listOf(progress.sent, progress.total))
    } ?: Wording(R.string.notification_working)

    // Freeing space up is two phases and neither of them is moving photographs to the
    // computer, which is what this line used to claim through both of them.
    is Transfers.Stage.Checking -> Wording(R.string.checking, listOf(stage.photographs))
    is Transfers.Stage.Removing -> Wording(R.string.removing, listOf(stage.paths.size))

    Transfers.Stage.Idle, Transfers.Stage.NoDesktop -> Wording(R.string.notification_working)
    is Transfers.Stage.Asking -> Wording(R.string.notification_asking)
    is Transfers.Stage.Finished -> Wording(R.string.notification_working)
}
