package top.wxx9248.photosync

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import top.wxx9248.photosync.media.Permissions
import top.wxx9248.photosync.net.FirstMeeting
import top.wxx9248.photosync.net.Identity
import top.wxx9248.photosync.net.Meeting
import top.wxx9248.photosync.pairing.FilePairingStorage
import top.wxx9248.photosync.session.Outcome
import top.wxx9248.photosync.session.Pairing
import top.wxx9248.photosync.ui.DoneScreen
import top.wxx9248.photosync.ui.DoneState
import top.wxx9248.photosync.ui.FreeUpScreen
import top.wxx9248.photosync.ui.FreeUpState
import top.wxx9248.photosync.ui.PairingScreen
import top.wxx9248.photosync.ui.PairingState
import top.wxx9248.photosync.ui.PermissionScreen
import top.wxx9248.photosync.ui.PermissionState
import top.wxx9248.photosync.ui.StartScreen
import top.wxx9248.photosync.ui.WaitingScreen

/**
 * The window the application opens on.
 *
 * The state machine that runs a session lives in the `session` module and knows nothing about
 * any of this. What is here is the order of `SPEC.md` §3.1's onboarding and the two taps of
 * §3.4: permission, then pairing, then Start, then the one prompt about freeing space. No
 * decision about what to send, and no knowledge of the wire.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                // Without this the first line of every screen sits under the status bar, which
                // is where the clock is.
                Surface(modifier = Modifier.safeDrawingPadding()) {
                    Onboarding()
                }
            }
        }
    }

    /** §3.1 in order: nothing further is offered until the step before it is done. */
    @Composable
    private fun Onboarding() {
        var allowed by remember { mutableStateOf(Permissions.canReadTheLibrary(this)) }
        var refused by remember { mutableStateOf(false) }

        val asking = rememberLauncherForActivityResult(
            ActivityResultContracts.RequestMultiplePermissions()
        ) {
            allowed = Permissions.canReadTheLibrary(this)
            refused = !allowed
        }

        if (!allowed) {
            PermissionScreen(PermissionState(refused)) {
                asking.launch(Permissions.missing(this).toTypedArray())
            }
            return
        }

        val pairing = remember { Pairing(FilePairingStorage(SyncService.pairingFile(this))) }
        var paired by remember { mutableStateOf(pairing.known()) }
        if (paired == null) {
            Meeting(onPaired = { paired = pairing.known() }, pairing = pairing)
            return
        }

        Session()
    }

    /** Step five of §3.1, which is where a person compares two screens. */
    @Composable
    private fun Meeting(onPaired: () -> Unit, pairing: Pairing) {
        var looking by remember { mutableStateOf(false) }
        var code by remember { mutableStateOf<String?>(null) }
        var ended by remember { mutableStateOf<Meeting?>(null) }

        LaunchedEffect(looking) {
            if (!looking) return@LaunchedEffect
            val met = FirstMeeting(
                applicationContext,
                Identity.loadOrCreate(),
                android.os.Build.MODEL,
            ).offer { shown -> code = shown }

            if (met is Meeting.Paired) {
                pairing.replace(met.desktop)
                onPaired()
            }
            ended = met
            looking = false
            code = null
        }

        when {
            looking && code == null -> WaitingScreen(R.string.pairing_looking)
            ended is Meeting.NotOpen -> WaitingScreen(R.string.pairing_not_open) {
                ended = null
                looking = true
            }
            ended is Meeting.NoDesktop -> WaitingScreen(R.string.no_desktop) {
                ended = null
                looking = true
            }
            else -> PairingScreen(PairingState(code, ended is Meeting.Refused)) {
                ended = null
                looking = true
            }
        }
    }

    /** What §3.4 offers once there is a desktop to talk to. */
    @Composable
    private fun Session() {
        val stage by Transfers.stage.collectAsState()
        when (val here = stage) {
            Transfers.Stage.Idle -> StartScreen { SyncService.start(this) }
            Transfers.Stage.Working -> WaitingScreen(R.string.transfer_working)
            Transfers.Stage.NoDesktop -> WaitingScreen(R.string.no_desktop) { Transfers.idle() }
            is Transfers.Stage.Asking -> FreeUpScreen(
                FreeUpState(
                    fromThisTransfer = here.freeUp.fromThisTransfer.toLong(),
                    fromEarlier = here.freeUp.fromEarlier.toLong(),
                    bytes = here.freeUp.bytes,
                ),
                onConfirm = Transfers::freeUp,
                onSkip = Transfers::keepThem,
            )
            is Transfers.Stage.Finished -> Done(here.outcome)
        }
    }

    @Composable
    private fun Done(outcome: Outcome) {
        when (outcome) {
            is Outcome.Finished -> DoneScreen(
                DoneState(sent = outcome.sent, freed = outcome.deleted, kept = outcome.kept)
            ) { Transfers.idle() }
            // A refusal is the desktop saying not now --- finishing a previous import, or out
            // of room. §6 has the phone try again rather than treat it as broken.
            is Outcome.Refused -> WaitingScreen(R.string.no_desktop) { Transfers.idle() }
        }
    }
}
