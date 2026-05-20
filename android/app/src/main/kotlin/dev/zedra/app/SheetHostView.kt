package dev.zedra.app

import android.content.Context
import android.util.Log
import android.view.MotionEvent
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.VelocityTracker
import android.view.ViewConfiguration
import com.google.android.material.bottomsheet.BottomSheetBehavior
import kotlin.math.abs

/**
 * Hosts the GPUI-rendered custom sheet content.
 *
 * Touch coordination mirrors the iOS sheet: a gesture belongs either to the
 * native sheet or to the embedded GPUI content, never both. We decide after
 * touch slop. Downward vertical drags at the content top are left for
 * BottomSheetBehavior; all other gestures temporarily disable sheet dragging
 * and are forwarded to GPUI.
 */
class SheetHostView(context: Context) : SurfaceView(context), SurfaceHolder.Callback {
    var bottomSheetBehavior: BottomSheetBehavior<*>? = null

    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop
    private var velocityTracker: VelocityTracker? = null
    private var downX = 0f
    private var downY = 0f
    private var pastSlop = false
    private var nativeGestureActive = false
    private var nativeMovesBeforeSheetClaim = 0

    init {
        holder.addCallback(this)
        isFocusable = true
        isFocusableInTouchMode = true
    }

    // BottomSheetBehavior consults this to route a downward drag: content scroll
    // when it can scroll up, sheet drag when it cannot. The analog of the iOS
    // `zedra_ios_sheet_content_is_at_top()` check.
    override fun canScrollVertically(direction: Int): Boolean =
        if (direction < 0) {
            !MainActivity.nativeSheetContentIsAtTop()
        } else {
            true
        }

    // --- Surface lifecycle --------------------------------------------------

    override fun surfaceCreated(holder: SurfaceHolder) {
        Log.d(TAG, "sheet surfaceCreated")
        nativeSheetSurfaceCreated(holder.surface)
        nativeSheetProcessSurfaceCommands()
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        Log.d(TAG, "sheet surfaceChanged: ${width}x$height")
        nativeSheetSurfaceChanged(format, width, height)
        nativeSheetProcessSurfaceCommands()
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        Log.d(TAG, "sheet surfaceDestroyed")
        nativeSheetSurfaceDestroyed()
        nativeSheetProcessSurfaceCommands()
    }

    // --- Touch --------------------------------------------------------------

    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.x
                downY = event.y
                pastSlop = false
                nativeMovesBeforeSheetClaim = 0
                nativeGestureActive = true
                protectContentGesture()
                velocityTracker = VelocityTracker.obtain()
                velocityTracker?.addMovement(event)
                nativeSheetTouchEvent(ACTION_DOWN, event.x, event.y, 0)
            }
            MotionEvent.ACTION_MOVE -> {
                velocityTracker?.addMovement(event)
                handleMove(event)
            }
            MotionEvent.ACTION_UP -> {
                velocityTracker?.addMovement(event)
                finishGesture(event, ACTION_UP)
            }
            MotionEvent.ACTION_CANCEL -> {
                finishGesture(event, ACTION_CANCEL)
            }
            else -> return super.onTouchEvent(event)
        }
        return true
    }

    private fun handleMove(event: MotionEvent) {
        if (!pastSlop) {
            if (abs(event.y - downY) < touchSlop && abs(event.x - downX) < touchSlop) {
                return
            }
            pastSlop = true
        }

        if (nativeGestureActive) {
            nativeSheetTouchEvent(ACTION_MOVE, event.x, event.y, 0)
            nativeMovesBeforeSheetClaim++
        }

        val dx = event.x - downX
        val dy = event.y - downY
        // Give GPUI the first real move before asking it whether the content
        // reached top; otherwise a fresh downward gesture can be claimed by the
        // sheet before the embedded scroll view has a chance to consume it.
        val sheetShouldOwn =
            nativeMovesBeforeSheetClaim > 1 &&
                dy > abs(dx) &&
                dy > 0f &&
                MainActivity.nativeSheetContentIsAtTop()

        if (sheetShouldOwn) {
            nativeSheetTouchEvent(ACTION_CANCEL, event.x, event.y, 0)
            nativeGestureActive = false
            releaseSheetGesture()
        }
    }

    private fun finishGesture(event: MotionEvent, terminalAction: Int) {
        velocityTracker?.computeCurrentVelocity(1000)
        val velX = velocityTracker?.xVelocity ?: 0f
        val velY = velocityTracker?.yVelocity ?: 0f

        if (nativeGestureActive && (abs(velX) > 150f || abs(velY) > 150f)) {
            nativeSheetFlingEvent(velX, velY)
        }

        if (nativeGestureActive) {
            nativeSheetTouchEvent(terminalAction, event.x, event.y, 0)
        }

        releaseSheetGesture()
        velocityTracker?.recycle()
        velocityTracker = null
        pastSlop = false
        nativeGestureActive = false
    }

    private fun protectContentGesture() {
        bottomSheetBehavior?.isDraggable = false
        parent?.requestDisallowInterceptTouchEvent(true)
    }

    private fun releaseSheetGesture() {
        parent?.requestDisallowInterceptTouchEvent(false)
        bottomSheetBehavior?.isDraggable = true
    }

    private external fun nativeSheetSurfaceCreated(surface: Surface)
    private external fun nativeSheetSurfaceChanged(format: Int, width: Int, height: Int)
    private external fun nativeSheetSurfaceDestroyed()
    private external fun nativeSheetTouchEvent(action: Int, x: Float, y: Float, pointerId: Int)
    private external fun nativeSheetFlingEvent(velocityX: Float, velocityY: Float)
    private external fun nativeSheetProcessSurfaceCommands()

    companion object {
        private const val TAG = "SheetHostView"
        private const val ACTION_DOWN = 0
        private const val ACTION_UP = 1
        private const val ACTION_MOVE = 2
        private const val ACTION_CANCEL = 3
    }
}
