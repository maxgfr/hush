package io.github.maxgfr.hush

import android.app.Application
import android.content.Context
import io.github.maxgfr.hush.core.Engine

class HushApplication : Application() {
    /** Started on first use, so tests that build their own engines do not start this one. */
    val engine: Engine by lazy { Engine(this) }
}

val Context.engine: Engine get() = (applicationContext as HushApplication).engine
