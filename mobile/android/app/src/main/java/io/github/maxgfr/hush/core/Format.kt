package io.github.maxgfr.hush.core

import java.util.Locale
import kotlin.math.ceil
import kotlin.math.roundToLong

/** Short labels for durations and sizes, as the web app writes them. */
object Format {
    fun french(): Boolean = Locale.getDefault().language == "fr"

    /** "59 min", "23 h", "6 d": coarse, for expiry countdowns. */
    fun duration(ms: Long, fr: Boolean = french()): String {
        val m = ceil(ms.coerceAtLeast(0) / 60_000.0).toLong()
        return when {
            m < 60 -> "$m min"
            m < 48 * 60 -> "${(m / 60.0).roundToLong()} h"
            else -> "${(m / 1440.0).roundToLong()} ${if (fr) "j" else "d"}"
        }
    }

    /** A message timer: "off", "30 s", "5 min", "1 h", "1 d", "1 wk". */
    fun ttl(seconds: Long, off: String, fr: Boolean = french()): String = when {
        seconds <= 0 -> off
        seconds < 60 -> "$seconds s"
        seconds < 3_600 -> "${seconds / 60} min"
        seconds < 86_400 -> "${seconds / 3_600} h"
        seconds < 604_800 -> "${seconds / 86_400} ${if (fr) "j" else "d"}"
        else -> if (fr) "${seconds / 604_800} sem." else "${seconds / 604_800} wk"
    }

    fun size(n: Long): String = when {
        n < 1024 -> "$n B"
        n < 1024L * 1024 -> String.format(Locale.ROOT, "%.1f KB", n / 1024.0)
        n < 1024L * 1024 * 1024 -> String.format(Locale.ROOT, "%.1f MB", n / (1024.0 * 1024))
        else -> String.format(Locale.ROOT, "%.2f GB", n / (1024.0 * 1024 * 1024))
    }
}

/** Finding a hush link in what other apps hand us. */
object LinkInput {
    private val URL = Regex("""https://maxgfr\.github\.io/hush/\S*#\S+""")

    /**
     * The link inside shared text ("Join me: https://…#2.s…"), or the text itself when it holds
     * no URL (a bare fragment, which the engine also reads). Null for nothing at all.
     */
    fun extract(text: String?): String? {
        val t = text?.trim().orEmpty()
        if (t.isEmpty()) return null
        return URL.find(t)?.value?.trimEnd('.', ',', ';', ')', '"', '\'') ?: t
    }

    /** Whether a VIEW intent's URI is one of ours. */
    fun isOurs(uri: String?): Boolean = uri != null && uri.startsWith(Settings.LINK_BASE) && '#' in uri
}
