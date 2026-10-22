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
 * tokens we are dialing, so a device heard over Bluetooth and Wi-Fi at once is dialed only once.
 */
class LinkHub(private val sink: LinkSink) {
    private val next = AtomicLong(1)
    private val links = ConcurrentHashMap<Long, Link>()
    // Token (hex) → the link dialed for it, or PENDING while connecting.
    private val dialing = ConcurrentHashMap<String, Long>()

    fun newId(): ULong = next.getAndIncrement().toULong()

    /** True when nobody dials this token yet: the caller now owns it until it opens or fails. */
    fun claim(token: String): Boolean = dialing.putIfAbsent(token, PENDING) == null

    /** A dial that failed before opening a link. */
    fun release(token: String) {
        dialing.remove(token, PENDING)
    }

    /** Registers a link and tells the engine; call before the link delivers any byte. */
    fun open(id: ULong, link: Link, kind: LinkKind, medium: Medium, token: ByteArray?) {
        links[id.toLong()] = link
        if (token != null) dialing[Tokens.toHex(token)] = id.toLong()
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
    }
}
