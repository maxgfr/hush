package io.github.maxgfr.hush.nearby.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import io.github.maxgfr.hush.nearby.Link
import io.github.maxgfr.hush.nearby.LinkHub
import io.github.maxgfr.hush.nearby.StreamLink
import io.github.maxgfr.hush.nearby.Tokens
import java.io.IOException
import java.util.UUID
import uniffi.hush_ffi.Medium

/**
 * The scanning side of Bluetooth: finds devices advertising a token we look for, reads the PSM
 * from their hush service and opens an L2CAP channel (a stream link). When L2CAP fails, the same
 * GATT connection carries packets instead: RX writes out, TX notifications in.
 */
@SuppressLint("MissingPermission") // BleMedium checks the Bluetooth permissions before any call.
class BleCentral(
    private val context: Context,
    private val adapter: BluetoothAdapter,
    private val hub: LinkHub,
) {
    private val main = Handler(Looper.getMainLooper())
    @Volatile private var wanted: Map<UUID, ByteArray> = emptyMap()
    private var scanning: ScanCallback? = null

    /** Look for exactly these tokens (empty: stop scanning). */
    @Synchronized
    fun update(tokens: List<ByteArray>) {
        val next = tokens.associateBy(Tokens::toUuid)
        if (next.keys == wanted.keys && (scanning != null) == next.isNotEmpty()) return
        wanted = next
        val scanner = adapter.bluetoothLeScanner
        scanning?.let { cb -> runCatching { scanner?.stopScan(cb) } }
        scanning = null
        if (next.isEmpty() || scanner == null) return
        val filters = next.keys.map { ScanFilter.Builder().setServiceUuid(ParcelUuid(it)).build() }
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .setCallbackType(ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
            .build()
        val cb = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) = found(result)
            override fun onBatchScanResults(results: MutableList<ScanResult>) = results.forEach(::found)
        }
        scanning = cb
        runCatching { scanner.startScan(filters, settings, cb) }.onFailure { scanning = null }
    }

    private fun found(result: ScanResult) {
        val uuids = result.scanRecord?.serviceUuids ?: return
        val token = uuids.firstNotNullOfOrNull { wanted[it.uuid] } ?: return
        val hex = Tokens.toHex(token)
        val endpoint = "ble:${result.device.address}"
        if (!hub.claim(hex, endpoint)) return
        Client(result.device, token, hex, endpoint).connect()
    }

    /** One GATT connection to an advertiser, until it carries a link (or fails). */
    private inner class Client(
        val device: BluetoothDevice,
        val token: ByteArray,
        val hex: String,
        val endpoint: String,
    ) : BluetoothGattCallback() {
        private var gatt: BluetoothGatt? = null
        private var service: BluetoothGattService? = null
        private var mtu = Gatt.DEFAULT_MTU
        private var opened = false
        private var failed = false
        private var packets: ClientLink? = null

        private val timeout = Runnable {
            synchronized(this) { if (!opened) fail() }
        }

        fun connect() {
            main.postDelayed(timeout, CONNECT_TIMEOUT_MS)
            gatt = device.connectGatt(context, false, this, BluetoothDevice.TRANSPORT_LE)
            if (gatt == null) synchronized(this) { fail() }
        }

        /** Gives the token back so it can be dialed again; nothing was opened. */
        private fun fail() {
            if (failed || opened) return
            failed = true
            main.removeCallbacks(timeout)
            hub.release(hex, endpoint)
            gatt?.let {
                runCatching { it.disconnect() }
                runCatching { it.close() }
            }
            gatt = null
        }

        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
            synchronized(this) {
                if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
                    if (!g.requestMtu(Gatt.WANTED_MTU)) g.discoverServices()
                    return
                }
                if (newState != BluetoothProfile.STATE_DISCONNECTED && status == BluetoothGatt.GATT_SUCCESS) return
                if (!opened) {
                    fail()
                    return
                }
                // An open stream lives on its own channel; a packet link dies with GATT.
                packets?.gone()
                runCatching { g.close() }
                gatt = null
            }
        }

        override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
            synchronized(this) {
                if (status == BluetoothGatt.GATT_SUCCESS) this.mtu = mtu
                if (!opened && !g.discoverServices()) fail()
            }
        }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            synchronized(this) {
                if (opened || failed) return
                val svc = g.getService(Tokens.toUuid(token))
                if (status != BluetoothGatt.GATT_SUCCESS || svc == null) return fail()
                service = svc
                val psm = svc.getCharacteristic(Gatt.PSM)
                if (psm == null || !g.readCharacteristic(psm)) usePackets(g)
            }
        }

        @Deprecated("Android 12 and older")
        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            @Suppress("DEPRECATION")
            onPsm(g, c, if (status == BluetoothGatt.GATT_SUCCESS) c.value else null)
        }

        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) {
            onPsm(g, c, if (status == BluetoothGatt.GATT_SUCCESS) value else null)
        }

        private fun onPsm(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray?) {
            if (c.uuid != Gatt.PSM) return
            val psm = Gatt.psmOf(value)
            if (psm == 0) {
                synchronized(this) { usePackets(g) }
                return
            }
            // connect() blocks: not on a Bluetooth callback thread.
            Thread({ openL2cap(g, psm) }, "hush-l2cap-dial").apply { isDaemon = true }.start()
        }

        private fun openL2cap(g: BluetoothGatt, psm: Int) {
            val socket = try {
                device.createInsecureL2capChannel(psm)
            } catch (ignored: IOException) {
                synchronized(this) { usePackets(g) }
                return
            }
            try {
                socket.connect()
            } catch (ignored: IOException) {
                runCatching { socket.close() }
                synchronized(this) { usePackets(g) }
                return
            }
            synchronized(this) {
                if (failed) {
                    runCatching { socket.close() }
                    return
                }
                opened = true
                main.removeCallbacks(timeout)
            }
            // GATT is kept until the stream ends: some stacks drop the link with the last client.
            StreamLink(hub, hub.newId(), socket.inputStream, socket.outputStream, socket) {
                runCatching { g.disconnect() }
                runCatching { g.close() }
            }.start(Medium.BLUETOOTH, token, endpoint)
        }

        private fun usePackets(g: BluetoothGatt) {
            if (opened || failed) return
            val tx = service?.getCharacteristic(Gatt.TX)
            val cccd = tx?.getDescriptor(Gatt.CCCD)
            if (tx == null || cccd == null || service?.getCharacteristic(Gatt.RX) == null) return fail()
            if (!g.setCharacteristicNotification(tx, true)) return fail()
            val value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            val ok = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                g.writeDescriptor(cccd, value) == BluetoothStatusCodes.SUCCESS
            } else {
                @Suppress("DEPRECATION")
                cccd.value = value
                @Suppress("DEPRECATION")
                g.writeDescriptor(cccd)
            }
            if (!ok) fail()
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) {
            val link = synchronized(this) {
                if (opened || failed || d.uuid != Gatt.CCCD) return
                if (status != BluetoothGatt.GATT_SUCCESS) return fail()
                val rx = service?.getCharacteristic(Gatt.RX) ?: return fail()
                opened = true
                main.removeCallbacks(timeout)
                ClientLink(hub.newId(), g, rx).also { packets = it }
            }
            hub.open(link.id, link, Gatt.packets(mtu), Medium.BLUETOOTH, token, endpoint)
        }

        @Deprecated("Android 12 and older")
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            @Suppress("DEPRECATION")
            val value = c.value ?: return
            onNotified(c, value.copyOf())
        }

        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) =
            onNotified(c, value)

        private fun onNotified(c: BluetoothGattCharacteristic, value: ByteArray) {
            if (c.uuid != Gatt.TX) return
            packets?.let { hub.data(it.id, value) }
        }

        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            if (c.uuid == Gatt.RX) packets?.onWritten()
        }
    }

    /** Packets to an advertiser, as RX writes without response, one in flight at a time. */
    private inner class ClientLink(
        val id: ULong,
        private val gatt: BluetoothGatt,
        private val rx: BluetoothGattCharacteristic,
    ) : Link, Runnable {
        private val queue = ArrayDeque<ByteArray>()
        private var inFlight: ByteArray? = null
        private var closed = false

        override fun send(data: ByteArray) {
            synchronized(this) {
                if (closed) return
                queue.addLast(data)
            }
            pump()
        }

        private fun pump() {
            val next = synchronized(this) {
                if (closed || inFlight != null) return
                queue.removeFirstOrNull()?.also { inFlight = it } ?: return
            }
            if (!write(next)) {
                synchronized(this) {
                    inFlight = null
                    queue.addFirst(next)
                }
                main.postDelayed(this, RETRY_MS)
            }
        }

        override fun run() = pump()

        private fun write(value: ByteArray): Boolean {
            val type = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
            return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                gatt.writeCharacteristic(rx, value, type) == BluetoothStatusCodes.SUCCESS
            } else {
                @Suppress("DEPRECATION")
                rx.writeType = type
                @Suppress("DEPRECATION")
                rx.value = value
                @Suppress("DEPRECATION")
                gatt.writeCharacteristic(rx)
            }
        }

        fun onWritten() {
            val done = synchronized(this) { inFlight.also { inFlight = null } } ?: return
            hub.sent(id, done.size)
            pump()
        }

        override fun close() {
            gone()
            runCatching { gatt.disconnect() }
        }

        fun gone() {
            synchronized(this) {
                if (closed) return
                closed = true
                queue.clear()
            }
            main.removeCallbacks(this)
            hub.closed(id)
        }
    }

    private companion object {
        const val CONNECT_TIMEOUT_MS = 20_000L
        const val RETRY_MS = 15L
    }
}
