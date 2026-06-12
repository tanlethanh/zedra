package dev.zedra.app

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.res.Configuration
import android.content.Intent
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaPlayer
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log
import org.json.JSONObject
import java.io.File
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.widget.FrameLayout
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.content.ContextCompat
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.credentials.CredentialManager
import androidx.credentials.CredentialManagerCallback
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetCredentialResponse
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import com.google.android.libraries.identity.googleid.GetSignInWithGoogleOption
import com.google.android.libraries.identity.googleid.GoogleIdTokenCredential
import com.google.firebase.messaging.FirebaseMessaging
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
    private var keyboardImeBottom = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        // Apply the persisted theme before super.onCreate so AppCompat picks up
        // the correct night mode as its initial state. If we set it later (e.g.
        // from a Rust JNI callback after the activity has attached), AppCompat
        // treats it as a mode change and recreates the Activity — which tears
        // down the SurfaceView and the GPUI text system, dropping our embedded
        // fonts on cold launch.
        AppCompatDelegate.setDefaultNightMode(
            if (readPersistedThemeIsDark(this)) AppCompatDelegate.MODE_NIGHT_YES
            else AppCompatDelegate.MODE_NIGHT_NO,
        )
        installSplashScreen()
        super.onCreate(savedInstanceState)

        ZedraFirebase.initialize(this)
        createDeltaNotificationChannel(this)
        bootstrap(this, APP_VERSION_VALUE, APP_BUILD_NUMBER_VALUE, DEVICE_NAME_VALUE)

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
        rootView.viewTreeObserver.addOnPreDrawListener {
            updateKeyboardAccessoryVisibility()
            true
        }
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
        nativeSetAppForeground(true)
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
        nativeSetAppForeground(false)
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
            keyboardImeBottom = imeBottom
            val params = keyboardAccessoryBar.layoutParams as FrameLayout.LayoutParams
            if (params.bottomMargin != imeBottom) {
                params.bottomMargin = imeBottom
                keyboardAccessoryBar.layoutParams = params
            }
            updateKeyboardAccessoryVisibility()
            if (imeBottom == 0) {
                keyboardAccessoryBar.stopRepeating()
            }
            insets
        }
        ViewCompat.requestApplyInsets(rootView)
    }

    private fun updateKeyboardAccessoryVisibility() {
        val visible = keyboardImeBottom > 0 && nativeKeyboardAccessoryVisible()
        keyboardAccessoryBar.visibility = if (visible) View.VISIBLE else View.GONE
        surfaceView.setKeyboardAccessoryHeight(
            if (visible) keyboardAccessoryBar.layoutParams.height else 0,
        )
        if (!visible) {
            keyboardAccessoryBar.stopRepeating()
        }
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
        private val DEVICE_NAME_VALUE: String = (Build.MODEL ?: "").trim()

        init {
            System.loadLibrary("zedra")
        }

        private var sSurfaceView: GpuiSurfaceView? = null
        private var sActivity: Activity? = null

        // Mirrors crates/zedra/src/settings.rs: `<filesDir>/zedra/settings.json`
        // with an optional `theme_preference` field of `"Dark"` or `"Light"`.
        // Default is dark to match `ThemePreference::default()` on the Rust side.
        private fun readPersistedThemeIsDark(activity: Activity): Boolean {
            val file = File(activity.filesDir, "zedra/settings.json")
            if (!file.exists()) return true
            return try {
                val pref = JSONObject(file.readText()).optString("theme_preference", "Dark")
                !pref.equals("Light", ignoreCase = true)
            } catch (e: Throwable) {
                Log.w(TAG, "settings.json parse failed; defaulting to dark", e)
                true
            }
        }
        // Set when Rust calls setNativeTheme before the activity / accessory bar exists.
        // Applied once onCreate finishes wiring views.
        private var pendingNativeIsDark: Boolean? = null

        private fun createDeltaNotificationChannel(context: android.content.Context) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
            val channel = NotificationChannel(
                ZedraMessagingService.CHANNEL_ID,
                "Zedra Notifications",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "Agent and workspace notifications from Delta"
            }
            val manager = context.getSystemService(NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(channel)
        }

        // ===== Native (downstream) =====
        @JvmStatic external fun bootstrap(
            activity: Activity,
            appVersion: String,
            appBuildNumber: String,
            deviceName: String,
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

        @JvmStatic external fun nativeKeyboardAccessoryVisible(): Boolean

        @JvmStatic external fun nativeSystemBackPressed(): Boolean

        @JvmStatic external fun nativeSetAppForeground(foreground: Boolean)

        @JvmStatic external fun nativeDeltaPushTokenResult(
            callbackId: Int,
            provider: String,
            token: String,
            environment: String,
        )

        @JvmStatic external fun nativeDeltaPushTokenError(callbackId: Int, message: String)

        @JvmStatic external fun nativeDeltaGoogleSignInResult(
            callbackId: Int,
            idToken: String,
            email: String,
        )

        @JvmStatic external fun nativeDeltaGoogleSignInError(callbackId: Int, message: String)

        // ===== Rust → Java callbacks =====

        @JvmStatic
        fun requestDeltaPushToken(callbackId: Int) {
            FirebaseMessaging.getInstance().token
                .addOnSuccessListener { token ->
                    nativeDeltaPushTokenResult(callbackId, "fcm", token, "")
                }
                .addOnFailureListener { error ->
                    nativeDeltaPushTokenError(
                        callbackId,
                        error.message ?: "FCM token fetch failed",
                    )
                }
        }

        @JvmStatic
        fun startDeltaGoogleSignIn(callbackId: Int) {
            val activity = sActivity ?: run {
                nativeDeltaGoogleSignInError(callbackId, "Activity not available")
                return
            }
            activity.runOnUiThread {
                val webClientId = activity.getString(R.string.google_web_client_id)
                val request = GetCredentialRequest.Builder()
                    .addCredentialOption(GetSignInWithGoogleOption.Builder(webClientId).build())
                    .build()
                CredentialManager.create(activity).getCredentialAsync(
                    activity,
                    request,
                    null,
                    ContextCompat.getMainExecutor(activity),
                    object : CredentialManagerCallback<GetCredentialResponse, GetCredentialException> {
                        override fun onResult(result: GetCredentialResponse) {
                            val credential = result.credential
                            if (credential is GoogleIdTokenCredential) {
                                nativeDeltaGoogleSignInResult(callbackId, credential.idToken, credential.id)
                            } else {
                                nativeDeltaGoogleSignInError(callbackId, "Unexpected credential type: ${credential.type}")
                            }
                        }
                        override fun onError(e: GetCredentialException) {
                            Log.e("Zedra", "Google sign-in failed", e)
                            val message = if (e is GetCredentialCancellationException) {
                                "Sign-in failed. Check your internet connection and try again."
                            } else {
                                e.message ?: "Google sign-in failed"
                            }
                            nativeDeltaGoogleSignInError(callbackId, message)
                        }
                    },
                )
            }
        }

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
            imageNames: Array<String?>,
        ) {
            NativePresentations.showSelection(callbackId, title, message, labels, styles, imageNames)
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

        private var sVibrator: Vibrator? = null

        private fun vibrator(): Vibrator? {
            sVibrator?.let { return it }
            val activity = sActivity ?: return null
            val vibrator =
                if (Build.VERSION.SDK_INT >= 31) {
                    (activity.getSystemService(Context.VIBRATOR_MANAGER_SERVICE) as? VibratorManager)
                        ?.defaultVibrator
                } else {
                    @Suppress("DEPRECATION")
                    activity.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
                }
            sVibrator = vibrator?.takeIf { it.hasVibrator() }
            return sVibrator
        }

        // performHapticFeedback is gated by the system touch-feedback setting,
        // which some OEMs (MIUI) disable by default — use Vibrator directly.
        // kind encoding matches HapticFeedback::to_i32() in platform_bridge.rs.
        @JvmStatic
        fun triggerHaptic(kind: Int) {
            val vibrator = vibrator() ?: return
            val (effect, fallbackMs) =
                when (kind) {
                    // ImpactLight, ImpactSoft, SelectionChanged
                    0, 3, 5 -> VibrationEffect.EFFECT_TICK to 10L
                    // ImpactMedium, ImpactRigid
                    1, 4 -> VibrationEffect.EFFECT_CLICK to 20L
                    // ImpactHeavy, NotificationWarning
                    2, 7 -> VibrationEffect.EFFECT_HEAVY_CLICK to 30L
                    // NotificationSuccess, NotificationError
                    6, 8 -> VibrationEffect.EFFECT_DOUBLE_CLICK to 30L
                    else -> return
                }
            when {
                Build.VERSION.SDK_INT >= 29 ->
                    vibrator.vibrate(VibrationEffect.createPredefined(effect))
                Build.VERSION.SDK_INT >= 26 ->
                    vibrator.vibrate(VibrationEffect.createOneShot(fallbackMs, VibrationEffect.DEFAULT_AMPLITUDE))
                else ->
                    @Suppress("DEPRECATION")
                    vibrator.vibrate(fallbackMs)
            }
        }

        // kind encoding matches SoundEffect::to_i32() in platform_bridge.rs.
        @JvmStatic
        fun playSound(kind: Int) {
            when (kind) {
                0 -> playBundledAudio("notification") // AgentNotification
            }
        }

        private fun playBundledAudio(name: String) {
            val activity = sActivity ?: return
            try {
                val context = activity.applicationContext
                val resId = context.resources.getIdentifier(name, "raw", context.packageName)
                if (resId == 0) {
                    Log.w(TAG, "playBundledAudio: missing raw resource $name")
                    return
                }
                val attrs =
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_NOTIFICATION)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                        .build()
                val audioManager =
                    context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
                val player =
                    MediaPlayer.create(
                        context,
                        resId,
                        attrs,
                        audioManager.generateAudioSessionId(),
                    ) ?: return
                player.setOnCompletionListener { it.release() }
                player.start()
            } catch (e: Exception) {
                Log.w(TAG, "playBundledAudio failed", e)
            }
        }
    }
}
