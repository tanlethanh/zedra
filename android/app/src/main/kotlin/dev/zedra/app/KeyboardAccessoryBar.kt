package dev.zedra.app

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Paint
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import kotlin.math.abs

/**
 * Terminal key bar. Renders either the compact single row or the extended
 * two-row keypad, with an IME composing field parked one page to the right:
 * a horizontal drag slides between them and follows the finger.
 *
 * Modifier state is owned by Rust (`zedra_terminal::keyboard_accessory`) because
 * it must also apply to characters committed by the software keyboard; this view
 * only renders the mask it is given.
 */
private const val MOD_SHIFT = 1
private const val MOD_CTRL = 2
private const val MOD_ALT = 4
private const val MOD_CMD = 8

/** Locked modifiers sit this far above the armed bits; see `sticky_modifier_mask`. */
private const val MOD_LOCK_SHIFT = 4

class KeyboardAccessoryBar(
    context: Context,
    private val sendKey: (String) -> Unit,
    private val sendComposedText: (String) -> Unit,
    // Hand the keyboard back to the terminal when the user leaves the composer.
    private val requestTerminalKeyboard: () -> Unit,
) : FrameLayout(context) {
    private data class KeySpec(
        val label: String,
        val key: String,
        val repeats: Boolean = false,
        val iconRes: Int? = null,
        val modifierBit: Int? = null,
    )

    private val topBorderPaint =
        Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = 0x33FFFFFF
            strokeWidth = context.resources.displayMetrics.density.coerceAtLeast(1f)
        }

    private val compactRow =
        listOf(
            KeySpec("Esc", "escape"),
            KeySpec("Tab", "tab"),
            KeySpec("←", "left", repeats = true, iconRes = R.drawable.ic_key_arrow_left),
            KeySpec("↓", "down", repeats = true, iconRes = R.drawable.ic_key_arrow_down),
            KeySpec("↑", "up", repeats = true, iconRes = R.drawable.ic_key_arrow_up),
            KeySpec("→", "right", repeats = true, iconRes = R.drawable.ic_key_arrow_right),
            KeySpec("⏎", "enter", iconRes = R.drawable.ic_key_return),
        )

    private val extendedTopRow =
        listOf(
            KeySpec("Esc", "escape"),
            KeySpec("Shift", "mod:shift", modifierBit = MOD_SHIFT),
            KeySpec("Tab", "tab"),
            KeySpec("/", "char:/"),
            KeySpec("-", "char:-"),
            KeySpec("↑", "up", repeats = true, iconRes = R.drawable.ic_key_arrow_up),
            KeySpec("⏎", "enter", iconRes = R.drawable.ic_key_return),
        )

    // Cmd only means anything to a macOS host; every other host gets a pipe,
    // which is real shell syntax and awkward to reach on a phone keyboard.
    private val platformSlot: KeySpec
        get() =
            if (cmdSlot) {
                KeySpec("Cmd", "mod:cmd", modifierBit = MOD_CMD)
            } else {
                KeySpec("|", "char:|")
            }

    private val extendedBottomRow: List<KeySpec>
        get() =
            listOf(
                KeySpec("⌫", "backspace", repeats = true, iconRes = R.drawable.ic_key_backspace),
                KeySpec("Ctrl", "mod:ctrl", modifierBit = MOD_CTRL),
                KeySpec("Alt", "mod:alt", modifierBit = MOD_ALT),
                platformSlot,
                KeySpec("←", "left", repeats = true, iconRes = R.drawable.ic_key_arrow_left),
                KeySpec("↓", "down", repeats = true, iconRes = R.drawable.ic_key_arrow_down),
                KeySpec("→", "right", repeats = true, iconRes = R.drawable.ic_key_arrow_right),
            )

    private val repeatInitialDelayMs = 350L
    private val repeatIntervalMs = 60L
    private val snapDurationMs = 180L
    private val handler = Handler(Looper.getMainLooper())
    private var repeatingKey: String? = null
    private var repeatFired = false
    private var isDarkTheme = true
    private var extended = false
    private var cmdSlot = false
    private var composing = false
    private var modifierMask = 0
    private val modifierViews = mutableListOf<Pair<Int, TextView>>()
    private val density = context.resources.displayMetrics.density
    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop
    private var dragging = false
    private var dragStartX = 0f
    private var dragStartY = 0f

    private val keysPage =
        LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setBaselineAligned(false)
        }

    private val composePage =
        LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            setBaselineAligned(false)
        }

    private val composeField =
        EditText(context).apply {
            hint = "Compose, then send"
            textSize = 15f
            maxLines = 4
            background = null
            setPadding((12 * density).toInt(), 0, (8 * density).toInt(), 0)
            // TYPE_TEXT_FLAG_MULTI_LINE would turn the IME's action key into a
            // newline key and swallow IME_ACTION_SEND; wrapping comes from
            // maxLines + no horizontal scrolling instead.
            inputType = InputType.TYPE_CLASS_TEXT
            setHorizontallyScrolling(false)
            imeOptions = EditorInfo.IME_ACTION_SEND
            setOnEditorActionListener { _, actionId, event ->
                // Some IMEs report the action, others just send the Enter key.
                val enterPressed =
                    event?.keyCode == KeyEvent.KEYCODE_ENTER && event.action == KeyEvent.ACTION_DOWN
                if (actionId == EditorInfo.IME_ACTION_SEND ||
                    actionId == EditorInfo.IME_ACTION_DONE ||
                    enterPressed
                ) {
                    submitComposedText()
                    true
                } else {
                    false
                }
            }
        }

    // ImpactLight — normal key taps use light haptics (matches
    // HapticFeedback::to_i32() in platform_bridge.rs).
    private fun triggerKeyHaptic() = MainActivity.triggerHaptic(0)

    private val repeatRunnable =
        object : Runnable {
            override fun run() {
                val key = repeatingKey ?: return
                // Held keys haptic on their first repeat only, not on every 60 ms
                // tick, so a held arrow stays non-buzzy.
                if (!repeatFired) {
                    repeatFired = true
                    triggerKeyHaptic()
                }
                sendKey(key)
                handler.postDelayed(this, repeatIntervalMs)
            }
        }

    /** Bar height in px for the current mode; the host reserves this much space. */
    val desiredHeightPx: Int
        // Two stacked rows would eat too much of the screen at full row height.
        // The composer keeps the same height so sliding never resizes the bar.
        get() = ((if (extended) 64f else 44f) * density).toInt()

    init {
        isFocusable = false
        isFocusableInTouchMode = false
        setWillNotDraw(false)
        visibility = GONE
        addView(keysPage, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        addView(composePage, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        buildComposePage()
        rebuildKeys()
        applyTheme(isDark = true)
    }

    override fun onDetachedFromWindow() {
        stopRepeating()
        super.onDetachedFromWindow()
    }

    override fun onSizeChanged(
        w: Int,
        h: Int,
        oldw: Int,
        oldh: Int,
    ) {
        super.onSizeChanged(w, h, oldw, oldh)
        settlePages(animated = false)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawLine(0f, 0f, width.toFloat(), 0f, topBorderPaint)
    }

    /** Drop the composer and its keyboard: the field owns the IME, which GPUI's own
     * keyboard dismissal cannot reach. */
    fun cancelComposing() {
        if (!composing) return
        setComposing(false, animated = false)
    }

    fun stopRepeating() {
        repeatingKey = null
        repeatFired = false
        handler.removeCallbacks(repeatRunnable)
    }

    /** Returns true when the reserved height changed, so the host can re-measure. */
    fun setLayout(
        enabled: Boolean,
        useCmdSlot: Boolean,
    ): Boolean {
        if (extended == enabled && cmdSlot == useCmdSlot) return false
        val heightChanged = extended != enabled
        extended = enabled
        cmdSlot = useCmdSlot
        if (!enabled && composing) {
            setComposing(false, animated = false)
        }
        rebuildKeys()
        return heightChanged
    }

    fun setModifierMask(mask: Int) {
        if (modifierMask == mask) return
        modifierMask = mask
        applyModifierHighlights()
    }

    // The two pages sit side by side: the keys occupy [-width, 0] and the composer
    // [0, width], so a drag reveals the composer progressively instead of swapping
    // it in at the end of a gesture.
    private fun pageOffset() = if (composing) -width.toFloat() else 0f

    private fun settlePages(animated: Boolean) {
        val target = pageOffset()
        if (animated) {
            keysPage.animate().translationX(target).setDuration(snapDurationMs).start()
            composePage.animate().translationX(target + width).setDuration(snapDurationMs).start()
        } else {
            applyDragOffset(0f)
        }
    }

    private fun applyDragOffset(dx: Float) {
        val offset = (pageOffset() + dx).coerceIn(-width.toFloat(), 0f)
        keysPage.translationX = offset
        composePage.translationX = offset + width
    }

    override fun onInterceptTouchEvent(event: MotionEvent): Boolean {
        if (!extended) return false
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                dragStartX = event.x
                dragStartY = event.y
                dragging = false
            }
            MotionEvent.ACTION_MOVE -> {
                val dx = event.x - dragStartX
                val dy = event.y - dragStartY
                if (abs(dx) > touchSlop && abs(dx) > abs(dy)) {
                    dragging = true
                    dragStartX = event.x
                    stopRepeating()
                    return true
                }
            }
        }
        return false
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (!dragging) return super.onTouchEvent(event)
        when (event.actionMasked) {
            MotionEvent.ACTION_MOVE -> applyDragOffset(event.x - dragStartX)
            MotionEvent.ACTION_UP,
            MotionEvent.ACTION_CANCEL,
            -> {
                val dx = event.x - dragStartX
                dragging = false
                // Past halfway the page flips; anything short of that springs back.
                val flipped = abs(dx) > width / 2f
                setComposing(composing != flipped, animated = true, keepKeyboard = true)
            }
        }
        return true
    }

    private fun submitComposedText() {
        val text = composeField.text.toString()
        if (text.isNotEmpty()) {
            android.util.Log.i("KeyboardAccessoryBar", "key-bar: submitting composed text (${text.length} chars)")
            // Text only: the composer fills the prompt, and the user decides when to run it.
            sendComposedText(text)
            composeField.setText("")
        }
    }

    /// `keepKeyboard` distinguishes the user stepping back to the keys — where the
    /// keyboard should stay up, now typing into the terminal — from a forced cancel
    /// (drawer, navigation, terminal tap), where it must go away with the composer.
    private fun setComposing(
        enabled: Boolean,
        animated: Boolean,
        keepKeyboard: Boolean = false,
    ) {
        val changed = composing != enabled
        composing = enabled
        settlePages(animated)
        if (!changed) return

        val ime = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
        if (enabled) {
            composeField.requestFocus()
            // The field is useless without a keyboard, and the pinned bar runs with
            // the IME down by definition.
            ime?.showSoftInput(composeField, InputMethodManager.SHOW_IMPLICIT)
            return
        }

        composeField.clearFocus()
        if (keepKeyboard) {
            requestTerminalKeyboard()
        } else {
            ime?.hideSoftInputFromWindow(windowToken, 0)
        }
    }

    private fun buildComposePage() {
        composePage.removeAllViews()
        composePage.addView(
            composeField,
            LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f),
        )
        // The IME's own send action submits, so this is the way out, not a second
        // submit control.
        composePage.addView(
            makeLabelButton("✕") { setComposing(false, animated = true, keepKeyboard = true) }.apply {
                contentDescription = "Close composer"
            },
            LinearLayout.LayoutParams(
                (52 * density).toInt(),
                LinearLayout.LayoutParams.MATCH_PARENT,
            ),
        )
    }

    private fun rebuildKeys() {
        stopRepeating()
        keysPage.removeAllViews()
        modifierViews.clear()

        if (extended) {
            keysPage.addView(makeRow(extendedTopRow), rowParams())
            keysPage.addView(makeRow(extendedBottomRow), rowParams())
        } else {
            keysPage.addView(makeRow(compactRow), rowParams())
        }
        applyTheme(isDarkTheme)
        applyModifierHighlights()
    }

    private fun rowParams() = LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f)

    private fun makeLabelButton(
        label: String,
        onPress: () -> Unit,
    ): TextView =
        TextView(context).apply {
            text = label
            textSize = 15f
            gravity = Gravity.CENTER
            isClickable = true
            setOnClickListener { onPress() }
        }

    private fun makeRow(specs: List<KeySpec>): View {
        val row =
            LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                setBaselineAligned(false)
            }
        specs.forEach { spec ->
            row.addView(
                makeButton(spec),
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f),
            )
        }
        return row
    }

    private fun applyModifierHighlights() {
        modifierViews.forEach { (bit, view) ->
            val armed = modifierMask and bit != 0
            val locked = modifierMask and (bit shl MOD_LOCK_SHIFT) != 0
            // Accent blue marks an active modifier; a locked one is fully opaque,
            // an armed one dimmer, so the two stay distinguishable without a fill.
            val accent = NativePresentations.currentAccentBlueColor()
            view.setTextColor(
                when {
                    locked -> accent
                    armed -> (accent and 0x00FFFFFF) or 0x99000000.toInt()
                    else -> foregroundColor
                },
            )
        }
    }

    private val foregroundColor: Int
        get() = if (isDarkTheme) 0xFFFFFFFF.toInt() else 0xFF1A1A1A.toInt()

    fun applyTheme(isDark: Boolean) {
        isDarkTheme = isDark
        val foreground = foregroundColor
        setBackgroundColor(if (isDark) 0xF50E0C0C.toInt() else 0xF5FFFFFF.toInt())
        topBorderPaint.color = if (isDark) 0x33FFFFFF else 0x22000000
        composeField.setTextColor(foreground)
        composeField.setHintTextColor(if (isDark) 0x80FFFFFF.toInt() else 0x80000000.toInt())
        tintChildren(keysPage, foreground)
        tintChildren(composePage, foreground)
        applyModifierHighlights()
        invalidate()
    }

    private fun tintChildren(
        group: LinearLayout,
        foreground: Int,
    ) {
        for (index in 0 until group.childCount) {
            when (val child = group.getChildAt(index)) {
                is LinearLayout -> tintChildren(child, foreground)
                is ImageView -> child.imageTintList = ColorStateList.valueOf(foreground)
                is EditText -> Unit
                is TextView -> child.setTextColor(foreground)
            }
        }
    }

    private fun makeButton(spec: KeySpec): View {
        val foreground = foregroundColor
        val view =
            if (spec.iconRes != null) {
                ImageView(context).apply {
                    setImageResource(spec.iconRes)
                    imageTintList = ColorStateList.valueOf(foreground)
                    scaleType = ImageView.ScaleType.CENTER
                    contentDescription = spec.label
                }
            } else {
                TextView(context).apply {
                    text = spec.label
                    textSize = if (extended) 14f else 16f
                    gravity = Gravity.CENTER
                    setTextColor(foreground)
                }
            }

        spec.modifierBit?.let { bit -> modifierViews.add(bit to (view as TextView)) }

        view.isClickable = true
        view.isFocusable = false
        view.isFocusableInTouchMode = false
        view.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    // Nothing is sent yet: a horizontal drag starting on this key
                    // belongs to the pager, and a key already sent cannot be taken
                    // back. Holds emit from the repeat timer, taps on release.
                    if (spec.repeats) {
                        startRepeating(spec.key)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    val repeated = repeatFired
                    stopRepeating()
                    if (!repeated) {
                        triggerKeyHaptic()
                        sendKey(spec.key)
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

    private fun startRepeating(key: String) {
        stopRepeating()
        repeatingKey = key
        handler.postDelayed(repeatRunnable, repeatInitialDelayMs)
    }
}
