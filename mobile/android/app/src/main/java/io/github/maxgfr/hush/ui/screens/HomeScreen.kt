package io.github.maxgfr.hush.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import io.github.maxgfr.hush.R
import io.github.maxgfr.hush.core.ChatState
import io.github.maxgfr.hush.core.Settings
import io.github.maxgfr.hush.core.UiState
import io.github.maxgfr.hush.nearby.NearbyPermissions
import io.github.maxgfr.hush.ui.MainViewModel
import io.github.maxgfr.hush.ui.Screen
import io.github.maxgfr.hush.ui.components.HIcon
import io.github.maxgfr.hush.ui.components.Hint
import io.github.maxgfr.hush.ui.components.Section
import uniffi.hush_ffi.VaultStatus

@Composable
fun HomeScreen(vm: MainViewModel, state: UiState, settings: Settings) {
    Column(
        Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 20.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HIcon(R.drawable.ic_shield)
            Spacer(Modifier.width(8.dp))
            Text(stringResource(R.string.app_name), style = MaterialTheme.typography.headlineSmall)
        }
        Text(stringResource(R.string.tagline), style = MaterialTheme.typography.headlineMedium)
        Hint(stringResource(R.string.intro))
        Button(onClick = { vm.go(Screen.New) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
            HIcon(R.drawable.ic_link)
            Spacer(Modifier.width(8.dp))
            Text(stringResource(R.string.new_conversation))
        }
        OutlinedButton(onClick = { vm.go(Screen.Join()) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
            HIcon(R.drawable.ic_qr)
            Spacer(Modifier.width(8.dp))
            Text(stringResource(R.string.join_with_link))
        }

        if (settings.nearby) NearbyPermissionCard(vm)

        if (state.live.isNotEmpty()) {
            Section {
                Text(stringResource(R.string.conversations), style = MaterialTheme.typography.titleMedium)
                state.live.forEach { c -> LiveChat(c) { vm.go(Screen.Chat(c.id)) } }
            }
        }

        Section {
            Tile(R.drawable.ic_people, stringResource(R.string.contacts), trailing = when {
                state.vault == VaultStatus.UNLOCKED && state.contacts.isNotEmpty() -> state.contacts.size.toString()
                else -> null
            }) { vm.go(Screen.Contacts) }
            Tile(R.drawable.ic_key, stringResource(R.string.my_id)) { vm.go(Screen.Identity) }
            Tile(R.drawable.ic_settings, stringResource(R.string.settings)) { vm.go(Screen.Settings) }
        }

        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Hint(stringResource(R.string.crypto_facts))
            Hint(stringResource(R.string.no_server))
        }
    }
}

@Composable
private fun Tile(icon: Int, label: String, trailing: String? = null, onClick: () -> Unit) {
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        HIcon(icon)
        Spacer(Modifier.width(12.dp))
        Text(label, modifier = Modifier.weight(1f))
        trailing?.let { Text(it) }
    }
}

@Composable
private fun LiveChat(c: ChatState, onClick: () -> Unit) {
    val title = when {
        c.group -> pluralStringResource(R.plurals.members, c.present.size + 1, c.present.size + 1)
        else -> c.present.firstOrNull()?.name?.ifBlank { null } ?: stringResource(R.string.them)
    }
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        ListItem(headlineContent = { Text(title) })
    }
}

/** Nearby is on but Android has not been told yes yet: explain, then ask. */
@Composable
private fun NearbyPermissionCard(vm: MainViewModel) {
    val context = LocalContext.current
    var missing by remember { mutableStateOf(NearbyPermissions.missing(context)) }
    var refused by remember { mutableStateOf(false) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        missing = NearbyPermissions.missing(context)
        refused = missing.isNotEmpty()
        if (it.values.any { granted -> granted }) vm.nearbyAllowed()
    }
    if (missing.isEmpty()) return
    Section {
        Text(stringResource(R.string.nearby), style = MaterialTheme.typography.titleMedium)
        Hint(stringResource(R.string.nearby_permission))
        if (refused) Hint(stringResource(R.string.nearby_denied))
        Button(onClick = { ask.launch(NearbyPermissions.all().toTypedArray()) }) {
            Text(stringResource(R.string.allow))
        }
    }
}
