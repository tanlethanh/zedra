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
 * agent session has focus. Layout mirrors `FullKeyboardView` on iOS:
 *
 *   Row 1 (chips):  shift  Ctrl  Cmd  Home  End  PgUp  PgD  ⌫
 *   Row 2 (flat):   ~  @  $  *  ^  %  =  `
 *   Row 3 (flat):   <  >  (  )  {  }  [  ]
 *   The lower ~40% of the panel is intentionally left blank for future
 *   additions (clipboard, snippets, agent macros).
 *
 * `✕` on the accessory bar hands focus back to the IME for prose typing.
 */
class FullKeyboardPanel(
    context: Context,
    @Suppress("unused") private val hostOs: HostOs,
    private val sendKey: (String, Int) -> Unit,
) : LinearLayout(context) {
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

    private enum class Style { CHIP, FLAT }

    private data class Key(
        val label: String,
        val kind: Kind,
        val style: Style,
        val repeats: Boolean = false,
    )

    private val chipRow =
        listOf(
            Key("shift", Kind.Modifier(AccessoryMods.SHIFT), Style.CHIP),
            Key("Ctrl", Kind.Modifier(AccessoryMods.CTRL), Style.CHIP),
            Key("Cmd", Kind.Modifier(AccessoryMods.CMD), Style.CHIP),
            Key("Home", Kind.Dispatch("home"), Style.CHIP),
            Key("End", Kind.Dispatch("end"), Style.CHIP),
            Key("PgUp", Kind.Dispatch("page_up"), Style.CHIP, repeats = true),
            Key("PgD", Kind.Dispatch("page_down"), Style.CHIP, repeats = true),
            Key("⌫", Kind.Dispatch("backspace"), Style.CHIP, repeats = true),
        )

    private val symbolRow1 =
        listOf(
            Key("~", Kind.Dispatch("char:~"), Style.FLAT),
            Key("@", Kind.Dispatch("char:@"), Style.FLAT),
            Key("$", Kind.Dispatch("char:$"), Style.FLAT),
            Key("*", Kind.Dispatch("char:*"), Style.FLAT),
            Key("^", Kind.Dispatch("char:^"), Style.FLAT),
            Key("%", Kind.Dispatch("char:%"), Style.FLAT),
            Key("=", Kind.Dispatch("char:="), Style.FLAT),
            Key("`", Kind.Dispatch("char:`"), Style.FLAT),
        )

    private val symbolRow2 =
        listOf(
            Key("<", Kind.Dispatch("char:<"), Style.FLAT),
            Key(">", Kind.Dispatch("char:>"), Style.FLAT),
            Key("(", Kind.Dispatch("char:("), Style.FLAT),
            Key(")", Kind.Dispatch("char:)"), Style.FLAT),
            Key("{", Kind.Dispatch("char:{"), Style.FLAT),
            Key("}", Kind.Dispatch("char:}"), Style.FLAT),
            Key("[", Kind.Dispatch("char:["), Style.FLAT),
            Key("]", Kind.Dispatch("char:]"), Style.FLAT),
        )

    private val density = context.resources.displayMetrics.density
    private val keyMargin = (3 * density).toInt()
    private val rowGapPx = (4 * density).toInt()
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
        setBackgroundColor(if (isDark) 0xFF14141A.toInt() else 0xFFD1D3D8.toInt())
        for ((view, key) in keyViews) {
            applyKeyStyle(view, key)
        }
    }

    private fun build() {
        removeAllViews()
        keyViews.clear()

        for (row in listOf(chipRow, symbolRow1, symbolRow2)) {
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

        // Reserve the bottom area for future content. A 2-weight blank view
        // pushes the key rows up so they occupy roughly the top 60%.
        addView(View(context), LayoutParams(LayoutParams.MATCH_PARENT, 0, 2f))
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun makeKeyView(key: Key): View {
        val view =
            TextView(context).apply {
                text = key.label
                textSize = if (key.style == Style.CHIP) 14f else 20f
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
        val foreground = if (isDarkTheme) 0xFFFFFFFF.toInt() else 0xFF101015.toInt()
        if (view is TextView) {
            view.setTextColor(foreground)
        }
        when (key.style) {
            Style.CHIP -> {
                val base = if (isDarkTheme) 0xFF4A4A52.toInt() else 0xFFA6A8AE.toInt()
                val armed = if (isDarkTheme) 0x47FFFFFF.toInt() else 0x38000000.toInt()
                val bg =
                    if (key.kind is Kind.Modifier && (armedMods and key.kind.bit) != 0) {
                        armed
                    } else {
                        base
                    }
                view.setBackgroundColor(bg)
            }
            Style.FLAT -> view.setBackgroundColor(0x00000000)
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
