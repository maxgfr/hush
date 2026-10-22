import CoreBluetooth
import Foundation
import Testing
@testable import HushKit

struct NearbyTokenTests {
    let token = Data((0..<16).map { UInt8($0 * 17) })

    @Test func hexIsLowercaseAndRoundTrips() {
        let hex = NearbyToken.hex(token)
        #expect(hex == "00112233445566778899aabbccddeeff")
        #expect(NearbyToken.token(hex: hex) == token)
        #expect(NearbyToken.token(hex: "00112233") == nil)
        #expect(NearbyToken.token(hex: String(repeating: "zz", count: 16)) == nil)
    }

    @Test func serviceNamesSurviveBonjourRenaming() {
        let hex = NearbyToken.hex(token)
        #expect(NearbyToken.token(serviceName: hex) == token)
        #expect(NearbyToken.token(serviceName: "\(hex) (2)") == token)
        #expect(NearbyToken.token(serviceName: hex.uppercased()) == token)
        #expect(NearbyToken.token(serviceName: "\(hex)x") == nil)
        #expect(NearbyToken.token(serviceName: "printer") == nil)
    }

    @Test func serviceUUIDIsTheTokenBigEndian() {
        let uuid = NearbyToken.uuid(token)
        #expect(uuid.uuidString == "00112233-4455-6677-8899-AABBCCDDEEFF")
        #expect(NearbyToken.token(uuid: uuid) == token)
        #expect(NearbyToken.token(uuid: CBUUID(string: "180D")) == nil)
        // The same bytes as Foundation's UUID, which is big-endian too.
        let u = UUID(uuidString: uuid.uuidString)!
        #expect(withUnsafeBytes(of: u.uuid) { Data($0) } == token)
    }

    @Test func characteristicsAndPSM() {
        #expect(NearbyGATT.psm.uuidString == "E261FBA8-836B-459C-84CC-E15CD0D124B4")
        #expect(NearbyGATT.rx.uuidString == "CB5F7AB3-A841-4A1B-9F80-5855197FD921")
        #expect(NearbyGATT.tx.uuidString == "66C5F983-F8D0-406D-AFB7-3CD1B35E1712")
        #expect(NearbyGATT.encode(psm: 0x0081) == Data([0x00, 0x81]))
        #expect(NearbyGATT.decode(psm: Data([0x10, 0x25])) == 0x1025)
        #expect(NearbyGATT.decode(psm: Data([1])) == nil)
    }
}
