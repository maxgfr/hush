package io.github.maxgfr.hush.core

import android.content.Context
import androidx.core.content.edit
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import uniffi.hush_ffi.AppConfig
import uniffi.hush_ffi.IceServerConfig

/** Preferences only: no key, message or contact is ever kept here. */
data class Settings(
    val name: String = "",
    /** Disappearing-message timer for what we send, in seconds (0 = off). */
    val ttl: Long = 0,
    val relays: List<String> = emptyList(),
    val trackers: List<String> = emptyList(),
    val stun: List<String> = emptyList(),
    val turnUrl: String = "",
    val turnUser: String = "",
    val turnCredential: String = "",
    val relayOnly: Boolean = false,
    val nearby: Boolean = true,
) {
    fun iceServers(): List<IceServerConfig> = buildList {
        if (stun.isNotEmpty()) add(IceServerConfig(stun, "", ""))
        if (turnUrl.isNotBlank()) add(IceServerConfig(listOf(turnUrl.trim()), turnUser, turnCredential))
    }

    fun config(outDir: String, dataDir: String): AppConfig = AppConfig(
        name = name,
        base = LINK_BASE,
        relays = relays,
        trackers = trackers,
        iceServers = iceServers(),
        relayOnly = relayOnly,
        outDir = outDir,
        dataDir = dataDir,
        local = nearby,
        ttl = ttl.toULong(),
    )

    companion object {
        const val LINK_BASE = "https://maxgfr.github.io/hush/"
        val TTL_SECONDS = listOf(0L, 30L, 300L, 3_600L, 86_400L, 604_800L)
        val LINK_MINUTES = listOf(10L, 60L, 1_440L, 10_080L)

        /** One URL per line (or space), keeping only the schemes each list takes. */
        fun lines(text: String, vararg schemes: String): List<String> =
            text.split(Regex("\\s+")).map(String::trim).filter { line ->
                line.isNotEmpty() && schemes.any { line.startsWith(it) }
            }
    }
}

/**
 * Settings in the app's private SharedPreferences. The TURN credential sits there too, in the
 * clear but readable only by this app: it is a relay password, not a secret of any conversation,
 * and the app is excluded from backups.
 */
class SettingsStore(context: Context, private val defaults: Settings) {
    private val prefs = context.getSharedPreferences("settings", Context.MODE_PRIVATE)
    private val _state = MutableStateFlow(load())
    val state: StateFlow<Settings> = _state

    private fun load(): Settings = Settings(
        name = prefs.getString("name", defaults.name) ?: "",
        ttl = prefs.getLong("ttl", defaults.ttl),
        relays = prefs.getString("relays", null)?.let { Settings.lines(it, "wss://") } ?: defaults.relays,
        trackers = prefs.getString("trackers", null)?.let { Settings.lines(it, "wss://") } ?: defaults.trackers,
        stun = prefs.getString("stun", null)?.let { Settings.lines(it, "stun:", "stuns:") } ?: defaults.stun,
        turnUrl = prefs.getString("turnUrl", "") ?: "",
        turnUser = prefs.getString("turnUser", "") ?: "",
        turnCredential = prefs.getString("turnCredential", "") ?: "",
        relayOnly = prefs.getBoolean("relayOnly", defaults.relayOnly),
        nearby = prefs.getBoolean("nearby", defaults.nearby),
    )

    fun update(change: (Settings) -> Settings): Settings {
        val s = change(_state.value)
        prefs.edit {
            putString("name", s.name)
            putLong("ttl", s.ttl)
            putString("relays", s.relays.joinToString("\n"))
            putString("trackers", s.trackers.joinToString("\n"))
            putString("stun", s.stun.joinToString("\n"))
            putString("turnUrl", s.turnUrl)
            putString("turnUser", s.turnUser)
            putString("turnCredential", s.turnCredential)
            putBoolean("relayOnly", s.relayOnly)
            putBoolean("nearby", s.nearby)
        }
        _state.value = s
        return s
    }
}
