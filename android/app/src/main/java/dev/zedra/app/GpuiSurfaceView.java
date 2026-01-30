package dev.zedra.app;

import android.content.Context;
import android.util.AttributeSet;
import android.util.Log;
import android.view.MotionEvent;
import android.view.KeyEvent;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;

/**
 * Custom SurfaceView for GPUI rendering.
 *
 * This view manages the native surface lifecycle and forwards
 * input events to the Rust GPUI implementation.
 */
public class GpuiSurfaceView extends SurfaceView implements SurfaceHolder.Callback {
    private static final String TAG = "GpuiSurfaceView";

    private long nativeHandle = 0;
    private boolean surfaceCreated = false;

    // Touch action constants matching Android MotionEvent
    private static final int ACTION_DOWN = 0;
    private static final int ACTION_UP = 1;
    private static final int ACTION_MOVE = 2;
    private static final int ACTION_CANCEL = 3;

    // Key action constants
    private static final int KEY_ACTION_DOWN = 0;
    private static final int KEY_ACTION_UP = 1;

    public GpuiSurfaceView(Context context) {
        super(context);
        init();
    }

    public GpuiSurfaceView(Context context, AttributeSet attrs) {
        super(context, attrs);
        init();
    }

    public GpuiSurfaceView(Context context, AttributeSet attrs, int defStyleAttr) {
        super(context, attrs, defStyleAttr);
        init();
    }

    private void init() {
        Log.d(TAG, "Initializing GpuiSurfaceView");

        // Set up the surface holder callback
        SurfaceHolder holder = getHolder();
        holder.addCallback(this);

        // Enable focus for keyboard events
        setFocusable(true);
        setFocusableInTouchMode(true);

        Log.d(TAG, "GpuiSurfaceView initialized");
    }

    /**
     * Set the native handle from MainActivity
     */
    public void setNativeHandle(long handle) {
        Log.d(TAG, "Setting native handle: " + handle);
        this.nativeHandle = handle;
    }

    /**
     * Check if surface is ready for rendering
     */
    public boolean isSurfaceCreated() {
        return surfaceCreated;
    }

    // SurfaceHolder.Callback implementation

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        Log.d(TAG, "surfaceCreated");
        surfaceCreated = true;

        if (nativeHandle != 0) {
            Surface surface = holder.getSurface();
            nativeSurfaceCreated(nativeHandle, surface);
            // Process surface creation immediately (don't wait for choreographer)
            nativeProcessSurfaceCommands();
        } else {
            Log.w(TAG, "surfaceCreated called but nativeHandle is 0");
        }
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        Log.d(TAG, String.format("surfaceChanged: %dx%d, format: %d", width, height, format));

        if (nativeHandle != 0) {
            nativeSurfaceChanged(nativeHandle, format, width, height);
            // Process surface change immediately
            nativeProcessSurfaceCommands();
        } else {
            Log.w(TAG, "surfaceChanged called but nativeHandle is 0");
        }
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        Log.d(TAG, "surfaceDestroyed");
        surfaceCreated = false;

        if (nativeHandle != 0) {
            nativeSurfaceDestroyed(nativeHandle);
        } else {
            Log.w(TAG, "surfaceDestroyed called but nativeHandle is 0");
        }
    }

    // Input event handling

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        if (nativeHandle == 0) {
            return super.onTouchEvent(event);
        }

        int action;
        switch (event.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                action = ACTION_DOWN;
                break;
            case MotionEvent.ACTION_UP:
                action = ACTION_UP;
                break;
            case MotionEvent.ACTION_MOVE:
                action = ACTION_MOVE;
                break;
            case MotionEvent.ACTION_CANCEL:
                action = ACTION_CANCEL;
                break;
            default:
                return super.onTouchEvent(event);
        }

        // Get pointer information
        int pointerIndex = event.getActionIndex();
        int pointerId = event.getPointerId(pointerIndex);
        float x = event.getX(pointerIndex);
        float y = event.getY(pointerIndex);

        // Forward to native
        nativeTouchEvent(nativeHandle, action, x, y, pointerId);

        return true;
    }

    @Override
    public boolean onKeyDown(int keyCode, KeyEvent event) {
        if (nativeHandle == 0) {
            return super.onKeyDown(keyCode, event);
        }

        int unicode = event.getUnicodeChar();
        nativeKeyEvent(nativeHandle, KEY_ACTION_DOWN, keyCode, unicode);

        return true;
    }

    @Override
    public boolean onKeyUp(int keyCode, KeyEvent event) {
        if (nativeHandle == 0) {
            return super.onKeyUp(keyCode, event);
        }

        int unicode = event.getUnicodeChar();
        nativeKeyEvent(nativeHandle, KEY_ACTION_UP, keyCode, unicode);

        return true;
    }

    // Native method declarations

    /**
     * Called when the native surface is created
     *
     * @param handle The native platform handle
     * @param surface The Android Surface object
     */
    private static native void nativeSurfaceCreated(long handle, Surface surface);

    /**
     * Process surface commands immediately (don't wait for choreographer)
     */
    private static native void nativeProcessSurfaceCommands();

    /**
     * Called when the native surface changes size or format
     *
     * @param handle The native platform handle
     * @param format The surface format
     * @param width The new width
     * @param height The new height
     */
    private static native void nativeSurfaceChanged(long handle, int format, int width, int height);

    /**
     * Called when the native surface is destroyed
     *
     * @param handle The native platform handle
     */
    private static native void nativeSurfaceDestroyed(long handle);

    /**
     * Forward touch event to native code
     *
     * @param handle The native platform handle
     * @param action The touch action (DOWN, UP, MOVE, CANCEL)
     * @param x The X coordinate
     * @param y The Y coordinate
     * @param pointerId The pointer ID for multi-touch
     */
    private static native void nativeTouchEvent(long handle, int action, float x, float y, int pointerId);

    /**
     * Forward key event to native code
     *
     * @param handle The native platform handle
     * @param action The key action (DOWN or UP)
     * @param keyCode The Android KeyCode
     * @param unicode The unicode character (0 if none)
     */
    private static native void nativeKeyEvent(long handle, int action, int keyCode, int unicode);
}
