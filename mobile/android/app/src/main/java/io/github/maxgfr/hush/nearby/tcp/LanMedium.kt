package io.github.maxgfr.hush.nearby.tcp

import io.github.maxgfr.hush.nearby.NearbyMedium
import io.github.maxgfr.hush.nearby.Tokens
import java.net.InetAddress

/** DNS-SD for `_hush._tcp`: Android's NSD on a real network, an in-process registry in tests. */
interface ServiceDiscovery {
    /** Publishes exactly these instances (name → port). */
    fun publish(services: Map<String, Int>)

    /** Looks for exactly these instance names (empty: stop); [found] may be called repeatedly. */
    fun browse(names: Set<String>, found: (name: String, host: InetAddress, port: Int) -> Unit)

    fun stop()

    companion object {
        const val SERVICE_TYPE = "_hush._tcp"
    }
}

/**
 * The local network: each advertised token is a `_hush._tcp` instance named after its hex, on
 * the port of that token's listener; a scanned token found there is dialed over TCP.
 */
class LanMedium(private val discovery: ServiceDiscovery, private val dialer: TcpDialer) : NearbyMedium {
    override fun advertise(tokens: List<ByteArray>, tcpPorts: Map<String, Int>) {
        discovery.publish(tokens.map(Tokens::toHex).mapNotNull { hex -> tcpPorts[hex]?.let { hex to it } }.toMap())
    }

    override fun scan(tokens: List<ByteArray>) {
        val wanted = tokens.associateBy(Tokens::toHex)
        discovery.browse(wanted.keys) { name, host, port ->
            wanted[name.lowercase()]?.let { dialer.dial(host, port, it) }
        }
    }

    override fun stop() = discovery.stop()
}
