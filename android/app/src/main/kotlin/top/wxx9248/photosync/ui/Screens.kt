package top.wxx9248.photosync.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import top.wxx9248.photosync.R

/**
 * What a session looks like to the person holding the phone.
 *
 * `SPEC.md` §3.4 allows exactly two taps in the ordinary case: Start, and the one prompt that
 * frees space afterwards. Everything else happens without asking, which is why these are
 * stateless composables over one immutable state each — rules K16 and K18. Nothing here
 * decides anything; it renders what it is given and reports what was pressed.
 */

/**
 * Step one of §3.1, before anything else can be shown.
 *
 * A person is told what is being asked for and why before the system dialog appears, because
 * the system's own wording says only which permission, never what for. [refused] is what they
 * see if they have already said no: the application cannot carry on, and pretending otherwise
 * would leave them tapping a button that does nothing.
 */
internal data class PermissionState(val refused: Boolean)

@Composable
internal fun PermissionScreen(state: PermissionState, onGrant: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(R.string.permission_title),
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            text = stringResource(
                if (state.refused) R.string.permission_refused else R.string.permission_why
            ),
            style = MaterialTheme.typography.bodyMedium,
        )
        Button(onClick = onGrant, modifier = Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.permission_grant))
        }
    }
}

/** The summary a person is shown before anything moves. §3.4, tap one. */
internal data class StartState(
    val newPhotos: Long,
    val alreadySafe: Long,
    val bytes: Long,
    val busy: Boolean,
    val progress: Float,
)

@Composable
internal fun StartScreen(state: StartState, onStart: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(R.string.start_summary, state.newPhotos, state.alreadySafe),
            style = MaterialTheme.typography.titleMedium,
        )
        if (state.busy) {
            LinearProgressIndicator(
                progress = { state.progress },
                modifier = Modifier.fillMaxWidth(),
            )
        } else {
            Button(onClick = onStart, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.start))
            }
        }
    }
}

/**
 * The single prompt of §8: how much can be freed, split between what came across just now and
 * what an earlier session had already stored. One prompt, whatever the count.
 */
internal data class FreeUpState(
    val fromThisTransfer: Long,
    val fromEarlier: Long,
    val bytes: Long,
)

@Composable
internal fun FreeUpScreen(state: FreeUpState, onConfirm: () -> Unit, onSkip: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(
                R.string.free_up_summary,
                state.fromThisTransfer + state.fromEarlier,
                state.fromThisTransfer,
                state.fromEarlier,
            ),
            style = MaterialTheme.typography.titleMedium,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = onConfirm) { Text(stringResource(R.string.free_up)) }
            OutlinedButton(onClick = onSkip) { Text(stringResource(R.string.keep_them)) }
        }
    }
}
