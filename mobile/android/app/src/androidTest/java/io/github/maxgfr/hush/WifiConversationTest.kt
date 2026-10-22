package io.github.maxgfr.hush

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import io.github.maxgfr.hush.core.Sas
import io.github.maxgfr.hush.nearby.EngineSink
import io.github.maxgfr.hush.nearby.LinkHub
import io.github.maxgfr.hush.nearby.NearbyTransport
import io.github.maxgfr.hush.nearby.tcp.LanMedium
import io.github.maxgfr.hush.nearby.tcp.ServiceDiscovery
import io.github.maxgfr.hush.nearby.tcp.TcpDialer
import io.github.maxgfr.hush.nearby.tcp.TcpServers
import java.io.File
import java.net.InetAddress
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.hush_ffi.AppConfig
import uniffi.hush_ffi.AppEvent
import uniffi.hush_ffi.AppListener
import uniffi.hush_ffi.Ending
import uniffi.hush_ffi.HushApp
import uniffi.hush_ffi.PeerPath
import uniffi.hush_ffi.sasNames
import uniffi.hush_ffi.sasRender

/**
 * Two engines in this process, no relay and no network: they find each other through an
 * in-process DNS-SD registry (standing in for NSD) and talk over the app's real TCP listener,
 * dialer and stream links on localhost.
 */
@RunWith(AndroidJUnit4::class)
class WifiConversationTest {
    /** `_hush._tcp` on this device only: what NSD would answer on a network. */
    private class Registry {
        private val services = HashMap<String, Int>()
        private val views = ArrayList<View>()

        inner class View : ServiceDiscovery {
            var mine: Map<String, Int> = emptyMap()
            var wanted: Set<String> = emptySet()
            var found: (String, InetAddress, Int) -> Unit = { _, _, _ -> }

            override fun publish(services: Map<String, Int>) {
                synchronized(this@Registry) {
                    mine.keys.forEach { this@Registry.services.remove(it) }
                    this@Registry.services.putAll(services)
                    mine = services
                }
                announce()
            }

            override fun browse(names: Set<String>, found: (name: String, host: InetAddress, port: Int) -> Unit) {
                synchronized(this@Registry) {
                    wanted = names
                    this.found = found
                }
                announce()
            }

            override fun stop() = publish(emptyMap())
        }

        fun view(): ServiceDiscovery = synchronized(this) { View().also { views.add(it) } }

        private fun announce() {
            val calls = synchronized(this) {
                views.flatMap { v -> services.filterKeys { it in v.wanted }.map { (name, port) -> Triple(v.found, name, port) } }
            }
            calls.forEach { (found, name, port) -> found(name, InetAddress.getLoopbackAddress(), port) }
        }
    }

    private class Person(name: String, registry: Registry) : AppListener {
        val events = LinkedBlockingQueue<AppEvent>()
        val seen = ArrayList<AppEvent>()
        val dir: File = File(InstrumentationRegistry.getInstrumentation().targetContext.cacheDir, "e2e-$name").apply {
            deleteRecursively()
            mkdirs()
        }
        val app: HushApp

        init {
            val sink = EngineSink()
            val hub = LinkHub(sink)
            val transport = NearbyTransport(hub, TcpServers(hub), listOf(LanMedium(registry.view(), TcpDialer(hub))))
            app = HushApp(
                AppConfig(
                    name = name,
                    base = "https://maxgfr.github.io/hush/",
                    relays = emptyList(),
                    trackers = emptyList(),
                    iceServers = emptyList(),
                    relayOnly = false,
                    outDir = File(dir, "out").apply { mkdirs() }.path,
                    dataDir = File(dir, "data").path,
                    local = true,
                    ttl = 0u,
                ),
                this,
                transport,
            )
            sink.attach(app)
        }

        override fun onEvent(event: AppEvent) {
            events.add(event)
        }

        /** The first event, seen or coming, that [pick] accepts, within 30 seconds. */
        fun <T> await(what: String, pick: (AppEvent) -> T?): T {
            seen.firstNotNullOfOrNull(pick)?.let { return it }
            val deadline = System.currentTimeMillis() + 30_000
            while (true) {
                val left = deadline - System.currentTimeMillis()
                val e = events.poll(left.coerceAtLeast(0), TimeUnit.MILLISECONDS)
                    ?: throw AssertionError("timed out waiting for $what; saw $seen")
                seen.add(e)
                pick(e)?.let { return it }
            }
        }

        fun close() {
            app.shutdown()
            app.close()
            dir.deleteRecursively()
        }
    }

    private val registry = Registry()
    private val alice = Person("Alice", registry)
    private val bob = Person("Bob", registry)

    @After
    fun tearDown() {
        alice.close()
        bob.close()
    }

    @Test
    fun aWholeLinkConversationOverTcp() {
        val ca = alice.app.createLink(false, 10u, null)
        val url = alice.await("the link") { (it as? AppEvent.ChatOpened)?.takeIf { e -> e.chat == ca }?.url }
        val cb = bob.app.joinLink(url, null)

        val (bobTag, sasA) = alice.await("Bob") { (it as? AppEvent.PeerJoined)?.takeIf { e -> e.chat == ca }?.let { e -> e.peer to e.sas } }
        val sasB = bob.await("Alice") { (it as? AppEvent.PeerJoined)?.takeIf { e -> e.chat == cb }?.sas }
        assertArrayEquals("both see the same emojis", sasA, sasB)
        assertEquals(5, sasA.size)
        alice.await("a Wi-Fi path") { (it as? AppEvent.PeerPath)?.takeIf { e -> e.chat == ca && e.path == PeerPath.WIFI } }
        bob.await("a Wi-Fi path") { (it as? AppEvent.PeerPath)?.takeIf { e -> e.chat == cb && e.path == PeerPath.WIFI } }

        alice.app.verify(ca, bobTag, true)
        bob.app.verify(cb, null, true)
        alice.await("verified") { (it as? AppEvent.PeerVerified)?.takeIf { e -> e.chat == ca && e.byMe && e.byPeer } }
        bob.await("verified") { (it as? AppEvent.PeerVerified)?.takeIf { e -> e.chat == cb && e.byMe && e.byPeer } }

        alice.app.sendText(ca, "hello Bob")
        val author = bob.await("hello Bob") {
            (it as? AppEvent.Message)?.takeIf { e -> e.chat == cb && e.peer != null && e.text == "hello Bob" }?.author
        }
        assertEquals("Alice", author)
        bob.app.sendText(cb, "hello Alice\nsecond line")
        alice.await("Bob's answer") {
            (it as? AppEvent.Message)?.takeIf { e -> e.chat == ca && e.peer != null && e.text == "hello Alice\nsecond line" }
        }

        // A file bigger than one chunk.
        val data = ByteArray(300_000) { (it * 31 % 251).toByte() }
        val src = File(alice.dir, "photo.bin").apply { writeBytes(data) }
        alice.app.sendFile(ca, src.path)
        val id = bob.await("the offer") {
            (it as? AppEvent.FileOffered)?.takeIf { e -> e.chat == cb && e.peer != null && e.name == "photo.bin" }?.id
        }
        bob.app.getFile(cb, id)
        val saved = bob.await("the file") { (it as? AppEvent.FileSaved)?.takeIf { e -> e.chat == cb }?.path }
        assertArrayEquals(data, File(saved).readBytes())
        alice.await("delivery") { (it as? AppEvent.FileDelivered)?.takeIf { e -> e.chat == ca } }

        alice.app.end(ca)
        val reason = bob.await("the end") { (it as? AppEvent.ChatEnded)?.takeIf { e -> e.chat == cb }?.reason }
        assertEquals(Ending.PEER_LEFT, reason)
    }

    @Test
    fun theAppsEmojiTableIsTheEngines() {
        val all = ByteArray(64) { it.toByte() }
        assertEquals(Sas.TABLE.map { it.glyph }, sasRender(all).split(" "))
        assertEquals(Sas.TABLE.map { it.en }, sasNames(all).split(", "))
        assertTrue(Sas.TABLE.all { it.fr.isNotBlank() })
    }
}
