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
    private val sendKey: (String, Int) -> Unit,
    private val togglePanel: () -> Unit,
) : LinearLayout(context) {
    private sealed class Kind {
        data class Key(val name: String, val fixedMods: Int = 0) : Kind()
        object TogglePanel : Kind()
    }

    private data class KeySpec(
        val label: String,
        val kind: Kind,
        val repeats: Boolean,
        val iconRes: Int? = null,
    )

    private val keySpecs =
        listOf(
            KeySpec("Esc", Kind.Key("escape"), false),
            KeySpec("Tab", Kind.Key("tab"), false),
            KeySpec("←", Kind.Key("left"), true, R.drawable.ic_key_arrow_left),
            KeySpec("↓", Kind.Key("down"), true, R.drawable.ic_key_arrow_down),
            KeySpec("↑", Kind.Key("up"), true, R.drawable.ic_key_arrow_up),
            KeySpec("→", Kind.Key("right"), true, R.drawable.ic_key_arrow_right),
            KeySpec("⏎", Kind.Key("enter"), false, R.drawable.ic_key_return),
            KeySpec("•••", Kind.TogglePanel, false),
        )

    private val topBorderPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = 0x33FFFFFF
            strokeWidth = context.resources.displayMetrics.density.coerceAtLeast(1f)
        }

    private val repeatInitialDelayMs = 350L
    private val repeatIntervalMs = 60L
    private val handler = Handler(Looper.getMainLooper())
    private var repeatingKey: Pair<String, Int>? = null
    private var isDarkTheme = true
    private var panelOpen = false
    private val buttons = mutableListOf<Pair<View, KeySpec>>()

    private val repeatRunnable =
        object : Runnable {
            override fun run() {
                val target = repeatingKey ?: return
                sendKey(target.first, target.second)
                handler.postDelayed(this, repeatIntervalMs)
            }
        }

    init {
        orientation = HORIZONTAL
        setBaselineAligned(false)
        isFocusable = false
        isFocusableInTouchMode = false
        setWillNotDraw(false)
        visibility = GONE
        applyTheme(isDark = true)

        keySpecs.forEach { spec ->
            val view = makeButton(spec)
            buttons.add(view to spec)
            addView(view, LayoutParams(0, LayoutParams.MATCH_PARENT, 1f))
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

    /** Update the `•••` label to `✕` (or vice versa) and cancel any repeat. */
    fun setPanelOpen(open: Boolean) {
        if (panelOpen == open) return
        panelOpen = open
        if (open) stopRepeating()
        for ((view, spec) in buttons) {
            if (spec.kind is Kind.TogglePanel && view is TextView) {
                view.text = if (open) "✕" else "•••"
            }
        }
    }

    fun applyTheme(isDark: Boolean) {
        isDarkTheme = isDark
        val foreground = if (isDark) 0xFFFFFFFF.toInt() else 0xFF1A1A1A.toInt()
        setBackgroundColor(if (isDark) 0xF50E0C0C.toInt() else 0xF5FFFFFF.toInt())
        topBorderPaint.color = if (isDark) 0x33FFFFFF else 0x22000000
        for ((view, _) in buttons) {
            when (view) {
                is ImageView -> view.imageTintList = android.content.res.ColorStateList.valueOf(foreground)
                is TextView -> view.setTextColor(foreground)
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
                    if (spec.repeats && spec.kind is Kind.Key) {
                        sendKey(spec.kind.name, spec.kind.fixedMods)
                        startRepeating(spec.kind.name, spec.kind.fixedMods)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (spec.repeats) {
                        stopRepeating()
                    } else {
                        when (val kind = spec.kind) {
                            is Kind.Key -> sendKey(kind.name, kind.fixedMods)
                            Kind.TogglePanel -> togglePanel()
                        }
                    }
                    true
                }
                MotionEvent.ACTION_CANCEL,
                MotionEvent.ACTION_OUTSIDE,
                -> {
                    stopRepeating()
                    true
                }
                else -> false
            }
        }
        return view
    }

    private fun startRepeating(name: String, mods: Int) {
        stopRepeating()
        repeatingKey = name to mods
        handler.postDelayed(repeatRunnable, repeatInitialDelayMs)
    }
}
