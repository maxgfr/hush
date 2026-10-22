package io.github.maxgfr.hush.nearby.wifidirect

import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pDevice
import android.net.wifi.p2p.WifiP2pManager
import android.net.wifi.p2p.nsd.WifiP2pDnsSdServiceInfo
import android.net.wifi.p2p.nsd.WifiP2pDnsSdServiceRequest
import android.os.Handler
import android.os.Looper
import androidx.core.content.ContextCompat
import io.github.maxgfr.hush.nearby.LinkHub
import io.github.maxgfr.hush.nearby.NearbyMedium
import io.github.maxgfr.hush.nearby.NearbyPermissions
import io.github.maxgfr.hush.nearby.Tokens
import io.github.maxgfr.hush.nearby.tcp.ServiceDiscovery
import io.github.maxgfr.hush.nearby.tcp.TcpDialer

/**
 * Wi-Fi Direct, for two Android phones with no network at all (best effort: phones differ a lot
 * here). The advertiser publishes a DNS-SD service named after the token's hex, with its TCP port
 * in the TXT record; the scanner asks to join that device's group, preferring it as group owner,
 * then dials the owner's address on that port.
 *
 * Joining a group puts this phone on the owner's network, on the word of a token anyone nearby
 * can copy: the scanner only does it once Bluetooth and the local network found nothing.
 */
@SuppressLint("MissingPermission") // Every entry point checks NearbyPermissions first.
class WifiDirectMedium(
    private val context: Context,
    private val hub: LinkHub,
    private val dialer: TcpDialer,
) : NearbyMedium {
    private val main = Handler(Looper.getMainLooper())
    private val manager: WifiP2pManager? = context.getSystemService(WifiP2pManager::class.java)
    private var channel: WifiP2pManager.Channel? = null
    private var receiver: BroadcastReceiver? = null
    @Volatile private var wanted: Map<String, ByteArray> = emptyMap()
    // Token (hex) → when we started looking for it.
    @Volatile private var since: Map<String, Long> = emptyMap()
    @Volatile private var advertising = false
    @Volatile private var pending: Pending? = null
    private var request: WifiP2pDnsSdServiceRequest? = null
    private var groupLinks = 0

    private class Pending(val hex: String, val token: ByteArray, val port: Int, val endpoint: String)

    private fun ready(): WifiP2pManager.Channel? {
        val m = manager ?: return null
        if (!NearbyPermissions.has(context, NearbyPermissions.wifiDirect())) return null
        channel?.let { return it }
        val c = m.initialize(context, Looper.getMainLooper(), null) ?: return null
        channel = c
        val r = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                if (intent.action == WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION) onConnectionChanged()
            }
        }
        ContextCompat.registerReceiver(
            context,
            r,
            IntentFilter(WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        receiver = r
        return c
    }

    override fun advertise(tokens: List<ByteArray>, tcpPorts: Map<String, Int>) {
        main.post { publish(tokens, tcpPorts) }
    }

    override fun scan(tokens: List<ByteArray>) {
        main.post { look(tokens) }
    }

    private fun publish(tokens: List<ByteArray>, tcpPorts: Map<String, Int>) {
        val c = ready() ?: return
        val m = manager ?: return
        m.clearLocalServices(c, null)
        advertising = false
        for (token in tokens) {
            val hex = Tokens.toHex(token)
            val port = tcpPorts[hex] ?: continue
            val info = WifiP2pDnsSdServiceInfo.newInstance(hex, ServiceDiscovery.SERVICE_TYPE, mapOf("port" to port.toString()))
            m.addLocalService(c, info, null)
            advertising = true
        }
        // A device is only found while it is itself discoverable.
        if (advertising) m.discoverPeers(c, null) else if (wanted.isEmpty()) m.stopPeerDiscovery(c, null)
    }

    private fun look(tokens: List<ByteArray>) {
        wanted = tokens.associateBy(Tokens::toHex)
        val now = System.currentTimeMillis()
        since = wanted.keys.associateWith { since[it] ?: now }
        val c = ready() ?: return
        val m = manager ?: return
        main.removeCallbacks(rediscover)
        request?.let { m.removeServiceRequest(c, it, null) }
        request = null
        if (wanted.isEmpty()) {
            if (!advertising) m.stopPeerDiscovery(c, null)
            return
        }
        m.setDnsSdResponseListeners(c, { _, _, _ -> }, ::onTxtRecord)
        val r = WifiP2pDnsSdServiceRequest.newInstance(ServiceDiscovery.SERVICE_TYPE)
        request = r
        m.addServiceRequest(c, r, null)
        rediscover.run()
    }

    // Service discovery stops on its own after a while: ask again while we are looking.
    private val rediscover = object : Runnable {
        override fun run() {
            val c = channel ?: return
            if (wanted.isEmpty()) return
            manager?.discoverServices(c, null)
            main.postDelayed(this, REDISCOVER_MS)
        }
    }

    private fun onTxtRecord(domain: String, txt: Map<String, String>, device: WifiP2pDevice) {
        val hex = domain.substringBefore('.').lowercase()
        val token = wanted[hex] ?: return
        val port = txt["port"]?.toIntOrNull() ?: return
        val c = channel ?: return
        val start = since[hex] ?: return
        if (System.currentTimeMillis() - start < FALLBACK_MS || hub.busy(hex)) return
        // One group at a time: a new one would tear down the one carrying a link.
        if (pending != null || groupLinks > 0) return
        val endpoint = "p2p:${device.deviceAddress}"
        if (!hub.claim(hex, endpoint)) return
        val p = Pending(hex, token, port, endpoint)
        pending = p
        main.postDelayed({ giveUp(p) }, p, CONNECT_TIMEOUT_MS)
        val config = WifiP2pConfig().apply {
            deviceAddress = device.deviceAddress
            // The advertiser listens: it should own the group, so we know where to dial.
            groupOwnerIntent = 0
        }
        manager?.connect(c, config, object : WifiP2pManager.ActionListener {
            override fun onSuccess() {}
            override fun onFailure(reason: Int) {
                pending?.let(::giveUp)
            }
        })
    }

    private fun onConnectionChanged() {
        val c = channel ?: return
        val p = pending ?: return
        manager?.requestConnectionInfo(c) { info ->
            if (info == null || !info.groupFormed || pending !== p) return@requestConnectionInfo
            pending = null
            main.removeCallbacksAndMessages(p)
            val owner = info.groupOwnerAddress
            if (info.isGroupOwner || owner == null) {
                // We own the group: we cannot tell the advertiser's address. Leave it to the next try.
                hub.release(p.hex, p.endpoint)
                if (groupLinks == 0) manager?.removeGroup(c, null)
                return@requestConnectionInfo
            }
            groupLinks++
            // The dial and then its link hold the claim; either one ending ends the group.
            dialer.dial(owner, p.port, p.token, claimed = p.endpoint) { main.post(::linkClosed) }
        }
    }

    private fun linkClosed() {
        groupLinks--
        if (groupLinks <= 0) {
            groupLinks = 0
            channel?.let { manager?.removeGroup(it, null) }
        }
    }

    private fun giveUp(p: Pending) {
        if (pending !== p) return
        pending = null
        main.removeCallbacksAndMessages(p)
        hub.release(p.hex, p.endpoint)
        channel?.let { manager?.cancelConnect(it, null) }
    }

    override fun stop() {
        main.post {
            main.removeCallbacks(rediscover)
            pending?.let(::giveUp)
            wanted = emptyMap()
            since = emptyMap()
            advertising = false
            val c = channel ?: return@post
            manager?.clearLocalServices(c, null)
            manager?.clearServiceRequests(c, null)
            manager?.stopPeerDiscovery(c, null)
            if (groupLinks > 0) manager?.removeGroup(c, null)
            groupLinks = 0
            receiver?.let { runCatching { context.unregisterReceiver(it) } }
            receiver = null
            c.close()
            channel = null
        }
    }

    private companion object {
        const val REDISCOVER_MS = 20_000L
        const val CONNECT_TIMEOUT_MS = 30_000L
        const val FALLBACK_MS = 15_000L
    }
}
