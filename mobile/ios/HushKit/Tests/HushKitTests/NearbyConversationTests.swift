import Foundation
import Testing
@testable import HushKit

/// Two engines in this process, no network at all: only the app's own nearby links between them.
struct NearbyConversationTests {
    /// Real TCP links (Network.framework) on the loopback interface; the loopback directory
    /// stands in for Bonjour, which is not dependable on CI machines.
    @Test(.timeLimit(.minutes(2)))
    func overWifiTCP() async throws {
        let dir = LoopbackDirectory()
        let alice = try Person("Alice", .init(wifi: .loopback(dir), bluetooth: false))
        let bob = try Person("Bob", .init(wifi: .loopback(dir), bluetooth: false))
        try await conversation(alice, bob, path: .wifi)
    }

    /// The L2CAP stream link over bound stream pairs, as a Bluetooth channel would give them.
    @Test(.timeLimit(.minutes(2)))
    func overBluetoothStreams() async throws {
        let alice = try Person("Alice", .init(wifi: nil, bluetooth: false))
        let bob = try Person("Bob", .init(wifi: nil, bluetooth: false))
        try await conversation(alice, bob, path: .bluetooth) {
            // Once Bob looks for Alice's token, "find" her.
            let token = try await until("Bob scans for Alice's token") {
                let a = alice.transport.queue.sync { alice.transport.advertising }
                let b = bob.transport.queue.sync { bob.transport.scanning }
                return a.intersection(b).first
            }
            var aIn: InputStream?, aOut: OutputStream?, bIn: InputStream?, bOut: OutputStream?
            Stream.getBoundStreams(withBufferSize: 4096, inputStream: &aIn, outputStream: &bOut)
            Stream.getBoundStreams(withBufferSize: 4096, inputStream: &bIn, outputStream: &aOut)
            alice.transport.openStream(input: aIn!, output: aOut!, token: nil, medium: .bluetooth)
            bob.transport.openStream(input: bIn!, output: bOut!, token: token, medium: .bluetooth)
        }
    }

    /// Link, emojis, verification, messages both ways, a file, and the end.
    func conversation(_ alice: Person, _ bob: Person, path: PeerPath, connect: () async throws -> Void = {}) async throws {
        let ca = try alice.app.createLink(group: false, ttlMin: 10, passphrase: nil)
        let url = try await alice.events.wait("the link") {
            if case let .chatOpened(chat, _, url, _) = $0, chat == ca { return url }
            return nil
        }
        let info = try linkInfo(input: url)
        #expect(!info.group && !info.passphrase)
        let cb = try bob.app.joinLink(input: url, passphrase: nil)
        try await connect()

        let (bobTag, sasA) = try await alice.events.wait("Bob") {
            if case let .peerJoined(chat, peer, _, sas, _) = $0, chat == ca { return (peer, sas) }
            return nil
        }
        let (_, sasB) = try await bob.events.wait("Alice") {
            if case let .peerJoined(chat, peer, _, sas, _) = $0, chat == cb { return (peer, sas) }
            return nil
        }
        #expect(sasA == sasB, "both see the same emojis")
        #expect(sasA.count == 5)
        #expect(sasRender(indices: sasA).split(separator: " ").count == 5)
        for (p, chat) in [(alice, ca), (bob, cb)] {
            let seen = try await p.events.wait("a path") {
                if case let .peerPath(c, _, path) = $0, c == chat, path != .connecting { return path }
                return nil
            }
            #expect(seen == path)
        }

        alice.app.verify(chat: ca, peer: bobTag, ok: true)
        bob.app.verify(chat: cb, peer: nil, ok: true)
        for (p, chat) in [(alice, ca), (bob, cb)] {
            try await p.events.wait("verified by both") {
                if case let .peerVerified(c, _, true, true) = $0, c == chat { return () }
                return nil
            }
        }

        alice.app.sendText(chat: ca, text: "hello Bob")
        let author = try await bob.events.wait("hello Bob") {
            if case let .message(c, peer?, _, author, "hello Bob", _, _, _) = $0, c == cb, !peer.isEmpty { return author }
            return nil
        }
        #expect(author == "Alice")
        bob.app.sendText(chat: cb, text: "hello Alice\nsecond line")
        try await alice.events.wait("hello Alice") {
            if case let .message(c, _?, _, "Bob", "hello Alice\nsecond line", _, _, _) = $0, c == ca { return () }
            return nil
        }

        // A file bigger than a chunk, staged the way the app stages picked files.
        let data = Data((0..<300_000).map { UInt8(truncatingIfNeeded: $0 * 31 % 251) })
        let staged = try alice.storage.stage(data, name: "photo.bin", chat: ca)
        alice.app.sendFile(chat: ca, path: staged.path)
        let id = try await bob.events.wait("the offer") {
            if case let .fileOffered(c, _?, id, "photo.bin", size, _, _) = $0, c == cb, size == 300_000 { return id }
            return nil
        }
        bob.app.getFile(chat: cb, id: id)
        let saved = try await bob.events.wait("the file") {
            if case let .fileSaved(c, i, path) = $0, c == cb, i == id { return path }
            return nil
        }
        let got = try Data(contentsOf: URL(fileURLWithPath: saved))
        #expect(got == data)
        #expect(saved.hasPrefix(bob.storage.received.path))
        try await alice.events.wait("delivery") {
            if case let .fileDelivered(c, i, _) = $0, c == ca, i == id { return () }
            return nil
        }

        alice.app.end(chat: ca)
        let reason = try await bob.events.wait("the end") {
            if case let .chatEnded(c, reason) = $0, c == cb { return reason }
            return nil
        }
        #expect(reason == .peerLeft)

        // What Bob's screens would show.
        var state = HushState()
        for e in bob.events.all { state.apply(e) }
        let chat = try #require(state.chats[cb])
        #expect(chat.ended == .peerLeft)
        #expect(chat.items.contains { $0.body == .text("hello Bob") && !$0.mine && $0.author == "Alice" })
        #expect(chat.items.contains { $0.body == .text("hello Alice\nsecond line") && $0.mine })
        #expect(chat.items.contains {
            if case let .file(f) = $0.body { return f.state == .saved(path: saved) }
            return false
        })

        for p in [alice, bob] { p.app.shutdown() }
    }
}

struct Person {
    let app: HushApp
    let events = Events()
    let transport: NearbyTransport
    let storage: Storage

    init(_ name: String, _ options: NearbyTransport.Options) throws {
        storage = try Storage.temporary()
        transport = NearbyTransport(options)
        app = try HushApp(
            config: AppConfig(
                name: name, base: linkBase, relays: [], trackers: [], iceServers: [], relayOnly: false,
                outDir: storage.received.path, dataDir: nil, local: true, ttl: 0
            ),
            listener: events,
            local: transport
        )
        transport.attach(app)
    }
}

/// Every event one engine sent, to wait for the ones a test expects.
final class Events: AppListener, @unchecked Sendable {
    private let lock = NSLock()
    private var seen: [AppEvent] = []

    func onEvent(event: AppEvent) {
        lock.withLock { seen.append(event) }
    }

    var all: [AppEvent] { lock.withLock { seen } }

    /// The first event (already seen or coming) that `pick` accepts, within 30 s.
    @discardableResult
    func wait<T>(_ what: String, _ pick: @escaping (AppEvent) -> T?) async throws -> T {
        try await until(what) { self.all.lazy.compactMap(pick).first }
    }
}

struct Timeout: Error, CustomStringConvertible {
    let what: String
    var description: String { "timed out waiting for \(what)" }
}

func until<T>(_ what: String, seconds: Double = 30, _ check: () throws -> T?) async throws -> T {
    let deadline = Date().addingTimeInterval(seconds)
    while Date() < deadline {
        if let t = try check() { return t }
        try await Task.sleep(nanoseconds: 50_000_000)
    }
    throw Timeout(what: what)
}
