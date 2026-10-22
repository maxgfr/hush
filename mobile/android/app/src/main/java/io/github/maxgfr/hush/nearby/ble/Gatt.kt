package io.github.maxgfr.hush.nearby.ble

import java.util.UUID
import uniffi.hush_ffi.LinkKind

/** The hush GATT service (PROTOCOL.md §8): its UUID is the token, its characteristics are fixed. */
object Gatt {
    /** Read: the L2CAP PSM to connect to, u16 big-endian. */
    val PSM: UUID = UUID.fromString("e261fba8-836b-459c-84cc-e15cd0d124b4")

    /** Write without response: packets from the scanner. */
    val RX: UUID = UUID.fromString("cb5f7ab3-a841-4a1b-9f80-5855197fd921")

    /** Notify: packets to the scanner. */
    val TX: UUID = UUID.fromString("66c5f983-f8d0-406d-afb7-3cd1b35e1712")

    /** Client Characteristic Configuration, which turns notifications on. */
    val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

    /** What we ask for; the radios settle on at most this. */
    const val WANTED_MTU = 517

    /** BLE's default ATT MTU, when none was negotiated. */
    const val DEFAULT_MTU = 23

    /**
     * Bytes in one write or notification: the ATT MTU minus the 3-byte ATT header, and never more
     * than 512, the largest attribute value.
     */
    fun packetSize(attMtu: Int): Int = (attMtu - 3).coerceIn(DEFAULT_MTU - 3, 512)

    fun packets(attMtu: Int): LinkKind = LinkKind.Packets(packetSize(attMtu).toUInt())

    fun psmBytes(psm: Int): ByteArray = byteArrayOf((psm ushr 8).toByte(), psm.toByte())

    /** The PSM read from the characteristic, or 0 when the value is not one. */
    fun psmOf(value: ByteArray?): Int =
        if (value == null || value.size != 2) 0 else ((value[0].toInt() and 0xff) shl 8) or (value[1].toInt() and 0xff)
}
