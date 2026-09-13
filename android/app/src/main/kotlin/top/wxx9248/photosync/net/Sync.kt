package top.wxx9248.photosync.net

import android.content.Context
import kotlinx.coroutines.coroutineScope
import top.wxx9248.photosync.media.MediaStoreLibrary
import top.wxx9248.photosync.session.DeviceId
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.FreeUp
import top.wxx9248.photosync.session.Outcome
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
        remove: suspend (List<DevicePath>) -> Set<DevicePath>,
    ): Outcome? {
        val address = Discovery(context).find() ?: return null

        val library = MediaStoreLibrary(context.contentResolver)
        // Enumerated once, here, and frozen for the whole session including any reconnect.
        // §3.2.
        val catalog = library.catalog()

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
                        session.freed(if (cleared.isEmpty()) emptySet() else remove(cleared))
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
