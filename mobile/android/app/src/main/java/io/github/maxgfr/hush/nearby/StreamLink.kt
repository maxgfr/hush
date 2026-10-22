package io.github.maxgfr.hush.nearby

import java.io.Closeable
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.atomic.AtomicBoolean
import uniffi.hush_ffi.LinkKind
import uniffi.hush_ffi.Medium

/**
 * A byte stream (a TCP socket or a Bluetooth L2CAP channel). One thread reads, one writes, so
 * `send` never blocks the engine. The engine does the framing: bytes go through untouched.
 */
class StreamLink(
    private val hub: LinkHub,
    val id: ULong,
    private val input: InputStream,
    private val output: OutputStream,
    private val resource: Closeable,
    private val onClosed: () -> Unit = {},
) : Link {
    private val queue = LinkedBlockingQueue<ByteArray>()
    private val closed = AtomicBoolean(false)

    /** Registers the link with the engine, then starts moving bytes. */
    fun start(medium: Medium, token: ByteArray?, endpoint: String? = null) {
        hub.open(id, this, LinkKind.Stream, medium, token, endpoint)
        Thread(::readLoop, "hush-link-$id-in").apply { isDaemon = true }.start()
        Thread(::writeLoop, "hush-link-$id-out").apply { isDaemon = true }.start()
    }

    override fun send(data: ByteArray) {
        if (!closed.get()) queue.offer(data)
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        queue.offer(STOP)
        try {
            resource.close()
        } catch (ignored: IOException) {
        }
        hub.closed(id)
        onClosed()
    }

    private fun readLoop() {
        val buf = ByteArray(64 * 1024)
        try {
            while (!closed.get()) {
                val n = input.read(buf)
                if (n < 0) break
                if (n > 0) hub.data(id, buf.copyOf(n))
            }
        } catch (ignored: IOException) {
        }
        close()
    }

    private fun writeLoop() {
        val batch = ArrayList<ByteArray>()
        try {
            while (true) {
                batch.add(queue.take())
                // Whatever else is queued goes out in the same write.
                queue.drainTo(batch)
                var bytes = 0
                for (chunk in batch) {
                    if (chunk === STOP) return
                    output.write(chunk)
                    bytes += chunk.size
                }
                output.flush()
                batch.clear()
                hub.sent(id, bytes)
            }
        } catch (ignored: IOException) {
        } catch (ignored: InterruptedException) {
        } finally {
            close()
        }
    }

    private companion object {
        val STOP = ByteArray(0)
    }
}
