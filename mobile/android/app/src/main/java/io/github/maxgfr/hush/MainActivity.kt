package io.github.maxgfr.hush

import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.maxgfr.hush.core.LinkInput
import io.github.maxgfr.hush.ui.MainViewModel
import io.github.maxgfr.hush.ui.Screen
import io.github.maxgfr.hush.ui.components.ClipboardExpiry
import io.github.maxgfr.hush.ui.components.NoKeyboardLearning
import io.github.maxgfr.hush.ui.screens.ChatScreen
import io.github.maxgfr.hush.ui.screens.ContactsScreen
import io.github.maxgfr.hush.ui.screens.DropReceiveScreen
import io.github.maxgfr.hush.ui.screens.DropScreen
import io.github.maxgfr.hush.ui.screens.EndedScreen
import io.github.maxgfr.hush.ui.screens.HomeScreen
import io.github.maxgfr.hush.ui.screens.IdentityScreen
import io.github.maxgfr.hush.ui.screens.JoinScreen
import io.github.maxgfr.hush.ui.screens.NewConversationScreen
import io.github.maxgfr.hush.ui.screens.PassphraseScreen
import io.github.maxgfr.hush.ui.screens.SettingsScreen
import io.github.maxgfr.hush.ui.theme.HushTheme

class MainActivity : ComponentActivity() {
    private val vm: MainViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // No screenshots, no screen recording, a blank card in the recent apps.
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        enableEdgeToEdge()
        if (savedInstanceState == null) handle(intent)
        setContent { HushTheme { NoKeyboardLearning { Root(vm) } } }
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) ClipboardExpiry.check(this)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    /** A hush link opened from a browser or messenger, or shared as text: offered, not joined. */
    private fun handle(intent: Intent?) {
        val link = when (intent?.action) {
            Intent.ACTION_VIEW -> intent.dataString?.takeIf(LinkInput::isOurs)
            Intent.ACTION_SEND -> LinkInput.extract(intent.getStringExtra(Intent.EXTRA_TEXT))
            else -> null
        } ?: return
        vm.offerLink(link)
    }
}

@Composable
private fun Root(vm: MainViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val settings by vm.settings.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val message = vm.message?.let { stringResource(it) }
    LaunchedEffect(message) {
        if (message != null) {
            vm.message = null
            snackbar.showSnackbar(message)
        }
    }
    BackHandler(enabled = vm.stack.size > 1) { vm.back() }
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Box(Modifier.fillMaxSize()) {
            when (val s = vm.screen) {
                Screen.Home -> HomeScreen(vm, state, settings)
                Screen.New -> NewConversationScreen(vm, settings)
                is Screen.Join -> JoinScreen(vm, s.error, s.link)
                is Screen.Passphrase -> PassphraseScreen(vm, s.link)
                is Screen.Chat -> ChatScreen(vm, state, settings, s.id)
                is Screen.Ended -> EndedScreen(vm, s.reason, s.kept)
                Screen.Contacts -> ContactsScreen(vm, state)
                Screen.Identity -> IdentityScreen(vm, state, settings)
                Screen.Settings -> SettingsScreen(vm, settings)
                Screen.Drop -> DropScreen(vm)
                is Screen.DropReceive -> DropReceiveScreen(vm, s.link)
            }
            SnackbarHost(
                snackbar,
                Modifier
                    .align(Alignment.BottomCenter)
                    .safeDrawingPadding()
                    .padding(bottom = 72.dp),
            )
        }
    }
}
