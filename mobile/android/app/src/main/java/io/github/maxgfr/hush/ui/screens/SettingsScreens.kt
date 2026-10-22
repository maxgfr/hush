package io.github.maxgfr.hush.ui.screens

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import io.github.maxgfr.hush.R
import io.github.maxgfr.hush.core.Format
import io.github.maxgfr.hush.core.Settings
import io.github.maxgfr.hush.core.UiState
import io.github.maxgfr.hush.ui.MainViewModel
import io.github.maxgfr.hush.ui.components.Choices
import io.github.maxgfr.hush.ui.components.HIcon
import io.github.maxgfr.hush.ui.components.Hint
import io.github.maxgfr.hush.ui.components.Page
import io.github.maxgfr.hush.ui.components.QrCode
import io.github.maxgfr.hush.ui.components.Section
import io.github.maxgfr.hush.ui.components.SwitchRow
import io.github.maxgfr.hush.ui.components.copySensitive
import kotlinx.coroutines.delay
import uniffi.hush_ffi.VaultStatus

@Composable
private fun NameField(vm: MainViewModel, settings: Settings) {
    var name by rememberSaveable { mutableStateOf(settings.name) }
    OutlinedTextField(
        value = name,
        onValueChange = {
            name = it.take(64)
            vm.updateSettings { s -> s.copy(name = name) }
        },
        label = { Text(stringResource(R.string.display_name)) },
        singleLine = true,
        modifier = Modifier.fillMaxWidth().testTag("name"),
    )
    Hint(stringResource(R.string.display_name_hint))
}

@Composable
fun IdentityScreen(vm: MainViewModel, state: UiState, settings: Settings) {
    val context = LocalContext.current
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(copied) {
        if (copied) {
            delay(1_500)
            copied = false
        }
    }
    Page(stringResource(R.string.my_id), onBack = vm::back) {
        val me = state.identity
        when {
            state.vault != VaultStatus.UNLOCKED -> VaultGate(vm, state)
            me == null -> Section {
                Hint(stringResource(R.string.id_intro))
                Button(onClick = vm::createIdentity, modifier = Modifier.fillMaxWidth().height(52.dp)) {
                    HIcon(R.drawable.ic_key)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.id_create))
                }
            }
            else -> {
                Section {
                    Hint(stringResource(R.string.id_intro))
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { QrCode(me) }
                    // Not selectable: the system's copy would skip the sensitive flag.
                    Text(me, fontFamily = FontFamily.Monospace, modifier = Modifier.testTag("my-id"))
                    Button(onClick = {
                        copySensitive(context, "hush", me)
                        copied = true
                    }) {
                        HIcon(if (copied) R.drawable.ic_check else R.drawable.ic_copy)
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(if (copied) R.string.copied else R.string.copy))
                    }
                }
                Section {
                    SwitchRow(
                        stringResource(R.string.reachable),
                        stringResource(R.string.reachable_hint),
                        state.reachable,
                        vm::setReachable,
                        tag = "reachable",
                    )
                }
            }
        }
        Section { NameField(vm, settings) }
    }
}

@Composable
fun SettingsScreen(vm: MainViewModel, settings: Settings) {
    val off = stringResource(R.string.timer_off)
    var relays by rememberSaveable { mutableStateOf(settings.relays.joinToString("\n")) }
    var trackers by rememberSaveable { mutableStateOf(settings.trackers.joinToString("\n")) }
    var stun by rememberSaveable { mutableStateOf(settings.stun.joinToString("\n")) }
    var turnUrl by rememberSaveable { mutableStateOf(settings.turnUrl) }
    var turnUser by rememberSaveable { mutableStateOf(settings.turnUser) }
    var turnCredential by remember { mutableStateOf(settings.turnCredential) }
    var saved by remember { mutableStateOf(false) }
    LaunchedEffect(saved) {
        if (saved) {
            delay(1_500)
            saved = false
        }
    }
    Page(stringResource(R.string.settings), onBack = vm::back) {
        Section {
            NameField(vm, settings)
            Text(stringResource(R.string.message_timer), style = MaterialTheme.typography.titleSmall)
            Choices(Settings.TTL_SECONDS, settings.ttl, { Format.ttl(it, off) }) { t ->
                vm.updateSettings { it.copy(ttl = t) }
            }
        }
        Section {
            SwitchRow(
                stringResource(R.string.nearby),
                stringResource(R.string.nearby_hint),
                settings.nearby,
                { on -> vm.updateSettings { it.copy(nearby = on) } },
                tag = "nearby",
            )
        }
        Section {
            Text(stringResource(R.string.network), style = MaterialTheme.typography.titleMedium)
            Hint(stringResource(R.string.network_hint))
            Lines(stringResource(R.string.relays), relays) { relays = it }
            Lines(stringResource(R.string.trackers), trackers) { trackers = it }
            Lines(stringResource(R.string.stun), stun) { stun = it }
            Text(stringResource(R.string.turn), style = MaterialTheme.typography.titleSmall)
            OutlinedTextField(
                value = turnUrl,
                onValueChange = { turnUrl = it },
                placeholder = { Text("turn:turn.example.org:3478") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = turnUser,
                onValueChange = { turnUser = it },
                label = { Text(stringResource(R.string.turn_user)) },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = turnCredential,
                onValueChange = { turnCredential = it },
                label = { Text(stringResource(R.string.turn_credential)) },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                modifier = Modifier.fillMaxWidth(),
            )
            Hint(stringResource(R.string.turn_note))
            SwitchRow(stringResource(R.string.relay_only), null, settings.relayOnly, { on ->
                vm.updateSettings { it.copy(relayOnly = on) }
            })
            Row {
                Button(onClick = {
                    vm.updateSettings {
                        it.copy(
                            relays = Settings.lines(relays, "wss://"),
                            trackers = Settings.lines(trackers, "wss://"),
                            stun = Settings.lines(stun, "stun:", "stuns:"),
                            turnUrl = turnUrl.trim(),
                            turnUser = turnUser,
                            turnCredential = turnCredential,
                        )
                    }
                    saved = true
                }) { Text(stringResource(if (saved) R.string.saved else R.string.save)) }
                Spacer(Modifier.width(8.dp))
                OutlinedButton(onClick = {
                    val d = vm.defaults
                    relays = d.relays.joinToString("\n")
                    trackers = d.trackers.joinToString("\n")
                    stun = d.stun.joinToString("\n")
                }) { Text(stringResource(R.string.reset)) }
            }
        }
    }
}

@Composable
private fun Lines(label: String, value: String, onChange: (String) -> Unit) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        minLines = 2,
        textStyle = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
        modifier = Modifier.fillMaxWidth(),
    )
}
