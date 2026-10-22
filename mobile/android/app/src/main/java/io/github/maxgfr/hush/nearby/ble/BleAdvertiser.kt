package io.github.maxgfr.hush.nearby.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertisingSet
import android.bluetooth.le.AdvertisingSetCallback
import android.bluetooth.le.AdvertisingSetParameters
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import io.github.maxgfr.hush.nearby.Tokens

/**
 * Advertises each token as a 128-bit service UUID, nothing else (no device name, no power level).
 * One advertising set per token; when the controller runs out of sets, one set takes the tokens
 * in turn.
 */
@SuppressLint("MissingPermission") // BleMedium checks the Bluetooth permissions before any call.
class BleAdvertiser(private val adapter: BluetoothAdapter) {
    private val main = Handler(Looper.getMainLooper())
    private val sets = HashMap<String, SetCallback>()
    private var rotation: Rotation? = null
    private var tokens: List<ByteArray> = emptyList()
    // The controller refused one more set: take turns instead.
    private var limited = false

    /** Advertise exactly these tokens. Main thread only. */
    fun update(tokens: List<ByteArray>) {
        this.tokens = tokens
        val advertiser = adapter.bluetoothLeAdvertiser
        if (advertiser == null || tokens.isEmpty()) {
            stopAll()
            return
        }
        if (limited && tokens.size > 1) {
            sets.values.forEach { runCatching { advertiser.stopAdvertisingSet(it) } }
            sets.clear()
            val r = rotation ?: Rotation().also {
                rotation = it
                advertiser.startAdvertisingSet(parameters(), data(tokens[0]), null, null, null, it)
            }
            r.tokens = tokens
            return
        }
        rotation?.let { r ->
            runCatching { advertiser.stopAdvertisingSet(r) }
            main.removeCallbacks(r)
        }
        rotation = null
        val wanted = tokens.associateBy(Tokens::toHex)
        for (gone in sets.keys - wanted.keys) {
            sets.remove(gone)?.let { runCatching { advertiser.stopAdvertisingSet(it) } }
        }
        for ((hex, token) in wanted) {
            if (hex in sets) continue
            val cb = SetCallback(hex)
            sets[hex] = cb
            advertiser.startAdvertisingSet(parameters(), data(token), null, null, null, cb)
        }
    }

    private fun stopAll() {
        val advertiser = adapter.bluetoothLeAdvertiser
        sets.values.forEach { cb -> advertiser?.let { runCatching { it.stopAdvertisingSet(cb) } } }
        sets.clear()
        rotation?.let { r ->
            main.removeCallbacks(r)
            advertiser?.let { runCatching { it.stopAdvertisingSet(r) } }
        }
        rotation = null
    }

    private fun parameters(): AdvertisingSetParameters = AdvertisingSetParameters.Builder()
        // Legacy advertising: every phone can hear it. Connectable legacy sets must be scannable.
        .setLegacyMode(true)
        .setConnectable(true)
        .setScannable(true)
        .setInterval(AdvertisingSetParameters.INTERVAL_LOW)
        .setTxPowerLevel(AdvertisingSetParameters.TX_POWER_MEDIUM)
        .build()

    private fun data(token: ByteArray): AdvertiseData = AdvertiseData.Builder()
        .addServiceUuid(ParcelUuid(Tokens.toUuid(token)))
        .setIncludeDeviceName(false)
        .setIncludeTxPowerLevel(false)
        .build()

    private inner class SetCallback(val hex: String) : AdvertisingSetCallback() {
        override fun onAdvertisingSetStarted(set: AdvertisingSet?, txPower: Int, status: Int) {
            if (status == ADVERTISE_SUCCESS) return
            main.post {
                if (sets[hex] !== this) return@post
                sets.remove(hex)
                // With no set running at all, the controller has no room for us: nothing to rotate.
                if (status == AdvertiseCallback.ADVERTISE_FAILED_TOO_MANY_ADVERTISERS && sets.isNotEmpty()) {
                    limited = true
                    update(tokens)
                }
            }
        }
    }

    /** One set whose data changes every few seconds. */
    private inner class Rotation : AdvertisingSetCallback(), Runnable {
        var tokens: List<ByteArray> = emptyList()
        private var set: AdvertisingSet? = null
        private var index = 0

        override fun onAdvertisingSetStarted(set: AdvertisingSet?, txPower: Int, status: Int) {
            main.post {
                if (rotation !== this) return@post
                if (status != ADVERTISE_SUCCESS || set == null) {
                    rotation = null
                    return@post
                }
                this.set = set
                main.postDelayed(this, ROTATE_MS)
            }
        }

        override fun run() {
            val s = set ?: return
            if (rotation !== this || tokens.isEmpty()) return
            index = (index + 1) % tokens.size
            runCatching { s.setAdvertisingData(data(tokens[index])) }
            main.postDelayed(this, ROTATE_MS)
        }
    }

    private companion object {
        const val ROTATE_MS = 2_000L
    }
}
