package top.wxx9248.photosync

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import top.wxx9248.photosync.media.Permissions
import top.wxx9248.photosync.ui.PermissionScreen
import top.wxx9248.photosync.ui.PermissionState
import top.wxx9248.photosync.ui.StartScreen
import top.wxx9248.photosync.ui.StartState

/**
 * The window the application opens on.
 *
 * The state machine that runs a session lives in the `session` module and knows nothing about
 * any of this. What is here is the onboarding of `SPEC.md` §3.1 and the two taps of §3.4, and
 * nothing else: no decision about what to send, and no knowledge of the wire.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                // Without this the first line of every screen sits under the status bar, which
                // is where the clock is.
                Surface(modifier = Modifier.safeDrawingPadding()) {
                    // Asked once and re-read when the answer comes back, so a person who
                    // grants from the system dialog sees the next screen rather than the same
                    // one. §3.1 calls this a sequential flow.
                    var allowed by remember { mutableStateOf(Permissions.canReadTheLibrary(this)) }
                    var refused by remember { mutableStateOf(false) }

                    val asking = rememberLauncherForActivityResult(
                        ActivityResultContracts.RequestMultiplePermissions()
                    ) {
                        allowed = Permissions.canReadTheLibrary(this)
                        refused = !allowed
                    }

                    if (allowed) {
                        Started()
                    } else {
                        PermissionScreen(PermissionState(refused)) {
                            asking.launch(Permissions.missing(this).toTypedArray())
                        }
                    }
                }
            }
        }
    }

    @androidx.compose.runtime.Composable
    private fun Started() {
        // The counts stay at zero until a session has run: they come from the desktop's diff,
        // and nothing has asked one yet. Wiring the service is the next thing this needs.
        var state by remember {
            mutableStateOf(
                StartState(
                    newPhotos = 0,
                    alreadySafe = 0,
                    bytes = 0,
                    busy = false,
                    progress = 0f,
                )
            )
        }
        StartScreen(state) {
            state = state.copy(busy = true)
            SyncService.start(this@MainActivity)
        }
    }
}
