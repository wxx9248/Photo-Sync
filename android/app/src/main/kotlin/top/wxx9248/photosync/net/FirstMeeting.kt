package top.wxx9248.photosync.net

import android.content.Context
import kotlinx.coroutines.coroutineScope
import top.wxx9248.photosync.session.DeviceId
import top.wxx9248.photosync.session.PairedDesktop

/**
 * The one time a phone meets a desktop it does not yet know. `SPEC.md` §5.2.
 *
 * Finding the desktop, showing the digits and waiting for the person at the other screen, in
 * the order those happen. What to do with the answer is the caller's: this hands back a
 * desktop and nothing is remembered here.
 */
internal class FirstMeeting(
    private val context: Context,
    private val identity: Identity,
    private val deviceName: String,
) {
    /**
     * Offers this phone to whichever desktop has pairing open.
     *
     * `show` is called with the six digits as soon as they are known, which is before the
     * desktop answers: a person has to be able to compare two screens, and the desktop does
     * not answer until they have.
     */
    suspend fun offer(show: (String) -> Unit): Meeting {
        val address = Discovery(context).find() ?: return Meeting.NoDesktop

        return coroutineScope {
            GrpcPairing.connect(address, identity, this).use { meeting ->
                val code = meeting.offer(DeviceId(identity.publicKeyPin().take(16)), deviceName)
                if (code == null) {
                    Meeting.NotOpen
                } else {
                    show(code)
                    meeting.settled()
                }
            }
        }
    }
}

/**
 * How an attempt at §5.2 ended.
 *
 * A desktop that never answered and a desktop that said no are different things to a person
 * standing there: one sends them to look at the computer, the other tells them it is on and
 * has refused. Collapsing both into "no" sends half of them to the wrong place.
 */
internal sealed interface Meeting {
    /** Nothing was advertising, which §5.1 treats as "is the computer on?" */
    data object NoDesktop : Meeting

    /** A desktop answered and the person at it declined. */
    data object Refused : Meeting

    /**
     * A desktop is there and would not talk to a phone it does not know.
     *
     * §5.2 only accepts an unknown key while somebody has opened pairing at the desktop, so
     * this is nearly always "nobody has opened it yet" rather than anything wrong.
     */
    data object NotOpen : Meeting

    data class Paired(val desktop: PairedDesktop) : Meeting
}
