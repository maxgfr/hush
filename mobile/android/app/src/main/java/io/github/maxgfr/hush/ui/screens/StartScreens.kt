package io.github.maxgfr.hush.ui.screens

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
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import io.github.maxgfr.hush.R
import io.github.maxgfr.hush.core.Format
import io.github.maxgfr.hush.core.Settings
import io.github.maxgfr.hush.ui.MainViewModel
import io.github.maxgfr.hush.ui.components.Choices
import io.github.maxgfr.hush.ui.components.ErrorText
import io.github.maxgfr.hush.ui.components.HIcon
import io.github.maxgfr.hush.ui.components.Hint
import io.github.maxgfr.hush.ui.components.Page
import io.github.maxgfr.hush.ui.components.QrScanner
import io.github.maxgfr.hush.ui.components.Section

@Composable
fun NewConversationScreen(vm: MainViewModel, settings: Settings) {
    var group by rememberSaveable { mutableStateOf(false) }
    var ttlMin by rememberSaveable { mutableLongStateOf(60L) }
    var ttl by rememberSaveable { mutableLongStateOf(settings.ttl) }
    var pass by rememberSaveable { mutableStateOf("") }
    val off = stringResource(R.string.timer_off)
    Page(stringResource(R.string.new_conversation), onBack = vm::back) {
        Section {
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                SegmentedButton(
                    selected = !group,
                    onClick = { group = false },
                    shape = SegmentedButtonDefaults.itemShape(0, 2),
                ) { Text(stringResource(R.string.kind_direct)) }
                SegmentedButton(
                    selected = group,
                    onClick = { group = true },
                    shape = SegmentedButtonDefaults.itemShape(1, 2),
                ) { Text(stringResource(R.string.kind_group)) }
            }
            Hint(stringResource(if (group) R.string.kind_group_hint else R.string.kind_direct_hint))

            Text(stringResource(R.string.link_expiry), style = MaterialTheme.typography.titleSmall)
            Choices(Settings.LINK_MINUTES, ttlMin, { Format.duration(it * 60_000) }) { ttlMin = it }

            Text(stringResource(R.string.message_timer), style = MaterialTheme.typography.titleSmall)
            Choices(Settings.TTL_SECONDS, ttl, { Format.ttl(it, off) }) { ttl = it }

            OutlinedTextField(
                value = pass,
                onValueChange = { pass = it },
                label = { Text(stringResource(R.string.passphrase)) },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                modifier = Modifier.fillMaxWidth(),
            )
            Hint(stringResource(R.string.passphrase_hint))

            Button(
                onClick = { vm.createLink(group, ttlMin, pass, ttl) },
                enabled = !vm.busy,
                modifier = Modifier.fillMaxWidth().height(52.dp),
            ) {
                Text(stringResource(if (vm.busy && pass.isNotEmpty()) R.string.deriving else R.string.create_link))
            }
        }
    }
}

@Composable
fun JoinScreen(vm: MainViewModel, error: Int?) {
    var value by rememberSaveable { mutableStateOf("") }
    var scanning by rememberSaveable { mutableStateOf(false) }
    Page(stringResource(R.string.join_with_link), onBack = vm::back) {
        Section {
            if (scanning) {
                QrScanner(
                    onResult = {
                        scanning = false
                        vm.openLink(it)
                    },
                    onCancel = { scanning = false },
                )
            } else {
                OutlinedTextField(
                    value = value,
                    onValueChange = { value = it },
                    label = { Text(stringResource(R.string.paste_link)) },
                    placeholder = { Text("https://maxgfr.github.io/hush/#2.s.…") },
                    minLines = 3,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
                    modifier = Modifier.fillMaxWidth(),
                )
                error?.let { ErrorText(stringResource(it)) }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Button(onClick = { vm.openLink(value) }, enabled = value.isNotBlank() && !vm.busy) {
                        Text(stringResource(R.string.join))
                    }
                    Spacer(Modifier.width(12.dp))
                    OutlinedButton(onClick = { scanning = true }) {
                        HIcon(R.drawable.ic_qr)
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(R.string.scan_qr))
                    }
                }
            }
        }
    }
}

@Composable
fun PassphraseScreen(vm: MainViewModel, link: String) {
    var pass by rememberSaveable { mutableStateOf("") }
    Page(stringResource(R.string.join_with_link), onBack = vm::back) {
        Section {
            Row(verticalAlignment = Alignment.CenterVertically) {
                HIcon(R.drawable.ic_lock)
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.protected_link), style = MaterialTheme.typography.titleMedium)
            }
            Hint(stringResource(R.string.protected_link_hint))
            OutlinedTextField(
                value = pass,
                onValueChange = { pass = it },
                label = { Text(stringResource(R.string.passphrase_label)) },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                modifier = Modifier.fillMaxWidth(),
            )
            Button(
                onClick = { vm.join(link, pass) },
                enabled = pass.isNotEmpty() && !vm.busy,
                modifier = Modifier.fillMaxWidth().height(52.dp),
            ) {
                Text(stringResource(if (vm.busy) R.string.deriving else R.string.join))
            }
        }
    }
}
