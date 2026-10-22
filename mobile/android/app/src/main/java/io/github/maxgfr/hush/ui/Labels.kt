package io.github.maxgfr.hush.ui

import androidx.annotation.StringRes
import io.github.maxgfr.hush.R
import uniffi.hush_ffi.Ending
import uniffi.hush_ffi.PeerPath

/** Which words the screens use for the engine's states. */
object Labels {
    @StringRes
    fun path(p: PeerPath): Int = when (p) {
        PeerPath.CONNECTING -> R.string.path_connecting
        PeerPath.BLUETOOTH -> R.string.path_bluetooth
        PeerPath.WIFI -> R.string.path_wifi
        PeerPath.DIRECT -> R.string.path_direct
        PeerPath.RELAYED -> R.string.path_relay
    }

    @StringRes
    fun ending(e: Ending): Int = when (e) {
        Ending.ENDED -> R.string.ended
        Ending.PEER_LEFT -> R.string.left
        Ending.TAKEN -> R.string.taken
        Ending.EXPIRED -> R.string.expired
        Ending.MISMATCH -> R.string.mismatch_closed
        Ending.FAILED -> R.string.failed
    }

    /** Endings that may mean someone else got hold of the link. */
    fun warns(e: Ending): Boolean = e == Ending.TAKEN || e == Ending.MISMATCH
}
