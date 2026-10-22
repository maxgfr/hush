import Foundation
import Network

/// How Wi-Fi peers find each other.
public enum WifiDiscovery: Sendable {
    /// DNS-SD `_hush._tcp` on the local network and Apple peer-to-peer Wi-Fi.
    case bonjour
    /// Listeners on the loopback interface, published in a directory: two engines in one process
    /// (tests), over the same TCP links as the real thing.
    case loopback(LoopbackDirectory)
}

/// The services of `WifiDiscovery.loopback`: instance name → port.
public final class LoopbackDirectory: @unchecked Sendable {
    private let lock = NSLock()
    private var services: [String: UInt16] = [:]
    private var watchers: [UUID: (DispatchQueue, ([String: UInt16]) -> Void)] = [:]

    public init() {}

    func publish(_ name: String, port: UInt16) {
        update { $0[name] = port }
    }

    func unpublish(_ name: String) {
        update { $0[name] = nil }
    }

    func watch(on queue: DispatchQueue, _ handler: @escaping ([String: UInt16]) -> Void) -> UUID {
        let id = UUID()
        let now: [String: UInt16] = lock.withLock {
            watchers[id] = (queue, handler)
            return services
        }
        queue.async { handler(now) }
        return id
    }

    func unwatch(_ id: UUID) {
        lock.withLock { _ = watchers.removeValue(forKey: id) }
    }

    private func update(_ change: (inout [String: UInt16]) -> Void) {
        let (now, list) = lock.withLock {
            change(&services)
            return (services, Array(watchers.values))
        }
        for (q, h) in list { q.async { h(now) } }
    }
}

/// Wi-Fi: a TCP listener published as `<hex(token)>._hush._tcp` per advertised token, and a
/// browser that dials the services named after scanned tokens. `includePeerToPeer` lets two
/// iPhones meet over AWDL with no router.
final class WifiMedium: NearbyMedium {
    static let serviceType = "_hush._tcp"

    private unowned let hub: NearbyTransport
    private let discovery: WifiDiscovery
    private var listeners: [Data: NWListener] = [:]
    private var browser: NWBrowser?
    private var watch: UUID?
    private var scanning: Set<Data> = []
    private var advertising: Set<Data> = []
    /// Where each scanned-for token was seen, best first.
    private var found: [Data: [NWEndpoint]] = [:]

    init(hub: NearbyTransport, discovery: WifiDiscovery) {
        self.hub = hub
        self.discovery = discovery
    }

    static func parameters(loopback: Bool) -> NWParameters {
        let tcp = NWProtocolTCP.Options()
        tcp.noDelay = true
        tcp.connectionTimeout = 10
        tcp.enableKeepalive = true
        tcp.keepaliveIdle = 10
        let p = NWParameters(tls: nil, tcp: tcp)
        p.includePeerToPeer = true
        if loopback { p.requiredInterfaceType = .loopback }
        return p
    }

    private var loopback: LoopbackDirectory? {
        if case let .loopback(d) = discovery { return d }
        return nil
    }

    // advertising

    func advertise(_ tokens: Set<Data>) {
        advertising = tokens
        for (token, l) in listeners where !tokens.contains(token) {
            // Accepted connections live on without their listener.
            l.cancel()
            listeners[token] = nil
            loopback?.unpublish(NearbyToken.hex(token))
        }
        for token in tokens where listeners[token] == nil {
            listen(token)
        }
    }

    private func listen(_ token: Data) {
        let name = NearbyToken.hex(token)
        let dir = loopback
        guard let l = try? NWListener(using: Self.parameters(loopback: dir != nil)) else { return }
        if dir == nil {
            l.service = NWListener.Service(name: name, type: Self.serviceType)
        }
        l.stateUpdateHandler = { [weak self, weak l] state in
            guard let self, let l else { return }
            switch state {
            case .ready:
                if let dir, let port = l.port?.rawValue { dir.publish(name, port: port) }
            case .failed:
                // Lost the network or the name: listen again while still wanted.
                l.cancel()
                if self.listeners[token] === l {
                    self.listeners[token] = nil
                    dir?.unpublish(name)
                    self.hub.queue.asyncAfter(deadline: .now() + 2) { [weak self] in
                        guard let self, self.advertising.contains(token), self.listeners[token] == nil else { return }
                        self.listen(token)
                    }
                }
            default:
                break
            }
        }
        l.newConnectionHandler = { [weak self] conn in
            guard let self else { return conn.cancel() }
            TCPLink(id: self.hub.newLinkId(), connection: conn, token: nil, hub: self.hub).start()
        }
        listeners[token] = l
        l.start(queue: hub.queue)
    }

    // scanning

    func scan(_ tokens: Set<Data>) {
        scanning = tokens
        if tokens.isEmpty {
            browser?.cancel()
            browser = nil
            if let watch { loopback?.unwatch(watch) }
            watch = nil
            found = [:]
            return
        }
        if browser == nil && watch == nil { browse() }
        dial()
    }

    func retry() {
        dial()
    }

    private func browse() {
        if let dir = loopback {
            watch = dir.watch(on: hub.queue) { [weak self] services in
                guard let self, self.watch != nil else { return }
                var found: [Data: [NWEndpoint]] = [:]
                for (name, port) in services {
                    guard let token = NearbyToken.token(serviceName: name), let p = NWEndpoint.Port(rawValue: port) else { continue }
                    found[token, default: []].append(.hostPort(host: .ipv4(.loopback), port: p))
                }
                self.found = found
                self.dial()
            }
            return
        }
        let b = NWBrowser(for: .bonjour(type: Self.serviceType, domain: nil), using: Self.parameters(loopback: false))
        b.browseResultsChangedHandler = { [weak self] results, _ in
            guard let self else { return }
            var found: [Data: [NWEndpoint]] = [:]
            for r in results {
                guard case let .service(name, _, _, _) = r.endpoint, let token = NearbyToken.token(serviceName: name) else { continue }
                found[token, default: []].append(r.endpoint)
            }
            self.found = found
            self.dial()
        }
        b.stateUpdateHandler = { [weak self, weak b] state in
            guard let self, let b, case .failed = state else { return }
            b.cancel()
            guard self.browser === b else { return }
            self.browser = nil
            self.hub.queue.asyncAfter(deadline: .now() + 2) { [weak self] in
                guard let self, !self.scanning.isEmpty, self.browser == nil else { return }
                self.browse()
            }
        }
        browser = b
        b.start(queue: hub.queue)
    }

    /// Dials every scanned-for token that was seen and nobody is connected for.
    private func dial() {
        for token in scanning {
            // Our own service, seen on our own interfaces.
            if advertising.contains(token) { continue }
            guard var places = found[token], let place = places.first, hub.claim(token) else { continue }
            // Next time, try another interface first.
            places.append(places.removeFirst())
            found[token] = places
            let conn = NWConnection(to: place, using: Self.parameters(loopback: loopback != nil))
            TCPLink(id: hub.newLinkId(), connection: conn, token: token, hub: hub).start()
        }
    }
}

/// A TCP connection: a byte-stream link.
final class TCPLink: NearbyLink {
    let id: UInt64
    let token: Data?
    private let connection: NWConnection
    private unowned let hub: NearbyTransport
    private var open = false
    private var ended = false

    init(id: UInt64, connection: NWConnection, token: Data?, hub: NearbyTransport) {
        self.id = id
        self.connection = connection
        self.token = token
        self.hub = hub
    }

    func start() {
        // The handler keeps the link alive until the connection ends.
        connection.stateUpdateHandler = { state in
            switch state {
            case .ready:
                guard !self.open, !self.ended else { return }
                self.open = true
                self.hub.opened(self, kind: .stream, medium: .wifi)
                self.receive()
            case .waiting:
                // No route yet: give up rather than hold the token.
                if !self.open { self.connection.cancel() }
            case .failed, .cancelled:
                self.finish()
            default:
                break
            }
        }
        connection.start(queue: hub.queue)
    }

    private func receive() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 1 << 16) { data, _, complete, error in
            if let data, !data.isEmpty { self.hub.received(self.id, data) }
            if complete || error != nil {
                self.connection.cancel()
            } else if !self.ended {
                self.receive()
            }
        }
    }

    func send(_ data: Data) {
        let n = data.count
        connection.send(content: data, completion: .contentProcessed { error in
            if error == nil { self.hub.wrote(self.id, n) }
        })
    }

    func close() {
        connection.cancel()
    }

    private func finish() {
        guard !ended else { return }
        ended = true
        connection.stateUpdateHandler = nil
        if open { hub.closed(id) } else { hub.dialFailed(token) }
    }
}
