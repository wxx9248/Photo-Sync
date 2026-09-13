package top.wxx9248.photosync.net

import android.content.Context
import android.util.Log
import kotlinx.coroutines.coroutineScope
import top.wxx9248.photosync.media.MediaStoreLibrary
import top.wxx9248.photosync.session.DeviceId
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.FreeUp
import top.wxx9248.photosync.session.Outcome
import top.wxx9248.photosync.session.RejectReason
import top.wxx9248.photosync.session.PairedDesktop
import top.wxx9248.photosync.session.Session

/**
 * One session, from finding the desktop to the summary.
 *
 * Everything that decides anything is in the `session` module and everything that touches the
 * platform is in the classes this uses. What is left here is the order the two go in, which is
 * short enough to read in one sitting — and that is the point of the split.
 */
internal class Sync(
    private val context: Context,
    private val identity: Identity,
    private val deviceName: String,
) {
    /**
     * Runs a session against the desktop this phone is paired with.
     *
     * `ask` is §8's one prompt, answered true to free the space up, and `remove` carries out
     * what its gates cleared. Both are passed in rather than reached for: the person answering
     * is in front of a screen this class knows nothing about, removing a photograph is a
     * request the platform puts to them, and a session driven by a test needs neither.
     *
     * Returns null when no desktop answered, which §5.1 treats as "is the computer on?"
     * rather than as a failure.
     */
    suspend fun run(
        paired: PairedDesktop,
        ask: suspend (FreeUp) -> Boolean,
        remove: suspend (List<DevicePath>) -> Removal,
    ): Outcome? {
        val library = MediaStoreLibrary(context.contentResolver)
        // Enumerated once, here, and frozen for the whole session including any reconnect.
        // §3.2.
        val catalog = library.catalog()

        // §6 rejoins a dropped connection by running the handshake again with that same
        // catalog: the desktop re-diffs, what already arrived drops out, and the work can only
        // shrink. Nothing is resumed from the phone's side --- a fresh session asks the
        // desktop what it still needs, which is the same code path that sent the first
        // photograph.
        repeat(ATTEMPTS) { attempt ->
            val outcome = attempt(paired, library, catalog, ask, remove)
            if (outcome !is Outcome.Refused || outcome.reason != RejectReason.UNREACHABLE) {
                return outcome
            }
            Log.i(TAG, "the connection did not survive; rejoining (${attempt + 1}/$ATTEMPTS)")
        }
        return null
    }

    private suspend fun attempt(
        paired: PairedDesktop,
        library: MediaStoreLibrary,
        catalog: top.wxx9248.photosync.session.Catalog,
        ask: suspend (FreeUp) -> Boolean,
        remove: suspend (List<DevicePath>) -> Removal,
    ): Outcome? {
        val address = Discovery(context).find() ?: return null

        val session = Session(
            device = DeviceId(identity.publicKeyPin().take(16)),
            name = deviceName,
            catalog = catalog,
            library = library,
        )

        // The upload of a file spans many exchanges, so it runs in this scope rather than
        // inside one of them. Leaving here waits for it, and closing the desktop ends it.
        coroutineScope {
            GrpcDesktop.connect(address, identity, paired.publicKey, this).use { desktop ->
                while (session.outcome == null) {
                    val waiting = session.asking
                    if (waiting != null) {
                        // Nothing crosses the wire while a person decides, and nothing is
                        // deleted until they have. §8.
                        if (ask(waiting)) session.freeUp() else session.keepThem()
                        continue
                    }
                    val cleared = session.freeing
                    if (cleared != null) {
                        val carried =
                            if (cleared.isEmpty()) Removal.NOTHING else remove(cleared)
                        session.freed(carried.gone, carried.declined)
                        continue
                    }
                    val said = session.next() ?: break
                    desktop.say(said)?.let { session.receive(it) }
                }
            }
        }
        return session.outcome
    }
}

private const val TAG = "PhotoSyncSession"

/**
 * How many times a dropped connection is rejoined before giving up.
 *
 * §6 does not put a number on it. Three is enough to ride out a lift or a microwave and few
 * enough that a desktop which has actually gone away stops being dialled.
 */
private const val ATTEMPTS = 3

/** What became of a removal: what went, and what a person turned down. §8 tells them apart. */
internal interface Removal {
    val gone: Set<DevicePath>
    val declined: Set<DevicePath>

    companion object {
        val NOTHING: Removal = object : Removal {
            override val gone: Set<DevicePath> = emptySet()
            override val declined: Set<DevicePath> = emptySet()
        }
    }
}
