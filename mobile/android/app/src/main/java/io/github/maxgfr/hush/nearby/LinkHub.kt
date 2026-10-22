package io.github.maxgfr.hush.nearby

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import uniffi.hush_ffi.LinkKind
import uniffi.hush_ffi.Medium

/** Where link events go: the engine (`HushApp.local_*`), or a fake in tests. */
interface LinkSink {
    fun opened(link: ULong, kind: LinkKind, medium: Medium, token: ByteArray?)
    fun data(link: ULong, data: ByteArray)
    fun sent(link: ULong, bytes: Int)
    fun closed(link: ULong)
}

/** One open connection to a nearby device: a byte stream or GATT packets. */
interface Link {
    /** Queues bytes; never blocks (the engine calls this from its own thread). */
    fun send(data: ByteArray)
    fun close()
}

/**
 * Every nearby link of every medium, by the id the engine knows it by. Also remembers which
 * endpoints we are dialing for which token, so a device heard twice is dialed once.
 *
 * A token is public (anyone scanning hears it), so claims are per endpoint: a stranger who
 * advertises a copied token holds only the dial to itself, never the one to the real device.
 */
class LinkHub(private val sink: LinkSink) {
    private val next = AtomicLong(1)
    private val links = ConcurrentHashMap<Long, Link>()
    // "token endpoint" → the link dialed there, or PENDING while connecting.
    private val dialing = ConcurrentHashMap<String, Long>()

    fun newId(): ULong = next.getAndIncrement().toULong()

    /**
     * True when nobody dials this token at this endpoint (a Bluetooth address, a host and port)
     * yet: the caller now owns the pair until it opens or fails.
     */
    fun claim(token: String, endpoint: String): Boolean = dialing.putIfAbsent(key(token, endpoint), PENDING) == null

    /** A dial that failed before opening a link. */
    fun release(token: String, endpoint: String) {
        dialing.remove(key(token, endpoint), PENDING)
    }

    /** Whether this token is being dialed, or has a link open, at any endpoint. */
    fun busy(token: String): Boolean = dialing.keys.any { it.startsWith("$token ") }

    /**
     * Registers a link and tells the engine; call before the link delivers any byte. [endpoint]:
     * the one claimed for [token], when we dialed.
     */
    fun open(id: ULong, link: Link, kind: LinkKind, medium: Medium, token: ByteArray?, endpoint: String? = null) {
        links[id.toLong()] = link
        if (token != null && endpoint != null) dialing[key(Tokens.toHex(token), endpoint)] = id.toLong()
        sink.opened(id, kind, medium, token)
    }

    fun data(id: ULong, data: ByteArray) {
        if (links.containsKey(id.toLong())) sink.data(id, data)
    }

    fun sent(id: ULong, bytes: Int) {
        if (links.containsKey(id.toLong())) sink.sent(id, bytes)
    }

    /** The link is gone (either side closed it): reported once. */
    fun closed(id: ULong) {
        if (links.remove(id.toLong()) == null) return
        dialing.entries.removeIf { it.value == id.toLong() }
        sink.closed(id)
    }

    fun send(id: ULong, data: ByteArray) {
        links[id.toLong()]?.send(data)
    }

    fun close(id: ULong) {
        links[id.toLong()]?.close()
    }

    fun closeAll() {
        links.values.toList().forEach { it.close() }
    }

    val count: Int get() = links.size

    private companion object {
        const val PENDING = 0L

        fun key(token: String, endpoint: String) = "$token $endpoint"
    }
}
