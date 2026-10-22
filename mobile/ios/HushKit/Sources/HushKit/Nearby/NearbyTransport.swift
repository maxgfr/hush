import Foundation

/// Where nearby links report to: the engine (`HushApp`), or a test.
public protocol LinkSink: AnyObject, Sendable {
    func linkOpened(_ link: UInt64, kind: LinkKind, medium: Medium, token: Data?)
    func linkData(_ link: UInt64, _ data: Data)
    func linkSent(_ link: UInt64, _ bytes: Int)
    func linkClosed(_ link: UInt64)
}

extension HushApp: LinkSink {
    public func linkOpened(_ link: UInt64, kind: LinkKind, medium: Medium, token: Data?) {
        localOpened(link: link, kind: kind, medium: medium, token: token)
    }

    public func linkData(_ link: UInt64, _ data: Data) {
        localData(link: link, data: data)
    }

    public func linkSent(_ link: UInt64, _ bytes: Int) {
        localSent(link: link, bytes: UInt64(bytes))
    }

    public func linkClosed(_ link: UInt64) {
        localClosed(link: link)
    }
}

/// One open connection to a nearby device. Every call happens on the transport's queue.
protocol NearbyLink: AnyObject {
    var id: UInt64 { get }
    /// The token we dialed for (nil when the other side dialed us).
    var token: Data? { get }
    func send(_ data: Data)
    func close()
}

/// A medium (Wi-Fi, Bluetooth) the transport drives. Every call happens on the transport's queue.
protocol NearbyMedium: AnyObject {
    func advertise(_ tokens: Set<Data>)
    func scan(_ tokens: Set<Data>)
    /// A token was released: dial it again if it is still wanted and was seen.
    func retry()
}

/// The engine's `LocalTransport`: Bluetooth LE and Wi-Fi (PROTOCOL.md §8). The engine decides
/// what to advertise and scan for and frames every byte; this only finds devices and moves bytes.
///
/// Everything runs on one serial queue, radios included, so no state is shared across threads.
public final class NearbyTransport: LocalTransport, @unchecked Sendable {
    public struct Options: Sendable {
        public var wifi: WifiDiscovery?
        public var bluetooth: Bool

        public init(wifi: WifiDiscovery? = .bonjour, bluetooth: Bool = true) {
            self.wifi = wifi
            self.bluetooth = bluetooth
        }
    }

    let queue = DispatchQueue(label: "hush.nearby")
    private weak var sink: (any LinkSink)?
    private var media: [NearbyMedium] = []
    private var links: [UInt64: NearbyLink] = [:]
    /// Tokens being dialed or connected, on any medium: one connection per token.
    private var claimed: Set<Data> = []
    private var nextLink: UInt64 = 0
    /// What the engine asked for last.
    private(set) var advertising: Set<Data> = []
    private(set) var scanning: Set<Data> = []

    public init(_ options: Options = Options()) {
        queue.sync {
            if let wifi = options.wifi { media.append(WifiMedium(hub: self, discovery: wifi)) }
            if options.bluetooth { media.append(BluetoothMedium(hub: self)) }
        }
    }

    /// Where links report to; set right after creating the engine with this transport.
    public func attach(_ sink: any LinkSink) {
        queue.async { self.sink = sink }
    }

    // LocalTransport: called on the engine's thread, must not block.

    public func advertise(tokens: [Data]) {
        let set = Set(tokens.filter { $0.count == NearbyToken.length })
        queue.async {
            self.advertising = set
            for m in self.media { m.advertise(set) }
        }
    }

    public func scan(tokens: [Data]) {
        let set = Set(tokens.filter { $0.count == NearbyToken.length })
        queue.async {
            self.scanning = set
            for m in self.media { m.scan(set) }
        }
    }

    public func send(link: UInt64, data: Data) {
        queue.async { self.links[link]?.send(data) }
    }

    public func close(link: UInt64) {
        queue.async {
            // The engine closed it: no `localClosed` back.
            guard let l = self.links.removeValue(forKey: link) else { return }
            self.release(l.token)
            l.close()
        }
    }

    // For the media, on the queue.

    func newLinkId() -> UInt64 {
        nextLink += 1
        return nextLink
    }

    /// Whether we may dial `token` now (nobody is dialing or connected for it).
    func claim(_ token: Data) -> Bool {
        claimed.insert(token).inserted
    }

    func release(_ token: Data?) {
        guard let token, claimed.remove(token) != nil else { return }
        for m in media { m.retry() }
    }

    /// A dial for `token` failed before a link opened.
    func dialFailed(_ token: Data?) {
        release(token)
    }

    func opened(_ link: NearbyLink, kind: LinkKind, medium: Medium) {
        guard let sink else {
            link.close()
            release(link.token)
            return
        }
        links[link.id] = link
        sink.linkOpened(link.id, kind: kind, medium: medium, token: link.token)
    }

    func received(_ link: UInt64, _ data: Data) {
        guard links[link] != nil else { return }
        sink?.linkData(link, data)
    }

    func wrote(_ link: UInt64, _ bytes: Int) {
        guard links[link] != nil, bytes > 0 else { return }
        sink?.linkSent(link, bytes)
    }

    /// The connection dropped on its own.
    func closed(_ link: UInt64) {
        guard let l = links.removeValue(forKey: link) else { return }
        release(l.token)
        sink?.linkClosed(link)
    }

    /// A stream link over streams that are already connected (an L2CAP channel; bound stream
    /// pairs in tests).
    func openStream(input: InputStream, output: OutputStream, token: Data?, medium: Medium) {
        queue.async {
            StreamLink(id: self.newLinkId(), input: input, output: output, token: token, medium: medium, owner: nil, hub: self).start()
        }
    }
}
