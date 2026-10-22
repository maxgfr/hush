import Foundation
import Testing
@testable import HushKit

struct HushStateTests {
    var s = HushState()

    init() {
        s.apply(.chatOpened(chat: 1, kind: .link(group: false, creator: true), url: "https://x/#2.d.k", expiresMs: 99))
    }

    @Test func aLinkOpensWaitingThenAPeerJoins() {
        var s = s
        let c = s.chats[1]!
        #expect(c.creator && !c.group && c.status == .waiting && !c.isOpen)
        #expect(c.url == "https://x/#2.d.k" && c.expiresMs == 99)
        s.apply(.chatStatus(chat: 1, status: .open))
        s.apply(.peerJoined(chat: 1, peer: "p", name: "Bob", sas: Data([1, 2, 3, 4, 5]), verified: false))
        s.apply(.peerPath(chat: 1, peer: "p", path: .wifi))
        let p = s.chats[1]!.peer("p")!
        #expect(p.name == "Bob" && p.path == .wifi && p.needsCheck)
        s.apply(.peerVerified(chat: 1, peer: "p", byMe: true, byPeer: false))
        #expect(s.chats[1]!.peer("p")!.needsCheck)
        s.apply(.peerVerified(chat: 1, peer: "p", byMe: true, byPeer: true))
        #expect(s.chats[1]!.peer("p")!.verified)
        s.apply(.peerRenamed(chat: 1, peer: "p", name: "Robert"))
        #expect(s.chats[1]!.peer("p")!.name == "Robert")
        // Joining again (a reconnection) replaces, never duplicates.
        s.apply(.peerJoined(chat: 1, peer: "p", name: "Robert", sas: Data(), verified: true))
        #expect(s.chats[1]!.peers.count == 1)
        s.apply(.peerLeft(chat: 1, peer: "p", name: "Robert"))
        #expect(!s.chats[1]!.isOpen)
    }

    @Test func messagesMineTheirsDeletedAndUnread() {
        var s = s
        s.apply(.message(chat: 1, peer: nil, id: "a", author: "Me", text: "hi", ts: 1, expiresAt: nil, past: false))
        s.apply(.message(chat: 1, peer: "p", id: "b", author: "Bob", text: "hello", ts: 2, expiresAt: nil, past: false))
        // The same message twice is shown once.
        s.apply(.message(chat: 1, peer: "p", id: "b", author: "Bob", text: "hello", ts: 2, expiresAt: nil, past: false))
        var c = s.chats[1]!
        #expect(c.items.map(\.id) == ["a", "b"])
        #expect(c.items[0].mine && !c.items[1].mine)
        #expect(c.unread == 1)
        s.visible = 1
        s.apply(.message(chat: 1, peer: "p", id: "c", author: "Bob", text: "again", ts: 3, expiresAt: nil, past: false))
        #expect(s.chats[1]!.unread == 1)
        s.apply(.messageDeleted(chat: 1, id: "b"))
        c = s.chats[1]!
        #expect(c.items.map(\.id) == ["a", "c"])
    }

    @Test func historyGoesInTimeOrder() {
        var s = s
        s.apply(.message(chat: 1, peer: "p", id: "new", author: "Bob", text: "now", ts: 50, expiresAt: nil, past: false))
        s.apply(.message(chat: 1, peer: "p", id: "old", author: "Bob", text: "then", ts: 10, expiresAt: nil, past: true))
        #expect(s.chats[1]!.items.map(\.id) == ["old", "new"])
        #expect(s.chats[1]!.unread == 1)
    }

    @Test func disappearingMessagesGo() {
        var s = s
        s.apply(.message(chat: 1, peer: "p", id: "a", author: "Bob", text: "brief", ts: 1, expiresAt: 1000, past: false))
        s.apply(.message(chat: 1, peer: "p", id: "b", author: "Bob", text: "stays", ts: 1, expiresAt: nil, past: false))
        let early = s.expire(now: 999)
        let due = s.expire(now: 1000)
        #expect(!early && due)
        #expect(s.chats[1]!.items.map(\.id) == ["b"])
    }

    @Test func filesTheirsAndOurs() {
        var s = s
        s.apply(.peerJoined(chat: 1, peer: "p", name: "Bob", sas: Data(), verified: true))
        s.apply(.fileOffered(chat: 1, peer: "p", id: "f", name: "a.pdf", size: 10, mime: "application/pdf", expiresAt: nil))
        s.apply(.fileOffered(chat: 1, peer: nil, id: "g", name: "b.png", size: 20, mime: "image/png", expiresAt: nil))
        func file(_ id: String) -> FileModel? {
            guard case let .file(f) = s.chats[1]!.items.first(where: { $0.id == id })?.body else { return nil }
            return f
        }
        #expect(file("f")?.state == .offered)
        #expect(s.chats[1]!.items.first { $0.id == "f" }?.author == "Bob")
        #expect(file("g")?.state == .sent)
        s.apply(.fileProgress(chat: 1, id: "f", done: 5, total: 10))
        #expect(file("f")?.state == .receiving(done: 5, total: 10))
        s.apply(.fileSaved(chat: 1, id: "f", path: "/tmp/a.pdf"))
        #expect(file("f")?.state == .saved(path: "/tmp/a.pdf"))
        s.apply(.fileProgress(chat: 1, id: "g", done: 5, total: 20))
        #expect(file("g")?.state == .sent)
        s.apply(.fileDelivered(chat: 1, id: "g", peer: "p"))
        #expect(file("g")?.state == .delivered)
        s.apply(.fileFailed(chat: 1, id: "f", reason: "cancelled"))
        #expect(file("f")?.state == .failed(reason: "cancelled"))
    }

    @Test func noticesOffersLockAndEnd() {
        var s = s
        s.apply(.notice(chat: 1, notice: .joined(name: "Bob")))
        s.apply(.notice(chat: 1, notice: .joined(name: "Bob")))
        #expect(s.chats[1]!.items.count == 2)
        #expect(s.chats[1]!.items.allSatisfy { $0.isNotice })
        #expect(s.chats[1]!.unread == 0)
        s.apply(.cardOffer(chat: 1, peer: "p", name: "Bob", id: "hush1x"))
        s.apply(.cardOffer(chat: 1, peer: "p", name: "Bob", id: "hush1x"))
        #expect(s.chats[1]!.offers.count == 1)
        s.dismissOffer(1, peer: "p")
        #expect(s.chats[1]!.offers.isEmpty)
        s.apply(.locked(chat: 1, locked: true))
        #expect(s.chats[1]!.locked)
        #expect(s.liveChats.count == 1)
        s.apply(.chatEnded(chat: 1, reason: .taken))
        #expect(s.chats[1]!.ended == .taken)
        #expect(s.liveChats.isEmpty)
        s.dismiss(1)
        #expect(s.chats.isEmpty && s.order.isEmpty)
    }

    @Test func vaultIdentityContactsLookups() {
        var s = s
        let c = ContactEntry(key: "k", id: "hush1a", name: "Ada", history: false, online: true, chat: 7)
        s.apply(.vault(status: .unlocked))
        s.apply(.identity(id: "hush1me"))
        s.apply(.contacts(list: [c]))
        s.apply(.requests(list: [RequestEntry(id: "hush1b", name: "B", note: "")]))
        s.apply(.reachable(on: true))
        s.apply(.lookup(id: "hush1c", status: .requested, chat: 3))
        #expect(s.vault == .unlocked && s.identity == "hush1me" && s.reachable)
        #expect(s.contacts == [c] && s.requests.count == 1)
        #expect(s.lookups["hush1c"] == LookupModel(status: .requested, chat: 3))
        // Locking hides what the vault holds.
        s.apply(.vault(status: .locked))
        #expect(s.contacts.isEmpty && s.requests.isEmpty)
        #expect(HushStore.lookupKey("  HUSH1C ") == "hush1c")
    }

    @Test func eventsForUnknownChatsAreIgnored() {
        var s = s
        let before = s
        s.apply(.message(chat: 9, peer: "p", id: "a", author: "", text: "", ts: 0, expiresAt: nil, past: false))
        s.apply(.peerJoined(chat: 9, peer: "p", name: "", sas: Data(), verified: false))
        s.apply(.chatEnded(chat: 9, reason: .ended))
        #expect(s == before)
    }
}
