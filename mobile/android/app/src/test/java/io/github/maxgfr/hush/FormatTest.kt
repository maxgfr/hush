package io.github.maxgfr.hush

import io.github.maxgfr.hush.core.Format
import io.github.maxgfr.hush.core.LinkInput
import io.github.maxgfr.hush.core.Sas
import io.github.maxgfr.hush.core.Settings
import io.github.maxgfr.hush.ui.Labels
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.hush_ffi.Ending
import uniffi.hush_ffi.PeerPath

class FormatTest {
    @Test
    fun durationsAreCoarse() {
        assertEquals("1 min", Format.duration(1, fr = false))
        assertEquals("59 min", Format.duration(59 * 60_000L, fr = false))
        assertEquals("1 h", Format.duration(60 * 60_000L, fr = false))
        assertEquals("24 h", Format.duration(24 * 3_600_000L, fr = false))
        assertEquals("7 d", Format.duration(7 * 86_400_000L, fr = false))
        assertEquals("7 j", Format.duration(7 * 86_400_000L, fr = true))
        assertEquals("0 min", Format.duration(-5, fr = false))
    }

    @Test
    fun timersReadLikeTheWebApp() {
        assertEquals(
            listOf("off", "30 s", "5 min", "1 h", "1 d", "1 wk"),
            Settings.TTL_SECONDS.map { Format.ttl(it, "off", fr = false) },
        )
        assertEquals("1 sem.", Format.ttl(604_800, "aucun", fr = true))
    }

    @Test
    fun sizes() {
        assertEquals("512 B", Format.size(512))
        assertEquals("1.5 KB", Format.size(1536))
        assertEquals("2.0 MB", Format.size(2L * 1024 * 1024))
    }

    @Test
    fun linksAreFoundInSharedText() {
        val url = "https://maxgfr.github.io/hush/#2.s.AbC-d_e.k3x9"
        assertEquals(url, LinkInput.extract("Join me: $url."))
        assertEquals(url, LinkInput.extract("$url\nsee you"))
        assertEquals("#2.s.AbC", LinkInput.extract("  #2.s.AbC "))
        assertNull(LinkInput.extract("   "))
        assertTrue(LinkInput.isOurs(url))
        assertFalse(LinkInput.isOurs("https://maxgfr.github.io/hush/"))
        assertFalse(LinkInput.isOurs("https://example.org/hush/#2.s"))
    }

    @Test
    fun theSasTableMatchesTheCore() {
        assertEquals(64, Sas.TABLE.size)
        assertEquals("🐶", Sas.emoji(0).glyph)
        assertEquals("Pin", Sas.emoji(63).en)
        assertEquals("Punaise", Sas.emoji(63).fr)
        // Indices wrap like the core's (i & 63).
        assertEquals(Sas.emoji(1), Sas.emoji(65))
    }

    @Test
    fun labelsCoverEveryState() {
        PeerPath.entries.forEach { Labels.path(it) }
        Ending.entries.forEach { Labels.ending(it) }
        assertTrue(Labels.warns(Ending.TAKEN))
        assertFalse(Labels.warns(Ending.ENDED))
    }

    @Test
    fun settingsKeepOnlyTheRightSchemes() {
        assertEquals(
            listOf("wss://a", "wss://b"),
            Settings.lines("wss://a\n  http://x \n\nwss://b", "wss://"),
        )
        assertEquals(listOf("stun:s:3478", "stuns:t"), Settings.lines("stun:s:3478 turn:u stuns:t", "stun:", "stuns:"))
        val s = Settings(stun = listOf("stun:s"), turnUrl = " turn:t ", turnUser = "u", turnCredential = "c")
        assertEquals(listOf(listOf("stun:s"), listOf("turn:t")), s.iceServers().map { it.urls })
        assertTrue(Settings().iceServers().isEmpty())
    }
}
