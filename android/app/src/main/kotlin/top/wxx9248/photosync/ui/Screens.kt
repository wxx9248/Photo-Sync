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

/**
 * Step two of §3.1: special access, which only a person can turn on.
 *
 * Offered once and skippable, because §8 has a fallback and a household that would rather
 * confirm each file is entitled to. Skippable is not the same as unasked: without it every
 * delete of a photograph this application did not take is refused, which is the whole of what
 * somebody pressed the button for.
 */
@Composable
internal fun ManageMediaScreen(onAllow: () -> Unit, onSkip: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(R.string.manage_title),
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            text = stringResource(R.string.manage_why),
            style = MaterialTheme.typography.bodyMedium,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = onAllow) { Text(stringResource(R.string.manage_grant)) }
            OutlinedButton(onClick = onSkip) { Text(stringResource(R.string.manage_skip)) }
        }
    }
}

/** Step three of §3.1: the exemption Android itself offers. */
@Composable
internal fun BatteryScreen(onAllow: () -> Unit, onSkip: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(R.string.battery_title),
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            text = stringResource(R.string.battery_why),
            style = MaterialTheme.typography.bodyMedium,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = onAllow) { Text(stringResource(R.string.battery_grant)) }
            OutlinedButton(onClick = onSkip) { Text(stringResource(R.string.manage_skip)) }
        }
    }
}

/**
 * Step four of §3.1: the settings only this phone's manufacturer knows about.
 *
 * Nothing here can be granted by an application. All it can do is say where the settings are,
 * in this phone's own words, and open the page they live behind.
 */
@Composable
internal fun KeepAliveScreen(brand: String, steps: Int, onOpen: () -> Unit, onDone: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(R.string.keepalive_title),
            style = MaterialTheme.typography.titleMedium,
        )
        Text(
            text = stringResource(R.string.keepalive_intro, brand),
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(text = stringResource(steps), style = MaterialTheme.typography.bodyMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(onClick = onOpen) { Text(stringResource(R.string.keepalive_open)) }
            OutlinedButton(onClick = onDone) { Text(stringResource(R.string.keepalive_done)) }
        }
    }
}

/**
 * Step five of §3.1: the six digits, on the phone's side of the comparison.
 *
 * [code] is null until the handshake has happened, and [refused] means the person at the
 * desktop said no --- which is an answer rather than a fault, and reads better as one.
 */
internal data class PairingState(val code: String?, val refused: Boolean)

@Composable
internal fun PairingScreen(state: PairingState, onLook: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(R.string.pairing_title),
            style = MaterialTheme.typography.titleMedium,
        )
        when {
            state.code != null -> {
                Text(text = state.code, style = MaterialTheme.typography.displaySmall)
                Text(
                    text = stringResource(R.string.pairing_compare),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
            state.refused -> {
                Text(
                    text = stringResource(R.string.pairing_refused),
                    style = MaterialTheme.typography.bodyMedium,
                )
                Button(onClick = onLook, modifier = Modifier.fillMaxWidth()) {
                    Text(stringResource(R.string.pair))
                }
            }
            else -> Button(onClick = onLook, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.pair))
            }
        }
    }
}

/** What a session came to, once there is nothing left to wait for. §3.4. */
internal data class DoneState(
    val sent: Long,
    val freed: Long,
    val kept: Long,
    val failed: Long,
)

@Composable
internal fun DoneScreen(state: DoneState, onAgain: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(R.string.transfer_done, state.sent, state.freed, state.kept),
            style = MaterialTheme.typography.titleMedium,
        )
        // Only when there were any. §8 counts a photograph the platform would not remove as a
        // failure, and a summary that leaves them out reads as though nothing happened.
        if (state.failed > 0) {
            Text(
                text = stringResource(R.string.transfer_failed, state.failed),
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        Button(onClick = onAgain, modifier = Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.again))
        }
    }
}

/**
 * What a transfer looks like while it runs. §3.4 shows progress rather than a spinner.
 *
 * Counted in photographs, because that is the unit a person has in mind. Until the desktop has
 * said what it wants there is no total, and a bar with no end is more honest as a line of
 * text than as a bar pretending to fill.
 */
@Composable
internal fun WorkingScreen(sent: Int?, total: Int?) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        if (sent == null || total == null || total == 0) {
            Text(
                text = stringResource(R.string.transfer_working),
                style = MaterialTheme.typography.titleMedium,
            )
            return@Column
        }
        Text(
            text = stringResource(R.string.transfer_progress, sent, total),
            style = MaterialTheme.typography.titleMedium,
        )
        LinearProgressIndicator(
            progress = { sent.toFloat() / total.toFloat() },
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

/** One line, for the states that are only waiting. */
@Composable
internal fun WaitingScreen(message: Int, onAgain: (() -> Unit)? = null) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(text = stringResource(message), style = MaterialTheme.typography.titleMedium)
        if (onAgain != null) {
            Button(onClick = onAgain, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.again))
            }
        }
    }
}

/**
 * Tap one of §3.4.
 *
 * It says nothing about how many photographs are new, because before a session the phone does
 * not know: that is the desktop's diff, and asking for one is what this button does. A count
 * here would be a number the phone made up.
 */
@Composable
internal fun StartScreen(onStart: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = stringResource(R.string.start_ready),
            style = MaterialTheme.typography.titleMedium,
        )
        Button(onClick = onStart, modifier = Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.start))
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
