import CoreBluetooth
import Foundation

/// Bluetooth LE (PROTOCOL.md §8).
/// - Advertising: one GATT service per token (UUID = token) with the PSM, RX and TX
///   characteristics, and one L2CAP channel published for all of them.
/// - Scanning: for the tokens' service UUIDs (explicit UUIDs also work in the background), then
///   connect, read the PSM and open an L2CAP stream; without L2CAP, RX/TX packets.
///
/// The managers are only created once there is something to advertise or scan for, so the
/// Bluetooth permission is not asked before Nearby is used.
final class BluetoothMedium: NSObject, NearbyMedium {
    private unowned let hub: NearbyTransport

    // advertising
    private var peripheral: CBPeripheralManager?
    private var advertising: Set<Data> = []
    private var advertised: Set<Data> = []
    private var services: [Data: CBMutableService] = [:]
    private var psm: CBL2CAPPSM?
    private var publishing = false
    private var served: [ServedKey: PeripheralGattLink] = [:]

    // scanning
    private var central: CBCentralManager?
    private var scanning: Set<Data> = []
    private var dials: [UUID: Dial] = [:]

    struct ServedKey: Hashable {
        let central: UUID
        let service: CBUUID
    }

    final class Dial {
        let peripheral: CBPeripheral
        var token: Data?
        var rx: CBCharacteristic?
        var tx: CBCharacteristic?
        var link: NearbyLink?
        var gatt: CentralGattLink?

        init(_ peripheral: CBPeripheral, token: Data?) {
            self.peripheral = peripheral
            self.token = token
        }
    }

    init(hub: NearbyTransport) {
        self.hub = hub
    }

    // MARK: advertising

    func advertise(_ tokens: Set<Data>) {
        advertising = tokens
        if peripheral == nil {
            guard !tokens.isEmpty else { return }
            peripheral = CBPeripheralManager(delegate: self, queue: hub.queue, options: nil)
            return
        }
        syncPeripheral()
    }

    private func syncPeripheral() {
        guard let pm = peripheral, pm.state == .poweredOn else { return }
        if psm == nil && !publishing && !advertising.isEmpty {
            publishing = true
            pm.publishL2CAPChannel(withEncryption: false)
        }
        for token in advertising where services[token] == nil {
            let s = CBMutableService(type: NearbyToken.uuid(token), primary: true)
            s.characteristics = [
                CBMutableCharacteristic(type: NearbyGATT.psm, properties: [.read], value: nil, permissions: [.readable]),
                CBMutableCharacteristic(type: NearbyGATT.rx, properties: [.writeWithoutResponse], value: nil, permissions: [.writeable]),
                CBMutableCharacteristic(type: NearbyGATT.tx, properties: [.notify], value: nil, permissions: [.readable]),
            ]
            services[token] = s
            pm.add(s)
        }
        // A service carrying a packet link stays until that link ends.
        let busy = Set(served.keys.map(\.service))
        for (token, s) in services where !advertising.contains(token) && !busy.contains(s.uuid) {
            pm.remove(s)
            services[token] = nil
        }
        if advertising != advertised {
            pm.stopAdvertising()
            advertised = advertising
            if !advertising.isEmpty {
                pm.startAdvertising([CBAdvertisementDataServiceUUIDsKey: advertising.map(NearbyToken.uuid)])
            }
        }
    }

    // MARK: scanning

    func scan(_ tokens: Set<Data>) {
        scanning = tokens
        if central == nil {
            guard !tokens.isEmpty else { return }
            central = CBCentralManager(delegate: self, queue: hub.queue, options: nil)
            return
        }
        // Dials nobody wants any more (a link that opened stays).
        for (id, d) in dials where d.link == nil && d.gatt == nil {
            if let t = d.token, tokens.contains(t) { continue }
            central?.cancelPeripheralConnection(d.peripheral)
            endDial(id)
        }
        restartScan()
    }

    func retry() {
        // The same peripheral is only reported again by a new scan.
        restartScan()
    }

    private func restartScan() {
        guard let c = central, c.state == .poweredOn else { return }
        c.stopScan()
        guard !scanning.isEmpty else { return }
        c.scanForPeripherals(withServices: scanning.map(NearbyToken.uuid), options: [CBCentralManagerScanOptionAllowDuplicatesKey: false])
    }

    private func endDial(_ id: UUID) {
        guard let d = dials.removeValue(forKey: id) else { return }
        if let link = d.link {
            hub.closed(link.id)
        } else if let gatt = d.gatt {
            hub.closed(gatt.id)
        } else {
            hub.dialFailed(d.token)
        }
    }

    private func fail(_ d: Dial) {
        central?.cancelPeripheralConnection(d.peripheral)
        endDial(d.peripheral.identifier)
    }

    /// No L2CAP: subscribe to TX, then packets both ways.
    private func fallBack(_ d: Dial) {
        guard d.link == nil, d.gatt == nil, let tx = d.tx, d.rx != nil else { return fail(d) }
        d.peripheral.setNotifyValue(true, for: tx)
    }
}

// MARK: - CBPeripheralManagerDelegate

extension BluetoothMedium: CBPeripheralManagerDelegate {
    func peripheralManagerDidUpdateState(_ pm: CBPeripheralManager) {
        if pm.state == .poweredOn {
            syncPeripheral()
            return
        }
        // Off or reset: everything published is gone.
        services = [:]
        advertised = []
        psm = nil
        publishing = false
        for (_, l) in served { l.finish() }
        served = [:]
    }

    func peripheralManager(_ pm: CBPeripheralManager, didPublishL2CAPChannel psm: CBL2CAPPSM, error: Error?) {
        publishing = false
        if error == nil { self.psm = psm }
    }

    func peripheralManager(_ pm: CBPeripheralManager, didOpen channel: CBL2CAPChannel?, error: Error?) {
        guard let channel, error == nil, let input = channel.inputStream, let output = channel.outputStream else { return }
        StreamLink(id: hub.newLinkId(), input: input, output: output, token: nil, medium: .bluetooth, owner: channel, hub: hub).start()
    }

    func peripheralManager(_ pm: CBPeripheralManager, didReceiveRead request: CBATTRequest) {
        guard request.characteristic.uuid == NearbyGATT.psm, let psm else {
            return pm.respond(to: request, withResult: .attributeNotFound)
        }
        let value = NearbyGATT.encode(psm: psm)
        guard request.offset <= value.count else { return pm.respond(to: request, withResult: .invalidOffset) }
        request.value = value.subdata(in: request.offset..<value.count)
        pm.respond(to: request, withResult: .success)
    }

    func peripheralManager(_ pm: CBPeripheralManager, central: CBCentral, didSubscribeTo characteristic: CBCharacteristic) {
        guard characteristic.uuid == NearbyGATT.tx, let tx = characteristic as? CBMutableCharacteristic,
              let service = characteristic.service?.uuid else { return }
        let key = ServedKey(central: central.identifier, service: service)
        served[key]?.finish()
        let link = PeripheralGattLink(id: hub.newLinkId(), manager: pm, central: central, tx: tx, hub: hub)
        served[key] = link
        hub.opened(link, kind: .packets(mtu: UInt32(central.maximumUpdateValueLength)), medium: .bluetooth)
    }

    func peripheralManager(_ pm: CBPeripheralManager, central: CBCentral, didUnsubscribeFrom characteristic: CBCharacteristic) {
        guard let service = characteristic.service?.uuid else { return }
        served.removeValue(forKey: ServedKey(central: central.identifier, service: service))?.finish()
        syncPeripheral()
    }

    func peripheralManager(_ pm: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
        for r in requests where r.characteristic.uuid == NearbyGATT.rx {
            guard let service = r.characteristic.service?.uuid, let value = r.value,
                  let link = served[ServedKey(central: r.central.identifier, service: service)] else { continue }
            hub.received(link.id, value)
        }
    }

    func peripheralManagerIsReady(toUpdateSubscribers pm: CBPeripheralManager) {
        for (_, l) in served { l.flush() }
    }
}

// MARK: - CBCentralManagerDelegate

extension BluetoothMedium: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        if c.state == .poweredOn {
            restartScan()
            return
        }
        for id in Array(dials.keys) { endDial(id) }
    }

    func centralManager(_ c: CBCentralManager, didDiscover p: CBPeripheral, advertisementData: [String: Any], rssi: NSNumber) {
        guard dials[p.identifier] == nil, !scanning.isEmpty else { return }
        let listed = (advertisementData[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID] ?? [])
            + (advertisementData[CBAdvertisementDataOverflowServiceUUIDsKey] as? [CBUUID] ?? [])
        let tokens = listed.compactMap(NearbyToken.token(uuid:)).filter(scanning.contains)
        var token: Data?
        if tokens.isEmpty {
            // In the background the UUIDs may not be listed: learn them from the services.
            guard listed.isEmpty else { return }
        } else {
            guard let t = tokens.first(where: hub.claim) else { return }
            token = t
        }
        dials[p.identifier] = Dial(p, token: token)
        c.connect(p, options: nil)
        // A connection that never completes must not hold the token.
        hub.queue.asyncAfter(deadline: .now() + 20) { [weak self] in
            guard let self, let d = self.dials[p.identifier], d.link == nil, d.gatt == nil else { return }
            self.fail(d)
        }
    }

    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        guard let d = dials[p.identifier] else { return c.cancelPeripheralConnection(p) }
        p.delegate = self
        let wanted = d.token.map { [NearbyToken.uuid($0)] } ?? scanning.map(NearbyToken.uuid)
        p.discoverServices(wanted)
    }

    func centralManager(_ c: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) {
        endDial(p.identifier)
    }

    func centralManager(_ c: CBCentralManager, didDisconnectPeripheral p: CBPeripheral, error: Error?) {
        endDial(p.identifier)
    }
}

// MARK: - CBPeripheralDelegate

extension BluetoothMedium: CBPeripheralDelegate {
    func peripheral(_ p: CBPeripheral, didDiscoverServices error: Error?) {
        guard let d = dials[p.identifier] else { return }
        let service = p.services?.first { s in
            guard let t = NearbyToken.token(uuid: s.uuid) else { return false }
            return d.token.map { $0 == t } ?? scanning.contains(t)
        }
        guard let service, let t = NearbyToken.token(uuid: service.uuid) else { return fail(d) }
        if d.token == nil {
            guard hub.claim(t) else {
                // Already connected for it some other way: leave quietly.
                dials[p.identifier] = nil
                central?.cancelPeripheralConnection(p)
                return
            }
            d.token = t
        }
        p.discoverCharacteristics([NearbyGATT.psm, NearbyGATT.rx, NearbyGATT.tx], for: service)
    }

    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard let d = dials[p.identifier] else { return }
        let chars = service.characteristics ?? []
        d.rx = chars.first { $0.uuid == NearbyGATT.rx }
        d.tx = chars.first { $0.uuid == NearbyGATT.tx }
        if let psm = chars.first(where: { $0.uuid == NearbyGATT.psm }) {
            p.readValue(for: psm)
        } else {
            fallBack(d)
        }
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard let d = dials[p.identifier] else { return }
        switch characteristic.uuid {
        case NearbyGATT.psm:
            guard error == nil, d.link == nil, let v = characteristic.value, let psm = NearbyGATT.decode(psm: v), psm != 0 else {
                return fallBack(d)
            }
            p.openL2CAPChannel(psm)
        case NearbyGATT.tx:
            guard let gatt = d.gatt, let v = characteristic.value else { return }
            hub.received(gatt.id, v)
        default:
            break
        }
    }

    func peripheral(_ p: CBPeripheral, didOpen channel: CBL2CAPChannel?, error: Error?) {
        guard let d = dials[p.identifier] else { return }
        guard error == nil, let channel, let input = channel.inputStream, let output = channel.outputStream else {
            return fallBack(d)
        }
        let link = StreamLink(id: hub.newLinkId(), input: input, output: output, token: d.token, medium: .bluetooth, owner: channel, hub: hub)
        link.onClose = { [weak self] in self?.central?.cancelPeripheralConnection(p) }
        d.link = link
        link.start()
    }

    func peripheral(_ p: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
        guard characteristic.uuid == NearbyGATT.tx, let d = dials[p.identifier], d.gatt == nil, d.link == nil else { return }
        guard error == nil, characteristic.isNotifying, let rx = d.rx else { return fail(d) }
        let link = CentralGattLink(id: hub.newLinkId(), peripheral: p, rx: rx, token: d.token, hub: hub)
        link.onClose = { [weak self] in self?.central?.cancelPeripheralConnection(p) }
        d.gatt = link
        hub.opened(link, kind: .packets(mtu: UInt32(p.maximumWriteValueLength(for: .withoutResponse))), medium: .bluetooth)
    }

    func peripheralIsReady(toSendWriteWithoutResponse p: CBPeripheral) {
        dials[p.identifier]?.gatt?.flush()
    }
}

// MARK: - packet links

/// The scanner's side of a GATT link: writes to RX, notifications from TX.
final class CentralGattLink: NearbyLink {
    let id: UInt64
    let token: Data?
    var onClose: (() -> Void)?
    private let peripheral: CBPeripheral
    private let rx: CBCharacteristic
    private unowned let hub: NearbyTransport
    private var pending: [Data] = []

    init(id: UInt64, peripheral: CBPeripheral, rx: CBCharacteristic, token: Data?, hub: NearbyTransport) {
        self.id = id
        self.peripheral = peripheral
        self.rx = rx
        self.token = token
        self.hub = hub
    }

    /// One packet per write: the engine sized them.
    func send(_ data: Data) {
        pending.append(data)
        flush()
    }

    func flush() {
        while let first = pending.first, peripheral.canSendWriteWithoutResponse {
            peripheral.writeValue(first, for: rx, type: .withoutResponse)
            pending.removeFirst()
            hub.wrote(id, first.count)
        }
    }

    func close() {
        pending = []
        onClose?()
        onClose = nil
    }
}

/// The advertiser's side of a GATT link: notifications on TX to one central, its writes on RX.
final class PeripheralGattLink: NearbyLink {
    let id: UInt64
    let token: Data? = nil
    private let manager: CBPeripheralManager
    private let central: CBCentral
    private let tx: CBMutableCharacteristic
    private unowned let hub: NearbyTransport
    private var pending: [Data] = []
    private var ended = false

    init(id: UInt64, manager: CBPeripheralManager, central: CBCentral, tx: CBMutableCharacteristic, hub: NearbyTransport) {
        self.id = id
        self.manager = manager
        self.central = central
        self.tx = tx
        self.hub = hub
    }

    func send(_ data: Data) {
        guard !ended else { return }
        pending.append(data)
        flush()
    }

    func flush() {
        while !ended, let first = pending.first {
            // False: the queue is full, `peripheralManagerIsReady` comes next.
            guard manager.updateValue(first, for: tx, onSubscribedCentrals: [central]) else { return }
            pending.removeFirst()
            hub.wrote(id, first.count)
        }
    }

    /// A peripheral cannot hang up on a central: it just stops talking to it.
    func close() {
        ended = true
        pending = []
    }

    /// The central left.
    func finish() {
        guard !ended else { return }
        close()
        hub.closed(id)
    }
}
