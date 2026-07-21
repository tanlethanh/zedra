package dev.zedra.app

import android.app.Activity
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import kotlin.math.roundToInt

// Non-blocking upload-progress HUD: spinner + message pinned near the top.
// Only one at a time — a new id retargets the card; dismiss ignores stale ids.
// Main thread only.
object ProgressHud {
    private var currentId: Int? = null
    private var card: LinearLayout? = null
    private var label: TextView? = null

    fun show(activity: Activity, id: Int, message: String) {
        card?.let {
            currentId = id
            label?.text = message
            it.bringToFront()
            return
        }
        val content = activity.findViewById<FrameLayout>(android.R.id.content) ?: return
        val density = activity.resources.displayMetrics.density
        fun dp(value: Float) = (value * density).roundToInt()

        // App's selected theme, not the OS setting — they can differ.
        val overlay = NativePresentations.currentOverlayColor()
        val cardColor = Color.argb(
            235,
            Color.red(overlay),
            Color.green(overlay),
            Color.blue(overlay),
        )
        val textColor = NativePresentations.currentTextPrimaryColor()
        val spinnerColor = NativePresentations.currentTextSecondaryColor()

        val text = TextView(activity).apply {
            text = message
            textSize = 14f
            setTextColor(textColor)
            maxLines = 1
        }
        // Wrap-content card: never covers the terminal; isClickable=false lets touches through.
        val view = LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            isClickable = false
            isFocusable = false
            background = GradientDrawable().apply {
                setColor(cardColor)
                cornerRadius = dp(14f).toFloat()
            }
            elevation = dp(8f).toFloat()
            setPadding(dp(16f), dp(12f), dp(18f), dp(12f))
            addView(
                ProgressBar(activity).apply {
                    isIndeterminate = true
                    indeterminateTintList = ColorStateList.valueOf(spinnerColor)
                },
                LinearLayout.LayoutParams(dp(18f), dp(18f)),
            )
            addView(
                text,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ).apply { leftMargin = dp(12f) },
            )
        }
        content.addView(
            view,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.TOP or Gravity.CENTER_HORIZONTAL,
            ).apply { topMargin = dp(24f) },
        )
        card = view
        label = text
        currentId = id
    }

    fun dismiss(id: Int) {
        if (id != currentId) return
        clear()
    }

    fun clear() {
        card?.let { (it.parent as? ViewGroup)?.removeView(it) }
        card = null
        label = null
        currentId = null
    }
}
