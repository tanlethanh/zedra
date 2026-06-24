package dev.zedra.app

import android.content.Context

/** No-op replacement used when the mobile no-telemetry build feature is enabled. */
object ZedraFirebase {
    @JvmStatic
    fun initialize(_context: Context) {}
}
