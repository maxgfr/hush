package io.github.maxgfr.hush.core

import android.content.Context
import io.github.maxgfr.hush.nearby.EngineSink
import io.github.maxgfr.hush.nearby.LinkHub
import io.github.maxgfr.hush.nearby.Nearby
import io.github.maxgfr.hush.nearby.NearbyTransport
import java.io.File
import uniffi.hush_ffi.HushApp
import uniffi.hush_ffi.defaultIceServers
import uniffi.hush_ffi.defaultRelays
import uniffi.hush_ffi.defaultTrackers

/**
 * The one engine of the process, with its radios and settings. Its blocking calls (anything that
 * answers) belong on Dispatchers.IO.
 */
class Engine(context: Context) {
    val store = AppStore()
    val settings: SettingsStore

    /** The web app's relays, trackers and STUN servers, for "Reset". */
    val defaults: Settings
    val nearby: NearbyTransport
    val app: HushApp

    /** Where received files land: app-private, shared only through the FileProvider on "Open". */
    val receivedDir: File

    /** Copies of the files we offer, which the engine reads when someone downloads them. */
    val outgoingDir: File

    init {
        val ctx = context.applicationContext
        defaults = Settings(
            relays = defaultRelays(),
            trackers = defaultTrackers(),
            stun = defaultIceServers().flatMap { it.urls },
        )
        settings = SettingsStore(ctx, defaults)
        val dataDir = File(ctx.filesDir, "hush").apply { mkdirs() }
        receivedDir = File(ctx.filesDir, "received").apply { mkdirs() }
        // Offers from a previous run are gone with their conversations.
        outgoingDir = File(ctx.cacheDir, "outgoing").apply {
            deleteRecursively()
            mkdirs()
        }
        val sink = EngineSink()
        nearby = Nearby.transport(ctx, LinkHub(sink))
        app = HushApp(settings.state.value.config(receivedDir.path, dataDir.path), store, nearby)
        sink.attach(app)
    }

    fun applyNetwork(s: Settings) = app.setNetwork(s.relays, s.trackers, s.iceServers(), s.relayOnly)
}
