package io.github.maxgfr.hush

import io.github.maxgfr.hush.nearby.LinkHub
import io.github.maxgfr.hush.nearby.LinkSink
import io.github.maxgfr.hush.nearby.StreamLink
import io.github.maxgfr.hush.nearby.Tokens
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.io.PipedInputStream
import java.io.PipedOutputStream
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.hush_ffi.LinkKind
import uniffi.hush_ffi.Medium

class LinkHubTest {
    private class Recorder : LinkSink {
        val events = LinkedBlockingQueue<String>()
        val data = LinkedBlockingQueue<ByteArray>()

        override fun opened(link: ULong, kind: LinkKind, medium: Medium, token: ByteArray?) {
            val k = if (kind is LinkKind.Packets) "Packets(${kind.mtu})" else "Stream"
            events.add("opened $link $k $medium ${token?.let(Tokens::toHex)}")
        }

        override fun data(link: ULong, data: ByteArray) {
            this.data.add(data)
        }

        override fun sent(link: ULong, bytes: Int) {
            events.add("sent $link $bytes")
        }

        override fun closed(link: ULong) {
            events.add("closed $link")
        }

        fun next(): String = events.poll(5, TimeUnit.SECONDS) ?: error("no event")
    }

    @Test
    fun anEndpointIsDialedOnceAtATime() {
        val hub = LinkHub(Recorder())
        assertFalse(hub.busy("aa"))
        assertTrue(hub.claim("aa", "ble:1"))
        assertFalse(hub.claim("aa", "ble:1"))
        assertTrue(hub.busy("aa"))
        hub.release("aa", "ble:1")
        assertFalse(hub.busy("aa"))
        assertTrue(hub.claim("aa", "ble:1"))
        assertTrue(hub.claim("bb", "ble:1"))
        assertFalse(hub.busy("a"))
    }

    @Test
    fun aCopiedTokenElsewhereDoesNotBlockTheRealDevice() {
        val sink = Recorder()
        val hub = LinkHub(sink)
        val token = ByteArray(16) { 3 }
        val hex = Tokens.toHex(token)
        // Someone advertises the token they overheard, and we dial them first.
        assertTrue(hub.claim(hex, "p2p:fake"))
        val id = hub.newId()
        val remote = PipedOutputStream()
        val input = PipedInputStream(remote)
        StreamLink(hub, id, input, ByteArrayOutputStream(), Closeable { input.close() }).start(Medium.WIFI, token, "p2p:fake")
        assertEquals("opened $id Stream WIFI $hex", sink.next())
        assertFalse(hub.claim(hex, "p2p:fake"))
        // The real device, heard over Bluetooth and the local network, is still dialed.
        assertTrue(hub.claim(hex, "ble:real"))
        assertTrue(hub.claim(hex, "tcp:10.0.0.2:4000"))
        // A late release of an endpoint whose link is open leaves that link's claim alone.
        hub.release(hex, "p2p:fake")
        assertFalse(hub.claim(hex, "p2p:fake"))
        hub.close(id)
        assertEquals("closed $id", sink.next())
        assertTrue(hub.claim(hex, "p2p:fake"))
    }

    @Test
    fun aStreamLinkMovesBytesBothWaysAndClosesOnce() {
        val sink = Recorder()
        val hub = LinkHub(sink)
        val token = ByteArray(16) { 7 }
        assertTrue(hub.claim(Tokens.toHex(token), "tcp:peer"))
        val remote = PipedOutputStream()
        val input = PipedInputStream(remote)
        val output = ByteArrayOutputStream()
        val id = hub.newId()
        val link = StreamLink(hub, id, input, output, Closeable { input.close() })
        link.start(Medium.WIFI, token, "tcp:peer")
        assertEquals("opened $id Stream WIFI ${Tokens.toHex(token)}", sink.next())
        // While the link is open, nobody dials that endpoint again.
        assertFalse(hub.claim(Tokens.toHex(token), "tcp:peer"))

        remote.write(byteArrayOf(1, 2, 3))
        remote.flush()
        assertArrayEquals(byteArrayOf(1, 2, 3), sink.data.poll(5, TimeUnit.SECONDS))

        hub.send(id, byteArrayOf(9, 9))
        assertEquals("sent $id 2", sink.next())
        assertArrayEquals(byteArrayOf(9, 9), output.toByteArray())

        hub.close(id)
        hub.close(id)
        assertEquals("closed $id", sink.next())
        assertEquals(null, sink.events.poll(300, TimeUnit.MILLISECONDS))
        assertEquals(0, hub.count)
        assertFalse(hub.busy(Tokens.toHex(token)))
        assertTrue(hub.claim(Tokens.toHex(token), "tcp:peer"))
    }
}
