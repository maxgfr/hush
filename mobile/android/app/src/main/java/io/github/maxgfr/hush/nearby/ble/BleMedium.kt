package io.github.maxgfr.hush.nearby.ble

import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import androidx.core.content.ContextCompat
import io.github.maxgfr.hush.nearby.LinkHub
import io.github.maxgfr.hush.nearby.NearbyMedium
import io.github.maxgfr.hush.nearby.NearbyPermissions

/**
 * Bluetooth LE, both roles. Does nothing while Bluetooth is off or its permissions are refused,
 * and calls [onRadioOn] when Bluetooth comes back so the transport can apply its sets again.
 */
class BleMedium(
    private val context: Context,
    private val hub: LinkHub,
    onRadioOn: () -> Unit,
) : NearbyMedium {
    private val manager: BluetoothManager? = context.getSystemService(BluetoothManager::class.java)
    @Volatile private var peripheral: BlePeripheral? = null
    @Volatile private var central: BleCentral? = null

    init {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context, intent: Intent) {
                val state = intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.ERROR)
                if (state == BluetoothAdapter.STATE_ON) onRadioOn()
                if (state == BluetoothAdapter.STATE_OFF) {
                    // Everything the radio held is gone; start afresh when it is back.
                    peripheral = null
                    central = null
                }
            }
        }
        ContextCompat.registerReceiver(
            context,
            receiver,
            IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
    }

    private fun usable(): Boolean =
        manager?.adapter?.isEnabled == true && NearbyPermissions.has(context, NearbyPermissions.bluetooth())

    override fun advertise(tokens: List<ByteArray>, tcpPorts: Map<String, Int>) {
        if (!usable()) return
        val m = manager ?: return
        val p = peripheral ?: BlePeripheral(context, m, hub).also { peripheral = it }
        p.update(tokens)
    }

    override fun scan(tokens: List<ByteArray>) {
        if (!usable()) return
        val adapter = manager?.adapter ?: return
        val c = central ?: BleCentral(context, adapter, hub).also { central = it }
        c.update(tokens)
    }

    override fun stop() {
        if (!usable()) return
        peripheral?.update(emptyList())
        central?.update(emptyList())
    }
}
