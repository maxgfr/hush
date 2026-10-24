import Foundation
import Observation

/// The web app's address: links we create start with it, and opening one opens the app.
public let linkBase = "https://maxgfr.github.io/hush/"

/// Hands a drop's progress to the caller, from the engine's thread.
final class DropProgressRelay: DropProgress, @unchecked Sendable {
    let report: @Sendable (DropPhase, UInt64, UInt64) -> Void

    init(_ report: @escaping @Sendable (DropPhase, UInt64, UInt64) -> Void) {
        self.report = report
    }

    func onProgress(phase: DropPhase, done: UInt64, total: UInt64) {
        report(phase, done, total)
    }
}

/// Hops engine events to the main queue, in order.
final class EventRelay: AppListener, @unchecked Sendable {
    weak var store: HushStore?

    func onEvent(event: AppEvent) {
        DispatchQueue.main.async {
            MainActor.assumeIsolated { self.store?.receive(event) }
        }
    }
}

/// The app's one engine and what it said, for the screens. Blocking engine calls run off the
/// main actor (Argon2id takes about a second).
@MainActor
@Observable
public final class HushStore {
    public private(set) var state = HushState()
    public private(set) var settings: Settings
    /// Conversations joined with a passphrase (a long wait may mean a wrong one).
    public var joinedWithPassphrase: Set<UInt64> = []

    @ObservationIgnored public let engine: any HushAppProtocol
    @ObservationIgnored public let storage: Storage
    @ObservationIgnored private let settingsStorage: SettingsStorage
    @ObservationIgnored private var timer: Timer?

    public init(engine: any HushAppProtocol, storage: Storage, settings: SettingsStorage) {
        self.engine = engine
        self.storage = storage
        settingsStorage = settings
        self.settings = settings.load()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
    }

    public struct Options {
        /// A throwaway engine: temporary storage, no network, no radios (UI tests).
        public var ephemeral: Bool

        public init(ephemeral: Bool = false) {
            self.ephemeral = ephemeral
        }
    }

    /// Starts the engine for this process.
    public static func start(_ options: Options = Options()) throws -> HushStore {
        let storage = try options.ephemeral ? Storage.temporary() : Storage.app()
        let saved: SettingsStorage
        if options.ephemeral {
            var s = Settings()
            s.relays = []
            s.trackers = []
            s.stun = []
            s.nearby = false
            saved = MemorySettingsStorage(s)
        } else {
            saved = DefaultsSettingsStorage()
        }
        let s = saved.load()
        let relay = EventRelay()
        let transport = options.ephemeral ? nil : NearbyTransport()
        let app = try HushApp(
            config: AppConfig(
                name: s.name,
                base: linkBase,
                relays: s.relays,
                trackers: s.trackers,
                iceServers: s.iceServers(credential: saved.turnCredential),
                relayOnly: s.relayOnly,
                outDir: storage.received.path,
                dataDir: storage.data.path,
                local: s.nearby && transport != nil,
                ttl: s.ttl
            ),
            listener: relay,
            local: transport
        )
        transport?.attach(app)
        let store = HushStore(engine: app, storage: storage, settings: saved)
        relay.store = store
        return store
    }

    func receive(_ event: AppEvent) {
        state.apply(event)
        if case let .chatEnded(chat, _) = event { storage.unstage(chat: chat) }
    }

    private func tick() {
        state.expire(now: Self.now)
    }

    static var now: UInt64 { UInt64(Date().timeIntervalSince1970 * 1000) }

    /// Runs a blocking engine call away from the main actor.
    private func off<T: Sendable>(_ call: @escaping @Sendable (any HushAppProtocol) throws -> T) async throws -> T {
        let engine = engine
        return try await Task.detached(priority: .userInitiated) { try call(engine) }.value
    }

    // MARK: settings

    public var turnCredential: String { settingsStorage.turnCredential }

    /// Saves the settings and tells the engine what changed.
    public func update(_ change: (inout Settings) -> Void, turnCredential: String? = nil) {
        let old = settings
        let oldCredential = settingsStorage.turnCredential
        change(&settings)
        settingsStorage.save(settings)
        if let turnCredential { settingsStorage.turnCredential = turnCredential }
        let credential = settingsStorage.turnCredential
        if settings.name != old.name { engine.setName(name: settings.name) }
        if settings.ttl != old.ttl { engine.setTtl(seconds: settings.ttl) }
        if settings.nearby != old.nearby { engine.setLocal(on: settings.nearby) }
        if settings.relays != old.relays || settings.trackers != old.trackers || settings.stun != old.stun
            || settings.turnURL != old.turnURL || settings.turnUsername != old.turnUsername
            || settings.relayOnly != old.relayOnly || credential != oldCredential {
            engine.setNetwork(
                relays: settings.relays,
                trackers: settings.trackers,
                iceServers: settings.iceServers(credential: credential),
                relayOnly: settings.relayOnly
            )
        }
    }

    // MARK: conversations

    public func chat(_ id: UInt64) -> ChatModel? {
        state.chats[id]
    }

    public func createLink(group: Bool, minutes: UInt64, passphrase: String?) async throws -> UInt64 {
        let pass = passphrase.flatMap { $0.isEmpty ? nil : $0 }
        return try await off { try $0.createLink(group: group, ttlMin: minutes, passphrase: pass) }
    }

    public func join(_ link: String, passphrase: String?) async throws -> UInt64 {
        let pass = passphrase.flatMap { $0.isEmpty ? nil : $0 }
        return try await off { try $0.joinLink(input: link, passphrase: pass) }
    }

    public func send(_ text: String, in chat: UInt64) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        engine.sendText(chat: chat, text: t)
    }

    /// Offers a picked file (copied into the caches first).
    public func sendFile(_ url: URL, in chat: UInt64) throws {
        let copy = try storage.stage(url, chat: chat)
        engine.sendFile(chat: chat, path: copy.path)
    }

    public func sendFile(_ data: Data, name: String, in chat: UInt64) throws {
        let copy = try storage.stage(data, name: name, chat: chat)
        engine.sendFile(chat: chat, path: copy.path)
    }

    // MARK: drops

    /// Where the plain copies of a drop wait while it is sealed: no conversation has this id.
    private static let dropStaging = UInt64.max

    /// Seals picked files on blob servers and returns the link to share (a few seconds per
    /// hundred megabytes, off the main actor).
    public func sendDrop(
        _ urls: [URL],
        minutes: UInt64,
        once: Bool,
        passphrase: String?,
        progress: @escaping @Sendable (DropPhase, UInt64, UInt64) -> Void
    ) async throws -> DropSent {
        let pass = passphrase.flatMap { $0.isEmpty ? nil : $0 }
        let paths = try urls.map { try storage.stage($0, chat: Self.dropStaging).path }
        defer { storage.unstage(chat: Self.dropStaging) }
        let relay = DropProgressRelay(progress)
        return try await off {
            try $0.sendDrop(
                paths: paths, ttlMin: minutes, once: once, passphrase: pass, toContacts: [], toSelf: false,
                servers: defaultBlobServers(), base: linkBase, progressTo: relay
            )
        }
    }

    /// Downloads a drop's sealed copy into the received files (resumes an interrupted one).
    public func receiveDrop(
        _ link: String,
        passphrase: String?,
        progress: @escaping @Sendable (DropPhase, UInt64, UInt64) -> Void
    ) async throws -> DropReceived {
        let pass = passphrase.flatMap { $0.isEmpty ? nil : $0 }
        let out = storage.received.path
        let relay = DropProgressRelay(progress)
        return try await off { try $0.receiveDrop(link: link, passphrase: pass, outDir: out, progressTo: relay) }
    }

    public func download(_ file: String, in chat: UInt64) {
        engine.getFile(chat: chat, id: file)
    }

    public func cancel(_ file: String, in chat: UInt64) {
        engine.cancelFile(chat: chat, id: file)
    }

    public func delete(_ message: String, in chat: UInt64) {
        engine.deleteMessage(chat: chat, id: message)
    }

    public func verify(_ chat: UInt64, peer: String, ok: Bool) {
        engine.verify(chat: chat, peer: peer, ok: ok)
    }

    public func setLocked(_ chat: UInt64, _ locked: Bool) {
        engine.lock(chat: chat, locked: locked)
    }

    public func end(_ chat: UInt64) {
        engine.end(chat: chat)
    }

    /// Offers our card to a verified peer (or accepts theirs).
    public func saveContact(_ chat: UInt64, peer: String) {
        engine.saveContact(chat: chat, peer: peer)
        state.dismissOffer(chat, peer: peer)
    }

    public func dismissOffer(_ chat: UInt64, peer: String) {
        state.dismissOffer(chat, peer: peer)
    }

    /// Leaves an ended conversation's screen.
    public func dismiss(_ chat: UInt64) {
        state.dismiss(chat)
    }

    public func setVisible(_ chat: UInt64?) {
        state.visible = chat
        if let chat { state.chats[chat]?.unread = 0 }
    }

    // MARK: vault, identity, contacts

    public func createVault(_ passphrase: String) async throws {
        try await off { try $0.createVault(passphrase: passphrase) }
    }

    public func unlockVault(_ passphrase: String) async throws {
        try await off { try $0.unlockVault(passphrase: passphrase) }
    }

    public func lockVault() {
        engine.lockVault()
    }

    public func eraseVault() async throws {
        try await off { try $0.eraseVault() }
    }

    public func exportBackup(_ passphrase: String) async throws -> Data {
        try await off { try $0.exportBackup(passphrase: passphrase) }
    }

    public func importBackup(_ file: Data, passphrase: String) async throws -> UInt64 {
        try await off { try $0.importBackup(file: file, passphrase: passphrase) }
    }

    @discardableResult
    public func ensureIdentity() async throws -> String {
        try await off { try $0.ensureIdentity() }
    }

    public func setReachable(_ on: Bool) {
        engine.setReachable(on: on)
    }

    /// The live conversation with a contact (and says we are here, so it connects).
    public func chatWith(_ key: String) async throws -> UInt64? {
        try await off { try $0.chatWith(key: key) }
    }

    /// Looks someone up by identifier; follow it in `state.lookups[lookupKey(id)]`.
    public func addById(_ id: String) {
        engine.addById(id: id)
    }

    /// How the engine files a lookup.
    public nonisolated static func lookupKey(_ id: String) -> String {
        id.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    public func accept(_ request: String) {
        engine.acceptRequest(id: request)
    }

    public func decline(_ request: String) {
        engine.declineRequest(id: request)
    }

    public func rename(_ key: String, to name: String) {
        engine.renameContact(key: key, name: name)
    }

    public func remove(_ key: String) {
        engine.removeContact(key: key)
    }

    public func setHistory(_ key: String, _ on: Bool) {
        engine.setHistory(key: key, on: on)
    }
}
