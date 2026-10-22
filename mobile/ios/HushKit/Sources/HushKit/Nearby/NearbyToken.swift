import CoreBluetooth
import Foundation

/// How a 16-byte discovery token is spelled on each medium (PROTOCOL.md §8).
public enum NearbyToken {
    public static let length = 16

    /// The DNS-SD instance name: lowercase hex.
    public static func hex(_ token: Data) -> String {
        token.map { String(format: "%02x", $0) }.joined()
    }

    public static func token(hex: String) -> Data? {
        guard hex.count == length * 2 else { return nil }
        var out = Data(capacity: length)
        var i = hex.startIndex
        while i < hex.endIndex {
            let j = hex.index(i, offsetBy: 2)
            guard let b = UInt8(hex[i..<j], radix: 16) else { return nil }
            out.append(b)
            i = j
        }
        return out
    }

    /// The token behind a Bonjour instance name. Several devices advertising the same token (the
    /// members of a group) get renamed "<hex> (2)" by Bonjour: that is still the token.
    public static func token(serviceName name: String) -> Data? {
        let hex = name.prefix(length * 2)
        let rest = name.dropFirst(length * 2)
        guard rest.isEmpty || rest.hasPrefix(" (") else { return nil }
        return token(hex: String(hex).lowercased())
    }

    /// The GATT service UUID: the token as a big-endian 128-bit UUID.
    public static func uuid(_ token: Data) -> CBUUID {
        CBUUID(data: token)
    }

    public static func token(uuid: CBUUID) -> Data? {
        uuid.data.count == length ? uuid.data : nil
    }
}

/// The GATT characteristics of a nearby service (PROTOCOL.md §8).
public enum NearbyGATT {
    /// Read: the L2CAP PSM, u16 big-endian.
    public static let psm = CBUUID(string: "E261FBA8-836B-459C-84CC-E15CD0D124B4")
    /// Write without response: packets from the scanner.
    public static let rx = CBUUID(string: "CB5F7AB3-A841-4A1B-9F80-5855197FD921")
    /// Notify: packets to the scanner.
    public static let tx = CBUUID(string: "66C5F983-F8D0-406D-AFB7-3CD1B35E1712")

    public static func encode(psm: UInt16) -> Data {
        Data([UInt8(psm >> 8), UInt8(psm & 0xff)])
    }

    public static func decode(psm data: Data) -> UInt16? {
        guard data.count == 2 else { return nil }
        let b = [UInt8](data)
        return UInt16(b[0]) << 8 | UInt16(b[1])
    }
}
