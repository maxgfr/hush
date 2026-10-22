package io.github.maxgfr.hush

import io.github.maxgfr.hush.core.FileItem
import io.github.maxgfr.hush.core.FileState
import io.github.maxgfr.hush.core.MessageItem
import io.github.maxgfr.hush.core.NoticeItem
import io.github.maxgfr.hush.core.Reducer
import io.github.maxgfr.hush.core.UiState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.hush_ffi.AppEvent
import uniffi.hush_ffi.ChatType
import uniffi.hush_ffi.Ending
import uniffi.hush_ffi.LookupStatus
import uniffi.hush_ffi.NoticeKind
import uniffi.hush_ffi.PeerPath
import uniffi.hush_ffi.Status
import uniffi.hush_ffi.VaultStatus

class ReducerTest {
    private val now = 1_000_000L

    private fun fold(vararg events: AppEvent, at: Long = now): UiState =
        events.fold(UiState()) { s, e -> Reducer.reduce(s, e, at) }

    private val opened = AppEvent.ChatOpened(1u, ChatType.Link(group = false, creator = true), "https://x/#2.s.a.b", 5_000_000u)
    private val joined = AppEvent.PeerJoined(1u, "p1", "Bob", byteArrayOf(0, 1, 63, 64, -1), false)

    @Test
    fun aLinkChatFromOpeningToTheEnd() {
        var s = fold(opened)
        val c = s.chats.getValue(1u)
        assertTrue(c.creator)
        assertFalse(c.group)
        assertEquals(5_000_000L, c.expiresMs)
        assertEquals(Status.WAITING, c.status)
        assertEquals(listOf(c), s.live)

        s = Reducer.reduce(s, joined, now)
        s = Reducer.reduce(s, AppEvent.ChatStatus(1u, Status.OPEN), now)
        s = Reducer.reduce(s, AppEvent.PeerPath(1u, "p1", PeerPath.WIFI), now)
        val p = s.chats.getValue(1u).peer("p1")!!
        // Indices come as signed bytes; the table wants 0..255.
        assertEquals(listOf(0, 1, 63, 64, 255), p.sas)
        assertEquals(PeerPath.WIFI, p.path)
        assertFalse(p.verified)

        s = Reducer.reduce(s, AppEvent.PeerVerified(1u, "p1", byMe = true, byPeer = false), now)
        assertFalse(s.chats.getValue(1u).peer("p1")!!.verified)
        s = Reducer.reduce(s, AppEvent.PeerVerified(1u, "p1", byMe = true, byPeer = true), now)
        assertTrue(s.chats.getValue(1u).peer("p1")!!.verified)

        s = Reducer.reduce(s, AppEvent.PeerRenamed(1u, "p1", "Robert"), now)
        assertEquals("Robert", s.chats.getValue(1u).peer("p1")!!.name)

        s = Reducer.reduce(s, AppEvent.ChatEnded(1u, Ending.PEER_LEFT), now)
        assertEquals(Ending.PEER_LEFT, s.chats.getValue(1u).ended)
        assertTrue(s.live.isEmpty())
        assertTrue(Reducer.forget(s, 1u).chats.isEmpty())
    }

    @Test
    fun messagesArriveAreReplacedDeletedAndDisappear() {
        var s = fold(opened, joined)
        s = Reducer.reduce(s, AppEvent.Message(1u, null, "m1", "Me", "hi", 10u, null, false), now)
        s = Reducer.reduce(s, AppEvent.Message(1u, "p1", "m2", "Bob", "hello", 11u, (now + 500).toULong(), false), now)
        // The same message again (history restored) replaces it rather than adding one.
        s = Reducer.reduce(s, AppEvent.Message(1u, "p1", "m2", "Bob", "hello", 11u, (now + 500).toULong(), true), now)
        val items = s.chats.getValue(1u).items
        assertEquals(2, items.size)
        assertTrue((items[0] as MessageItem).mine)
        assertTrue((items[1] as MessageItem).past)

        s = Reducer.reduce(s, AppEvent.MessageDeleted(1u, "m1"), now)
        assertEquals(listOf("m2"), s.chats.getValue(1u).items.map { it.id })

        // Any event after the timer ran out leaves nothing of it.
        s = Reducer.reduce(s, AppEvent.Locked(1u, true), now + 1_000)
        assertTrue(s.chats.getValue(1u).items.isEmpty())
        assertTrue(s.chats.getValue(1u).locked)
    }

    @Test
    fun aReceivedFileGoesFromOfferToSaved() {
        var s = fold(opened, joined, AppEvent.FileOffered(1u, "p1", "f1", "a.bin", 300u, "application/octet-stream", null))
        assertEquals(FileState.OFFERED, file(s).state)
        s = Reducer.downloading(s, 1u, "f1")
        assertEquals(FileState.DOWNLOADING, file(s).state)
        s = Reducer.reduce(s, AppEvent.FileProgress(1u, "f1", 150u, 300u), now)
        assertEquals(0.5f, file(s).progress)
        s = Reducer.cancelled(s, 1u, "f1")
        assertEquals(FileState.OFFERED, file(s).state)
        s = Reducer.reduce(s, AppEvent.FileSaved(1u, "f1", "/data/received/a.bin"), now)
        assertEquals(FileState.SAVED, file(s).state)
        assertEquals("/data/received/a.bin", file(s).path)
    }

    @Test
    fun aSentFileIsDeliveredOrFails() {
        var s = fold(opened, joined, AppEvent.FileOffered(1u, null, "f1", "a.bin", 300u, "", null))
        assertTrue(file(s).mine)
        assertEquals(FileState.SENDING, file(s).state)
        s = Reducer.reduce(s, AppEvent.FileProgress(1u, "f1", 100u, 300u), now)
        assertEquals(FileState.SENDING, file(s).state)
        val delivered = Reducer.reduce(s, AppEvent.FileDelivered(1u, "f1", "p1"), now)
        assertEquals(FileState.DELIVERED, file(delivered).state)
        val failed = Reducer.reduce(s, AppEvent.FileFailed(1u, "f1", "lost"), now)
        assertEquals(FileState.FAILED, file(failed).state)
        assertEquals("lost", file(failed).reason)
    }

    @Test
    fun noticesOffersAndDepartures() {
        var s = fold(opened, joined, AppEvent.Notice(1u, NoticeKind.Joined("Bob")))
        assertTrue(s.chats.getValue(1u).items.single() is NoticeItem)
        s = Reducer.reduce(s, AppEvent.CardOffer(1u, "p1", "Bob", "hush1x"), now)
        s = Reducer.reduce(s, AppEvent.CardOffer(1u, "p1", "Bob", "hush1x"), now)
        assertEquals(1, s.chats.getValue(1u).offers.size)
        assertTrue(Reducer.dismissOffer(s, 1u, "p1").chats.getValue(1u).offers.isEmpty())
        s = Reducer.reduce(s, AppEvent.PeerLeft(1u, "p1", "Bob"), now)
        val c = s.chats.getValue(1u)
        assertTrue(c.present.isEmpty())
        assertTrue(c.offers.isEmpty())
    }

    @Test
    fun eventsForUnknownChatsChangeNothing() {
        val s = fold(AppEvent.Message(9u, null, "m", "", "x", 1u, null, false), AppEvent.ChatStatus(9u, Status.OPEN))
        assertTrue(s.chats.isEmpty())
    }

    @Test
    fun vaultIdentityContactsAndLookups() {
        val s = fold(
            AppEvent.Vault(VaultStatus.UNLOCKED),
            AppEvent.Identity("hush1abc"),
            AppEvent.Reachable(true),
            AppEvent.Lookup("hush1def", LookupStatus.SEARCHING, null),
        )
        assertEquals(VaultStatus.UNLOCKED, s.vault)
        assertEquals("hush1abc", s.identity)
        assertTrue(s.reachable)
        assertEquals(LookupStatus.SEARCHING, s.lookup!!.status)
        assertNull(s.lookup!!.chat)
    }

    private fun file(s: UiState): FileItem = s.chats.getValue(1u).items.filterIsInstance<FileItem>().single()
}
