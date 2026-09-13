package top.wxx9248.photosync

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.IntentSenderRequest
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withContext
import top.wxx9248.photosync.media.Deletions
import top.wxx9248.photosync.media.KeepAlive
import top.wxx9248.photosync.media.MediaStoreLibrary
import top.wxx9248.photosync.media.Permissions
import top.wxx9248.photosync.net.FirstMeeting
import top.wxx9248.photosync.net.Identity
import top.wxx9248.photosync.net.Meeting
import top.wxx9248.photosync.pairing.FilePairingStorage
import top.wxx9248.photosync.session.DevicePath
import top.wxx9248.photosync.session.Outcome
import top.wxx9248.photosync.session.Pairing
import top.wxx9248.photosync.session.RejectReason
import top.wxx9248.photosync.ui.DoneScreen
import top.wxx9248.photosync.ui.DoneState
import top.wxx9248.photosync.ui.FreeUpScreen
import top.wxx9248.photosync.ui.FreeUpState
import top.wxx9248.photosync.ui.PairingScreen
import top.wxx9248.photosync.ui.PairingState
import top.wxx9248.photosync.ui.BatteryScreen
import top.wxx9248.photosync.ui.KeepAliveScreen
import top.wxx9248.photosync.ui.ManageMediaScreen
import top.wxx9248.photosync.ui.PermissionScreen
import top.wxx9248.photosync.ui.PermissionState
import top.wxx9248.photosync.ui.StartScreen
import top.wxx9248.photosync.ui.WaitingScreen
import top.wxx9248.photosync.ui.WorkingScreen

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

    /**
     * What a person has already been told, so they are not told it again.
     *
     * §3.5 keeps the pairing credential and nothing about a transfer. This is neither: it is
     * what the application has said to the person holding it, and forgetting that would mean
     * showing the same page of manufacturer settings on every launch.
     */
    private val onboarding by lazy { Onboarding(this) }

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

        // Step two: special access, offered once. Skipping it is a decision a person is
        // allowed to make, so it is remembered for this run rather than asked again on every
        // recomposition.
        var askedToManage by remember {
            mutableStateOf(
                Permissions.canManageMedia(this) ||
                    onboarding.asked(Onboarding.Step.MEDIA_MANAGEMENT)
            )
        }
        val managing = rememberLauncherForActivityResult(
            ActivityResultContracts.StartActivityForResult()
        ) {
            onboarding.remember(Onboarding.Step.MEDIA_MANAGEMENT)
            askedToManage = true
        }

        if (!askedToManage) {
            ManageMediaScreen(
                onAllow = { managing.launch(Permissions.manageMediaRequest(this)) },
                onSkip = {
                    onboarding.remember(Onboarding.Step.MEDIA_MANAGEMENT)
                    askedToManage = true
                },
            )
            return
        }

        // Step three: the exemption Android itself offers, which is the only part of §3.3 an
        // application can ask for.
        var askedToStayAwake by remember {
            mutableStateOf(
                KeepAlive.exempt(this) || onboarding.asked(Onboarding.Step.BATTERY_EXEMPTION)
            )
        }
        val exempting = rememberLauncherForActivityResult(
            ActivityResultContracts.StartActivityForResult()
        ) {
            onboarding.remember(Onboarding.Step.BATTERY_EXEMPTION)
            askedToStayAwake = true
        }

        if (!askedToStayAwake) {
            BatteryScreen(
                onAllow = { exempting.launch(KeepAlive.exemptionRequest(this)) },
                onSkip = {
                    onboarding.remember(Onboarding.Step.BATTERY_EXEMPTION)
                    askedToStayAwake = true
                },
            )
            return
        }

        // Step four: the settings only this phone's manufacturer knows about. Nothing can be
        // granted here, so it is shown once and remembered --- a person who has read it should
        // not be shown it again every time they open the application.
        val steps = remember { KeepAlive.brandSteps(this) }
        var readTheSteps by remember {
            mutableStateOf(onboarding.asked(Onboarding.Step.BRAND_STEPS))
        }
        if (steps != null && !readTheSteps) {
            KeepAliveScreen(
                brand = android.os.Build.MANUFACTURER,
                steps = steps,
                onOpen = { startActivity(KeepAlive.settings(this)) },
                onDone = {
                    onboarding.remember(Onboarding.Step.BRAND_STEPS)
                    readTheSteps = true
                },
            )
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
            is Transfers.Stage.Working ->
                WorkingScreen(here.progress?.sent, here.progress?.total)
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
            is Transfers.Stage.Removing -> Removing(here.paths)
            is Transfers.Stage.Finished -> Done(here.outcome)
        }
    }

    /**
     * Asks the platform to remove what §8's gates cleared.
     *
     * This is an activity's job and nothing else's: a phone cannot delete a photograph it did
     * not take without a request the system puts to a person. With media management granted
     * they see nothing and it happens; without it they confirm, which §8 calls the fallback.
     *
     * What went is read back from the media store rather than assumed from the result code,
     * because a person may have let some through and not others.
     */
    @Composable
    private fun Removing(paths: List<DevicePath>) {
        val answered = remember { Channel<Boolean>(Channel.UNLIMITED) }
        val asking = rememberLauncherForActivityResult(
            ActivityResultContracts.StartIntentSenderForResult()
        ) { answered.trySend(it.resultCode == RESULT_OK) }

        LaunchedEffect(paths) {
            val library = MediaStoreLibrary(contentResolver)
            val declined = mutableSetOf<DevicePath>()

            // Batched, because a binder transaction has a size limit a few thousand
            // identifiers would exceed. §8. The batch is kept beside its answer, since that
            // is the granularity a person answers at.
            for (batch in paths.chunked(Deletions.BATCH)) {
                // Each of these is a media-store query per photograph, and five hundred of
                // them on the main thread is how a phone stops answering. Rule K11: the
                // suspending work chooses its own thread; only the launcher needs this one.
                val request = withContext(Dispatchers.IO) {
                    Deletions.request(contentResolver, library.uris(batch))
                }
                asking.launch(IntentSenderRequest.Builder(request.intentSender).build())
                if (!answered.receive()) {
                    declined.addAll(batch)
                }
            }
            Transfers.removed(withContext(Dispatchers.IO) { library.gone(paths) }, declined)
        }

        WaitingScreen(R.string.transfer_working)
    }

    @Composable
    private fun Done(outcome: Outcome) {
        when (outcome) {
            is Outcome.Finished -> DoneScreen(
                DoneState(
                    sent = outcome.sent,
                    freed = outcome.deleted,
                    kept = outcome.kept,
                    failed = outcome.failed,
                )
            ) { Transfers.idle() }
            // Each refusal names something different to do about it. They used to share the
            // "is the computer on?" wording, which was only ever right for one of them.
            is Outcome.Refused -> WaitingScreen(
                when (outcome.reason) {
                    RejectReason.UNREACHABLE -> R.string.no_desktop
                    RejectReason.COMMIT_IN_PROGRESS -> R.string.refused_busy
                    RejectReason.NO_SPACE -> R.string.refused_space
                    RejectReason.NOT_PAIRED -> R.string.refused_unpaired
                    RejectReason.PROTOCOL_VERSION -> R.string.refused_version
                }
            ) { Transfers.idle() }
        }
    }
}
