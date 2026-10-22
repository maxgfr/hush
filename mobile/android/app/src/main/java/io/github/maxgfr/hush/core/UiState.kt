package io.github.maxgfr.hush.core

import uniffi.hush_ffi.ChatType
import uniffi.hush_ffi.ContactEntry
import uniffi.hush_ffi.Ending
import uniffi.hush_ffi.LookupStatus
import uniffi.hush_ffi.NoticeKind
import uniffi.hush_ffi.PeerPath
import uniffi.hush_ffi.RequestEntry
import uniffi.hush_ffi.Status
import uniffi.hush_ffi.VaultStatus

/** Everything the screens show, rebuilt from the engine's events (see [Reducer]). */
data class UiState(
    val chats: Map<ULong, ChatState> = emptyMap(),
    val vault: VaultStatus = VaultStatus.NONE,
    val identity: String? = null,
    val contacts: List<ContactEntry> = emptyList(),
    val requests: List<RequestEntry> = emptyList(),
    val reachable: Boolean = false,
    val lookup: Lookup? = null,
) {
    /** Conversations still going, oldest first. */
    val live: List<ChatState> get() = chats.values.filter { it.ended == null }.sortedBy { it.id }
}

data class Lookup(val id: String, val status: LookupStatus, val chat: ULong?)

data class ChatState(
    val id: ULong,
    val kind: ChatType,
    val url: String? = null,
    /** When the link stops working (ms since the epoch), for links. */
    val expiresMs: Long? = null,
    val status: Status = Status.WAITING,
    /** In the order they joined. */
    val peers: List<PeerState> = emptyList(),
    val items: List<ChatItem> = emptyList(),
    val locked: Boolean = false,
    val offers: List<CardOffer> = emptyList(),
    val ended: Ending? = null,
) {
    val group: Boolean get() = (kind as? ChatType.Link)?.group == true
    val creator: Boolean get() = (kind as? ChatType.Link)?.creator == true
    val isLink: Boolean get() = kind is ChatType.Link
    val contactKey: String? get() = (kind as? ChatType.Contact)?.key
    val present: List<PeerState> get() = peers.filter { !it.left }
    val passphrase: Boolean get() = url?.endsWith(".p") == true

    fun peer(tag: String): PeerState? = peers.firstOrNull { it.tag == tag }
}

data class PeerState(
    val tag: String,
    val name: String,
    /** Emoji indices to compare; empty for contacts (verified when saved). */
    val sas: List<Int>,
    val byMe: Boolean,
    val byPeer: Boolean,
    val path: PeerPath = PeerPath.CONNECTING,
    val left: Boolean = false,
) {
    val verified: Boolean get() = byMe && byPeer
}

data class CardOffer(val peer: String, val name: String, val id: String)

sealed interface ChatItem {
    val id: String
    val ts: Long

    /** When it disappears (ms since the epoch), if it does. */
    val expiresAt: Long?
}

data class MessageItem(
    override val id: String,
    /** Null for ours. */
    val peer: String?,
    val author: String,
    val text: String,
    override val ts: Long,
    override val expiresAt: Long?,
    val past: Boolean,
) : ChatItem {
    val mine: Boolean get() = peer == null
}

enum class FileState { OFFERED, DOWNLOADING, SAVED, FAILED, SENDING, DELIVERED }

data class FileItem(
    override val id: String,
    /** Null for ours. */
    val peer: String?,
    val name: String,
    val size: Long,
    val mime: String,
    override val ts: Long,
    override val expiresAt: Long?,
    val state: FileState,
    val done: Long = 0,
    val total: Long = size,
    val path: String? = null,
    val reason: String? = null,
) : ChatItem {
    val mine: Boolean get() = peer == null
    val progress: Float get() = if (total <= 0) 0f else (done.toFloat() / total).coerceIn(0f, 1f)
}

data class NoticeItem(
    override val id: String,
    val notice: NoticeKind,
    override val ts: Long,
) : ChatItem {
    override val expiresAt: Long? get() = null
}
