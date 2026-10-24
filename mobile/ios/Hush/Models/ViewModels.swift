import Foundation
import HushKit
import Observation

/// Where the app is.
enum Route: Hashable {
    case new
    case join(String?)
    case chat(UInt64)
    case contacts
    case identity
    case settings
    case drop
    case dropReceive(String)
}

@MainActor
@Observable
final class Router {
    var path: [Route] = []

    /// A conversation replaces the screens that led to it.
    func show(chat: UInt64) {
        path = [.chat(chat)]
    }

    func home() {
        path = []
    }

    /// A hush link opened from elsewhere (a universal link, a code scanned by the camera app).
    @discardableResult
    func open(_ url: URL) -> Bool {
        let text = url.absoluteString
        if DropModel.isDrop(text) {
            path = [.dropReceive(text)]
            return true
        }
        guard JoinModel.isLink(text) else { return false }
        path = [.join(text)]
        return true
    }
}

@MainActor
@Observable
final class NewConversationModel {
    var group = false
    var minutes: UInt64 = 60
    var passphrase = ""
    var busy = false
    var error: String?

    /// Creates the link (a passphrase costs a second of Argon2id, off the main actor).
    func create(_ store: HushStore) async -> UInt64? {
        busy = true
        error = nil
        defer { busy = false }
        do {
            return try await store.createLink(group: group, minutes: minutes, passphrase: passphrase)
        } catch {
            self.error = String(describing: error)
            return nil
        }
    }
}

@MainActor
@Observable
final class JoinModel {
    enum Problem: Error, Equatable {
        case invalid
        case outdated
        case expired
    }

    var input = ""
    var passphrase = ""
    var problem: Problem?
    /// The link needs a passphrase: ask for it before joining.
    var askPassphrase = false
    var busy = false
    var link: String?

    static func isLink(_ text: String) -> Bool {
        (try? linkInfo(input: text.trimmingCharacters(in: .whitespacesAndNewlines))) != nil
    }

    /// What is wrong with a link, if anything, and whether it wants a passphrase.
    static func check(_ text: String, now: Date = Date()) -> Result<LinkInfo, Problem> {
        do {
            let info = try linkInfo(input: text.trimmingCharacters(in: .whitespacesAndNewlines))
            if info.expiresMs <= UInt64(now.timeIntervalSince1970 * 1000) { return .failure(.expired) }
            return .success(info)
        } catch HushError.Core(name: "Outdated") {
            return .failure(.outdated)
        } catch {
            return .failure(.invalid)
        }
    }

    var problemText: String? {
        switch problem {
        case .invalid: String(localized: "invalidLink")
        case .outdated: String(localized: "outdatedLink")
        case .expired: String(localized: "expiredLink")
        case nil: nil
        }
    }

    /// A link opened from elsewhere (a universal link): shown with what is wrong with it, joined only
    /// once Join is tapped, so a web page or another app cannot connect us on its own.
    func prefill(_ text: String) {
        input = text
        if case let .failure(p) = Self.check(text) { problem = p } else { problem = nil }
    }

    /// Checks the link; joins it unless it needs a passphrase first.
    func submit(_ text: String, store: HushStore) async -> UInt64? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        switch Self.check(text) {
        case let .failure(p):
            problem = p
            return nil
        case let .success(info):
            problem = nil
            link = text
            if info.passphrase && passphrase.isEmpty {
                askPassphrase = true
                return nil
            }
            return await join(store)
        }
    }

    func join(_ store: HushStore) async -> UInt64? {
        guard let link else { return nil }
        busy = true
        defer { busy = false }
        do {
            let chat = try await store.join(link, passphrase: askPassphrase ? passphrase : nil)
            if askPassphrase { store.joinedWithPassphrase.insert(chat) }
            return chat
        } catch {
            problem = .invalid
            return nil
        }
    }
}

@MainActor
@Observable
final class VaultModel {
    static let minLength = 10

    var passphrase = ""
    var confirm = ""
    var error: String?
    var busy = false

    /// What is wrong with a new passphrase (the vault's, a backup's), if anything.
    static func problem(_ passphrase: String, confirm: String) -> String? {
        if passphrase.count < minLength { return String(localized: "vaultTooShort") }
        if passphrase != confirm { return String(localized: "vaultMismatch") }
        return nil
    }

    /// Creates the vault (`creating`) or unlocks it. True when done.
    func submit(_ store: HushStore, creating: Bool) async -> Bool {
        if creating, let problem = Self.problem(passphrase, confirm: confirm) {
            error = problem
            return false
        }
        error = nil
        busy = true
        defer { busy = false }
        do {
            if creating {
                try await store.createVault(passphrase)
            } else {
                try await store.unlockVault(passphrase)
            }
            passphrase = ""
            confirm = ""
            return true
        } catch {
            self.error = creating ? String(localized: "vaultTooShort") : String(localized: "vaultWrong")
            return false
        }
    }
}

@MainActor
@Observable
final class AddByIdModel {
    var input = ""
    var invalid = false
    /// The identifier being looked up, as the engine files it.
    var pending: String?

    func submit(_ store: HushStore) {
        let id = HushStore.lookupKey(input)
        guard isIdentifier(input: id) else {
            invalid = true
            return
        }
        invalid = false
        pending = id
        store.addById(id)
    }

    func status(in state: HushState) -> LookupModel? {
        pending.flatMap { state.lookups[$0] }
    }
}

/// Sending files by link, or receiving a drop: sealed on blob servers that only see ciphertext.
@MainActor
@Observable
final class DropModel {
    var files: [URL] = []
    var minutes: UInt64 = 1440
    var once = true
    var passphrase = ""
    var busy = false
    var phase: DropPhase?
    var done: UInt64 = 0
    var total: UInt64 = 0
    var url: String?
    var expiresMs: UInt64 = 0
    var saved: [String] = []
    var sender: String?
    var deleted = false
    var error: LocalizedStringResource?

    /// A link whose files come from a sealed copy (a drop link with one, or a drop sealed to us).
    static func isDrop(_ text: String) -> Bool {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if isSealedDrop(input: t) { return true }
        guard let info = try? linkInfo(input: t) else { return false }
        return info.drop && info.sealedCopy
    }

    static func needsPassphrase(_ text: String) -> Bool {
        (try? linkInfo(input: text.trimmingCharacters(in: .whitespacesAndNewlines)))?.passphrase ?? false
    }

    private func report() -> @Sendable (DropPhase, UInt64, UInt64) -> Void {
        { phase, done, total in
            Task { @MainActor in
                self.phase = phase
                self.done = done
                self.total = total
            }
        }
    }

    func send(_ store: HushStore) async {
        guard !files.isEmpty, !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            let sent = try await store.sendDrop(files, minutes: minutes, once: once, passphrase: passphrase, progress: report())
            url = sent.url
            expiresMs = sent.expiresMs
        } catch {
            self.error = "dropFailed"
        }
    }

    func receive(_ link: String, store: HushStore) async {
        guard !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            let got = try await store.receiveDrop(link, passphrase: passphrase, progress: report())
            saved = got.paths
            sender = got.sender
            deleted = got.deleted
        } catch HushError.Core(name: "Decrypt") {
            error = "dropWrongKey"
        } catch HushError.Core(name: "Expired") {
            error = "expiredLink"
        } catch {
            error = "\(error)".contains("not on its servers") ? "dropGone" : "dropFailed"
        }
    }
}
