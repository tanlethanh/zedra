package dev.zedra.app

import android.annotation.SuppressLint
import android.content.Context
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView

/**
 * In-app QWERTY panel that replaces the system soft keyboard while the user is
 * driving a terminal / agent session. Each tap is forwarded via the same wire
 * format as the accessory bar (`char:<c>`, named keys, mod bitmask).
 *
 * The panel is intended to be parked in the same screen slot the IME would
 * occupy, so the host should size it to the IME height it just hid.
 */
class FullKeyboardPanel(
    context: Context,
    private val sendKey: (String, Int) -> Unit,
) : LinearLayout(context) {
    private enum class Layer { LETTERS, NUMBERS, SYMBOLS }

    private enum class ShiftState { OFF, ONE_SHOT, LOCKED }

    private sealed class Kind {
        data class Char(val value: String) : Kind()
        data class Named(val name: String) : Kind()
        object Shift : Kind()
        object Backspace : Kind()
        object Space : Kind()
        object Enter : Kind()
        data class ToggleLayer(val target: Layer) : Kind()
    }

    private data class Key(val label: String, val kind: Kind, val weight: Float)

    private val density = context.resources.displayMetrics.density
    private val keyMargin = (3 * density).toInt()
    private var shift: ShiftState = ShiftState.OFF
    private var layer: Layer = Layer.LETTERS
    private var isDarkTheme = true
    private var lastShiftTapMs = 0L

    init {
        orientation = VERTICAL
        setBaselineAligned(false)
        isFocusable = false
        isFocusableInTouchMode = false
        applyTheme(isDark = true)
        rebuild()
    }

    fun applyTheme(isDark: Boolean) {
        isDarkTheme = isDark
        setBackgroundColor(if (isDark) 0xFF1E1E20.toInt() else 0xFFD1D3D8.toInt())
        rebuild()
    }

    private fun rebuild() {
        removeAllViews()
        val rows = layoutFor(layer)
        for (row in rows) {
            val rowView =
                LinearLayout(context).apply {
                    orientation = HORIZONTAL
                    setBaselineAligned(false)
                }
            for (key in row) {
                rowView.addView(makeButton(key), buildRowParams(key.weight))
            }
            addView(
                rowView,
                LayoutParams(LayoutParams.MATCH_PARENT, 0, 1f),
            )
        }
    }

    private fun layoutFor(layer: Layer): List<List<Key>> {
        return when (layer) {
            Layer.LETTERS -> {
                val row1 = "qwertyuiop".map { Key(displayLetter(it.toString()), Kind.Char(it.toString()), 1f) }
                val row2 = "asdfghjkl".map { Key(displayLetter(it.toString()), Kind.Char(it.toString()), 1f) }
                val row3 = mutableListOf<Key>(Key(shiftLabel(), Kind.Shift, 1.5f))
                row3.addAll("zxcvbnm".map { Key(displayLetter(it.toString()), Kind.Char(it.toString()), 1f) })
                row3.add(Key("⌫", Kind.Backspace, 1.5f))
                val row4 =
                    listOf(
                        Key("123", Kind.ToggleLayer(Layer.NUMBERS), 1.5f),
                        Key("space", Kind.Space, 5f),
                        Key("return", Kind.Enter, 2f),
                    )
                listOf(row1, row2, row3, row4)
            }
            Layer.NUMBERS -> {
                val row1 = "1234567890".map { Key(it.toString(), Kind.Char(it.toString()), 1f) }
                val row2 =
                    listOf("-", "/", ":", ";", "(", ")", "$", "&", "@", "\"").map { Key(it, Kind.Char(it), 1f) }
                val row3 = mutableListOf<Key>(Key("#+=", Kind.ToggleLayer(Layer.SYMBOLS), 1.5f))
                row3.addAll(listOf(".", ",", "?", "!", "'").map { Key(it, Kind.Char(it), 1f) })
                row3.add(Key("⌫", Kind.Backspace, 1.5f))
                val row4 =
                    listOf(
                        Key("ABC", Kind.ToggleLayer(Layer.LETTERS), 1.5f),
                        Key("space", Kind.Space, 5f),
                        Key("return", Kind.Enter, 2f),
                    )
                listOf(row1, row2, row3, row4)
            }
            Layer.SYMBOLS -> {
                val row1 =
                    listOf("[", "]", "{", "}", "#", "%", "^", "*", "+", "=").map { Key(it, Kind.Char(it), 1f) }
                val row2 =
                    listOf("_", "\\", "|", "~", "<", ">", "€", "£", "¥", "•").map { Key(it, Kind.Char(it), 1f) }
                val row3 = mutableListOf<Key>(Key("123", Kind.ToggleLayer(Layer.NUMBERS), 1.5f))
                row3.addAll(listOf(".", ",", "?", "!", "'").map { Key(it, Kind.Char(it), 1f) })
                row3.add(Key("⌫", Kind.Backspace, 1.5f))
                val row4 =
                    listOf(
                        Key("ABC", Kind.ToggleLayer(Layer.LETTERS), 1.5f),
                        Key("space", Kind.Space, 5f),
                        Key("return", Kind.Enter, 2f),
                    )
                listOf(row1, row2, row3, row4)
            }
        }
    }

    private fun displayLetter(c: String): String = if (shift == ShiftState.OFF) c else c.uppercase()

    private fun shiftLabel(): String =
        when (shift) {
            ShiftState.OFF -> "⇧"
            ShiftState.ONE_SHOT -> "⬆"
            ShiftState.LOCKED -> "⇪"
        }

    @SuppressLint("ClickableViewAccessibility")
    private fun makeButton(key: Key): View {
        val isSpecial =
            when (key.kind) {
                is Kind.Char -> false
                else -> true
            }
        val foreground = if (isDarkTheme) 0xFFFFFFFF.toInt() else 0xFF101015.toInt()
        val baseBg =
            if (isDarkTheme) {
                if (isSpecial) 0xFF333339.toInt() else 0xFF4A4A52.toInt()
            } else {
                if (isSpecial) 0xFFA6A8AE.toInt() else 0xFFFFFFFF.toInt()
            }
        val view =
            TextView(context).apply {
                text = key.label
                textSize = 16f
                gravity = Gravity.CENTER
                setTextColor(foreground)
                setBackgroundColor(baseBg)
                isClickable = true
                setOnClickListener { handleKey(key) }
            }
        if (key.kind is Kind.Shift && shift == ShiftState.LOCKED) {
            view.setBackgroundColor(
                if (isDarkTheme) 0x40FFFFFF.toInt() else 0x33000000.toInt(),
            )
        }
        return view
    }

    private fun buildRowParams(weight: Float): LayoutParams =
        LayoutParams(0, LayoutParams.MATCH_PARENT, weight).apply {
            setMargins(keyMargin, keyMargin, keyMargin, keyMargin)
        }

    private fun handleKey(key: Key) {
        when (val kind = key.kind) {
            is Kind.Char -> {
                val outgoing = if (shift == ShiftState.OFF) kind.value else kind.value.uppercase()
                sendKey("char:$outgoing", 0)
                if (shift == ShiftState.ONE_SHOT) {
                    shift = ShiftState.OFF
                    rebuild()
                }
            }
            is Kind.Named -> sendKey(kind.name, 0)
            Kind.Shift -> handleShiftTap()
            Kind.Backspace -> sendKey("backspace", 0)
            Kind.Space -> sendKey("char: ", 0)
            Kind.Enter -> sendKey("enter", 0)
            is Kind.ToggleLayer -> {
                layer = kind.target
                shift = ShiftState.OFF
                rebuild()
            }
        }
    }

    private fun handleShiftTap() {
        val now = System.currentTimeMillis()
        val isDoubleTap = now - lastShiftTapMs < 350
        lastShiftTapMs = now

        shift =
            if (isDoubleTap) {
                ShiftState.LOCKED
            } else {
                when (shift) {
                    ShiftState.OFF -> ShiftState.ONE_SHOT
                    ShiftState.ONE_SHOT -> ShiftState.OFF
                    ShiftState.LOCKED -> ShiftState.OFF
                }
            }
        rebuild()
    }
}
