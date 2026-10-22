package io.github.maxgfr.hush.nearby.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothServerSocket
import android.bluetooth.BluetoothStatusCodes
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import io.github.maxgfr.hush.nearby.Link
import io.github.maxgfr.hush.nearby.LinkHub
import io.github.maxgfr.hush.nearby.StreamLink
import io.github.maxgfr.hush.nearby.Tokens
import java.io.IOException
import uniffi.hush_ffi.Medium

/**
 * The advertising side of Bluetooth: one GATT service per advertised token (PSM, RX, TX), an
 * L2CAP listener whose PSM the service gives, and the advertisements themselves. A scanner that
 * opens L2CAP gets a stream link; one that subscribes to TX instead gets a packet link.
 */
@SuppressLint("MissingPermission") // BleMedium checks the Bluetooth permissions before any call.
class BlePeripheral(
    private val context: Context,
    private val manager: BluetoothManager,
    private val hub: LinkHub,
) {
    private val adapter = manager.adapter
    private val main = Handler(Looper.getMainLooper())
    private val advertiser = BleAdvertiser(adapter)
    private var server: BluetoothGattServer? = null
    private var l2cap: BluetoothServerSocket? = null
    @Volatile private var psm = 0
    private val services = HashMap<String, BluetoothGattService>()
    private val toAdd = ArrayDeque<BluetoothGattService>()
    private var adding = false
    private var tokens: List<ByteArray> = emptyList()
    private val links = HashMap<String, ServerLink>()
    private val mtus = HashMap<String, Int>()

    /** Serve and advertise exactly these tokens (empty: stop advertising). */
    @Synchronized
    fun update(tokens: List<ByteArray>) {
        this.tokens = tokens
        if (tokens.isEmpty()) {
            main.post { advertiser.update(emptyList()) }
            closeIfIdle()
            return
        }
        val s = server ?: manager.openGattServer(context, callback)?.also { server = it } ?: return
        listenL2cap()
        val wanted = tokens.map(Tokens::toHex).toSet()
        // A packet link may still write to an older token's service: those go once it is closed.
        if (links.isEmpty()) {
            for (gone in services.keys - wanted) services.remove(gone)?.let { svc ->
                if (!toAdd.remove(svc)) runCatching { s.removeService(svc) }
            }
        }
        for (token in tokens) {
            val hex = Tokens.toHex(token)
            if (hex in services) continue
            val svc = service(token)
            services[hex] = svc
            toAdd.addLast(svc)
        }
        addNext()
    }

    private fun service(token: ByteArray): BluetoothGattService =
        BluetoothGattService(Tokens.toUuid(token), BluetoothGattService.SERVICE_TYPE_PRIMARY).apply {
            addCharacteristic(
                BluetoothGattCharacteristic(
                    Gatt.PSM,
                    BluetoothGattCharacteristic.PROPERTY_READ,
                    BluetoothGattCharacteristic.PERMISSION_READ,
                ),
            )
            addCharacteristic(
                BluetoothGattCharacteristic(
                    Gatt.RX,
                    BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE,
                    BluetoothGattCharacteristic.PERMISSION_WRITE,
                ),
            )
            addCharacteristic(
                BluetoothGattCharacteristic(Gatt.TX, BluetoothGattCharacteristic.PROPERTY_NOTIFY, 0).apply {
                    addDescriptor(
                        BluetoothGattDescriptor(
                            Gatt.CCCD,
                            BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE,
                        ),
                    )
                },
            )
        }

    // The stack takes one service at a time; advertise once they are all in place.
    private fun addNext() {
        if (adding) return
        val s = server ?: return
        val next = toAdd.removeFirstOrNull()
        if (next == null) {
            val current = tokens
            main.post { advertiser.update(current) }
            return
        }
        adding = s.addService(next)
        if (!adding) addNext()
    }

    private fun listenL2cap() {
        if (l2cap != null) return
        val socket = try {
            adapter.listenUsingInsecureL2capChannel()
        } catch (ignored: IOException) {
            return
        }
        l2cap = socket
        psm = socket.psm
        Thread({
            while (true) {
                val s = try {
                    socket.accept()
                } catch (ignored: IOException) {
                    break
                }
                StreamLink(hub, hub.newId(), s.inputStream, s.outputStream, s).start(Medium.BLUETOOTH, null)
            }
        }, "hush-l2cap-accept").apply { isDaemon = true }.start()
    }

    @Synchronized
    private fun closeIfIdle() {
        if (tokens.isNotEmpty() || links.isNotEmpty()) return
        runCatching { l2cap?.close() }
        l2cap = null
        psm = 0
        runCatching { server?.close() }
        server = null
        services.clear()
        toAdd.clear()
        adding = false
    }

    /** The packet link of a central, opened when it subscribes to TX (or writes RX first). */
    @Synchronized
    private fun linkFor(device: BluetoothDevice, service: BluetoothGattService): ServerLink? {
        links[device.address]?.let { return it }
        val tx = service.getCharacteristic(Gatt.TX) ?: return null
        val link = ServerLink(hub.newId(), device, tx)
        links[device.address] = link
        hub.open(link.id, link, Gatt.packets(mtus[device.address] ?: Gatt.DEFAULT_MTU), Medium.BLUETOOTH, null)
        return link
    }

    @Synchronized
    private fun dropLink(device: BluetoothDevice): ServerLink? = links.remove(device.address)

    private val callback = object : BluetoothGattServerCallback() {
        override fun onServiceAdded(status: Int, service: BluetoothGattService) {
            synchronized(this@BlePeripheral) {
                adding = false
                addNext()
            }
        }

        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                synchronized(this@BlePeripheral) { mtus.remove(device.address) }
                dropLink(device)?.gone()
                closeIfIdle()
            }
        }

        override fun onMtuChanged(device: BluetoothDevice, mtu: Int) {
            synchronized(this@BlePeripheral) { mtus[device.address] = mtu }
        }

        override fun onCharacteristicReadRequest(
            device: BluetoothDevice,
            requestId: Int,
            offset: Int,
            characteristic: BluetoothGattCharacteristic,
        ) {
            val s = server ?: return
            if (characteristic.uuid != Gatt.PSM || psm == 0) {
                s.sendResponse(device, requestId, BluetoothGatt.GATT_READ_NOT_PERMITTED, offset, null)
                return
            }
            val value = Gatt.psmBytes(psm)
            s.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, value.copyOfRange(offset.coerceAtMost(2), 2))
        }

        override fun onCharacteristicWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            characteristic: BluetoothGattCharacteristic,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray?,
        ) {
            val ok = characteristic.uuid == Gatt.RX && value != null && !preparedWrite
            if (ok) linkFor(device, characteristic.service)?.let { hub.data(it.id, value!!) }
            if (responseNeeded) {
                val status = if (ok) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_WRITE_NOT_PERMITTED
                server?.sendResponse(device, requestId, status, offset, null)
            }
        }

        override fun onDescriptorReadRequest(
            device: BluetoothDevice,
            requestId: Int,
            offset: Int,
            descriptor: BluetoothGattDescriptor,
        ) {
            val on = synchronized(this@BlePeripheral) { links.containsKey(device.address) }
            val value = if (on) {
                BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            } else {
                BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
            }
            server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, value)
        }

        override fun onDescriptorWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            descriptor: BluetoothGattDescriptor,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray?,
        ) {
            val isTx = descriptor.uuid == Gatt.CCCD && descriptor.characteristic.uuid == Gatt.TX
            if (isTx && value != null) {
                if (value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)) {
                    linkFor(device, descriptor.characteristic.service)
                } else if (value.contentEquals(BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE)) {
                    dropLink(device)?.gone()
                }
            }
            if (responseNeeded) {
                val status = if (isTx) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_WRITE_NOT_PERMITTED
                server?.sendResponse(device, requestId, status, offset, null)
            }
        }

        override fun onNotificationSent(device: BluetoothDevice, status: Int) {
            synchronized(this@BlePeripheral) { links[device.address] }?.onSent()
        }
    }

    /** Packets to one central, as TX notifications, one in flight at a time. */
    private inner class ServerLink(
        val id: ULong,
        val device: BluetoothDevice,
        val tx: BluetoothGattCharacteristic,
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
            if (!notify(next)) {
                // The stack is busy: try again shortly.
                synchronized(this) {
                    inFlight = null
                    queue.addFirst(next)
                }
                main.postDelayed(this, RETRY_MS)
            }
        }

        override fun run() = pump()

        fun onSent() {
            val done = synchronized(this) { inFlight.also { inFlight = null } } ?: return
            hub.sent(id, done.size)
            pump()
        }

        private fun notify(value: ByteArray): Boolean {
            val s = server ?: return false
            return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                s.notifyCharacteristicChanged(device, tx, false, value) == BluetoothStatusCodes.SUCCESS
            } else {
                @Suppress("DEPRECATION")
                tx.value = value
                @Suppress("DEPRECATION")
                s.notifyCharacteristicChanged(device, tx, false)
            }
        }

        /** The engine closes it: drop the central. */
        override fun close() {
            dropLink(device)
            gone()
            runCatching { server?.cancelConnection(device) }
            closeIfIdle()
        }

        /** The central left or unsubscribed. */
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
        const val RETRY_MS = 15L
    }
}
