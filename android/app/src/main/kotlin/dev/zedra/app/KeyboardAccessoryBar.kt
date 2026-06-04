package dev.zedra.app

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView

class KeyboardAccessoryBar(
    context: Context,
    private val sendKey: (String) -> Unit,
) : LinearLayout(context) {
    private data class KeySpec(
        val label: String,
        val key: String,
        val repeats: Boolean,
        val iconRes: Int? = null,
    )

    private val topBorderPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = 0x33FFFFFF
            strokeWidth = context.resources.displayMetrics.density.coerceAtLeast(1f)
        }

    private val keySpecs =
        listOf(
            KeySpec("Esc", "escape", false),
            KeySpec("Tab", "tab", false),
            KeySpec("←", "left", true, R.drawable.ic_key_arrow_left),
            KeySpec("↓", "down", true, R.drawable.ic_key_arrow_down),
            KeySpec("↑", "up", true, R.drawable.ic_key_arrow_up),
            KeySpec("→", "right", true, R.drawable.ic_key_arrow_right),
            KeySpec("⏎", "enter", false, R.drawable.ic_key_return),
        )

    private val repeatInitialDelayMs = 350L
    private val repeatIntervalMs = 60L
    private val handler = Handler(Looper.getMainLooper())
    private var repeatingKey: String? = null
    private var isDarkTheme = true

    private val repeatRunnable =
        object : Runnable {
            override fun run() {
                val key = repeatingKey ?: return
                sendKey(key)
                handler.postDelayed(this, repeatIntervalMs)
            }
        }

    init {
        orientation = HORIZONTAL
        setBaselineAligned(false)
        isFocusable = false
        isFocusableInTouchMode = false
        setWillNotDraw(false)
        applyTheme(isDark = true)
        visibility = GONE

        keySpecs.forEach { spec ->
            addView(makeButton(spec), LayoutParams(0, LayoutParams.MATCH_PARENT, 1f))
        }
    }

    override fun onDetachedFromWindow() {
        stopRepeating()
        super.onDetachedFromWindow()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawLine(0f, 0f, width.toFloat(), 0f, topBorderPaint)
    }

    fun stopRepeating() {
        repeatingKey = null
        handler.removeCallbacks(repeatRunnable)
    }

    fun applyTheme(isDark: Boolean) {
        isDarkTheme = isDark
        val foreground = if (isDark) 0xFFFFFFFF.toInt() else 0xFF1A1A1A.toInt()
        setBackgroundColor(if (isDark) 0xF50E0C0C.toInt() else 0xF5FFFFFF.toInt())
        topBorderPaint.color = if (isDark) 0x33FFFFFF else 0x22000000
        for (index in 0 until childCount) {
            when (val child = getChildAt(index)) {
                is ImageView -> child.imageTintList = android.content.res.ColorStateList.valueOf(foreground)
                is TextView -> child.setTextColor(foreground)
            }
        }
        invalidate()
    }

    private fun makeButton(spec: KeySpec): View {
        val foreground = if (isDarkTheme) 0xFFFFFFFF.toInt() else 0xFF1A1A1A.toInt()
        val view =
            if (spec.iconRes != null) {
                ImageView(context).apply {
                    setImageResource(spec.iconRes)
                    imageTintList = android.content.res.ColorStateList.valueOf(foreground)
                    scaleType = ImageView.ScaleType.CENTER
                    contentDescription = spec.label
                }
            } else {
                TextView(context).apply {
                    text = spec.label
                    textSize = 16f
                    gravity = Gravity.CENTER
                    setTextColor(foreground)
                }
            }

        view.isClickable = true
        view.isFocusable = false
        view.isFocusableInTouchMode = false
        view.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    if (spec.repeats) {
                        sendKey(spec.key)
                        startRepeating(spec.key)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (spec.repeats) {
                        stopRepeating()
                    } else {
                        sendKey(spec.key)
                    }
                    true
                }
                MotionEvent.ACTION_CANCEL,
                MotionEvent.ACTION_OUTSIDE -> {
                    stopRepeating()
                    true
                }
                else -> false
            }
        }
        return view
    }

    private fun startRepeating(key: String) {
        stopRepeating()
        repeatingKey = key
        handler.postDelayed(repeatRunnable, repeatInitialDelayMs)
    }
}
