package io.github.maxgfr.hush

import io.github.maxgfr.hush.nearby.Tokens
import io.github.maxgfr.hush.nearby.ble.Gatt
import java.util.UUID
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import uniffi.hush_ffi.LinkKind

class TokensTest {
    private val token = ByteArray(16) { it.toByte() }

    @Test
    fun aTokenIsABigEndianUuid() {
        // PROTOCOL.md §8: the 16 bytes, in order, as a 128-bit UUID. iOS reads them the same way.
        assertEquals(UUID.fromString("00010203-0405-0607-0809-0a0b0c0d0e0f"), Tokens.toUuid(token))
        val high = ByteArray(16) { (0xf0 + it).toByte() }
        assertEquals("f0f1f2f3-f4f5-f6f7-f8f9-fafbfcfdfeff", Tokens.toUuid(high).toString())
        assertArrayEquals(high, Tokens.fromUuid(Tokens.toUuid(high)))
    }

    @Test
    fun anInstanceNameIsLowercaseHex() {
        assertEquals("000102030405060708090a0b0c0d0e0f", Tokens.toHex(token))
        assertArrayEquals(token, Tokens.fromHex("000102030405060708090A0B0C0D0E0F"))
        assertNull(Tokens.fromHex("0001"))
        assertNull(Tokens.fromHex("zz0102030405060708090a0b0c0d0e0f"))
        assertEquals(Tokens.toUuid(token).toString().replace("-", ""), Tokens.toHex(token))
    }

    @Test(expected = IllegalArgumentException::class)
    fun onlySixteenBytesMakeAToken() {
        Tokens.toUuid(ByteArray(8))
    }

    @Test
    fun gattPacketsAreTheMtuMinusTheHeader() {
        assertEquals(LinkKind.Packets(182u), Gatt.packets(185))
        assertEquals(LinkKind.Packets(20u), Gatt.packets(23))
        // Never more than the largest attribute value, never less than BLE's minimum.
        assertEquals(LinkKind.Packets(512u), Gatt.packets(517))
        assertEquals(LinkKind.Packets(20u), Gatt.packets(0))
    }

    @Test
    fun thePsmIsAU16BigEndian() {
        assertArrayEquals(byteArrayOf(0x00, 0x80.toByte()), Gatt.psmBytes(0x80))
        assertEquals(0x1234, Gatt.psmOf(Gatt.psmBytes(0x1234)))
        assertEquals(0, Gatt.psmOf(byteArrayOf(1)))
        assertEquals(0, Gatt.psmOf(null))
    }

    @Test
    fun theCharacteristicsAreTheProtocols() {
        assertEquals("e261fba8-836b-459c-84cc-e15cd0d124b4", Gatt.PSM.toString())
        assertEquals("cb5f7ab3-a841-4a1b-9f80-5855197fd921", Gatt.RX.toString())
        assertEquals("66c5f983-f8d0-406d-afb7-3cd1b35e1712", Gatt.TX.toString())
    }
}
