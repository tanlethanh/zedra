package dev.zedra.app

import android.app.Activity
import android.content.res.Configuration
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.KeyEvent
import android.view.View
import android.widget.FrameLayout
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import dev.zed.gpui.GpuiRuntimeController
import dev.zed.gpui.GpuiSurfaceView

/**
 * Thin host Activity. Owns deeplink handling and the static surface for
 * Rust→Java callbacks (alerts, haptics, keyboard, native presentations).
 * Everything else — surface lifecycle, touch / IME / fling forwarding,
 * Choreographer-driven frame loop, app-active broadcasts — is owned by
 * `dev.zed.gpui.GpuiRuntimeController` shipped from the framework.
 */
class MainActivity : AppCompatActivity() {
    private lateinit var runtime: GpuiRuntimeController
    private lateinit var rootView: FrameLayout
    private lateinit var surfaceView: GpuiSurfaceView
    private lateinit var keyboardAccessoryBar: KeyboardAccessoryBar

    override fun onCreate(savedInstanceState: Bundle?) {
        installSplashScreen()
        super.onCreate(savedInstanceState)

        ZedraFirebase.initialize(this)
        bootstrap(this, APP_VERSION_VALUE, APP_BUILD_NUMBER_VALUE)

        runtime = GpuiRuntimeController(this)
        runtime.initialize()

        rootView = FrameLayout(this)
        surfaceView = runtime.attach(rootView)
        keyboardAccessoryBar = KeyboardAccessoryBar(this) { key ->
            nativeKeyboardAccessoryKey(key)
        }
        rootView.addView(
            keyboardAccessoryBar,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                (44 * resources.displayMetrics.density).toInt(),
                Gravity.BOTTOM,
            ),
        )
        installKeyboardAccessoryInsets()
        sSurfaceView = surfaceView
        sActivity = this
        NativePresentations.register(this, rootView)
        pendingNativeIsDark?.let { isDark ->
            keyboardAccessoryBar.applyTheme(isDark)
            NativePresentations.setDarkTheme(isDark)
            pendingNativeIsDark = null
        }

        setContentView(rootView)

        // Builds the GPUI App and registers the finish-launching callback.
        // Unlike iOS, the callback is fired by the framework on first surface
        // attach (see GpuiSurfaceView.nativeSurfaceCreated) — Android's
        // SurfaceView is async, and GPUI's first draw can only succeed once
        // the GPU surface is bound.
        zedraLaunchGpui()

        handleDeeplinkIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleDeeplinkIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        runtime.onResume()
        if (::surfaceView.isInitialized) {
            surfaceView.requestFocus()
        }
    }

    override fun onPause() {
        super.onPause()
        if (::keyboardAccessoryBar.isInitialized) {
            keyboardAccessoryBar.stopRepeating()
        }
        runtime.onPause()
    }

    override fun onStop() {
        super.onStop()
        runtime.onStop()
    }

    override fun onDestroy() {
        if (::keyboardAccessoryBar.isInitialized) {
            keyboardAccessoryBar.stopRepeating()
        }
        NativePresentations.unregister()
        sSurfaceView = null
        sActivity = null
        runtime.onDestroy()
        super.onDestroy()
    }

    private fun handleDeeplinkIntent(intent: Intent?) {
        if (intent == null || intent.action != Intent.ACTION_VIEW) return
        val uri = intent.data ?: return
        Log.d(TAG, "Deeplink received: $uri")
        nativeDeeplinkReceived(uri.toString())
    }

    // dispatchKeyEvent runs before the view hierarchy, so it intercepts KEYCODE_BACK before
    // GpuiSurfaceView.onKeyDown() can consume it. This covers hardware back buttons and MIUI's
    // gesture-nav implementation which sends KEYCODE_BACK as a key event (source=SOURCE_KEYBOARD).
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (event.keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_UP) {
            if (nativeSystemBackPressed()) {
                return true
            }
        }
        return super.dispatchKeyEvent(event)
    }

    private fun installKeyboardAccessoryInsets() {
        ViewCompat.setOnApplyWindowInsetsListener(rootView) { _, insets ->
            val imeBottom = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
            val params = keyboardAccessoryBar.layoutParams as FrameLayout.LayoutParams
            if (params.bottomMargin != imeBottom) {
                params.bottomMargin = imeBottom
                keyboardAccessoryBar.layoutParams = params
            }
            keyboardAccessoryBar.visibility = if (imeBottom > 0) View.VISIBLE else View.GONE
            surfaceView.setKeyboardAccessoryHeight(
                if (imeBottom > 0) keyboardAccessoryBar.layoutParams.height else 0,
            )
            if (imeBottom == 0) {
                keyboardAccessoryBar.stopRepeating()
            }
            insets
        }
        ViewCompat.requestApplyInsets(rootView)
    }

    companion object {
        private const val TAG = "MainActivity"

        // Pre-compute strings exposed via JNI. Calling kotlin.text.StringsKt
        // (`.trim()` etc.) from a native-thread JNI invocation can recursively
        // trip class-init paths and surface as StackOverflowError. Resolving
        // these at companion-init time keeps the JNI hot path a plain field
        // read.
        private val APP_VERSION_VALUE: String = (BuildConfig.VERSION_NAME ?: "").trim()
        private val APP_BUILD_NUMBER_VALUE: String = BuildConfig.VERSION_CODE.toString()

        init {
            System.loadLibrary("zedra")
        }

        private var sSurfaceView: GpuiSurfaceView? = null
        private var sActivity: Activity? = null
        // Set when Rust calls setNativeTheme before the activity / accessory bar exists.
        // Applied once onCreate finishes wiring views.
        private var pendingNativeIsDark: Boolean? = null

        // ===== Native (downstream) =====
        @JvmStatic external fun bootstrap(
            activity: Activity,
            appVersion: String,
            appBuildNumber: String,
        )

        @JvmStatic external fun zedraLaunchGpui()

        @JvmStatic external fun nativeDeeplinkReceived(url: String)

        @JvmStatic external fun nativeAlertResult(callbackId: Int, buttonIndex: Int)

        @JvmStatic external fun nativeAlertDismiss(callbackId: Int)

        @JvmStatic external fun nativeSelectionResult(callbackId: Int, buttonIndex: Int)

        @JvmStatic external fun nativeSelectionDismiss(callbackId: Int)

        @JvmStatic external fun nativeTextInputResult(callbackId: Int, value: String)

        @JvmStatic external fun nativeTextInputDismiss(callbackId: Int)

        @JvmStatic external fun nativeFloatingButtonPressed(callbackId: Int)

        @JvmStatic external fun nativeDictationPreviewDismiss(previewId: Int)

        @JvmStatic external fun nativeNotificationAction(callbackId: Int)

        @JvmStatic external fun nativeNotificationDismiss(callbackId: Int)

        @JvmStatic external fun nativeSheetContentIsAtTop(): Boolean

        @JvmStatic external fun nativeKeyboardAccessoryKey(key: String)

        @JvmStatic external fun nativeSystemBackPressed(): Boolean

        // ===== Rust → Java callbacks =====

        @JvmStatic
        fun showKeyboard() {
            sSurfaceView?.post { sSurfaceView?.requestKeyboard() }
        }

        @JvmStatic
        fun hideKeyboard() {
            sSurfaceView?.post { sSurfaceView?.dismissKeyboard() }
        }

        @JvmStatic
        fun launchQrScanner() {
            val activity = sActivity ?: return
            activity.runOnUiThread {
                activity.startActivity(Intent(activity, QRScannerActivity::class.java))
            }
        }

        @JvmStatic
        fun openUrl(url: String) {
            val activity = sActivity ?: return
            activity.runOnUiThread {
                activity.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
            }
        }

        /** Returns 1 for dark, 0 for light, -1 when unavailable. */
        @JvmStatic
        fun systemInDarkTheme(): Int {
            val activity = sActivity ?: return -1
            val nightMode =
                activity.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK
            return when (nightMode) {
                Configuration.UI_MODE_NIGHT_YES -> 1
                Configuration.UI_MODE_NIGHT_NO -> 0
                else -> -1
            }
        }

        @JvmStatic
        fun setNativeTheme(isDark: Boolean) {
            AppCompatDelegate.setDefaultNightMode(
                if (isDark) AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO
            )
            val activity = sActivity as? MainActivity
            if (activity == null) {
                pendingNativeIsDark = isDark
                return
            }
            activity.runOnUiThread {
                if (activity::keyboardAccessoryBar.isInitialized) {
                    activity.keyboardAccessoryBar.applyTheme(isDark)
                } else {
                    pendingNativeIsDark = isDark
                }
                NativePresentations.setDarkTheme(isDark)
            }
        }

        @JvmStatic
        fun showAlert(
            callbackId: Int,
            title: String?,
            message: String?,
            labels: Array<String>,
            styles: IntArray,
        ) {
            NativePresentations.showAlert(callbackId, title, message, labels, styles)
        }

        @JvmStatic
        fun showSelection(
            callbackId: Int,
            title: String?,
            message: String?,
            labels: Array<String>,
            styles: IntArray,
        ) {
            NativePresentations.showSelection(callbackId, title, message, labels, styles)
        }

        @JvmStatic
        fun showListPicker(
            callbackId: Int,
            title: String?,
            message: String?,
            labels: Array<String>,
            subtitles: Array<String?>,
            imageNames: Array<String?>,
        ) {
            NativePresentations.showListPicker(callbackId, title, message, labels, subtitles, imageNames)
        }

        @JvmStatic
        fun showTextInput(
            callbackId: Int,
            title: String?,
            placeholder: String?,
            initialValue: String?,
        ) {
            NativePresentations.showTextInput(callbackId, title, placeholder, initialValue)
        }

        @JvmStatic
        fun presentCustomSheet(
            detents: IntArray,
            initialDetent: Int,
            showsGrabber: Boolean,
            expandsOnScrollEdge: Boolean,
            modalInPresentation: Boolean,
            cornerRadius: Float,
        ) {
            NativePresentations.presentCustomSheet(
                detents,
                initialDetent,
                showsGrabber,
                expandsOnScrollEdge,
                modalInPresentation,
                cornerRadius,
            )
        }

        @JvmStatic
        fun updateNativeFloatingButton(
            id: Int,
            imageName: String,
            accessibilityLabel: String,
            x: Float,
            y: Float,
            width: Float,
            height: Float,
            iconSize: Float,
            iconWeight: Int,
        ) {
            NativePresentations.updateNativeFloatingButton(
                id,
                imageName,
                accessibilityLabel,
                x,
                y,
                width,
                height,
                iconSize,
                iconWeight,
            )
        }

        @JvmStatic
        fun hideNativeFloatingButton(id: Int) {
            NativePresentations.hideNativeFloatingButton(id)
        }

        @JvmStatic
        fun updateNativeDictationPreview(id: Int, text: String, bottomOffset: Float) {
            NativePresentations.updateNativeDictationPreview(id, text, bottomOffset)
        }

        @JvmStatic
        fun hideNativeDictationPreview(id: Int) {
            NativePresentations.hideNativeDictationPreview(id)
        }

        @JvmStatic
        fun showNativeNotification(
            id: Int,
            title: String,
            message: String,
            imageName: String,
            kind: Int,
            durationSecs: Float,
            autoClose: Boolean,
        ) {
            NativePresentations.showNativeNotification(
                id,
                title,
                message,
                imageName,
                kind,
                durationSecs,
                autoClose,
            )
        }

        @JvmStatic
        fun triggerHaptic(kind: Int) {
            val view = sSurfaceView ?: return
            view.post {
                val constant =
                    when (kind) {
                        0, 3, 5 -> HapticFeedbackConstants.KEYBOARD_TAP
                        1 -> HapticFeedbackConstants.VIRTUAL_KEY
                        2, 4 -> HapticFeedbackConstants.LONG_PRESS
                        6 ->
                            if (Build.VERSION.SDK_INT >= 30) {
                                HapticFeedbackConstants.CONFIRM
                            } else {
                                HapticFeedbackConstants.VIRTUAL_KEY
                            }
                        7 -> HapticFeedbackConstants.CONTEXT_CLICK
                        8 ->
                            if (Build.VERSION.SDK_INT >= 30) {
                                HapticFeedbackConstants.REJECT
                            } else {
                                HapticFeedbackConstants.LONG_PRESS
                            }
                        else -> return@post
                    }
                view.performHapticFeedback(constant)
            }
        }
    }
}
