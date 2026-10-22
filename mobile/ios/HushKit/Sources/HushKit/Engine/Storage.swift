import Foundation

/// Where the engine keeps things, all inside the app's container.
public struct Storage: Sendable {
    /// The vault (only written once one is created).
    public let data: URL
    /// Files received, until the app starts again.
    public let received: URL
    /// Copies of the files we send, the engine reads them from here.
    public let outgoing: URL

    public init(data: URL, received: URL, outgoing: URL) {
        self.data = data
        self.received = received
        self.outgoing = outgoing
    }

    /// Application Support/hush and Application Support/received, encrypted while the device is
    /// locked and kept out of backups; picked files are copied to the caches.
    public static func app() throws -> Storage {
        let fm = FileManager.default
        let support = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let caches = try fm.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        let s = Storage(
            data: support.appendingPathComponent("hush", isDirectory: true),
            received: support.appendingPathComponent("received", isDirectory: true),
            outgoing: caches.appendingPathComponent("outgoing", isDirectory: true)
        )
        try s.prepare()
        return s
    }

    /// A throwaway place (tests, UI tests).
    public static func temporary() throws -> Storage {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("hush-\(UUID().uuidString)", isDirectory: true)
        let s = Storage(
            data: root.appendingPathComponent("data", isDirectory: true),
            received: root.appendingPathComponent("received", isDirectory: true),
            outgoing: root.appendingPathComponent("outgoing", isDirectory: true)
        )
        try s.prepare()
        return s
    }

    /// Creates the folders; what the last run received or sent is erased (nothing is kept).
    public func prepare() throws {
        let fm = FileManager.default
        for dir in [received, outgoing] where fm.fileExists(atPath: dir.path) {
            try? fm.removeItem(at: dir)
        }
        for dir in [data, received, outgoing] {
            try fm.createDirectory(at: dir, withIntermediateDirectories: true, attributes: Self.protection)
            var url = dir
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? url.setResourceValues(values)
        }
    }

    private static var protection: [FileAttributeKey: Any]? {
        #if os(iOS)
        [.protectionKey: FileProtectionType.complete]
        #else
        nil
        #endif
    }

    private func staging(_ chat: UInt64) throws -> URL {
        let dir = outgoing.appendingPathComponent("\(chat)", isDirectory: true).appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: Self.protection)
        return dir
    }

    /// Copies a file the user picked so the engine can read it after the picker's access ends.
    public func stage(_ source: URL, chat: UInt64) throws -> URL {
        let dir = try staging(chat)
        let dest = dir.appendingPathComponent(source.lastPathComponent)
        let scoped = source.startAccessingSecurityScopedResource()
        defer { if scoped { source.stopAccessingSecurityScopedResource() } }
        try FileManager.default.copyItem(at: source, to: dest)
        return dest
    }

    /// Writes picked data (a photo) under a name.
    public func stage(_ data: Data, name: String, chat: UInt64) throws -> URL {
        let dir = try staging(chat)
        let dest = dir.appendingPathComponent(name)
        try data.write(to: dest, options: Self.writeOptions)
        return dest
    }

    /// The copies made for a conversation that ended.
    public func unstage(chat: UInt64) {
        try? FileManager.default.removeItem(at: outgoing.appendingPathComponent("\(chat)", isDirectory: true))
    }

    private static var writeOptions: Data.WritingOptions {
        #if os(iOS)
        [.atomic, .completeFileProtection]
        #else
        [.atomic]
        #endif
    }
}
