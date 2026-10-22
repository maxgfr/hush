package io.github.maxgfr.hush.ui.screens

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.maxgfr.hush.R
import io.github.maxgfr.hush.ui.Labels
import io.github.maxgfr.hush.ui.MainViewModel
import io.github.maxgfr.hush.ui.components.Section
import uniffi.hush_ffi.Ending

@Composable
fun EndedScreen(vm: MainViewModel, reason: Ending, kept: Boolean) {
    val warn = Labels.warns(reason)
    BackHandler { vm.leaveEnded() }
    Column(
        Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            painterResource(if (warn) R.drawable.ic_warning else R.drawable.ic_check),
            contentDescription = null,
            tint = if (warn) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(48.dp),
        )
        Text(stringResource(Labels.ending(reason)), style = MaterialTheme.typography.headlineSmall, textAlign = TextAlign.Center)
        if (reason == Ending.TAKEN) {
            Section { Text(stringResource(R.string.taken_hint), color = MaterialTheme.colorScheme.error) }
        }
        Text(
            stringResource(if (kept) R.string.wiped_kept else R.string.wiped),
            textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = vm::leaveEnded, modifier = Modifier.fillMaxWidth().height(52.dp)) {
            Text(stringResource(R.string.start_over))
        }
    }
}
