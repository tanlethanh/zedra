package dev.zedra.app

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView

/**
 * Host OS reported by `MainActivity.nativeActiveHostOs`. Matches Rust
 * `key_encoding::HostOs`. `Unknown` is treated as macOS per product direction.
 */
enum class HostOs(val displayName: String) {
    MacOs("macOS"),
    Linux("Linux"),
    Windows("Windows"),
    ;

    companion object {
        fun fromU8(value: Int): HostOs =
            when (value) {
                2 -> Linux
                3 -> Windows
                else -> MacOs
            }
    }
}

/**
 * Desktop-only key panel that replaces the system IME while a terminal or
 * agent session has focus. Visual chrome (background, foreground, border,
 * font size, row height) mirrors `KeyboardAccessoryBar` so the panel reads
 * as a natural extension of the bar above it. The mock drives only the key
 * set and their position.
 *
 *   Row 1: shift  Ctrl  Cmd   Home  End  PgUp  PgD   ⌫
 *   Row 2: ~  @  $  *  ^  %  =  `
 *   Row 3: <  >  (  )  {  }  [  ]
 *   The lower ~40% of the panel is intentionally left blank for future
 *   additions (clipboard, snippets, agent macros).
 */
class FullKeyboardPanel(
    context: Context,
    private val hostOs: HostOs,
    private val sendKey: (String, Int) -> Unit,
) : FrameLayout(context) {
    private object AccessoryMods {
        const val SHIFT = 0b0001
        const val ALT = 0b0010
        const val CTRL = 0b0100

        // Cmd / Super — tracked for UI armed state; legacy terminal encoder
        // drops the bit because no PTY byte representation exists. Reserved
        // for routing via a future host-side RPC.
        const val CMD = 0b1000
    }

    private sealed class Kind {
        data class Dispatch(val name: String, val fixedMods: Int = 0) : Kind()
        data class Modifier(val bit: Int) : Kind()
    }

    private data class Key(
        val label: String,
        val kind: Kind,
        val repeats: Boolean = false,
    )

    private val modNavRow =
        listOf(
            Key("Shift", Kind.Modifier(AccessoryMods.SHIFT)),
            Key("Ctrl", Kind.Modifier(AccessoryMods.CTRL)),
            Key("Cmd", Kind.Modifier(AccessoryMods.CMD)),
            Key("Home", Kind.Dispatch("home")),
            Key("End", Kind.Dispatch("end")),
            Key("PgUp", Kind.Dispatch("page_up"), repeats = true),
            Key("PgD", Kind.Dispatch("page_down"), repeats = true),
            Key("⌫", Kind.Dispatch("backspace"), repeats = true),
        )

    private val symbolRow1 =
        listOf(
            Key("~", Kind.Dispatch("char:~")),
            Key("@", Kind.Dispatch("char:@")),
            Key("$", Kind.Dispatch("char:$")),
            Key("*", Kind.Dispatch("char:*")),
            Key("^", Kind.Dispatch("char:^")),
            Key("%", Kind.Dispatch("char:%")),
            Key("=", Kind.Dispatch("char:=")),
            Key("`", Kind.Dispatch("char:`")),
        )

    private val symbolRow2 =
        listOf(
            Key("<", Kind.Dispatch("char:<")),
            Key(">", Kind.Dispatch("char:>")),
            Key("(", Kind.Dispatch("char:(")),
            Key(")", Kind.Dispatch("char:)")),
            Key("{", Kind.Dispatch("char:{")),
            Key("}", Kind.Dispatch("char:}")),
            Key("[", Kind.Dispatch("char:[")),
            Key("]", Kind.Dispatch("char:]")),
        )

    private val density = context.resources.displayMetrics.density
    private val rowHeightPx = (44 * density).toInt()
    private val repeatInitialDelayMs = 350L
    private val repeatIntervalMs = 60L
    private val handler = Handler(Looper.getMainLooper())
    private val topBorderPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = 0x33FFFFFF
            strokeWidth = density.coerceAtLeast(1f)
        }
    private var repeatingKey: Pair<String, Int>? = null
    private var isDarkTheme = true
    private var armedMods: Int = 0
    private val keyViews = mutableListOf<Pair<View, Key>>()
    private val rowsContainer: LinearLayout
    private val hostBadge: TextView

    private val repeatRunnable =
        object : Runnable {
            override fun run() {
                val target = repeatingKey ?: return
                sendKey(target.first, target.second)
                handler.postDelayed(this, repeatIntervalMs)
            }
        }

    init {
        isFocusable = false
        isFocusableInTouchMode = false
        setWillNotDraw(false)

        rowsContainer =
            LinearLayout(context).apply {
                orientation = LinearLayout.VERTICAL
                setBaselineAligned(false)
            }
        addView(
            rowsContainer,
            LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT, Gravity.TOP),
        )

        hostBadge =
            TextView(context).apply {
                text = hostOs.displayName
                textSize = 10f
                alpha = 0.55f
                gravity = Gravity.END
            }
        addView(
            hostBadge,
            LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT, Gravity.END or Gravity.BOTTOM)
                .apply { setMargins(0, 0, (8 * density).toInt(), (4 * density).toInt()) },
        )

        build()
        applyTheme(isDark = true)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawLine(0f, 0f, width.toFloat(), 0f, topBorderPaint)
    }

    fun applyTheme(isDark: Boolean) {
        isDarkTheme = isDark
        setBackgroundColor(if (isDark) 0xF50E0C0C.toInt() else 0xF5FFFFFF.toInt())
        topBorderPaint.color = if (isDark) 0x33FFFFFF else 0x22000000
        val foreground = if (isDark) 0xFFFFFFFF.toInt() else 0xFF1A1A1A.toInt()
        hostBadge.setTextColor(foreground)
        for ((view, key) in keyViews) {
            applyKeyStyle(view, key)
        }
        invalidate()
    }

    private fun build() {
        rowsContainer.removeAllViews()
        keyViews.clear()

        for (row in listOf(modNavRow, symbolRow1, symbolRow2)) {
            val rowView =
                LinearLayout(context).apply {
                    orientation = LinearLayout.HORIZONTAL
                    setBaselineAligned(false)
                }
            for (key in row) {
                val view = makeKeyView(key)
                keyViews.add(view to key)
                // Edge-to-edge columns matching the accessory bar above.
                rowView.addView(
                    view,
                    LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f),
                )
            }
            rowsContainer.addView(
                rowView,
                LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, rowHeightPx),
            )
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun makeKeyView(key: Key): View {
        val isBackspace = key.kind is Kind.Dispatch && key.kind.name == "backspace"
        val view =
            TextView(context).apply {
                text = key.label
                // Backspace glyph reads small at 16sp; bump it while keeping the
                // column width aligned with the rest of the row.
                textSize = if (isBackspace) 22f else 16f
                gravity = Gravity.CENTER
                isClickable = true
                isFocusable = false
            }
        applyKeyStyle(view, key)
        view.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    if (key.repeats && key.kind is Kind.Dispatch) {
                        val combined = armedMods or key.kind.fixedMods
                        sendKey(key.kind.name, combined)
                        if (armedMods != 0) {
                            armedMods = 0
                            refreshModifierHighlights()
                        }
                        startRepeating(key.kind.name, combined)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (key.repeats) {
                        stopRepeating()
                    } else {
                        handleKey(key)
                    }
                    true
                }
                MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_OUTSIDE -> {
                    stopRepeating()
                    true
                }
                else -> false
            }
        }
        return view
    }

    private fun applyKeyStyle(view: View, key: Key) {
        val foreground = if (isDarkTheme) 0xFFFFFFFF.toInt() else 0xFF1A1A1A.toInt()
        if (view is TextView) {
            view.setTextColor(foreground)
        }
        val armed = if (isDarkTheme) 0x2EFFFFFF.toInt() else 0x1F000000.toInt()
        val bg =
            if (key.kind is Kind.Modifier && (armedMods and key.kind.bit) != 0) {
                armed
            } else {
                0x00000000
            }
        view.setBackgroundColor(bg)
    }

    private fun refreshModifierHighlights() {
        for ((view, key) in keyViews) {
            applyKeyStyle(view, key)
        }
    }

    private fun handleKey(key: Key) {
        when (val kind = key.kind) {
            is Kind.Dispatch -> {
                val combined = armedMods or kind.fixedMods
                sendKey(kind.name, combined)
                if (armedMods != 0) {
                    armedMods = 0
                    refreshModifierHighlights()
                }
            }
            is Kind.Modifier -> {
                armedMods = armedMods xor kind.bit
                refreshModifierHighlights()
            }
        }
    }

    private fun startRepeating(name: String, mods: Int) {
        stopRepeating()
        repeatingKey = name to mods
        handler.postDelayed(repeatRunnable, repeatInitialDelayMs)
    }

    fun stopRepeating() {
        repeatingKey = null
        handler.removeCallbacks(repeatRunnable)
    }

    override fun onDetachedFromWindow() {
        stopRepeating()
        super.onDetachedFromWindow()
    }
}
