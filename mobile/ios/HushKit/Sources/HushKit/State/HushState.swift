import Foundation

/// Someone in a conversation.
public struct PeerModel: Identifiable, Equatable, Sendable {
    /// The engine's tag for this peer.
    public let id: String
    public var name: String
    /// Emoji indices to compare (empty for contacts, verified when saved).
    public var sas: Data
    public var verifiedByMe: Bool
    public var verifiedByPeer: Bool
    public var path: PeerPath = .connecting

    public var verified: Bool { verifiedByMe && verifiedByPeer }
    /// The emoji panel is still to be shown.
    public var needsCheck: Bool { !sas.isEmpty && !verified }
}

public struct FileModel: Equatable, Sendable {
    public enum State: Equatable, Sendable {
        /// Theirs, not asked for yet.
        case offered
        case receiving(done: UInt64, total: UInt64)
        /// Theirs, saved at this path in the app's container.
        case saved(path: String)
        /// Ours, offered.
        case sent
        /// Ours, received intact by someone.
        case delivered
        case failed(reason: String)

        /// Theirs, still coming.
        var theirs: Bool {
            switch self {
            case .offered, .receiving: true
            default: false
            }
        }
    }

    public let id: String
    public var name: String
    public var size: UInt64
    public var mime: String
    public var state: State
}

public struct ChatItem: Identifiable, Equatable, Sendable {
    public enum Body: Equatable, Sendable {
        case text(String)
        case file(FileModel)
        case notice(NoticeKind)
    }

    public let id: String
    /// The author's tag; nil for ours (and for notices).
    public var peer: String?
    public var author: String
    public var ts: UInt64
    public var expiresAt: UInt64?
    public var body: Body
    public var mine: Bool

    public var isNotice: Bool {
        if case .notice = body { return true }
        return false
    }
}

public struct CardOfferModel: Identifiable, Equatable, Sendable {
    public var id: String { peer }
    public let peer: String
    public let name: String
    public let identifier: String
}

/// One conversation as the screens show it.
public struct ChatModel: Identifiable, Equatable, Sendable {
    public let id: UInt64
    public var kind: ChatType
    /// The link to share, for a link we created.
    public var url: String?
    /// When the link stops working (ms since the epoch).
    public var expiresMs: UInt64?
    public var status: Status = .waiting
    public var peers: [PeerModel] = []
    public var locked = false
    public var items: [ChatItem] = []
    /// A hand-carried code to show (offline mode).
    public var code: String?
    public var offers: [CardOfferModel] = []
    public var ended: Ending?
    public var unread = 0

    public var group: Bool {
        if case let .link(group, _) = kind { return group }
        return false
    }

    public var creator: Bool {
        if case let .link(_, creator) = kind { return creator }
        return false
    }

    public var contactKey: String? {
        if case let .contact(key) = kind { return key }
        return nil
    }

    public var isOpen: Bool { !peers.isEmpty }

    public func peer(_ tag: String) -> PeerModel? {
        peers.first { $0.id == tag }
    }
}

public struct LookupModel: Equatable, Sendable {
    public var status: LookupStatus
    public var chat: UInt64?
}

/// Everything the engine said, reduced to what the screens show. Pure: tested on its own.
public struct HushState: Equatable, Sendable {
    public var chats: [UInt64: ChatModel] = [:]
    /// Conversations in the order they opened.
    public var order: [UInt64] = []
    public var vault: VaultStatus = .none
    public var identity: String?
    public var contacts: [ContactEntry] = []
    public var requests: [RequestEntry] = []
    public var reachable = false
    public var lookups: [String: LookupModel] = [:]
    /// The conversation on screen: its messages are not unread.
    public var visible: UInt64?

    public init() {}

    public var liveChats: [ChatModel] {
        order.compactMap { chats[$0] }.filter { $0.ended == nil }
    }

    public mutating func apply(_ event: AppEvent) {
        switch event {
        case let .chatOpened(chat, kind, url, expiresMs):
            if chats[chat] == nil { order.append(chat) }
            chats[chat] = ChatModel(id: chat, kind: kind, url: url, expiresMs: expiresMs)
        case let .chatStatus(chat, status):
            chats[chat]?.status = status
        case let .code(chat, code):
            chats[chat]?.code = code
        case let .peerJoined(chat, peer, name, sas, verified):
            let p = PeerModel(id: peer, name: name, sas: sas, verifiedByMe: verified, verifiedByPeer: verified)
            update(chat) { c in
                if let i = c.peers.firstIndex(where: { $0.id == peer }) {
                    c.peers[i] = p
                } else {
                    c.peers.append(p)
                }
            }
        case let .peerRenamed(chat, peer, name):
            updatePeer(chat, peer) { $0.name = name }
        case let .peerVerified(chat, peer, byMe, byPeer):
            updatePeer(chat, peer) {
                $0.verifiedByMe = byMe
                $0.verifiedByPeer = byPeer
            }
        case let .peerPath(chat, peer, path):
            updatePeer(chat, peer) { $0.path = path }
        case let .peerLeft(chat, peer, _):
            update(chat) { c in
                c.peers.removeAll { $0.id == peer }
                c.offers.removeAll { $0.peer == peer }
            }
        case let .locked(chat, locked):
            chats[chat]?.locked = locked
        case let .message(chat, peer, id, author, text, ts, expiresAt, past):
            let item = ChatItem(id: id, peer: peer, author: author, ts: ts, expiresAt: expiresAt, body: .text(text), mine: peer == nil)
            add(item, to: chat, past: past)
        case let .messageDeleted(chat, id):
            // Notices are ours alone: no one deletes them.
            update(chat) { $0.items.removeAll { $0.id == id && !$0.isNotice } }
        case let .fileOffered(chat, peer, id, name, size, mime, expiresAt):
            let file = FileModel(id: id, name: name, size: size, mime: mime, state: peer == nil ? .sent : .offered)
            let author = peer.flatMap { chats[chat]?.peer($0)?.name } ?? ""
            let ts = UInt64(Date().timeIntervalSince1970 * 1000)
            add(ChatItem(id: id, peer: peer, author: author, ts: ts, expiresAt: expiresAt, body: .file(file), mine: peer == nil), to: chat, past: false)
        case let .fileProgress(chat, id, done, total):
            updateFile(chat, id) { f in
                // Ours too, when we send it: only theirs show progress, until saved.
                guard f.state.theirs else { return }
                f.state = .receiving(done: done, total: total)
            }
        case let .fileSaved(chat, id, path):
            // Once: a second file under the same id never takes the first one's place.
            updateFile(chat, id) { f in
                guard f.state.theirs else { return }
                f.state = .saved(path: path)
            }
        case let .fileFailed(chat, id, reason):
            updateFile(chat, id) { $0.state = .failed(reason: reason) }
        case let .fileDelivered(chat, id, _):
            updateFile(chat, id) { f in
                guard f.state == .sent || f.state == .delivered else { return }
                f.state = .delivered
            }
        case let .notice(chat, notice):
            // An id no one can guess, so a message cannot take it first.
            let ts = UInt64(Date().timeIntervalSince1970 * 1000)
            add(ChatItem(id: "notice-\(UUID().uuidString)", peer: nil, author: "", ts: ts, expiresAt: nil, body: .notice(notice), mine: false), to: chat, past: false)
        case let .cardOffer(chat, peer, name, id):
            update(chat) { c in
                c.offers.removeAll { $0.peer == peer }
                c.offers.append(CardOfferModel(peer: peer, name: name, identifier: id))
            }
        case let .chatEnded(chat, reason):
            chats[chat]?.ended = reason
        case let .vault(status):
            vault = status
            if status != .unlocked {
                contacts = []
                requests = []
            }
        case let .identity(id):
            identity = id
        case let .contacts(list):
            contacts = list
        case let .requests(list):
            requests = list
        case let .reachable(on):
            reachable = on
        case let .lookup(id, status, chat):
            lookups[id] = LookupModel(status: status, chat: chat)
        }
    }

    /// Drops disappearing messages whose time has come; true when something went.
    @discardableResult
    public mutating func expire(now: UInt64) -> Bool {
        var changed = false
        for (id, c) in chats {
            let kept = c.items.filter { ($0.expiresAt ?? .max) > now }
            if kept.count != c.items.count {
                chats[id]?.items = kept
                changed = true
            }
        }
        return changed
    }

    /// Forgets an ended conversation once its end screen is left.
    public mutating func dismiss(_ chat: UInt64) {
        chats[chat] = nil
        order.removeAll { $0 == chat }
    }

    public mutating func dismissOffer(_ chat: UInt64, peer: String) {
        chats[chat]?.offers.removeAll { $0.peer == peer }
    }

    private mutating func update(_ chat: UInt64, _ change: (inout ChatModel) -> Void) {
        guard var c = chats[chat] else { return }
        change(&c)
        chats[chat] = c
    }

    private mutating func updatePeer(_ chat: UInt64, _ peer: String, _ change: (inout PeerModel) -> Void) {
        update(chat) { c in
            guard let i = c.peers.firstIndex(where: { $0.id == peer }) else { return }
            change(&c.peers[i])
        }
    }

    private mutating func updateFile(_ chat: UInt64, _ id: String, _ change: (inout FileModel) -> Void) {
        update(chat) { c in
            guard let i = c.items.firstIndex(where: { $0.id == id }), case var .file(f) = c.items[i].body else { return }
            change(&f)
            c.items[i].body = .file(f)
        }
    }

    private mutating func add(_ item: ChatItem, to chat: UInt64, past: Bool) {
        let unseen = visible != chat
        update(chat) { c in
            // An id already shown keeps what it shows: a repeat, or someone reusing it, is dropped.
            guard !c.items.contains(where: { $0.id == item.id }) else { return }
            if past {
                // History restored from the vault: in time order, before what is new.
                let i = c.items.firstIndex { $0.ts > item.ts } ?? c.items.endIndex
                c.items.insert(item, at: i)
            } else {
                c.items.append(item)
            }
            if !item.mine && !item.isNotice && !past && unseen { c.unread += 1 }
        }
    }
}
