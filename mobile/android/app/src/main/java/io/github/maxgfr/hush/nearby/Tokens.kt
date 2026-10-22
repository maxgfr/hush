package io.github.maxgfr.hush.nearby

import java.nio.ByteBuffer
import java.util.UUID

/**
 * Discovery tokens (PROTOCOL.md §8) in the shapes the radios need: a Bluetooth service UUID (the
 * 16 bytes read as a big-endian 128-bit UUID) and a DNS-SD instance name (lowercase hex).
 */
object Tokens {
    const val SIZE = 16

    fun toUuid(token: ByteArray): UUID {
        require(token.size == SIZE) { "a token is $SIZE bytes" }
        val b = ByteBuffer.wrap(token)
        return UUID(b.long, b.long)
    }

    fun fromUuid(uuid: UUID): ByteArray =
        ByteBuffer.allocate(SIZE).putLong(uuid.mostSignificantBits).putLong(uuid.leastSignificantBits).array()

    fun toHex(token: ByteArray): String {
        val out = StringBuilder(token.size * 2)
        for (b in token) {
            val v = b.toInt() and 0xff
            out.append(HEX[v ushr 4]).append(HEX[v and 0x0f])
        }
        return out.toString()
    }

    /** A token from its instance name, or null for anything that is not 32 hex digits. */
    fun fromHex(name: String): ByteArray? {
        val s = name.trim().lowercase()
        if (s.length != SIZE * 2) return null
        val out = ByteArray(SIZE)
        for (i in 0 until SIZE) {
            val hi = HEX.indexOf(s[2 * i])
            val lo = HEX.indexOf(s[2 * i + 1])
            if (hi < 0 || lo < 0) return null
            out[i] = ((hi shl 4) or lo).toByte()
        }
        return out
    }

    private const val HEX = "0123456789abcdef"
}
