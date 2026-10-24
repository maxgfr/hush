package io.github.maxgfr.hush.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import io.github.maxgfr.hush.R
import io.github.maxgfr.hush.core.Format
import io.github.maxgfr.hush.core.Settings
import io.github.maxgfr.hush.ui.DropUi
import io.github.maxgfr.hush.ui.MainViewModel
import io.github.maxgfr.hush.ui.components.Choices
import io.github.maxgfr.hush.ui.components.ErrorText
import io.github.maxgfr.hush.ui.components.HIcon
import io.github.maxgfr.hush.ui.components.Hint
import io.github.maxgfr.hush.ui.components.Page
import io.github.maxgfr.hush.ui.components.QrCode
import io.github.maxgfr.hush.ui.components.Section
import io.github.maxgfr.hush.ui.components.SwitchRow
import io.github.maxgfr.hush.ui.components.copySensitive
import io.github.maxgfr.hush.ui.components.shareText
import uniffi.hush_ffi.DropPhase
import uniffi.hush_ffi.linkInfo

/** Send files by link: they are sealed here, then wait on blob servers that only see ciphertext. */
@Composable
fun DropScreen(vm: MainViewModel) {
    val d = vm.drop
    var picked by remember { mutableStateOf(listOf<android.net.Uri>()) }
    var ttlMin by rememberSaveable { mutableLongStateOf(1_440L) }
    var once by rememberSaveable { mutableStateOf(false) }
    // Secrets are never saveable: the saved instance state outlives the process.
    var pass by remember { mutableStateOf("") }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
        if (it.isNotEmpty()) picked = (picked + it).distinct()
    }
    Page(stringResource(R.string.send_files), onBack = vm::back) {
        when {
            d.url != null -> Section { DropLink(d) }
            d.running -> Section { Progress(d) }
            else -> Section {
                OutlinedButton(onClick = { pick.launch(arrayOf("*/*")) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
                    HIcon(R.drawable.ic_attach)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.pick_files))
                }
                if (picked.isNotEmpty()) Hint(pluralStringResource(R.plurals.files_picked_n, picked.size, picked.size))
                Text(stringResource(R.string.link_expiry), style = MaterialTheme.typography.titleSmall)
                Choices(Settings.LINK_MINUTES, ttlMin, { Format.duration(it * 60_000) }) { ttlMin = it }
                SwitchRow(stringResource(R.string.drop_once), null, once, { once = it })
                OutlinedTextField(
                    value = pass,
                    onValueChange = { pass = it },
                    label = { Text(stringResource(R.string.passphrase)) },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                    modifier = Modifier.fillMaxWidth(),
                )
                Hint(stringResource(R.string.drop_hint))
                d.error?.let { ErrorText(stringResource(it)) }
                Button(
                    onClick = { vm.sendDrop(picked, ttlMin, once, pass) },
                    enabled = picked.isNotEmpty(),
                    modifier = Modifier.fillMaxWidth().height(52.dp),
                ) {
                    HIcon(R.drawable.ic_lock)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.seal_and_send))
                }
            }
        }
    }
}

@Composable
private fun Progress(d: DropUi) {
    val label = when (d.phase) {
        DropPhase.SEALING, null -> R.string.drop_sealing
        DropPhase.UPLOADING -> R.string.drop_uploading
        DropPhase.DOWNLOADING -> R.string.drop_downloading
    }
    Text(stringResource(label), style = MaterialTheme.typography.titleSmall)
    LinearProgressIndicator(
        progress = { if (d.total > 0) d.done.toFloat() / d.total else 0f },
        modifier = Modifier.fillMaxWidth(),
    )
    if (d.total > 0) Hint("${Format.size(d.done)} / ${Format.size(d.total)}")
}

@Composable
private fun ColumnScope.DropLink(d: DropUi) {
    val context = LocalContext.current
    val url = d.url ?: return
    Text(stringResource(R.string.send_this_link), style = MaterialTheme.typography.titleMedium)
    QrCode(url, modifier = Modifier.align(Alignment.CenterHorizontally))
    Text(url, style = MaterialTheme.typography.bodySmall)
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Button(onClick = { copySensitive(context, "hush", url, expire = true) }) {
            HIcon(R.drawable.ic_copy)
            Spacer(Modifier.width(8.dp))
            Text(stringResource(R.string.copy))
        }
        OutlinedButton(onClick = { shareText(context, url) }) {
            HIcon(R.drawable.ic_share)
            Spacer(Modifier.width(8.dp))
            Text(stringResource(R.string.share))
        }
    }
    Hint(stringResource(R.string.drop_ready, Format.duration(d.expiresMs - System.currentTimeMillis())))
}

/** A drop link: shows what it is, then downloads its sealed copy. */
@Composable
fun DropReceiveScreen(vm: MainViewModel, link: String) {
    val d = vm.drop
    val context = LocalContext.current
    val needsPass = remember(link) { runCatching { linkInfo(link).passphrase }.getOrDefault(false) }
    var pass by remember { mutableStateOf("") }
    Page(stringResource(R.string.files_for_you), onBack = vm::back) {
        Section {
            when {
                d.saved.isNotEmpty() -> {
                    d.sender?.let { Hint(stringResource(R.string.drop_from, it)) }
                    Text(stringResource(R.string.drop_saved), style = MaterialTheme.typography.titleSmall)
                    d.saved.forEach { path ->
                        OutlinedButton(onClick = { openFile(context, path) }, modifier = Modifier.fillMaxWidth()) {
                            HIcon(R.drawable.ic_file)
                            Spacer(Modifier.width(8.dp))
                            Text(path.substringAfterLast('/'), modifier = Modifier.weight(1f))
                        }
                    }
                    if (d.deleted) Hint(stringResource(R.string.drop_deleted))
                }
                d.running -> Progress(d)
                else -> {
                    Hint(stringResource(R.string.drop_receive_hint))
                    if (needsPass) {
                        OutlinedTextField(
                            value = pass,
                            onValueChange = { pass = it },
                            label = { Text(stringResource(R.string.passphrase)) },
                            singleLine = true,
                            visualTransformation = PasswordVisualTransformation(),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                    d.error?.let { ErrorText(stringResource(it)) }
                    Button(
                        onClick = { vm.receiveDrop(link, pass) },
                        enabled = !needsPass || pass.isNotEmpty(),
                        modifier = Modifier.fillMaxWidth().height(52.dp),
                    ) {
                        HIcon(R.drawable.ic_download)
                        Spacer(Modifier.width(8.dp))
                        Text(stringResource(R.string.download_all))
                    }
                }
            }
        }
    }
}
