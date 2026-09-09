package top.wxx9248.photosync

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import top.wxx9248.photosync.ui.StartScreen
import top.wxx9248.photosync.ui.StartState

/**
 * The window the application opens on.
 *
 * The state machine that runs a session lives in the `session` module and knows nothing about
 * any of this. What is here is the two taps of `SPEC.md` §3.4 and the wiring that starts the
 * foreground service, so that a transfer survives the screen going off.
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                Surface {
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
        }
    }
}
