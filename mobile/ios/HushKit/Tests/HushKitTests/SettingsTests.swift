import Foundation
import Testing
@testable import HushKit

struct SettingsTests {
    @Test func defaultsComeFromTheEngine() {
        let s = Settings()
        #expect(s.relays == defaultRelays())
        #expect(s.trackers == defaultTrackers())
        #expect(s.nearby)
        #expect(s.ttl == 0)
    }

    @Test func iceServersAddTURNOnlyWhenSet() {
        var s = Settings()
        s.stun = ["stun:a:3478"]
        #expect(s.iceServers(credential: "x") == [IceServerConfig(urls: ["stun:a:3478"], username: "", credential: "")])
        s.turnURL = " turn:t:3478 "
        s.turnUsername = "me"
        #expect(s.iceServers(credential: "secret").last == IceServerConfig(urls: ["turn:t:3478"], username: "me", credential: "secret"))
    }

    @Test func oldSavedSettingsKeepNewDefaults() throws {
        let s = try JSONDecoder().decode(Settings.self, from: Data(#"{"name":"Ada","ttl":30}"#.utf8))
        #expect(s.name == "Ada")
        #expect(s.ttl == 30)
        #expect(s.nearby)
        #expect(s.relays == defaultRelays())
    }

    @Test func savedInUserDefaultsWithoutTheCredential() throws {
        let defaults = try #require(UserDefaults(suiteName: "hush-test-\(UUID().uuidString)"))
        let secrets = MemorySecrets()
        let storage = DefaultsSettingsStorage(defaults: defaults, secrets: secrets)
        var s = Settings()
        s.name = "Ada"
        storage.save(s)
        storage.turnCredential = "hunter2"
        #expect(storage.load() == s)
        #expect(secrets.values["turn"] == "hunter2")
        let raw = try #require(defaults.data(forKey: "hush.settings"))
        #expect(!String(decoding: raw, as: UTF8.self).contains("hunter2"))
        storage.turnCredential = ""
        #expect(secrets.values["turn"] == nil)
    }

    @Test func linesAreCleaned() {
        #expect(Settings.lines(" wss://a \n\n wss://b\r\n") == ["wss://a", "wss://b"])
    }
}

final class MemorySecrets: SecretStore {
    var values: [String: String] = [:]
    func get(_ account: String) -> String? { values[account] }
    func set(_ account: String, _ value: String) { values[account] = value }
    func delete(_ account: String) { values[account] = nil }
}
