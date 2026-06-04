package dev.zedra.app;

import androidx.appcompat.app.AlertDialog;
import androidx.appcompat.app.AppCompatActivity;
import androidx.appcompat.app.AppCompatDelegate;
import androidx.core.app.ActivityCompat;
import androidx.core.content.ContextCompat;
import androidx.core.splashscreen.SplashScreen;

import android.Manifest;
import android.app.Activity;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.Choreographer;
import android.view.HapticFeedbackConstants;
import android.view.View;
import android.view.Window;
import android.widget.FrameLayout;
import android.widget.TextView;

import com.google.android.material.snackbar.Snackbar;
import com.google.firebase.messaging.FirebaseMessaging;

public class MainActivity extends AppCompatActivity {
    private static final String TAG = "MainActivity";

    /// Notification channel shared by Delta push messages and in-app banners.
    /// Must match ZedraMessagingService.CHANNEL_ID.
    static final String DELTA_NOTIFICATION_CHANNEL_ID = "zedra_delta";
    private static final int POST_NOTIFICATIONS_REQUEST_CODE = 7301;

    static {
        System.loadLibrary("zedra");
    }

    // GPUI native handle
    private long gpuiHandle = 0;
    private GpuiSurfaceView surfaceView;
    private Choreographer choreographer;
    private boolean isRunning = false;

    // Static reference for JNI keyboard callbacks
    private static GpuiSurfaceView sSurfaceView;

    // Static reference to current activity for launching intents from JNI
    private static Activity sActivity;

    /**
     * Show the soft keyboard (called from Rust via JNI)
     */
    public static void showKeyboard() {
        if (sSurfaceView != null) {
            sSurfaceView.post(() -> sSurfaceView.requestKeyboard());
        }
    }

    /**
     * Launch the QR scanner activity (called from Rust via JNI)
     */
    public static void launchQrScanner() {
        Log.d(TAG, "launchQrScanner called from native");
        if (sActivity != null) {
            sActivity.runOnUiThread(() -> {
                Intent intent = new Intent(sActivity, QRScannerActivity.class);
                sActivity.startActivity(intent);
            });
        }
    }

    /**
     * Open a URL in the system browser (called from Rust via JNI)
     */
    public static void openUrl(String url) {
        Log.d(TAG, "openUrl called from native: " + url);
        if (sActivity != null) {
            sActivity.runOnUiThread(() -> {
                Intent intent = new Intent(Intent.ACTION_VIEW, Uri.parse(url));
                sActivity.startActivity(intent);
            });
        }
    }

    /**
     * Get the user-facing app version (Android versionName).
     */
    public static String getAppVersion() {
        if (sActivity == null) {
            return "";
        }
        try {
            String packageName = sActivity.getPackageName();
            android.content.pm.PackageInfo info =
                sActivity.getPackageManager().getPackageInfo(packageName, 0);
            String versionName = info.versionName == null ? "" : info.versionName.trim();
            return versionName;
        } catch (Exception e) {
            Log.e(TAG, "Failed to read app version", e);
            return "";
        }
    }

    /**
     * Get the app build number (Android versionCode / longVersionCode).
     */
    public static String getAppBuildNumber() {
        if (sActivity == null) {
            return "";
        }
        try {
            String packageName = sActivity.getPackageName();
            android.content.pm.PackageInfo info =
                sActivity.getPackageManager().getPackageInfo(packageName, 0);
            long versionCode = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P
                ? info.getLongVersionCode()
                : info.versionCode;
            return String.valueOf(versionCode);
        } catch (Exception e) {
            Log.e(TAG, "Failed to read app build number", e);
            return "";
        }
    }

    /**
     * Show a native alert dialog (called from Rust via JNI)
     */
    public static void showAlert(
        int callbackId,
        String title,
        String message,
        String[] labels,
        int[] styles
    ) {
        Log.d(TAG, "showAlert called from native");
        if (sActivity == null) {
            return;
        }
        sActivity.runOnUiThread(() -> {
            String[] safeLabels = (labels != null && labels.length > 0)
                ? labels
                : new String[] {"OK"};
            int[] safeStyles = (styles != null && styles.length == safeLabels.length)
                ? styles
                : new int[safeLabels.length];

            if (safeLabels.length > 3) {
                Log.w(TAG, "showAlert supports up to 3 buttons on Android; truncating");
            }

            AlertDialog.Builder builder = new AlertDialog.Builder(sActivity);
            if (title != null && !title.isEmpty()) {
                builder.setTitle(title);
            }
            if (message != null && !message.isEmpty()) {
                builder.setMessage(message);
            }

            int cancelIndex = -1;
            for (int i = 0; i < safeStyles.length; i++) {
                if (safeStyles[i] == 1) {
                    cancelIndex = i;
                    break;
                }
            }
            final int fallbackIndex = cancelIndex >= 0 ? cancelIndex : Math.max(0, safeLabels.length - 1);
            builder.setOnCancelListener(dialog -> nativeAlertResult(callbackId, fallbackIndex));

            builder.setPositiveButton(safeLabels[0], (dialog, which) -> nativeAlertResult(callbackId, 0));
            if (safeLabels.length > 1) {
                builder.setNegativeButton(safeLabels[1], (dialog, which) -> nativeAlertResult(callbackId, 1));
            }
            if (safeLabels.length > 2) {
                builder.setNeutralButton(safeLabels[2], (dialog, which) -> nativeAlertResult(callbackId, 2));
            }

            builder.show();
        });
    }

    /**
     * Show a native dismissible selection sheet (called from Rust via JNI)
     */
    public static void showSelection(
        int callbackId,
        String title,
        String message,
        String[] labels,
        int[] styles
    ) {
        Log.d(TAG, "showSelection called from native");
        if (sActivity == null) {
            return;
        }
        sActivity.runOnUiThread(() -> {
            String[] safeLabels = (labels != null && labels.length > 0)
                ? labels
                : new String[] {"OK"};

            AlertDialog.Builder builder = new AlertDialog.Builder(sActivity);
            if (title != null && !title.isEmpty()) {
                builder.setTitle(title);
            }
            if (message != null && !message.isEmpty()) {
                builder.setMessage(message);
            }
            builder.setItems(safeLabels, (dialog, which) -> nativeSelectionResult(callbackId, which));
            builder.setOnCancelListener(dialog -> nativeSelectionDismiss(callbackId));

            AlertDialog dialog = builder.create();
            dialog.setCanceledOnTouchOutside(true);
            dialog.show();
        });
    }

    /**
     * Trigger a haptic feedback pattern (called from Rust via JNI).
     *
     * kind values match HapticFeedback::to_i32() in platform_bridge.rs:
     *   0=ImpactLight, 1=ImpactMedium, 2=ImpactHeavy, 3=ImpactSoft, 4=ImpactRigid,
     *   5=SelectionChanged, 6=NotificationSuccess, 7=NotificationWarning, 8=NotificationError
     */
    public static void triggerHaptic(int kind) {
        if (sSurfaceView == null) return;
        sSurfaceView.post(() -> {
            int constant;
            switch (kind) {
                case 0: // ImpactLight
                case 3: // ImpactSoft
                case 5: // SelectionChanged
                    constant = HapticFeedbackConstants.KEYBOARD_TAP;
                    break;
                case 1: // ImpactMedium
                    constant = HapticFeedbackConstants.VIRTUAL_KEY;
                    break;
                case 2: // ImpactHeavy
                case 4: // ImpactRigid
                    constant = HapticFeedbackConstants.LONG_PRESS;
                    break;
                case 6: // NotificationSuccess
                    constant = Build.VERSION.SDK_INT >= 30
                        ? HapticFeedbackConstants.CONFIRM
                        : HapticFeedbackConstants.VIRTUAL_KEY;
                    break;
                case 7: // NotificationWarning
                    constant = HapticFeedbackConstants.CONTEXT_CLICK;
                    break;
                case 8: // NotificationError
                    constant = Build.VERSION.SDK_INT >= 30
                        ? HapticFeedbackConstants.REJECT
                        : HapticFeedbackConstants.LONG_PRESS;
                    break;
                default:
                    return;
            }
            sSurfaceView.performHapticFeedback(constant);
        });
    }

    /**
     * Hide the soft keyboard (called from Rust via JNI)
     */
    public static void hideKeyboard() {
        if (sSurfaceView != null) {
            sSurfaceView.post(() -> sSurfaceView.dismissKeyboard());
        }
    }

    /**
     * Native device name for Delta node labels (called from Rust via JNI).
     */
    public static String getDeltaDeviceName() {
        String manufacturer = Build.MANUFACTURER == null ? "" : Build.MANUFACTURER.trim();
        String model = Build.MODEL == null ? "" : Build.MODEL.trim();
        if (model.isEmpty()) {
            return manufacturer;
        }
        if (!manufacturer.isEmpty() && !model.toLowerCase().startsWith(manufacturer.toLowerCase())) {
            return manufacturer + " " + model;
        }
        return model;
    }

    /**
     * Present a transient in-app notification banner (called from Rust via JNI).
     *
     * kind values match NativeNotificationKind: 0=Info, 1=Success, 2=Warning, 3=Error.
     * A tap reports nativeNotificationAction; any other dismissal reports
     * nativeNotificationDismiss, so the two are mutually exclusive.
     */
    public static void showNativeNotification(
        int callbackId,
        String title,
        String message,
        int kind,
        float durationSecs,
        boolean autoClose
    ) {
        if (sActivity == null || sSurfaceView == null) {
            return;
        }
        sActivity.runOnUiThread(() -> {
            StringBuilder text = new StringBuilder();
            if (title != null && !title.isEmpty()) {
                text.append(title);
            }
            if (message != null && !message.isEmpty()) {
                if (text.length() > 0) {
                    text.append('\n');
                }
                text.append(message);
            }

            Snackbar snackbar = Snackbar.make(sSurfaceView, text.toString(), Snackbar.LENGTH_INDEFINITE);
            if (autoClose) {
                snackbar.setDuration(Math.max(1000, Math.round(durationSecs * 1000f)));
            }
            snackbar.setBackgroundTint(notificationAccentColor(kind));

            View snackbarView = snackbar.getView();
            TextView textView = snackbarView.findViewById(
                com.google.android.material.R.id.snackbar_text);
            if (textView != null) {
                textView.setMaxLines(4);
                textView.setTextColor(0xFFFFFFFF);
            }

            // A tap fires the action; suppress the dismiss callback so the two paths
            // never both fire for one banner.
            final boolean[] actionFired = { false };
            snackbarView.setOnClickListener(view -> {
                if (actionFired[0]) {
                    return;
                }
                actionFired[0] = true;
                nativeNotificationAction(callbackId);
                snackbar.dismiss();
            });
            snackbar.addCallback(new Snackbar.Callback() {
                @Override
                public void onDismissed(Snackbar transientBar, int event) {
                    if (!actionFired[0]) {
                        nativeNotificationDismiss(callbackId);
                    }
                }
            });
            snackbar.show();
        });
    }

    private static int notificationAccentColor(int kind) {
        switch (kind) {
            case 1: // Success
                return 0xFF1E7A3D;
            case 2: // Warning
                return 0xFF8A6D00;
            case 3: // Error
                return 0xFF8B2E2E;
            default: // Info
                return 0xFF2C2C2E;
        }
    }

    /**
     * Request the FCM registration token for Delta push (called from Rust via JNI).
     *
     * Delivers the token via nativeDeltaPushTokenResult or nativeDeltaPushTokenError.
     */
    public static void requestDeltaPushToken(int callbackId) {
        if (sActivity == null) {
            nativeDeltaPushTokenError(callbackId, "Activity is not available");
            return;
        }
        sActivity.runOnUiThread(() -> {
            // POST_NOTIFICATIONS (API 33+) only gates whether notifications display;
            // the FCM token can be fetched regardless, so request it best-effort.
            maybeRequestNotificationPermission();
            try {
                FirebaseMessaging.getInstance().getToken().addOnCompleteListener(task -> {
                    if (!task.isSuccessful() || task.getResult() == null || task.getResult().isEmpty()) {
                        String reason = task.getException() != null
                            ? task.getException().getLocalizedMessage()
                            : null;
                        nativeDeltaPushTokenError(
                            callbackId,
                            reason != null ? reason : "Failed to obtain FCM token");
                        return;
                    }
                    nativeDeltaPushTokenResult(callbackId, "fcm", task.getResult(), null);
                });
            } catch (Throwable t) {
                Log.e(TAG, "requestDeltaPushToken failed", t);
                String reason = t.getLocalizedMessage();
                nativeDeltaPushTokenError(
                    callbackId,
                    reason != null ? reason : "Firebase is not configured");
            }
        });
    }

    private static void maybeRequestNotificationPermission() {
        if (sActivity == null || Build.VERSION.SDK_INT < 33) {
            return;
        }
        if (ContextCompat.checkSelfPermission(sActivity, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED) {
            ActivityCompat.requestPermissions(
                sActivity,
                new String[] { Manifest.permission.POST_NOTIFICATIONS },
                POST_NOTIFICATIONS_REQUEST_CODE);
        }
    }

    private void createDeltaNotificationChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) {
            return;
        }
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager == null) {
            return;
        }
        NotificationChannel channel = new NotificationChannel(
            DELTA_NOTIFICATION_CHANNEL_ID,
            "Delta notifications",
            NotificationManager.IMPORTANCE_HIGH);
        channel.setDescription("Workspace and agent updates delivered through Delta.");
        manager.createNotificationChannel(channel);
    }

    // Choreographer frame callback for command processing
    private final Choreographer.FrameCallback frameCallback = new Choreographer.FrameCallback() {
        @Override
        public void doFrame(long frameTimeNanos) {
            if (isRunning && gpuiHandle != 0) {
                // Process commands from Rust on main thread
                gpuiProcessCommands();
                
                // Schedule next frame
                choreographer.postFrameCallback(this);
            }
        }
    };

    // Original native methods (for testing)
    public static native void initRust();
    public static native String rustGreeting(String input);
    public static native void rustOnResume();
    public static native void rustOnPause();

    // GPUI native methods
    private static native void gpuiInitMainThread();
    private static native void gpuiProcessCommands();
    private static native void gpuiProcessCriticalCommands(); // Process Initialize immediately
    private static native long gpuiInit(Object activity);
    private static native void gpuiDestroy(long handle);
    private static native void gpuiResume(long handle);
    private static native void gpuiPause(long handle);
    private static native float getDisplayDensity(Object activity);
    private static native void nativeDeeplinkReceived(String url);
    private static native void nativeAlertResult(int callbackId, int buttonIndex);
    private static native void nativeSelectionResult(int callbackId, int buttonIndex);
    private static native void nativeSelectionDismiss(int callbackId);
    private static native void nativeDeltaPushTokenResult(
        int callbackId, String provider, String token, String environment);
    private static native void nativeDeltaPushTokenError(int callbackId, String message);
    private static native void nativeNotificationAction(int callbackId);
    private static native void nativeNotificationDismiss(int callbackId);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // Keep Android native UI (including IME helper text) aligned with Zedra's dark theme.
        AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_YES);
        SplashScreen.installSplashScreen(this);
        super.onCreate(savedInstanceState);
        Log.d(TAG, "onCreate");

        // Initialize Choreographer for frame callbacks
        choreographer = Choreographer.getInstance();

        // Register the Delta notification channel before any push/banner can fire.
        createDeltaNotificationChannel();

        // Set up edge-to-edge display
        Window window = getWindow();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false);
        } else {
            View decorView = window.getDecorView();
            decorView.setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
            );
        }

        // Initialize AndroidApp thread-local storage on main thread
        Log.d(TAG, "Initializing main thread AndroidApp");
        gpuiInitMainThread();

        // Initialize GPUI
        Log.d(TAG, "Initializing GPUI");
        gpuiHandle = gpuiInit(this);
        if (gpuiHandle == 0) {
            Log.e(TAG, "Failed to initialize GPUI");
            // Fall back to showing error message
            setContentView(R.layout.activity_main);
            TextView textView = findViewById(R.id.text_view);
            textView.setText("Failed to initialize GPUI");
            return;
        }
        Log.d(TAG, "GPUI initialized with handle: " + gpuiHandle);

        // Get display density before processing init commands so Rust has the
        // correct scale factor when creating the GPUI platform.
        float density = getDisplayDensity(this);
        Log.d(TAG, "Display density: " + density);

        // Process Initialize command immediately (don't wait for choreographer)
        Log.d(TAG, "Processing initialization commands");
        gpuiProcessCriticalCommands();

        // Create the GPUI surface view
        surfaceView = new GpuiSurfaceView(this);
        surfaceView.setNativeHandle(gpuiHandle);
        sSurfaceView = surfaceView; // Store for JNI keyboard callbacks
        sActivity = this; // Store for JNI intent launching

        // Set the surface view as the content view
        setContentView(surfaceView);

        // Request focus for input events
        surfaceView.requestFocus();

        // Check if we were launched via a deeplink intent
        handleDeeplinkIntent(getIntent());

        Log.d(TAG, "onCreate completed");
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        handleDeeplinkIntent(intent);
    }

    private void handleDeeplinkIntent(Intent intent) {
        if (intent == null) return;
        if (Intent.ACTION_VIEW.equals(intent.getAction())) {
            Uri uri = intent.getData();
            if (uri != null) {
                Log.d(TAG, "Deeplink received: " + uri.toString());
                nativeDeeplinkReceived(uri.toString());
            }
        }
    }

    @Override
    protected void onStart() {
        super.onStart();
        Log.d(TAG, "onStart");
    }

    @Override
    protected void onResume() {
        super.onResume();
        Log.d(TAG, "onResume");

        // Resume GPUI rendering
        if (gpuiHandle != 0) {
            gpuiResume(gpuiHandle);
        }

        // Request focus for input
        if (surfaceView != null) {
            surfaceView.requestFocus();
        }

        // Start frame callback loop for command processing
        isRunning = true;
        choreographer.postFrameCallback(frameCallback);

        // Keep original for compatibility
        rustOnResume();
    }

    @Override
    protected void onPause() {
        super.onPause();
        Log.d(TAG, "onPause");

        // Stop frame callback loop
        isRunning = false;

        // Pause GPUI rendering
        if (gpuiHandle != 0) {
            gpuiPause(gpuiHandle);
        }

        // Keep original for compatibility
        rustOnPause();
    }

    @Override
    protected void onStop() {
        super.onStop();
        Log.d(TAG, "onStop");
    }

    @Override
    protected void onDestroy() {
        Log.d(TAG, "onDestroy");

        sActivity = null;

        // Destroy GPUI
        if (gpuiHandle != 0) {
            gpuiDestroy(gpuiHandle);
            gpuiHandle = 0;
        }

        super.onDestroy();
    }
}
