package io.github.maxgfr.hush.ui

import android.app.Application
import android.net.Uri
import android.provider.OpenableColumns
import androidx.annotation.StringRes
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.maxgfr.hush.R
import io.github.maxgfr.hush.core.Reducer
import io.github.maxgfr.hush.core.Settings
import io.github.maxgfr.hush.core.UiState
import io.github.maxgfr.hush.engine
import io.github.maxgfr.hush.service.HushService
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.hush_ffi.ChatType
import uniffi.hush_ffi.Ending
import uniffi.hush_ffi.HushException
import uniffi.hush_ffi.LookupStatus
import uniffi.hush_ffi.VaultStatus
import uniffi.hush_ffi.isIdentifier
import uniffi.hush_ffi.linkInfo

sealed interface Screen {
    data object Home : Screen
    data object New : Screen
    data class Join(@StringRes val error: Int? = null) : Screen
    data class Passphrase(val link: String) : Screen
    data class Chat(val id: ULong) : Screen
    data class Ended(val reason: Ending, val kept: Boolean) : Screen
    data object Contacts : Screen
    data object Identity : Screen
    data object Settings : Screen
}

/** Navigation and every action of the screens; engine calls that block run on Dispatchers.IO. */
class MainViewModel(application: Application) : AndroidViewModel(application) {
    private val engine = application.engine
    private val app = engine.app
    val state: StateFlow<UiState> = engine.store.state
    val settings: StateFlow<Settings> = engine.settings.state
    val defaults: Settings get() = engine.defaults

    var stack by mutableStateOf(listOf<Screen>(Screen.Home))
        private set
    val screen: Screen get() = stack.last()

    /** Something to tell in a snackbar, once. */
    var message by mutableStateOf<Int?>(null)

    var busy by mutableStateOf(false)
        private set

    /** Vault, backup and lookup errors, shown where they happened. */
    var vaultError by mutableStateOf<Int?>(null)
    var backupResult by mutableStateOf<String?>(null)
    var idError by mutableStateOf<Int?>(null)
    private var awaitingStranger = false

    init {
        viewModelScope.launch { state.collect(::follow) }
        // The foreground service runs while a conversation is open or the radios are looking.
        viewModelScope.launch {
            combine(state, engine.nearby.active) { s, radios -> s.live.isNotEmpty() || radios }
                .distinctUntilChanged()
                .collect { HushService.set(application, it) }
        }
    }

    fun go(s: Screen) {
        stack = stack + s
    }

    fun back() {
        if (stack.size > 1) stack = stack.dropLast(1)
    }

    fun home() {
        stack = listOf(Screen.Home)
    }

    private fun replace(s: Screen) {
        stack = stack.dropLast(1) + s
    }

    /** Moves along with the engine: an ended conversation, a contact found, a request accepted. */
    private fun follow(s: UiState) {
        val top = screen
        if (top is Screen.Chat) {
            val c = s.chats[top.id]
            if (c?.ended != null) {
                if (c.isLink) {
                    replace(Screen.Ended(c.ended, s.vault == VaultStatus.UNLOCKED))
                } else {
                    back()
                }
                engine.store.update { Reducer.forget(it, c.id) }
            }
        }
        // Conversations that ended out of sight leave nothing behind either.
        val gone = s.chats.values.filter { it.ended != null && (top !is Screen.Chat || it.id != top.id) }
        if (gone.isNotEmpty()) engine.store.update { st -> gone.fold(st) { acc, c -> Reducer.forget(acc, c.id) } }
        val l = s.lookup
        if (l?.status == LookupStatus.REQUESTED && l.chat != null && top == Screen.Contacts) {
            engine.store.update { it.copy(lookup = null) }
            go(Screen.Chat(l.chat))
        }
        if (awaitingStranger && top == Screen.Contacts) {
            s.live.lastOrNull { it.kind is ChatType.Stranger }?.let {
                awaitingStranger = false
                go(Screen.Chat(it.id))
            }
        }
    }

    private fun io(block: suspend () -> Unit) {
        viewModelScope.launch(Dispatchers.IO) { block() }
    }

    private suspend fun main(block: () -> Unit) = withContext(Dispatchers.Main) { block() }

    // ── conversations ──

    fun createLink(group: Boolean, ttlMin: Long, passphrase: String, ttl: Long) {
        if (busy) return
        busy = true
        io {
            app.setTtl(ttl.toULong())
            val result = runCatching { app.createLink(group, ttlMin.toULong(), passphrase.ifEmpty { null }) }
            main {
                busy = false
                result.onSuccess { replace(Screen.Chat(it)) }.onFailure { message = R.string.error_generic }
            }
        }
    }

    /** A link pasted, scanned or handed over by another app. */
    fun openLink(input: String) {
        val link = input.trim()
        if (link.isEmpty()) return
        io {
            val info = try {
                linkInfo(link)
            } catch (e: HushException) {
                val outdated = e is HushException.Core && e.name == "Outdated"
                main { showJoin(if (outdated) R.string.outdated_link else R.string.invalid_link) }
                return@io
            }
            when {
                info.expiresMs.toLong() < System.currentTimeMillis() -> main { showJoin(R.string.expired_link) }
                info.passphrase -> main { showOn(Screen.Passphrase(link)) }
                else -> join(link, null)
            }
        }
    }

    private fun showJoin(@StringRes error: Int) = showOn(Screen.Join(error))

    private fun showOn(s: Screen) {
        if (screen is Screen.Join || screen is Screen.Passphrase) replace(s) else go(s)
    }

    /** Joining with a passphrase runs Argon2id: about a second, off the main thread. */
    fun join(link: String, passphrase: String?) {
        viewModelScope.launch {
            busy = true
            val result = withContext(Dispatchers.IO) { runCatching { app.joinLink(link, passphrase) } }
            busy = false
            result
                .onSuccess { chat ->
                    if (screen is Screen.Join || screen is Screen.Passphrase) replace(Screen.Chat(chat)) else go(Screen.Chat(chat))
                }
                .onFailure { showJoin(R.string.invalid_link) }
        }
    }

    fun send(chat: ULong, text: String) = io { app.sendText(chat, text) }

    /** Copies picked files into the cache, where the engine can read them, then offers them. */
    fun sendFiles(chat: ULong, uris: List<Uri>) = io {
        val resolver = getApplication<Application>().contentResolver
        for (uri in uris) {
            val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
            val dir = File(engine.outgoingDir, UUID.randomUUID().toString()).apply { mkdirs() }
            val file = File(dir, safeName(name))
            val ok = runCatching {
                resolver.openInputStream(uri)?.use { input -> file.outputStream().use { input.copyTo(it) } } != null
            }.getOrDefault(false)
            if (ok) app.sendFile(chat, file.path) else main { message = R.string.file_failed }
        }
    }

    fun download(chat: ULong, id: String) {
        engine.store.update { Reducer.downloading(it, chat, id) }
        io { app.getFile(chat, id) }
    }

    fun cancelFile(chat: ULong, id: String) {
        engine.store.update { Reducer.cancelled(it, chat, id) }
        io { app.cancelFile(chat, id) }
    }

    fun delete(chat: ULong, id: String) = io { app.deleteMessage(chat, id) }

    fun verify(chat: ULong, peer: String, ok: Boolean) = io { app.verify(chat, peer, ok) }

    fun lock(chat: ULong, locked: Boolean) = io { app.lock(chat, locked) }

    fun end(chat: ULong) = io { app.end(chat) }

    fun setTtl(seconds: Long) = io { app.setTtl(seconds.toULong()) }

    /** "Save contact": both must have compared the emojis, and our vault must be open. */
    fun saveContact(chat: ULong, peer: String) {
        val s = state.value
        val p = s.chats[chat]?.peer(peer) ?: return
        when {
            !p.verified -> message = R.string.verify_first
            s.vault != VaultStatus.UNLOCKED -> go(Screen.Contacts)
            else -> io { app.saveContact(chat, peer) }
        }
    }

    fun dismissOffer(chat: ULong, peer: String) = engine.store.update { Reducer.dismissOffer(it, chat, peer) }

    fun leaveEnded() {
        home()
    }

    // ── vault, identity, contacts ──

    fun openVault(passphrase: String, confirm: String?) {
        val creating = state.value.vault == VaultStatus.NONE
        if (creating && passphrase.length < MIN_VAULT) {
            vaultError = R.string.vault_too_short
            return
        }
        if (creating && passphrase != confirm) {
            vaultError = R.string.vault_mismatch
            return
        }
        viewModelScope.launch {
            busy = true
            vaultError = null
            val r = withContext(Dispatchers.IO) {
                runCatching { if (creating) app.createVault(passphrase) else app.unlockVault(passphrase) }
            }
            busy = false
            if (r.isFailure) vaultError = if (creating) R.string.vault_too_short else R.string.vault_wrong
        }
    }

    fun lockVault() = io { app.lockVault() }

    fun eraseVault() = io { runCatching { app.eraseVault() } }

    fun createIdentity() = io { runCatching { app.ensureIdentity() } }

    fun setReachable(on: Boolean) = io { app.setReachable(on) }

    fun chatWith(key: String) {
        viewModelScope.launch {
            val chat = withContext(Dispatchers.IO) { runCatching { app.chatWith(key) }.getOrNull() }
            if (chat != null) go(Screen.Chat(chat)) else message = R.string.contact_offline
        }
    }

    fun setHistory(key: String, on: Boolean) = io { app.setHistory(key, on) }

    fun rename(key: String, name: String) = io { app.renameContact(key, name.trim()) }

    fun removeContact(key: String) = io { app.removeContact(key) }

    fun addById(id: String) {
        val clean = id.trim()
        if (!isIdentifier(clean)) {
            idError = R.string.invalid_id
            return
        }
        idError = null
        io { app.addById(clean) }
    }

    fun accept(id: String) {
        awaitingStranger = true
        io { app.acceptRequest(id) }
    }

    fun decline(id: String) = io { app.declineRequest(id) }

    fun exportBackup(passphrase: String, target: Uri) {
        viewModelScope.launch {
            busy = true
            val ok = withContext(Dispatchers.IO) {
                runCatching {
                    val bytes = app.exportBackup(passphrase)
                    getApplication<Application>().contentResolver.openOutputStream(target)?.use { it.write(bytes) }
                        ?: error("no stream")
                }.isSuccess
            }
            busy = false
            backupResult = getApplication<Application>().getString(if (ok) R.string.backup_saved else R.string.error_generic)
        }
    }

    fun importBackup(passphrase: String, source: Uri) {
        viewModelScope.launch {
            busy = true
            val n = withContext(Dispatchers.IO) {
                runCatching {
                    val bytes = getApplication<Application>().contentResolver.openInputStream(source)?.use { it.readBytes() }
                        ?: error("no stream")
                    app.importBackup(bytes, passphrase)
                }.getOrNull()
            }
            busy = false
            val ctx = getApplication<Application>()
            backupResult = if (n != null) ctx.resources.getQuantityString(R.plurals.backup_done, n.toInt(), n.toInt()) else ctx.getString(R.string.backup_bad)
        }
    }

    // ── settings ──

    fun updateSettings(change: (Settings) -> Settings) {
        val before = settings.value
        val s = engine.settings.update(change)
        io {
            if (s.name != before.name) app.setName(s.name)
            if (s.ttl != before.ttl) app.setTtl(s.ttl.toULong())
            if (s.nearby != before.nearby) app.setLocal(s.nearby)
            if (s.relays != before.relays || s.trackers != before.trackers || s.stun != before.stun ||
                s.turnUrl != before.turnUrl || s.turnUser != before.turnUser ||
                s.turnCredential != before.turnCredential || s.relayOnly != before.relayOnly
            ) {
                engine.applyNetwork(s)
            }
        }
    }

    /** Permissions were just granted: the radios can do what the engine asked. */
    fun nearbyAllowed() = engine.nearby.refresh()

    private companion object {
        const val MIN_VAULT = 10

        fun safeName(name: String?): String {
            val n = name?.substringAfterLast('/')?.replace(Regex("[\\u0000-\\u001f\\\\:*?\"<>|]"), "_")?.trim()
            return if (n.isNullOrEmpty() || n == "." || n == "..") "file" else n.take(200)
        }
    }
}
