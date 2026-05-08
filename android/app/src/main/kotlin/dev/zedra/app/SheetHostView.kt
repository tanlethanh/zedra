package dev.zedra.app

import android.content.Context
import android.util.Log
import android.view.MotionEvent
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.VelocityTracker
import android.view.ViewConfiguration

class SheetHostView(context: Context) : SurfaceView(context), SurfaceHolder.Callback {
    var expandsOnScrollEdge: Boolean = true
    private var velocityTracker: VelocityTracker? = null
    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop
    private var downY = 0f

    init {
        holder.addCallback(this)
        isFocusable = true
        isFocusableInTouchMode = true
    }

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

    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downY = event.y
                parent?.requestDisallowInterceptTouchEvent(true)
                velocityTracker = VelocityTracker.obtain()
                velocityTracker?.addMovement(event)
                forwardTouch(ACTION_DOWN, event)
            }
            MotionEvent.ACTION_MOVE -> {
                velocityTracker?.addMovement(event)
                val draggingDown = event.y - downY > touchSlop
                val handOffToSheet = expandsOnScrollEdge &&
                    draggingDown &&
                    MainActivity.nativeSheetContentIsAtTop()
                parent?.requestDisallowInterceptTouchEvent(!handOffToSheet)
                forwardTouch(ACTION_MOVE, event)
            }
            MotionEvent.ACTION_UP -> {
                velocityTracker?.addMovement(event)
                velocityTracker?.computeCurrentVelocity(1000)
                val velX = velocityTracker?.xVelocity ?: 0f
                val velY = velocityTracker?.yVelocity ?: 0f
                if (kotlin.math.abs(velX) > 150f || kotlin.math.abs(velY) > 150f) {
                    nativeSheetFlingEvent(velX, velY)
                }
                velocityTracker?.recycle()
                velocityTracker = null
                parent?.requestDisallowInterceptTouchEvent(false)
                forwardTouch(ACTION_UP, event)
            }
            MotionEvent.ACTION_CANCEL -> {
                velocityTracker?.recycle()
                velocityTracker = null
                parent?.requestDisallowInterceptTouchEvent(false)
                forwardTouch(ACTION_CANCEL, event)
            }
            else -> return super.onTouchEvent(event)
        }
        return true
    }

    private fun forwardTouch(action: Int, event: MotionEvent) {
        val pointerIndex = event.actionIndex.coerceAtMost(event.pointerCount - 1)
        nativeSheetTouchEvent(
            action,
            event.getX(pointerIndex),
            event.getY(pointerIndex),
            event.getPointerId(pointerIndex),
        )
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
