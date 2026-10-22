package io.github.maxgfr.hush.nearby

import android.content.Context
import io.github.maxgfr.hush.nearby.ble.BleMedium
import io.github.maxgfr.hush.nearby.tcp.LanMedium
import io.github.maxgfr.hush.nearby.tcp.NsdDiscovery
import io.github.maxgfr.hush.nearby.tcp.TcpDialer
import io.github.maxgfr.hush.nearby.tcp.TcpServers
import io.github.maxgfr.hush.nearby.wifidirect.WifiDirectMedium
import uniffi.hush_ffi.HushApp
import uniffi.hush_ffi.LinkKind
import uniffi.hush_ffi.Medium

/**
 * Hands link events to the engine. The engine is built with its transport, so the transport
 * exists first: anything it reports before [attach] waits here.
 */
class EngineSink : LinkSink {
    @Volatile private var app: HushApp? = null
    private val early = ArrayList<(HushApp) -> Unit>()

    fun attach(app: HushApp) {
        synchronized(this) {
            this.app = app
            early.forEach { it(app) }
            early.clear()
        }
    }

    private fun call(f: (HushApp) -> Unit) {
        val a = app ?: synchronized(this) {
            app ?: run {
                early.add(f)
                return
            }
        }
        f(a)
    }

    override fun opened(link: ULong, kind: LinkKind, medium: Medium, token: ByteArray?) =
        call { it.localOpened(link, kind, medium, token) }

    override fun data(link: ULong, data: ByteArray) = call { it.localData(link, data) }

    override fun sent(link: ULong, bytes: Int) = call { it.localSent(link, bytes.toULong()) }

    override fun closed(link: ULong) = call { it.localClosed(link) }
}

object Nearby {
    /** Every medium the phone has: Bluetooth LE, the local network (NSD) and Wi-Fi Direct. */
    fun transport(context: Context, hub: LinkHub): NearbyTransport {
        val ctx = context.applicationContext
        val dialer = TcpDialer(hub)
        lateinit var transport: NearbyTransport
        val mediums = listOf(
            LanMedium(NsdDiscovery(ctx), dialer),
            BleMedium(ctx, hub) { transport.refresh() },
            WifiDirectMedium(ctx, hub, dialer),
        )
        transport = NearbyTransport(hub, TcpServers(hub), mediums)
        return transport
    }
}
