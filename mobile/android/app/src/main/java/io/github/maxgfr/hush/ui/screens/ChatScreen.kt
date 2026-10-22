package io.github.maxgfr.hush.ui.screens

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import io.github.maxgfr.hush.R
import io.github.maxgfr.hush.core.ChatItem
import io.github.maxgfr.hush.core.ChatState
import io.github.maxgfr.hush.core.FileItem
import io.github.maxgfr.hush.core.FileState
import io.github.maxgfr.hush.core.Format
import io.github.maxgfr.hush.core.MessageItem
import io.github.maxgfr.hush.core.NoticeItem
import io.github.maxgfr.hush.core.PeerState
import io.github.maxgfr.hush.core.Sas
import io.github.maxgfr.hush.core.Settings
import io.github.maxgfr.hush.core.UiState
import io.github.maxgfr.hush.ui.Labels
import io.github.maxgfr.hush.ui.MainViewModel
import io.github.maxgfr.hush.ui.components.HIcon
import io.github.maxgfr.hush.ui.components.Hint
import io.github.maxgfr.hush.ui.components.QrCode
import io.github.maxgfr.hush.ui.components.Section
import io.github.maxgfr.hush.ui.components.copySensitive
import io.github.maxgfr.hush.ui.components.shareText
import java.io.File
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.delay
import uniffi.hush_ffi.NoticeKind
import uniffi.hush_ffi.Status

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatScreen(vm: MainViewModel, state: UiState, settings: Settings, id: ULong) {
    val chat = state.chats[id] ?: return
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        // Disappearing messages and the expiry countdown follow the clock.
        while (true) {
            delay(1_000)
            now = System.currentTimeMillis()
        }
    }
    val peers = chat.present
    val open = peers.isNotEmpty()
    val visible = chat.items.filter { (it.expiresAt ?: Long.MAX_VALUE) > now }
    var menu by remember { mutableStateOf(false) }
    var timerMenu by remember { mutableStateOf(false) }
    var ttl by rememberSaveable { mutableLongStateOf(settings.ttl) }
    var confirmEnd by remember { mutableStateOf(false) }
    var inviting by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<MessageItem?>(null) }
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
        if (it.isNotEmpty()) vm.sendFiles(id, it)
    }
    val contactName = chat.contactKey?.let { key -> state.contacts.firstOrNull { it.key == key }?.name }
    val title = when {
        chat.group -> pluralStringResource(R.plurals.members, peers.size + 1, peers.size + 1)
        !contactName.isNullOrBlank() -> contactName
        open -> peers[0].name.ifBlank { stringResource(R.string.them) }
        else -> stringResource(R.string.app_name)
    }
    val end: () -> Unit = {
        if (open) {
            confirmEnd = true
        } else {
            vm.end(id)
        }
    }
    val off = stringResource(R.string.timer_off_short)

    BackHandler { vm.back() }

    Scaffold(
        modifier = Modifier.imePadding(),
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(title, maxLines = 1)
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (open && !chat.group) {
                                PathBadge(peers[0])
                                Spacer(Modifier.width(8.dp))
                            }
                            Text(
                                stringResource(R.string.e2e_pq),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                },
                navigationIcon = {
                    IconButton(onClick = vm::back) { HIcon(R.drawable.ic_back, stringResource(R.string.back)) }
                },
                actions = {
                    Box {
                        TextButton(onClick = { timerMenu = true }) {
                            HIcon(R.drawable.ic_timer, stringResource(R.string.message_timer))
                            Text(Format.ttl(ttl, off))
                        }
                        DropdownMenu(expanded = timerMenu, onDismissRequest = { timerMenu = false }) {
                            Settings.TTL_SECONDS.forEach { s ->
                                DropdownMenuItem(text = { Text(Format.ttl(s, off)) }, onClick = {
                                    ttl = s
                                    timerMenu = false
                                    vm.setTtl(s)
                                })
                            }
                        }
                    }
                    if (chat.group || open) {
                        Box {
                            IconButton(onClick = { menu = true }) { HIcon(R.drawable.ic_more, stringResource(R.string.menu)) }
                            ChatMenu(vm, chat, menu, onDismiss = { menu = false }, onInvite = { inviting = !inviting }, onEnd = end)
                        }
                    }
                    if (chat.isLink) {
                        TextButton(onClick = end, modifier = Modifier.testTag("end")) {
                            Text(stringResource(R.string.end_conversation), color = MaterialTheme.colorScheme.error)
                        }
                    }
                },
            )
        },
        bottomBar = { if (open) Composer(onSend = { vm.send(id, it) }, onAttach = { pick.launch(arrayOf("*/*")) }) },
    ) { padding ->
        val list = rememberLazyListState()
        LaunchedEffect(visible.size) { if (visible.isNotEmpty()) list.animateScrollToItem(list.layoutInfo.totalItemsCount.coerceAtLeast(1) - 1) }
        LazyColumn(
            state = list,
            modifier = Modifier.fillMaxSize().padding(padding),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (!open) item("waiting") { Waiting(chat, now) }
            if (chat.group && open && inviting && chat.url != null) item("invite") { ShareLink(chat, now) }
            if (chat.group && open) item("members") { Members(peers) }
            if (chat.locked) item("locked") { NoticeLine(stringResource(R.string.locked)) }
            items(peers.filter { !it.verified && it.sas.isNotEmpty() }, key = { "verify-${it.tag}" }) { p ->
                Verify(chat, p, onAnswer = { ok -> vm.verify(id, p.tag, ok) })
            }
            items(chat.offers, key = { "offer-${it.peer}" }) { o ->
                Section {
                    Text(stringResource(R.string.contact_asked, o.name.ifBlank { stringResource(R.string.them) }))
                    Row {
                        Button(onClick = {
                            vm.saveContact(id, o.peer)
                            vm.dismissOffer(id, o.peer)
                        }) { Text(stringResource(R.string.save_contact)) }
                        Spacer(Modifier.width(8.dp))
                        TextButton(onClick = { vm.dismissOffer(id, o.peer) }) { Text(stringResource(R.string.close)) }
                    }
                }
            }
            if (open) item("session") { NoticeLine(stringResource(R.string.new_session_started)) }
            items(visible, key = { "${it::class.simpleName}-${it.id}" }) { item ->
                Item(vm, chat, item, onDelete = { deleting = it })
            }
        }
    }

    if (confirmEnd) {
        AlertDialog(
            onDismissRequest = { confirmEnd = false },
            text = { Text(stringResource(R.string.end_confirm)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmEnd = false
                    vm.end(id)
                }) { Text(stringResource(R.string.end_conversation)) }
            },
            dismissButton = { TextButton(onClick = { confirmEnd = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
    deleting?.let { m ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            text = { Text(stringResource(R.string.delete_confirm)) },
            confirmButton = {
                TextButton(onClick = {
                    deleting = null
                    vm.delete(id, m.id)
                }) { Text(stringResource(R.string.delete)) }
            },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}

@Composable
private fun ChatMenu(
    vm: MainViewModel,
    chat: ChatState,
    expanded: Boolean,
    onDismiss: () -> Unit,
    onInvite: () -> Unit,
    onEnd: () -> Unit,
) {
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss) {
        if (chat.group) {
            if (chat.url != null) {
                DropdownMenuItem(text = { Text(stringResource(R.string.invite)) }, onClick = {
                    onDismiss()
                    onInvite()
                })
            }
            DropdownMenuItem(
                text = { Text(stringResource(if (chat.locked) R.string.unlock else R.string.lock)) },
                onClick = {
                    onDismiss()
                    vm.lock(chat.id, !chat.locked)
                },
            )
        }
        if (chat.contactKey == null) {
            chat.present.forEach { p ->
                val who = if (chat.group) " · ${p.name.ifBlank { stringResource(R.string.someone) }}" else ""
                DropdownMenuItem(text = { Text(stringResource(R.string.save_contact) + who) }, onClick = {
                    onDismiss()
                    vm.saveContact(chat.id, p.tag)
                })
            }
        } else {
            DropdownMenuItem(text = { Text(stringResource(R.string.end_conversation)) }, onClick = {
                onDismiss()
                onEnd()
            })
        }
    }
}

@Composable
private fun PathBadge(p: PeerState) {
    Text(
        buildString {
            append(stringResource(Labels.path(p.path)))
            if (p.verified) append(" · ").append(stringResource(R.string.verified))
        },
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.testTag("path"),
    )
}

@Composable
private fun Members(peers: List<PeerState>) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        peers.forEach { p ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(p.name.ifBlank { stringResource(R.string.someone) }, modifier = Modifier.weight(1f))
                PathBadge(p)
            }
        }
    }
}

@Composable
private fun NoticeLine(text: String) {
    Text(
        text,
        modifier = Modifier.fillMaxWidth(),
        textAlign = TextAlign.Center,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun Waiting(chat: ChatState, now: Long) {
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        if (chat.creator && chat.url != null) ShareLink(chat, now)
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("status")) {
            CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
            Spacer(Modifier.width(10.dp))
            Text(
                stringResource(
                    when {
                        chat.status == Status.CONNECTING -> R.string.connecting
                        chat.isLink && !chat.creator -> R.string.waiting_creator
                        else -> R.string.waiting_peer
                    },
                ),
            )
        }
    }
}

/** The link to send, as a QR code and as text, with what the other person needs to know. */
@Composable
private fun ShareLink(chat: ChatState, now: Long) {
    val url = chat.url ?: return
    val context = LocalContext.current
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(copied) {
        if (copied) {
            delay(1_500)
            copied = false
        }
    }
    Section {
        Text(stringResource(R.string.send_this_link), style = MaterialTheme.typography.titleMedium)
        Hint(stringResource(R.string.send_this_link_hint))
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { QrCode(url) }
        SelectionContainer {
            Text(
                url,
                fontFamily = FontFamily.Monospace,
                fontSize = 12.sp,
                modifier = Modifier.testTag("share-link"),
            )
        }
        Row {
            Button(onClick = {
                copySensitive(context, "hush", url)
                copied = true
            }) {
                HIcon(if (copied) R.drawable.ic_check else R.drawable.ic_copy)
                Spacer(Modifier.width(8.dp))
                Text(stringResource(if (copied) R.string.copied else R.string.copy))
            }
            Spacer(Modifier.width(8.dp))
            OutlinedButton(onClick = { shareText(context, url) }) {
                HIcon(R.drawable.ic_share)
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.share))
            }
        }
        chat.expiresMs?.let { expires ->
            Hint(
                stringResource(R.string.expires_in, Format.duration(expires - now)) + " · " +
                    stringResource(if (chat.group) R.string.anyone_with_link else R.string.single_use),
            )
        }
        if (chat.passphrase) Hint(stringResource(R.string.share_passphrase))
    }
}

/** The emojis to compare with one peer, until both sides said they match. */
@Composable
private fun Verify(chat: ChatState, p: PeerState, onAnswer: (Boolean) -> Unit) {
    val name = p.name.ifBlank { stringResource(R.string.them) }
    val fr = Format.french()
    Section {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HIcon(R.drawable.ic_shield)
            Spacer(Modifier.width(8.dp))
            Text(
                if (chat.group) stringResource(R.string.verify_title_peer, name) else stringResource(R.string.verify_title),
                style = MaterialTheme.typography.titleMedium,
            )
        }
        Hint(stringResource(R.string.verify_hint, name))
        Row(Modifier.fillMaxWidth().testTag("sas"), horizontalArrangement = Arrangement.SpaceEvenly) {
            p.sas.forEach { i ->
                val e = Sas.emoji(i)
                Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.widthIn(max = 64.dp)) {
                    Text(e.glyph, fontSize = 30.sp)
                    Text(if (fr) e.fr else e.en, style = MaterialTheme.typography.labelSmall, textAlign = TextAlign.Center)
                }
            }
        }
        if (p.byMe) {
            Hint(stringResource(R.string.waiting_their_check, name))
        } else {
            Row {
                Button(onClick = { onAnswer(true) }) { Text(stringResource(R.string.match)) }
                Spacer(Modifier.width(8.dp))
                OutlinedButton(onClick = { onAnswer(false) }) {
                    Text(stringResource(R.string.no_match), color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }
}

@Composable
private fun Item(vm: MainViewModel, chat: ChatState, item: ChatItem, onDelete: (MessageItem) -> Unit) {
    when (item) {
        is NoticeItem -> NoticeLine(noticeText(item.notice))
        is MessageItem -> Bubble(chat, item.mine, item.author, item.ts, item.expiresAt != null, onLongPress = {
            if (item.mine) onDelete(item)
        }) {
            Text(item.text)
        }
        is FileItem -> Bubble(chat, item.mine, chat.peer(item.peer ?: "")?.name ?: "", item.ts, item.expiresAt != null) {
            FileCard(vm, chat, item)
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Bubble(
    chat: ChatState,
    mine: Boolean,
    author: String,
    ts: Long,
    timed: Boolean,
    onLongPress: () -> Unit = {},
    content: @Composable () -> Unit,
) {
    Column(Modifier.fillMaxWidth(), horizontalAlignment = if (mine) Alignment.End else Alignment.Start) {
        if (chat.group && !mine) {
            Text(
                author.ifBlank { stringResource(R.string.someone) },
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.primary,
            )
        }
        Surface(
            shape = RoundedCornerShape(16.dp),
            color = if (mine) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = Modifier
                .widthIn(max = 320.dp)
                .combinedClickable(onClick = {}, onLongClick = onLongPress)
                .testTag("message"),
        ) {
            Column(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                content()
                Row(Modifier.align(Alignment.End), verticalAlignment = Alignment.CenterVertically) {
                    if (timed) HIcon(R.drawable.ic_timer, modifier = Modifier.size(12.dp))
                    Text(
                        DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(ts)),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

@Composable
private fun FileCard(vm: MainViewModel, chat: ChatState, f: FileItem) {
    val context = LocalContext.current
    Column(Modifier.widthIn(min = 200.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            HIcon(R.drawable.ic_file)
            Spacer(Modifier.width(8.dp))
            Column(Modifier.weight(1f)) {
                Text(f.name, maxLines = 2)
                Hint(Format.size(f.size))
            }
        }
        when (f.state) {
            FileState.OFFERED -> Button(onClick = { vm.download(chat.id, f.id) }) {
                HIcon(R.drawable.ic_download)
                Spacer(Modifier.width(6.dp))
                Text(stringResource(R.string.download))
            }
            FileState.DOWNLOADING -> {
                LinearProgressIndicator(progress = { f.progress }, modifier = Modifier.fillMaxWidth())
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Hint(stringResource(R.string.downloading, (f.progress * 100).toInt()), Modifier.weight(1f))
                    TextButton(onClick = { vm.cancelFile(chat.id, f.id) }) { Text(stringResource(R.string.cancel)) }
                }
            }
            FileState.SAVED -> Button(onClick = { f.path?.let { openFile(context, it, f.mime) } }) {
                Text(stringResource(R.string.open))
            }
            FileState.SENDING ->
                if (f.done > 0) {
                    LinearProgressIndicator(progress = { f.progress }, modifier = Modifier.fillMaxWidth())
                } else {
                    Hint(stringResource(R.string.sending))
                }
            FileState.DELIVERED -> Hint(stringResource(R.string.delivered))
            FileState.FAILED -> Text(
                stringResource(R.string.file_failed),
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

/** Hands a received file to another app, read-only, through the FileProvider. */
private fun openFile(context: Context, path: String, mime: String) {
    val uri = try {
        FileProvider.getUriForFile(context, "${context.packageName}.files", File(path))
    } catch (ignored: IllegalArgumentException) {
        return
    }
    val view = Intent(Intent.ACTION_VIEW)
        .setDataAndType(uri, mime.ifBlank { "application/octet-stream" })
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    try {
        context.startActivity(view)
    } catch (ignored: ActivityNotFoundException) {
        Toast.makeText(context, R.string.no_app_to_open, Toast.LENGTH_SHORT).show()
    }
}

@Composable
private fun noticeText(n: NoticeKind): String = when (n) {
    is NoticeKind.Joined -> stringResource(R.string.notice_joined, n.name.ifBlank { stringResource(R.string.someone) })
    is NoticeKind.Left -> stringResource(R.string.notice_left, n.name.ifBlank { stringResource(R.string.someone) })
    is NoticeKind.Renamed -> stringResource(R.string.renamed, n.old, n.name)
    is NoticeKind.Locked -> stringResource(if (n.locked) R.string.locked_by else R.string.unlocked_by, n.by)
    is NoticeKind.ContactSaved -> stringResource(R.string.contact_saved, n.name)
    is NoticeKind.ContactWaiting -> stringResource(R.string.contact_waiting, n.name)
    NoticeKind.VerifyFirst -> stringResource(R.string.verify_first)
    NoticeKind.VaultNeeded -> stringResource(R.string.vault_needed)
    NoticeKind.FilesNeedDirect -> stringResource(R.string.direct_needed)
    is NoticeKind.CannotSend -> stringResource(R.string.cannot_send, n.reason)
    NoticeKind.NotACode -> stringResource(R.string.invalid_code)
}

@Composable
private fun Composer(onSend: (String) -> Unit, onAttach: () -> Unit) {
    var text by rememberSaveable { mutableStateOf("") }
    val send = {
        if (text.isNotBlank()) {
            onSend(text)
            text = ""
        }
    }
    Surface(tonalElevation = 2.dp) {
        Row(
            Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surface)
                .padding(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onAttach) { HIcon(R.drawable.ic_attach, stringResource(R.string.attach)) }
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                placeholder = { Text(stringResource(R.string.message_placeholder)) },
                maxLines = 5,
                modifier = Modifier.weight(1f).testTag("composer"),
            )
            IconButton(onClick = send, enabled = text.isNotBlank(), modifier = Modifier.testTag("send")) {
                HIcon(R.drawable.ic_send, stringResource(R.string.send))
            }
        }
    }
}
