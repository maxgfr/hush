package io.github.maxgfr.hush.core

import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import uniffi.hush_ffi.AppEvent
import uniffi.hush_ffi.AppListener

/**
 * Receives the engine's events on its thread and folds them into [state] on the main thread, so
 * the UI only ever sees whole, immutable states.
 */
class AppStore : AppListener {
    private val main = Handler(Looper.getMainLooper())
    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    override fun onEvent(event: AppEvent) {
        main.post { _state.value = Reducer.reduce(_state.value, event, System.currentTimeMillis()) }
    }

    /** A change the UI makes itself (main thread). */
    fun update(change: (UiState) -> UiState) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            _state.value = change(_state.value)
        } else {
            main.post { _state.value = change(_state.value) }
        }
    }
}
