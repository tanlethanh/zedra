package dev.zedra.app

import android.annotation.SuppressLint
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView

/**
 * Desktop-only key panel that replaces the system IME while a terminal or
 * agent session has focus. Carries only the keys / combos that the soft
 * keyboard doesn't surface as a single tap. There is no QWERTY here — users
 * tap `✕` and go back to the IME for prose typing, IME (Vietnamese / CJK),
 * dictation, autocorrect, gesture typing.
 *
 * Every tap dispatches `(name, mods)` matching the accessory bar's wire
 * format (`char:<c>` for single chars, named keys, Shift/Alt/Ctrl bitmask).
 */
class FullKeyboardPanel(
    context: Context,
    private val sendKey: (String, Int) -> Unit,
) : LinearLayout(context) {
    private object AccessoryMods {
        const val SHIFT = 0b001
        const val ALT = 0b010
        const val CTRL = 0b100
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

    private val symbolRow =
        listOf(
            Key("`", Kind.Dispatch("char:`")),
            Key("~", Kind.Dispatch("char:~")),
            Key("|", Kind.Dispatch("char:|")),
            Key("\\", Kind.Dispatch("char:\\")),
            Key("<", Kind.Dispatch("char:<")),
            Key(">", Kind.Dispatch("char:>")),
            Key("{", Kind.Dispatch("char:{")),
            Key("}", Kind.Dispatch("char:}")),
            Key("[", Kind.Dispatch("char:[")),
            Key("]", Kind.Dispatch("char:]")),
        )

    private val navRow =
        listOf(
            Key("Home", Kind.Dispatch("home")),
            Key("End", Kind.Dispatch("end")),
            Key("PgUp", Kind.Dispatch("page_up"), repeats = true),
            Key("PgDn", Kind.Dispatch("page_down"), repeats = true),
            Key("←", Kind.Dispatch("left"), repeats = true),
            Key("↓", Kind.Dispatch("down"), repeats = true),
            Key("↑", Kind.Dispatch("up"), repeats = true),
            Key("→", Kind.Dispatch("right"), repeats = true),
        )

    private val controlRow =
        listOf(
            Key("Esc", Kind.Dispatch("escape")),
            Key("Tab", Kind.Dispatch("tab")),
            Key("⇧⏎", Kind.Dispatch("enter", AccessoryMods.SHIFT)),
            Key("Shift", Kind.Modifier(AccessoryMods.SHIFT)),
            Key("Ctrl", Kind.Modifier(AccessoryMods.CTRL)),
            Key("⌃C", Kind.Dispatch("char:c", AccessoryMods.CTRL)),
            Key("⌃D", Kind.Dispatch("char:d", AccessoryMods.CTRL)),
            Key("⌃R", Kind.Dispatch("char:r", AccessoryMods.CTRL)),
        )

    private val density = context.resources.displayMetrics.density
    private val keyMargin = (3 * density).toInt()
    private val repeatInitialDelayMs = 350L
    private val repeatIntervalMs = 60L
    private val handler = Handler(Looper.getMainLooper())
    private var repeatingKey: Pair<String, Int>? = null
    private var isDarkTheme = true
    private var armedMods: Int = 0
    private val keyViews = mutableListOf<Pair<View, Key>>()

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
        applyTheme(isDark = true)
        build()
    }

    fun applyTheme(isDark: Boolean) {
        isDarkTheme = isDark
        setBackgroundColor(if (isDark) 0xFF1E1E20.toInt() else 0xFFD1D3D8.toInt())
        for ((view, key) in keyViews) {
            applyKeyStyle(view, key)
        }
    }

    private fun build() {
        removeAllViews()
        keyViews.clear()
        for (row in listOf(symbolRow, navRow, controlRow)) {
            val rowView =
                LinearLayout(context).apply {
                    orientation = HORIZONTAL
                    setBaselineAligned(false)
                }
            for (key in row) {
                val view = makeKeyView(key)
                keyViews.add(view to key)
                rowView.addView(
                    view,
                    LayoutParams(0, LayoutParams.MATCH_PARENT, 1f).apply {
                        setMargins(keyMargin, keyMargin, keyMargin, keyMargin)
                    },
                )
            }
            addView(rowView, LayoutParams(LayoutParams.MATCH_PARENT, 0, 1f))
        }
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun makeKeyView(key: Key): View {
        val view =
            TextView(context).apply {
                text = key.label
                textSize = 16f
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
        val isModifier = key.kind is Kind.Modifier
        val foreground = if (isDarkTheme) 0xFFFFFFFF.toInt() else 0xFF101015.toInt()
        val baseBg =
            if (isDarkTheme) {
                if (isModifier) 0xFF333339.toInt() else 0xFF4A4A52.toInt()
            } else {
                if (isModifier) 0xFFA6A8AE.toInt() else 0xFFFFFFFF.toInt()
            }
        val highlight = if (isDarkTheme) 0x40FFFFFF.toInt() else 0x33000000.toInt()
        val bg =
            if (key.kind is Kind.Modifier && (armedMods and key.kind.bit) != 0) {
                highlight
            } else {
                baseBg
            }
        view.setBackgroundColor(bg)
        if (view is TextView) {
            view.setTextColor(foreground)
        }
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
