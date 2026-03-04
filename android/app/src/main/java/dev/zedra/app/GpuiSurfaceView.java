package dev.zedra.app;

import android.content.Context;
import android.graphics.Rect;
import android.os.Build;
import android.text.InputType;
import android.util.AttributeSet;
import android.util.Log;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.VelocityTracker;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowInsetsCompat;

/**
 * Custom SurfaceView for GPUI rendering.
 *
 * This view manages the native surface lifecycle and forwards
 * input events to the Rust GPUI implementation.
 * Includes IME (soft keyboard) support for terminal input.
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

    // Velocity tracking for fling gestures
    private VelocityTracker velocityTracker = null;

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

        // Detect soft keyboard and system bar insets via WindowInsets
        ViewCompat.setOnApplyWindowInsetsListener(this, (v, insets) -> {
            // IME (keyboard) inset
            int imeHeight = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom;
            nativeKeyboardHeightChanged(imeHeight);

            // System bar insets (status bar top, navigation bar bottom)
            int systemTop = insets.getInsets(WindowInsetsCompat.Type.systemBars()).top;
            int systemBottom = insets.getInsets(WindowInsetsCompat.Type.systemBars()).bottom;
            nativeSystemInsetsChanged(systemTop, systemBottom);

            return insets;
        });

        Log.d(TAG, "GpuiSurfaceView initialized");
    }

    /**
     * Exclude the left edge from system gesture navigation so the app can
     * detect drawer swipe gestures.  Android limits exclusion rects to 200dp
     * vertically per edge, but we claim the full height and let the system
     * cap it.
     */
    @Override
    protected void onLayout(boolean changed, int left, int top, int right, int bottom) {
        super.onLayout(changed, left, top, right, bottom);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            int edgeWidth = (int) (60 * getResources().getDisplayMetrics().density); // 60dp
            java.util.List<Rect> exclusions = java.util.Collections.singletonList(
                new Rect(0, 0, edgeWidth, bottom - top)
            );
            setSystemGestureExclusionRects(exclusions);
            Log.d(TAG, "Set gesture exclusion rect: 0,0," + edgeWidth + "," + (bottom - top));
        }
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

    // IME (Soft Keyboard) Support

    private boolean keyboardRequested = false;

    @Override
    public boolean onCheckIsTextEditor() {
        return keyboardRequested;
    }

    /**
     * Request the soft keyboard to appear (call from Rust via JNI when a text input is focused)
     */
    public void requestKeyboard() {
        keyboardRequested = true;
        showSoftKeyboard();
    }

    /**
     * Dismiss the soft keyboard
     */
    public void dismissKeyboard() {
        keyboardRequested = false;
        hideSoftKeyboard();
    }

    // Track the current text content for proper IME handling
    private StringBuilder currentText = new StringBuilder();

    @Override
    public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT;
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI | EditorInfo.IME_ACTION_NONE;

        final GpuiSurfaceView view = this;

        return new BaseInputConnection(this, false) {
            @Override
            public boolean setComposingText(CharSequence text, int newCursorPosition) {
                // For composing text, we don't send anything yet - wait for commit
                Log.d(TAG, "setComposingText: " + text + " (ignored, waiting for commit)");
                return true;
            }

            @Override
            public boolean finishComposingText() {
                Log.d(TAG, "finishComposingText");
                return true;
            }

            @Override
            public boolean commitText(CharSequence text, int newCursorPosition) {
                Log.d(TAG, "commitText: " + text);
                // Only send the final committed text to native
                if (nativeHandle != 0 && text != null && text.length() > 0) {
                    nativeImeInput(nativeHandle, text.toString());
                }
                return true;
            }

            @Override
            public boolean deleteSurroundingText(int beforeLength, int afterLength) {
                Log.d(TAG, "deleteSurroundingText: before=" + beforeLength + ", after=" + afterLength);
                // Send backspace for deletion requests
                if (nativeHandle != 0) {
                    for (int i = 0; i < beforeLength; i++) {
                        nativeKeyEvent(nativeHandle, KEY_ACTION_DOWN, 67, 0); // KEYCODE_DEL
                    }
                }
                return true;
            }

            @Override
            public boolean sendKeyEvent(KeyEvent event) {
                if (event.getAction() == KeyEvent.ACTION_DOWN) {
                    Log.d(TAG, "sendKeyEvent: keyCode=" + event.getKeyCode());
                    nativeKeyEvent(nativeHandle, KEY_ACTION_DOWN,
                            event.getKeyCode(), event.getUnicodeChar());
                }
                return true;
            }
        };
    }

    /**
     * Show the soft keyboard
     */
    public void showSoftKeyboard() {
        requestFocus();
        InputMethodManager imm = (InputMethodManager) getContext()
                .getSystemService(Context.INPUT_METHOD_SERVICE);
        if (imm != null) {
            imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT);
        }
    }

    /**
     * Hide the soft keyboard
     */
    public void hideSoftKeyboard() {
        InputMethodManager imm = (InputMethodManager) getContext()
                .getSystemService(Context.INPUT_METHOD_SERVICE);
        if (imm != null) {
            imm.hideSoftInputFromWindow(getWindowToken(), 0);
        }
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
        Log.d(TAG, "onTouchEvent: action=" + event.getActionMasked() + ", nativeHandle=" + nativeHandle);
        if (nativeHandle == 0) {
            Log.w(TAG, "Touch event ignored - nativeHandle is 0");
            return super.onTouchEvent(event);
        }

        int action;
        switch (event.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                action = ACTION_DOWN;
                // Obtain velocity tracker for fling detection
                if (velocityTracker == null) {
                    velocityTracker = VelocityTracker.obtain();
                } else {
                    velocityTracker.clear();
                }
                velocityTracker.addMovement(event);
                break;
            case MotionEvent.ACTION_UP:
                action = ACTION_UP;
                // Compute fling velocity on touch release. Tap vs drag classification
                // is handled in Rust (platform.rs handle_touch), which clears the fling
                // if the touch was a tap. We always forward velocity here.
                if (velocityTracker != null) {
                    velocityTracker.addMovement(event);
                    velocityTracker.computeCurrentVelocity(1000); // pixels per second
                    float velX = velocityTracker.getXVelocity();
                    float velY = velocityTracker.getYVelocity();
                    if (Math.abs(velX) > 150 || Math.abs(velY) > 150) {
                        nativeFlingEvent(nativeHandle, velX, velY);
                    }
                    velocityTracker.recycle();
                    velocityTracker = null;
                }
                break;
            case MotionEvent.ACTION_MOVE:
                action = ACTION_MOVE;
                if (velocityTracker != null) {
                    velocityTracker.addMovement(event);
                }
                break;
            case MotionEvent.ACTION_CANCEL:
                action = ACTION_CANCEL;
                if (velocityTracker != null) {
                    velocityTracker.recycle();
                    velocityTracker = null;
                }
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

    private static native void nativeSurfaceCreated(long handle, Surface surface);
    private static native void nativeProcessSurfaceCommands();
    private static native void nativeSurfaceChanged(long handle, int format, int width, int height);
    private static native void nativeSurfaceDestroyed(long handle);
    private static native void nativeTouchEvent(long handle, int action, float x, float y, int pointerId);
    private static native void nativeKeyEvent(long handle, int action, int keyCode, int unicode);
    private static native void nativeImeInput(long handle, String text);
    private static native void nativeFlingEvent(long handle, float velocityX, float velocityY);
    private static native void nativeKeyboardHeightChanged(int height);
    private static native void nativeSystemInsetsChanged(int top, int bottom);
}
