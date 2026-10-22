import Foundation
import HushKit

/// How the app words what the engine says, in the interface's language.
enum Wording {
    /// Link lifetimes offered, in minutes: 10 min, 1 h, 1 day, 7 days.
    static let linkMinutes: [UInt64] = [10, 60, 1440, 10_080]
    /// Disappearing-message timers offered, in seconds (0 = off).
    static let ttlSeconds: [UInt64] = [0, 30, 300, 3600, 86_400, 604_800]

    static var french: Bool {
        Bundle.main.preferredLocalizations.first?.hasPrefix("fr") ?? false
    }

    /// "59 min", "23 h", "6 d": coarse, for expiry countdowns.
    static func duration(ms: Int64) -> String {
        let m = max(0, Int64((Double(ms) / 60_000).rounded(.up)))
        if m < 60 { return "\(m) min" }
        if m < 48 * 60 { return "\(Int((Double(m) / 60).rounded())) h" }
        return "\(Int((Double(m) / 1440).rounded())) \(french ? "j" : "d")"
    }

    static func ttl(_ seconds: UInt64, short: Bool = false) -> String {
        if seconds == 0 { return short ? String(localized: "timerOffShort") : String(localized: "timerOff") }
        if seconds < 60 { return "\(seconds) s" }
        if seconds < 3600 { return "\(seconds / 60) min" }
        if seconds < 86_400 { return "\(seconds / 3600) h" }
        if seconds < 604_800 { return "\(seconds / 86_400) \(french ? "j" : "d")" }
        return french ? "\(seconds / 604_800) sem." : "\(seconds / 604_800) wk"
    }

    static func size(_ n: UInt64) -> String {
        let d = Double(n)
        if n < 1024 { return "\(n) B" }
        if n < 1 << 20 { return String(format: "%.1f KB", d / 1024) }
        if n < 1 << 30 { return String(format: "%.1f MB", d / 1_048_576) }
        return String(format: "%.2f GB", d / 1_073_741_824)
    }

    static func path(_ p: PeerPath) -> String {
        switch p {
        case .bluetooth: String(localized: "pathBluetooth")
        case .wifi: String(localized: "pathWifi")
        case .direct: String(localized: "direct")
        case .relayed: String(localized: "relay")
        case .connecting: String(localized: "connectingShort")
        }
    }

    static func notice(_ n: NoticeKind) -> String {
        switch n {
        case let .joined(name): String(localized: "noticeJoined \(name)")
        case let .left(name): String(localized: "noticeLeft \(name)")
        case let .renamed(old, name): String(localized: "renamed \(old) \(name)")
        case let .locked(by, locked): locked ? String(localized: "lockedBy \(by)") : String(localized: "unlockedBy \(by)")
        case let .contactSaved(name): String(localized: "contactSaved \(name)")
        case let .contactWaiting(name): String(localized: "contactWaiting \(name)")
        case .verifyFirst: String(localized: "verifyFirst")
        case .vaultNeeded: String(localized: "vaultNeeded")
        case .filesNeedDirect: String(localized: "directNeeded")
        case let .cannotSend(reason): String(localized: "cannotSend \(reason)")
        case .notACode: String(localized: "invalidCode")
        }
    }

    static func endTitle(_ e: Ending) -> String {
        switch e {
        case .ended: String(localized: "ended")
        case .peerLeft: String(localized: "left")
        case .taken: String(localized: "taken")
        case .expired: String(localized: "expired")
        case .mismatch: String(localized: "mismatchClosed")
        case .failed: String(localized: "failed")
        }
    }

    /// The five emojis and their spoken names.
    static func sas(_ indices: Data) -> [(emoji: String, name: String)] {
        let emojis = sasRender(indices: indices).split(separator: " ").map(String.init)
        let english = sasNames(indices: indices).components(separatedBy: ", ")
        return emojis.indices.map { i in
            let idx = Int(indices[indices.startIndex + i] & 63)
            let name = french ? sasFrench[idx] : (i < english.count ? english[i] : "")
            return (emojis[i], name)
        }
    }

    /// French names of the emojis, in the engine's order (crates/hush-core/src/sas.rs).
    static let sasFrench = [
        "Chien", "Chat", "Lion", "Cheval", "Licorne", "Cochon", "Éléphant", "Lapin", "Panda", "Coq",
        "Pingouin", "Tortue", "Poisson", "Pieuvre", "Papillon", "Fleur", "Arbre", "Cactus", "Champignon",
        "Globe", "Lune", "Nuage", "Feu", "Banane", "Pomme", "Fraise", "Maïs", "Pizza", "Gâteau", "Cœur",
        "Sourire", "Robot", "Chapeau", "Lunettes", "Clé à molette", "Père Noël", "Pouce", "Parapluie",
        "Sablier", "Réveil", "Cadeau", "Ampoule", "Livre", "Crayon", "Trombone", "Ciseaux", "Cadenas",
        "Clé", "Marteau", "Téléphone", "Drapeau", "Train", "Vélo", "Avion", "Fusée", "Trophée", "Ballon",
        "Guitare", "Trompette", "Cloche", "Ancre", "Casque", "Dossier", "Punaise",
    ]
}
