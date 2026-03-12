package dev.zedra.app;

import androidx.appcompat.app.AppCompatActivity;
import androidx.core.splashscreen.SplashScreen;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.Choreographer;
import android.view.View;
import android.view.Window;
import android.widget.FrameLayout;
import android.widget.TextView;

public class MainActivity extends AppCompatActivity {
    private static final String TAG = "MainActivity";

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
        Log.d(TAG, "showKeyboard called from native");
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
     * Hide the soft keyboard (called from Rust via JNI)
     */
    public static void hideKeyboard() {
        Log.d(TAG, "hideKeyboard called from native");
        if (sSurfaceView != null) {
            sSurfaceView.post(() -> sSurfaceView.dismissKeyboard());
        }
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

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        SplashScreen.installSplashScreen(this);
        super.onCreate(savedInstanceState);
        Log.d(TAG, "onCreate");

        // Initialize Choreographer for frame callbacks
        choreographer = Choreographer.getInstance();

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
