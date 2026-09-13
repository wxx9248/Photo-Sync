package top.wxx9248.photosync

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.FreeUp
import top.wxx9248.photosync.session.Outcome

/**
 * What the session is doing, for the screen that is not running it.
 *
 * The transfer lives in a foreground service because `SPEC.md` §3.3 needs it to survive the
 * screen going off, and the person answering §8's prompt is in an activity. Those are two
 * entry points into one process with one session between them, and this is the one place that
 * session is visible from both. Rule 4.4 warns about state like this; the alternative on
 * Android is a bound service, which is the same state with a harder way to reach it.
 *
 * Nothing here decides anything. The service publishes, the screen renders and answers.
 */
internal object Transfers {
    private val current = MutableStateFlow<Stage>(Stage.Idle)

    /** Where the session has got to. */
    val stage: StateFlow<Stage> = current.asStateFlow()

    private var answering: CompletableDeferred<Boolean>? = null

    sealed interface Stage {
        /** Nothing is running. */
        data object Idle : Stage

        /** A desktop is being looked for, or photographs are crossing. */
        data object Working : Stage

        /** No desktop answered, which §5.1 treats as "is the computer on?" */
        data object NoDesktop : Stage

        /** §8's one prompt, waiting on a person. */
        data class Asking(val freeUp: FreeUp) : Stage

        /**
         * §8's gates cleared these, and the platform has to be asked to remove them.
         *
         * A phone cannot delete a photograph it did not take without putting the request to a
         * person, and only an activity can do that, so the service waits here.
         */
        data class Removing(val paths: List<DevicePath>) : Stage

        /** The session ended, one way or another. */
        data class Finished(val outcome: Outcome) : Stage
    }

    fun working() {
        current.value = Stage.Working
    }

    fun noDesktop() {
        current.value = Stage.NoDesktop
    }

    fun finished(outcome: Outcome) {
        current.value = Stage.Finished(outcome)
    }

    fun idle() {
        current.value = Stage.Idle
    }

    /**
     * Puts the prompt on screen and waits for an answer.
     *
     * Suspends until somebody answers, which is what §8 asks for: the session stops here
     * rather than guessing.
     */
    suspend fun ask(freeUp: FreeUp): Boolean {
        val answer = CompletableDeferred<Boolean>()
        answering = answer
        current.value = Stage.Asking(freeUp)
        val said = answer.await()
        current.value = Stage.Working
        return said
    }

    /** The person said yes. */
    fun freeUp() {
        answering?.complete(true)
    }

    /** The person said no. */
    fun keepThem() {
        answering?.complete(false)
    }

    private var removing: CompletableDeferred<Set<DevicePath>>? = null

    /** Puts the removal in front of whoever can carry it out, and waits. */
    suspend fun remove(paths: List<DevicePath>): Set<DevicePath> {
        val carried = CompletableDeferred<Set<DevicePath>>()
        removing = carried
        current.value = Stage.Removing(paths)
        val gone = carried.await()
        current.value = Stage.Working
        return gone
    }

    /** What actually went, as the screen found it afterwards. */
    fun removed(gone: Set<DevicePath>) {
        removing?.complete(gone)
    }
}
