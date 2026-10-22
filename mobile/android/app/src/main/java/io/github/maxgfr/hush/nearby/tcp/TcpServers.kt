package io.github.maxgfr.hush.nearby.tcp

import io.github.maxgfr.hush.nearby.LinkHub
import io.github.maxgfr.hush.nearby.StreamLink
import io.github.maxgfr.hush.nearby.Tokens
import java.io.IOException
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import uniffi.hush_ffi.Medium

/**
 * One TCP listener per advertised token, on every interface (the local network and a Wi-Fi Direct
 * group alike). Whoever connects gets a stream link; the engine decides what it is worth.
 */
class TcpServers(private val hub: LinkHub, private val address: InetAddress? = null) {
    private val servers = HashMap<String, ServerSocket>()

    /** Listens for exactly these tokens; returns each one's port, by hex. */
    @Synchronized
    fun update(tokens: List<ByteArray>): Map<String, Int> {
        val wanted = tokens.map(Tokens::toHex).toSet()
        for (gone in servers.keys - wanted) {
            // Links already accepted stay open: only the listener goes.
            try {
                servers.remove(gone)?.close()
            } catch (ignored: IOException) {
            }
        }
        for (hex in wanted - servers.keys) {
            val server = try {
                ServerSocket().apply {
                    reuseAddress = true
                    bind(InetSocketAddress(address, 0))
                }
            } catch (ignored: IOException) {
                continue
            }
            servers[hex] = server
            Thread({ accept(server) }, "hush-tcp-accept").apply { isDaemon = true }.start()
        }
        return servers.mapValues { it.value.localPort }
    }

    private fun accept(server: ServerSocket) {
        while (!server.isClosed) {
            val socket = try {
                server.accept()
            } catch (ignored: IOException) {
                return
            }
            Tcp.link(hub, socket).start(Medium.WIFI, null)
        }
    }
}

/** Dials a device found for a token. */
class TcpDialer(private val hub: LinkHub) {
    /**
     * Connects to [host]:[port] for [token], on a thread of its own. [claimed]: the caller already
     * holds the token's claim in the hub.
     */
    fun dial(
        host: InetAddress,
        port: Int,
        token: ByteArray,
        claimed: Boolean = false,
        onClosed: () -> Unit = {},
    ) {
        val hex = Tokens.toHex(token)
        if (!claimed && !hub.claim(hex)) return
        Thread({
            val socket = Socket()
            try {
                socket.connect(InetSocketAddress(host, port), CONNECT_TIMEOUT_MS)
                Tcp.link(hub, socket, onClosed).start(Medium.WIFI, token)
            } catch (ignored: IOException) {
                try {
                    socket.close()
                } catch (ignored: IOException) {
                }
                hub.release(hex)
                onClosed()
            }
        }, "hush-tcp-dial").apply { isDaemon = true }.start()
    }

    private companion object {
        const val CONNECT_TIMEOUT_MS = 8_000
    }
}

internal object Tcp {
    fun link(hub: LinkHub, socket: Socket, onClosed: () -> Unit = {}): StreamLink {
        socket.tcpNoDelay = true
        socket.keepAlive = true
        return StreamLink(hub, hub.newId(), socket.getInputStream(), socket.getOutputStream(), socket, onClosed)
    }
}
