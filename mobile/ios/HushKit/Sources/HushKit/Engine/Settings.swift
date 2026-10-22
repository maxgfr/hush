import Foundation
import Security

/// Preferences: none of them is a secret, so they live in `UserDefaults`. The TURN credential is
/// the exception and lives in the Keychain (`SecretStore`).
public struct Settings: Codable, Equatable, Sendable {
    public var name = ""
    /// Disappearing-message timer for what we send, in seconds (0 = off).
    public var ttl: UInt64 = 0
    public var relays: [String] = defaultRelays()
    public var trackers: [String] = defaultTrackers()
    public var stun: [String] = defaultIceServers().flatMap(\.urls)
    public var turnURL = ""
    public var turnUsername = ""
    public var relayOnly = false
    /// Bluetooth and Wi-Fi.
    public var nearby = true

    public init() {}

    public static let defaults = Settings()

    /// What the engine needs from these, with the TURN credential.
    public func iceServers(credential: String) -> [IceServerConfig] {
        var out: [IceServerConfig] = []
        if !stun.isEmpty { out.append(IceServerConfig(urls: stun, username: "", credential: "")) }
        let turn = turnURL.trimmingCharacters(in: .whitespaces)
        if !turn.isEmpty { out.append(IceServerConfig(urls: [turn], username: turnUsername, credential: credential)) }
        return out
    }

    /// Lines of a text field, cleaned.
    public static func lines(_ text: String) -> [String] {
        text.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    public init(from decoder: Decoder) throws {
        // Fields added later fall back to their defaults.
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = Settings()
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? d.name
        ttl = try c.decodeIfPresent(UInt64.self, forKey: .ttl) ?? d.ttl
        relays = try c.decodeIfPresent([String].self, forKey: .relays) ?? d.relays
        trackers = try c.decodeIfPresent([String].self, forKey: .trackers) ?? d.trackers
        stun = try c.decodeIfPresent([String].self, forKey: .stun) ?? d.stun
        turnURL = try c.decodeIfPresent(String.self, forKey: .turnURL) ?? d.turnURL
        turnUsername = try c.decodeIfPresent(String.self, forKey: .turnUsername) ?? d.turnUsername
        relayOnly = try c.decodeIfPresent(Bool.self, forKey: .relayOnly) ?? d.relayOnly
        nearby = try c.decodeIfPresent(Bool.self, forKey: .nearby) ?? d.nearby
    }
}

/// Where settings are kept.
public protocol SettingsStorage: AnyObject {
    func load() -> Settings
    func save(_ settings: Settings)
    var turnCredential: String { get set }
}

public final class DefaultsSettingsStorage: SettingsStorage {
    private let defaults: UserDefaults
    private let secrets: SecretStore
    private let key = "hush.settings"

    public init(defaults: UserDefaults = .standard, secrets: SecretStore = KeychainStore()) {
        self.defaults = defaults
        self.secrets = secrets
    }

    public func load() -> Settings {
        guard let data = defaults.data(forKey: key), let s = try? JSONDecoder().decode(Settings.self, from: data) else {
            return Settings()
        }
        return s
    }

    public func save(_ settings: Settings) {
        if let data = try? JSONEncoder().encode(settings) { defaults.set(data, forKey: key) }
    }

    public var turnCredential: String {
        get { secrets.get("turn") ?? "" }
        set { newValue.isEmpty ? secrets.delete("turn") : secrets.set("turn", newValue) }
    }
}

/// A few small secrets.
public protocol SecretStore {
    func get(_ account: String) -> String?
    func set(_ account: String, _ value: String)
    func delete(_ account: String)
}

/// Generic passwords in the Keychain, readable once the device was unlocked, never synced or
/// migrated to another device.
public struct KeychainStore: SecretStore {
    let service: String

    public init(service: String = "io.github.maxgfr.hush") {
        self.service = service
    }

    private func query(_ account: String) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: account]
    }

    public func get(_ account: String) -> String? {
        var q = query(account)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    public func set(_ account: String, _ value: String) {
        delete(account)
        var q = query(account)
        q[kSecValueData as String] = Data(value.utf8)
        q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(q as CFDictionary, nil)
    }

    public func delete(_ account: String) {
        SecItemDelete(query(account) as CFDictionary)
    }
}

/// For tests and previews.
public final class MemorySettingsStorage: SettingsStorage {
    public var settings: Settings
    public var turnCredential = ""

    public init(_ settings: Settings = Settings()) {
        self.settings = settings
    }

    public func load() -> Settings { settings }
    public func save(_ settings: Settings) { self.settings = settings }
}
