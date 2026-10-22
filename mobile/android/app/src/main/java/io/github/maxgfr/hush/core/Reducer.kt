package io.github.maxgfr.hush.core

import uniffi.hush_ffi.AppEvent

/** Folds the engine's events into [UiState]. Pure: the store runs it on the main thread. */
object Reducer {
    fun reduce(state: UiState, event: AppEvent, now: Long): UiState = when (event) {
        is AppEvent.ChatOpened -> state.copy(
            chats = state.chats + (
                event.chat to (state.chats[event.chat] ?: ChatState(event.chat, event.kind)).copy(
                    kind = event.kind,
                    url = event.url,
                    expiresMs = event.expiresMs?.toLong(),
                    ended = null,
                )
                ),
        )
        is AppEvent.ChatStatus -> state.chat(event.chat) { it.copy(status = event.status) }
        is AppEvent.Code -> state
        is AppEvent.PeerJoined -> state.chat(event.chat) { c ->
            val p = PeerState(
                tag = event.peer,
                name = event.name,
                sas = event.sas.map { it.toInt() and 0xff },
                byMe = event.verified,
                byPeer = event.verified,
            )
            c.copy(peers = c.peers.filter { it.tag != event.peer } + p)
        }
        is AppEvent.PeerRenamed -> state.peer(event.chat, event.peer) { it.copy(name = event.name) }
        is AppEvent.PeerVerified -> state.peer(event.chat, event.peer) {
            it.copy(byMe = event.byMe, byPeer = event.byPeer)
        }
        is AppEvent.PeerPath -> state.peer(event.chat, event.peer) { it.copy(path = event.path) }
        is AppEvent.PeerLeft -> state.chat(event.chat) { c ->
            c.copy(
                peers = c.peers.map { if (it.tag == event.peer) it.copy(left = true) else it },
                offers = c.offers.filter { it.peer != event.peer },
            )
        }
        is AppEvent.Locked -> state.chat(event.chat) { it.copy(locked = event.locked) }
        is AppEvent.Message -> state.chat(event.chat) { c ->
            c.put(
                MessageItem(
                    id = event.id,
                    peer = event.peer,
                    author = event.author,
                    text = event.text,
                    ts = event.ts.toLong(),
                    expiresAt = event.expiresAt?.toLong(),
                    past = event.past,
                ),
            )
        }
        is AppEvent.MessageDeleted -> state.chat(event.chat) { c -> c.copy(items = c.items.filter { it.id != event.id }) }
        is AppEvent.FileOffered -> state.chat(event.chat) { c ->
            c.put(
                FileItem(
                    id = event.id,
                    peer = event.peer,
                    name = event.name,
                    size = event.size.toLong(),
                    mime = event.mime,
                    ts = now,
                    expiresAt = event.expiresAt?.toLong(),
                    state = if (event.peer == null) FileState.SENDING else FileState.OFFERED,
                ),
            )
        }
        is AppEvent.FileProgress -> state.file(event.chat, event.id) {
            it.copy(
                state = if (it.mine) FileState.SENDING else FileState.DOWNLOADING,
                done = event.done.toLong(),
                total = event.total.toLong(),
            )
        }
        is AppEvent.FileSaved -> state.file(event.chat, event.id) {
            it.copy(state = FileState.SAVED, path = event.path, done = it.total)
        }
        is AppEvent.FileFailed -> state.file(event.chat, event.id) {
            it.copy(state = FileState.FAILED, reason = event.reason)
        }
        is AppEvent.FileDelivered -> state.file(event.chat, event.id) {
            it.copy(state = FileState.DELIVERED, done = it.total)
        }
        is AppEvent.Notice -> state.chat(event.chat) { c ->
            c.copy(items = c.items + NoticeItem("notice-${c.items.size}-$now", event.notice, now))
        }
        is AppEvent.CardOffer -> state.chat(event.chat) { c ->
            c.copy(offers = c.offers.filter { it.peer != event.peer } + CardOffer(event.peer, event.name, event.id))
        }
        is AppEvent.ChatEnded -> state.chat(event.chat) { it.copy(ended = event.reason) }
        is AppEvent.Vault -> state.copy(vault = event.status)
        is AppEvent.Identity -> state.copy(identity = event.id)
        is AppEvent.Contacts -> state.copy(contacts = event.list)
        is AppEvent.Requests -> state.copy(requests = event.list)
        is AppEvent.Reachable -> state.copy(reachable = event.on)
        is AppEvent.Lookup -> state.copy(lookup = Lookup(event.id, event.status, event.chat))
    }.prune(now)

    /** Drops what has disappeared, so a timer's end leaves nothing behind in memory either. */
    fun UiState.prune(now: Long): UiState {
        if (chats.values.none { c -> c.items.any { (it.expiresAt ?: Long.MAX_VALUE) <= now } }) return this
        return copy(chats = chats.mapValues { (_, c) -> c.copy(items = c.items.filter { (it.expiresAt ?: Long.MAX_VALUE) > now }) })
    }

    // Local changes the UI makes before the engine answers.

    fun downloading(state: UiState, chat: ULong, id: String): UiState =
        state.file(chat, id) { it.copy(state = FileState.DOWNLOADING, done = 0) }

    fun cancelled(state: UiState, chat: ULong, id: String): UiState =
        state.file(chat, id) { it.copy(state = FileState.OFFERED, done = 0) }

    fun dismissOffer(state: UiState, chat: ULong, peer: String): UiState =
        state.chat(chat) { c -> c.copy(offers = c.offers.filter { it.peer != peer }) }

    /** Leaves an ended conversation behind for good. */
    fun forget(state: UiState, chat: ULong): UiState = state.copy(chats = state.chats - chat)

    private inline fun UiState.chat(id: ULong, change: (ChatState) -> ChatState): UiState {
        val c = chats[id] ?: return this
        return copy(chats = chats + (id to change(c)))
    }

    private inline fun UiState.peer(chat: ULong, tag: String, crossinline change: (PeerState) -> PeerState): UiState =
        chat(chat) { c -> c.copy(peers = c.peers.map { if (it.tag == tag) change(it) else it }) }

    private inline fun UiState.file(chat: ULong, id: String, crossinline change: (FileItem) -> FileItem): UiState =
        chat(chat) { c -> c.copy(items = c.items.map { if (it is FileItem && it.id == id) change(it) else it }) }

    /** Adds an item, or replaces the one with the same id (history restored twice, say). */
    private fun ChatState.put(item: ChatItem): ChatState {
        val i = items.indexOfFirst { it.id == item.id && it::class == item::class }
        return copy(items = if (i < 0) items + item else items.toMutableList().also { it[i] = item })
    }
}
