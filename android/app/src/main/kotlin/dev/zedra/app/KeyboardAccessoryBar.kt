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

/**
 * Modifier bitmask matching Rust `key_encoding::Mods`
 * (shift = bit 0, alt = bit 1, ctrl = bit 2).
 */
private object AccessoryMods {
    const val NONE = 0
    const val SHIFT = 0b001
    const val ALT = 0b010
    const val CTRL = 0b100
}

class KeyboardAccessoryBar(
    context: Context,
    private val sendKey: (String, Int) -> Unit,
) : LinearLayout(context) {
    private sealed class Kind {
        data class Key(val name: String, val fixedMods: Int = 0) : Kind()
        data class Modifier(val bit: Int) : Kind()
        object ToggleDetail : Kind()
    }

    private data class KeySpec(
        val label: String,
        val kind: Kind,
        val repeats: Boolean,
        val iconRes: Int? = null,
    )

    private val primarySpecs =
        listOf(
            KeySpec("Esc", Kind.Key("escape"), false),
            KeySpec("Tab", Kind.Key("tab"), false),
            KeySpec("←", Kind.Key("left"), true, R.drawable.ic_key_arrow_left),
            KeySpec("↓", Kind.Key("down"), true, R.drawable.ic_key_arrow_down),
            KeySpec("↑", Kind.Key("up"), true, R.drawable.ic_key_arrow_up),
            KeySpec("→", Kind.Key("right"), true, R.drawable.ic_key_arrow_right),
            KeySpec("⏎", Kind.Key("enter"), false, R.drawable.ic_key_return),
            KeySpec("•••", Kind.ToggleDetail, false),
        )

    private val detailSpecs =
        listOf(
            KeySpec("Ctrl", Kind.Modifier(AccessoryMods.CTRL), false),
            KeySpec("Alt", Kind.Modifier(AccessoryMods.ALT), false),
            KeySpec("Shift", Kind.Modifier(AccessoryMods.SHIFT), false),
            KeySpec("⇧⇥", Kind.Key("tab", AccessoryMods.SHIFT), false),
            KeySpec("⌃C", Kind.Key("char:c", AccessoryMods.CTRL), false),
            KeySpec("⌃D", Kind.Key("char:d", AccessoryMods.CTRL), false),
            KeySpec("⌃R", Kind.Key("char:r", AccessoryMods.CTRL), false),
            KeySpec("Home", Kind.Key("home"), false),
            KeySpec("End", Kind.Key("end"), false),
            KeySpec("PgUp", Kind.Key("page_up"), true),
            KeySpec("PgDn", Kind.Key("page_down"), true),
        )

    private val topBorderPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = 0x33FFFFFF
            strokeWidth = context.resources.displayMetrics.density.coerceAtLeast(1f)
        }

    private val rowHeightPx = (44 * context.resources.displayMetrics.density).toInt()
    private val repeatInitialDelayMs = 350L
    private val repeatIntervalMs = 60L
    private val handler = Handler(Looper.getMainLooper())
    private var repeatingKey: Pair<String, Int>? = null
    private var isDarkTheme = true
    private var detailExpanded = false
    private var armedMods = 0
    private val primaryRow: LinearLayout
    private val detailRow: LinearLayout
    private val primaryButtons = mutableListOf<Pair<View, KeySpec>>()
    private val detailButtons = mutableListOf<Pair<View, KeySpec>>()

    /** Notified whenever the bar's measured height changes (collapse/expand). */
    var onHeightChanged: ((Int) -> Unit)? = null

    private val repeatRunnable =
        object : Runnable {
            override fun run() {
                val target = repeatingKey ?: return
                sendKey(target.first, target.second)
                handler.postDelayed(this, repeatIntervalMs)
            }
        }

    init {
        orientation = VERTICAL
        setBaselineAligned(false)
        isFocusable = false
        isFocusableInTouchMode = false
        setWillNotDraw(false)
        visibility = GONE

        primaryRow = buildRow(primarySpecs, primaryButtons)
        detailRow =
            buildRow(detailSpecs, detailButtons).apply {
                visibility = GONE
            }

        addView(primaryRow, LayoutParams(LayoutParams.MATCH_PARENT, rowHeightPx))
        addView(detailRow, LayoutParams(LayoutParams.MATCH_PARENT, rowHeightPx))

        applyTheme(isDark = true)
        refreshModifierHighlights()
    }

    /** Total height the bar wants, used by the host to size its layout slot. */
    val currentHeightPx: Int
        get() = if (detailExpanded) rowHeightPx * 2 else rowHeightPx

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
        setBackgroundColor(if (isDark) 0xF50E0C0C.toInt() else 0xF5FFFFFF.toInt())
        topBorderPaint.color = if (isDark) 0x33FFFFFF else 0x22000000
        val foreground = if (isDark) 0xFFFFFFFF.toInt() else 0xFF1A1A1A.toInt()
        for ((view, _) in primaryButtons + detailButtons) {
            when (view) {
                is ImageView -> view.imageTintList = android.content.res.ColorStateList.valueOf(foreground)
                is TextView -> view.setTextColor(foreground)
            }
        }
        invalidate()
        refreshModifierHighlights()
    }

    private fun buildRow(
        specs: List<KeySpec>,
        sink: MutableList<Pair<View, KeySpec>>,
    ): LinearLayout {
        val row =
            LinearLayout(context).apply {
                orientation = HORIZONTAL
                setBaselineAligned(false)
            }
        specs.forEach { spec ->
            val view = makeButton(spec)
            sink.add(view to spec)
            row.addView(view, LayoutParams(0, LayoutParams.MATCH_PARENT, 1f))
        }
        return row
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
                        val combined = armedMods or spec.kind.fixedMods
                        sendKey(spec.kind.name, combined)
                        if (armedMods != 0) {
                            armedMods = 0
                            refreshModifierHighlights()
                        }
                        startRepeating(spec.kind.name, combined)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (spec.repeats) {
                        stopRepeating()
                    } else {
                        handleSpec(spec)
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

    private fun handleSpec(spec: KeySpec) {
        when (val kind = spec.kind) {
            Kind.ToggleDetail -> setDetailExpanded(!detailExpanded)
            is Kind.Modifier -> {
                armedMods = armedMods xor kind.bit
                refreshModifierHighlights()
            }
            is Kind.Key -> {
                val combined = armedMods or kind.fixedMods
                sendKey(kind.name, combined)
                if (armedMods != 0) {
                    armedMods = 0
                    refreshModifierHighlights()
                }
            }
        }
    }

    private fun setDetailExpanded(expanded: Boolean) {
        if (detailExpanded == expanded) return
        detailExpanded = expanded
        detailRow.visibility = if (expanded) VISIBLE else GONE
        if (!expanded) {
            armedMods = 0
        }
        refreshModifierHighlights()
        layoutParams?.let { params ->
            if (params.height != currentHeightPx) {
                params.height = currentHeightPx
                layoutParams = params
            }
        }
        onHeightChanged?.invoke(currentHeightPx)
    }

    private fun refreshModifierHighlights() {
        val highlight = if (isDarkTheme) 0x33FFFFFF.toInt() else 0x22000000.toInt()
        for ((view, spec) in detailButtons) {
            view.setBackgroundColor(
                when (val kind = spec.kind) {
                    is Kind.Modifier -> if ((armedMods and kind.bit) != 0) highlight else 0x00000000
                    Kind.ToggleDetail -> if (detailExpanded) highlight else 0x00000000
                    is Kind.Key -> 0x00000000
                },
            )
        }
        for ((view, spec) in primaryButtons) {
            if (spec.kind is Kind.ToggleDetail) {
                view.setBackgroundColor(if (detailExpanded) highlight else 0x00000000)
            }
        }
    }

    private fun startRepeating(name: String, mods: Int) {
        stopRepeating()
        repeatingKey = name to mods
        handler.postDelayed(repeatRunnable, repeatInitialDelayMs)
    }
}

