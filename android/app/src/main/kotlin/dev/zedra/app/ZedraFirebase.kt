package dev.zedra.app

import android.content.Context
import android.os.Bundle
import android.util.Log
import com.google.firebase.FirebaseApp
import com.google.firebase.analytics.FirebaseAnalytics
import com.google.firebase.crashlytics.FirebaseCrashlytics

object ZedraFirebase {
    private const val TAG = "ZedraFirebase"

    @Volatile private var didInitialize = false
    @Volatile private var analytics: FirebaseAnalytics? = null

    // Default-off: collection stays disabled until the user is confirmed opted-in
    // at runtime via setCollectionEnabled (telemetry::apply_persisted_optout).
    // Debug builds never collect.
    @Volatile private var collectionEnabled = false

    @JvmStatic
    fun initialize(context: Context) {
        if (didInitialize) return

        synchronized(this) {
            if (didInitialize) return

            val appContext = context.applicationContext
            val app = FirebaseApp.getApps(appContext).firstOrNull()
                ?: if (BuildConfig.DEBUG) {
                    null
                } else {
                    initializeFirebaseApp(appContext)
                }

            if (app != null) {
                analytics = FirebaseAnalytics.getInstance(appContext).also {
                    it.setAnalyticsCollectionEnabled(collectionEnabled)
                }
                crashlyticsOrNull(ignoreCollection = true)
                    ?.setCrashlyticsCollectionEnabled(collectionEnabled)
            }
            didInitialize = true
        }
    }

    @JvmStatic
    fun logEvent(name: String, keys: Array<String>, values: Array<String>) {
        if (!collectionEnabled) return
        val analytics = analytics ?: return
        val params = Bundle()
        val count = minOf(keys.size, values.size)
        for (index in 0 until count) {
            val key = keys[index]
            if (key.isNotEmpty()) {
                params.putString(key, values[index])
            }
        }
        analytics.logEvent(name, params)
    }

    @JvmStatic
    fun recordError(message: String, file: String, line: Int) {
        val crashlytics = crashlyticsOrNull() ?: return
        val fileName = file.ifBlank { "unknown" }
        crashlytics.recordException(IllegalStateException("[$fileName:$line] $message"))
    }

    @JvmStatic
    fun recordPanic(message: String, location: String) {
        val crashlytics = crashlyticsOrNull() ?: return
        val locationName = location.ifBlank { "unknown" }
        val fullMessage = "Rust panic at $locationName: $message"
        crashlytics.log(fullMessage)
        crashlytics.recordException(RuntimeException(fullMessage))
    }

    @JvmStatic
    fun setUserId(userId: String) {
        if (!collectionEnabled) return
        analytics?.setUserId(userId)
        crashlyticsOrNull()?.setUserId(userId)
    }

    @JvmStatic
    fun setCustomKey(key: String, value: String) {
        if (!collectionEnabled || key.isEmpty()) return
        crashlyticsOrNull()?.setCustomKey(key, value)
    }

    @JvmStatic
    fun setCollectionEnabled(enabled: Boolean) {
        collectionEnabled = enabled && !BuildConfig.DEBUG
        analytics?.setAnalyticsCollectionEnabled(collectionEnabled)
        crashlyticsOrNull(ignoreCollection = true)
            ?.setCrashlyticsCollectionEnabled(collectionEnabled)
    }

    private fun initializeFirebaseApp(context: Context): FirebaseApp? {
        return try {
            FirebaseApp.initializeApp(context)
        } catch (error: IllegalStateException) {
            Log.i(TAG, "Firebase config not available; telemetry disabled", error)
            null
        }
    }

    private fun crashlyticsOrNull(ignoreCollection: Boolean = false): FirebaseCrashlytics? {
        if (!didInitialize && !ignoreCollection) return null
        if (!collectionEnabled && !ignoreCollection) return null
        return try {
            FirebaseCrashlytics.getInstance()
        } catch (error: IllegalStateException) {
            null
        }
    }
}
