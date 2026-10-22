import Foundation

/// A byte-stream link over a pair of Foundation streams: a Bluetooth L2CAP channel.
final class StreamLink: NSObject, NearbyLink, StreamDelegate {
    let id: UInt64
    let token: Data?
    /// Runs once the link is down (the scanner hangs up the Bluetooth connection).
    var onClose: (() -> Void)?
    private let input: InputStream
    private let output: OutputStream
    private unowned let hub: NearbyTransport
    private let medium: Medium
    /// What holds the streams open (the L2CAP channel), kept as long as the link.
    private var owner: AnyObject?
    /// Written in order; the first one may be partly written.
    private var pending: [Data] = []
    private var offset = 0
    private var ended = false

    init(id: UInt64, input: InputStream, output: OutputStream, token: Data?, medium: Medium, owner: AnyObject?, hub: NearbyTransport) {
        self.id = id
        self.input = input
        self.output = output
        self.token = token
        self.medium = medium
        self.owner = owner
        self.hub = hub
    }

    /// Opens the streams on the transport's queue and reports the link (it is already connected).
    func start() {
        input.delegate = self
        output.delegate = self
        CFReadStreamSetDispatchQueue(input, hub.queue)
        CFWriteStreamSetDispatchQueue(output, hub.queue)
        input.open()
        output.open()
        hub.opened(self, kind: .stream, medium: medium)
    }

    func stream(_ stream: Stream, handle event: Stream.Event) {
        if event.contains(.hasBytesAvailable) { read() }
        if event.contains(.hasSpaceAvailable) { flush() }
        if event.contains(.errorOccurred) || event.contains(.endEncountered) { finish() }
    }

    private func read() {
        var buf = [UInt8](repeating: 0, count: 1 << 16)
        while !ended && input.hasBytesAvailable {
            let n = input.read(&buf, maxLength: buf.count)
            if n < 0 { return finish() }
            if n == 0 { break }
            hub.received(id, Data(buf[0..<n]))
        }
    }

    func send(_ data: Data) {
        guard !ended, !data.isEmpty else { return }
        pending.append(data)
        flush()
    }

    private func flush() {
        while !ended, output.hasSpaceAvailable, let first = pending.first {
            let n = first.withUnsafeBytes { raw -> Int in
                guard let base = raw.bindMemory(to: UInt8.self).baseAddress else { return 0 }
                return output.write(base + offset, maxLength: first.count - offset)
            }
            if n < 0 { return finish() }
            if n == 0 { break }
            hub.wrote(id, n)
            offset += n
            if offset == first.count {
                pending.removeFirst()
                offset = 0
            }
        }
    }

    func close() {
        teardown()
    }

    private func finish() {
        guard !ended else { return }
        teardown()
        hub.closed(id)
    }

    private func teardown() {
        ended = true
        pending = []
        for s in [input, output] as [Stream] {
            s.delegate = nil
            s.close()
        }
        CFReadStreamSetDispatchQueue(input, nil)
        CFWriteStreamSetDispatchQueue(output, nil)
        owner = nil
        onClose?()
        onClose = nil
    }
}
