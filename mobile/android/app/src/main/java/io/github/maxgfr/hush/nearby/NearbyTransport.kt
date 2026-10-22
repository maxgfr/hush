package io.github.maxgfr.hush.nearby

import io.github.maxgfr.hush.nearby.tcp.TcpServers
import java.util.concurrent.Executors
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import uniffi.hush_ffi.LocalTransport

/** One way of finding and reaching nearby devices: Bluetooth LE, the local network, Wi-Fi Direct. */
interface NearbyMedium {
    /**
     * Advertise exactly these tokens (empty: stop). [tcpPorts] maps each token's hex to the port
     * of its TCP listener, for the Wi-Fi mediums.
     */
    fun advertise(tokens: List<ByteArray>, tcpPorts: Map<String, Int>)

    /** Look for exactly these tokens (empty: stop). */
    fun scan(tokens: List<ByteArray>)

    fun stop()
}

/**
 * The engine's `LocalTransport`: what it asks to advertise and scan goes to every medium, and its
 * bytes go to the link they belong to. Calls from the engine hop to one worker thread, in order,
 * so the engine is never blocked by a radio.
 */
class NearbyTransport(
    private val hub: LinkHub,
    private val servers: TcpServers?,
    private val mediums: List<NearbyMedium>,
) : LocalTransport {
    private val worker = Executors.newSingleThreadExecutor { r -> Thread(r, "hush-nearby").apply { isDaemon = true } }
    @Volatile private var advertised: List<ByteArray> = emptyList()
    @Volatile private var scanned: List<ByteArray> = emptyList()

    private val _active = MutableStateFlow(false)

    /** Whether the radios are advertising or scanning for anything (the foreground service shows it). */
    val active: StateFlow<Boolean> = _active

    override fun advertise(tokens: List<ByteArray>) {
        advertised = tokens
        worker.execute { applyAdvertise(tokens) }
    }

    override fun scan(tokens: List<ByteArray>) {
        scanned = tokens
        worker.execute { applyScan(tokens) }
    }

    override fun send(link: ULong, data: ByteArray) = hub.send(link, data)

    override fun close(link: ULong) {
        worker.execute { hub.close(link) }
    }

    /** Applies the current sets again: after a permission was granted or a radio came back on. */
    fun refresh() {
        worker.execute {
            applyAdvertise(advertised)
            applyScan(scanned)
        }
    }

    fun stop() {
        worker.execute {
            mediums.forEach { safely { it.stop() } }
            servers?.update(emptyList())
            hub.closeAll()
            _active.value = false
        }
    }

    private fun applyAdvertise(tokens: List<ByteArray>) {
        val ports = servers?.update(tokens) ?: emptyMap()
        mediums.forEach { safely { it.advertise(tokens, ports) } }
        _active.value = advertised.isNotEmpty() || scanned.isNotEmpty()
    }

    private fun applyScan(tokens: List<ByteArray>) {
        mediums.forEach { safely { it.scan(tokens) } }
        _active.value = advertised.isNotEmpty() || scanned.isNotEmpty()
    }

    // A radio that is off, refused or misbehaving must not take the others down.
    private inline fun safely(block: () -> Unit) {
        try {
            block()
        } catch (_: RuntimeException) {
        }
    }
}
