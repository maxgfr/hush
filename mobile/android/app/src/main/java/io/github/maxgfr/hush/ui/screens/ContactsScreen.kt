package io.github.maxgfr.hush.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.maxgfr.hush.R
import io.github.maxgfr.hush.core.UiState
import io.github.maxgfr.hush.ui.MainViewModel
import io.github.maxgfr.hush.ui.components.ErrorText
import io.github.maxgfr.hush.ui.components.HIcon
import io.github.maxgfr.hush.ui.components.Hint
import io.github.maxgfr.hush.ui.components.Page
import io.github.maxgfr.hush.ui.components.QrScanner
import io.github.maxgfr.hush.ui.components.Section
import io.github.maxgfr.hush.ui.components.SwitchRow
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import uniffi.hush_ffi.ContactEntry
import uniffi.hush_ffi.LookupStatus
import uniffi.hush_ffi.VaultStatus

@Composable
fun ContactsScreen(vm: MainViewModel, state: UiState) {
    val unlocked = state.vault == VaultStatus.UNLOCKED
    var erase by remember { mutableStateOf(false) }
    Page(
        stringResource(R.string.contacts),
        onBack = vm::back,
        actions = {
            if (unlocked) {
                TextButton(onClick = vm::lockVault) {
                    HIcon(R.drawable.ic_lock)
                    Text(stringResource(R.string.vault_lock))
                }
            }
        },
    ) {
        if (!unlocked) {
            VaultGate(vm, state)
            return@Page
        }
        if (state.requests.isNotEmpty()) {
            Section {
                Text(stringResource(R.string.requests), style = MaterialTheme.typography.titleMedium)
                state.requests.forEach { r ->
                    Text(stringResource(R.string.wants_to_connect, r.name.ifBlank { "?" }, r.id.take(14) + "…"))
                    if (r.note.isNotBlank()) Hint(r.note)
                    Row {
                        Button(onClick = { vm.accept(r.id) }) { Text(stringResource(R.string.accept)) }
                        Spacer(Modifier.width(8.dp))
                        TextButton(onClick = { vm.decline(r.id) }) { Text(stringResource(R.string.decline)) }
                    }
                }
            }
        }
        Section {
            if (state.contacts.isEmpty()) {
                Hint(stringResource(R.string.no_contacts))
            } else {
                state.contacts.sortedBy { it.name.ifBlank { it.id }.lowercase() }.forEachIndexed { i, c ->
                    if (i > 0) HorizontalDivider()
                    ContactRow(vm, c)
                }
            }
        }
        if (state.identity != null) AddById(vm, state)
        Backup(vm)
        TextButton(onClick = { erase = true }) {
            Text(stringResource(R.string.vault_delete), color = MaterialTheme.colorScheme.error)
        }
    }
    if (erase) {
        AlertDialog(
            onDismissRequest = { erase = false },
            text = { Text(stringResource(R.string.vault_delete_confirm)) },
            confirmButton = {
                TextButton(onClick = {
                    erase = false
                    vm.eraseVault()
                }) { Text(stringResource(R.string.vault_delete), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { erase = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@Composable
private fun ContactRow(vm: MainViewModel, c: ContactEntry) {
    var expanded by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var removing by remember { mutableStateOf(false) }
    val label = c.name.ifBlank { c.id.take(14) + "…" }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier
                    .size(10.dp)
                    .clip(CircleShape)
                    .background(if (c.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant),
            )
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                Text(label)
                Hint(stringResource(if (c.online) R.string.online else R.string.offline_contact))
            }
            TextButton(onClick = { vm.chatWith(c.key) }) { Text(stringResource(R.string.connect)) }
            IconButton(onClick = { expanded = !expanded }) { HIcon(R.drawable.ic_more, stringResource(R.string.menu)) }
        }
        if (expanded) {
            Text(c.id, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
            SwitchRow(stringResource(R.string.history), stringResource(R.string.history_hint), c.history, { vm.setHistory(c.key, it) })
            Row {
                OutlinedButton(onClick = { renaming = true }) { Text(stringResource(R.string.rename)) }
                Spacer(Modifier.width(8.dp))
                TextButton(onClick = { removing = true }) {
                    Text(stringResource(R.string.remove_contact), color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }
    if (renaming) {
        var name by remember { mutableStateOf(c.name) }
        AlertDialog(
            onDismissRequest = { renaming = false },
            title = { Text(stringResource(R.string.rename)) },
            text = { OutlinedTextField(value = name, onValueChange = { name = it.take(64) }, singleLine = true) },
            confirmButton = {
                TextButton(onClick = {
                    renaming = false
                    vm.rename(c.key, name)
                }) { Text(stringResource(R.string.save)) }
            },
            dismissButton = { TextButton(onClick = { renaming = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
    if (removing) {
        AlertDialog(
            onDismissRequest = { removing = false },
            text = { Text(stringResource(R.string.remove_contact_confirm, label)) },
            confirmButton = {
                TextButton(onClick = {
                    removing = false
                    vm.removeContact(c.key)
                }) { Text(stringResource(R.string.remove_contact), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { removing = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@Composable
private fun AddById(vm: MainViewModel, state: UiState) {
    var id by rememberSaveable { mutableStateOf("") }
    var scanning by remember { mutableStateOf(false) }
    val lookup = state.lookup?.takeIf { it.id == id.trim().lowercase() }
    Section {
        Text(stringResource(R.string.add_by_id), style = MaterialTheme.typography.titleMedium)
        Hint(stringResource(R.string.add_by_id_hint))
        if (scanning) {
            QrScanner(onResult = {
                id = it.trim()
                scanning = false
            }, onCancel = { scanning = false })
        }
        OutlinedTextField(
            value = id,
            onValueChange = {
                id = it
                vm.idError = null
            },
            placeholder = { Text(stringResource(R.string.id_placeholder)) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii, autoCorrectEnabled = false),
            modifier = Modifier.fillMaxWidth().testTag("add-id"),
        )
        when (lookup?.status) {
            LookupStatus.SEARCHING -> Busy(stringResource(R.string.searching, id.take(14) + "…"))
            LookupStatus.REQUESTED -> Busy(stringResource(R.string.request_sent))
            LookupStatus.NOT_FOUND -> ErrorText(stringResource(R.string.not_found))
            LookupStatus.DECLINED -> ErrorText(stringResource(R.string.declined))
            null -> {}
        }
        vm.idError?.let { ErrorText(stringResource(it)) }
        Row {
            Button(
                onClick = { vm.addById(id) },
                enabled = id.isNotBlank() && lookup?.status != LookupStatus.SEARCHING,
            ) { Text(stringResource(R.string.connect)) }
            Spacer(Modifier.width(8.dp))
            OutlinedButton(onClick = { scanning = true }) { HIcon(R.drawable.ic_qr, stringResource(R.string.scan_qr)) }
        }
    }
}

@Composable
private fun Busy(text: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        Spacer(Modifier.width(8.dp))
        Text(text)
    }
}

@Composable
private fun Backup(vm: MainViewModel) {
    var pass by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<Int?>(null) }
    val export = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri ->
        if (uri != null) vm.exportBackup(pass, uri)
    }
    val import = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) vm.importBackup(pass, uri)
    }
    Section {
        Text(stringResource(R.string.backup), style = MaterialTheme.typography.titleMedium)
        Hint(stringResource(R.string.backup_intro))
        OutlinedTextField(
            value = pass,
            onValueChange = {
                pass = it
                error = null
            },
            label = { Text(stringResource(R.string.backup_password)) },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth(),
        )
        // The file can be copied anywhere and attacked offline: as strong as the vault, typed twice.
        OutlinedTextField(
            value = confirm,
            onValueChange = {
                confirm = it
                error = null
            },
            label = { Text(stringResource(R.string.backup_password_confirm)) },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth(),
        )
        Hint(stringResource(R.string.vault_password_hint))
        error?.let { ErrorText(stringResource(it)) }
        Row {
            OutlinedButton(onClick = {
                error = vm.backupPassphraseError(pass, confirm)
                if (error == null) {
                    val day = SimpleDateFormat("yyyy-MM-dd", Locale.ROOT).format(Date())
                    export.launch("hush-backup-$day.hush")
                }
            }, enabled = pass.isNotEmpty() && !vm.busy) { Text(stringResource(R.string.backup_export)) }
            Spacer(Modifier.width(8.dp))
            OutlinedButton(onClick = { import.launch(arrayOf("*/*")) }, enabled = pass.isNotEmpty() && !vm.busy) {
                Text(stringResource(R.string.backup_import))
            }
        }
        vm.backupResult?.let { Hint(it) }
    }
}

/** Create or unlock the vault before anything that needs to be kept. */
@Composable
fun VaultGate(vm: MainViewModel, state: UiState) {
    val creating = state.vault == VaultStatus.NONE
    var pass by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }
    Section {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HIcon(R.drawable.ic_lock)
            Spacer(Modifier.width(8.dp))
            Text(stringResource(R.string.vault_title), style = MaterialTheme.typography.titleMedium)
        }
        if (creating) Hint(stringResource(R.string.vault_intro))
        OutlinedTextField(
            value = pass,
            onValueChange = { pass = it },
            label = { Text(stringResource(R.string.vault_password)) },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.fillMaxWidth().testTag("vault-pass"),
        )
        if (creating) {
            Hint(stringResource(R.string.vault_password_hint))
            OutlinedTextField(
                value = confirm,
                onValueChange = { confirm = it },
                label = { Text(stringResource(R.string.vault_password_confirm)) },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                modifier = Modifier.fillMaxWidth().testTag("vault-confirm"),
            )
        }
        vm.vaultError?.let { ErrorText(stringResource(it)) }
        Button(
            onClick = { vm.openVault(pass, if (creating) confirm else null) },
            enabled = pass.isNotEmpty() && !vm.busy,
            modifier = Modifier.fillMaxWidth().height(52.dp),
        ) {
            Text(
                stringResource(
                    when {
                        vm.busy -> R.string.vault_working
                        creating -> R.string.vault_create
                        else -> R.string.vault_unlock
                    },
                ),
            )
        }
    }
}
