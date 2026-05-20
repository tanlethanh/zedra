package dev.zedra.app

import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AlertDialog
import com.google.android.material.bottomsheet.BottomSheetBehavior
import com.google.android.material.bottomsheet.BottomSheetDialog
import kotlin.math.max
import kotlin.math.roundToInt

object NativePresentations {
    private val mainHandler = Handler(Looper.getMainLooper())
    private var activity: MainActivity? = null
    private var rootView: FrameLayout? = null
    private val floatingButtons = mutableMapOf<Int, TextView>()
    private val dictationPreviews = mutableMapOf<Int, TextView>()
    private val notifications = mutableMapOf<Int, View>()
    private var sheetDialog: BottomSheetDialog? = null

    @JvmStatic
    fun register(activity: MainActivity, rootView: FrameLayout) {
        this.activity = activity
        this.rootView = rootView
    }

    @JvmStatic
    fun unregister() {
        sheetDialog?.dismiss()
        sheetDialog = null
        floatingButtons.values.forEach { rootView?.removeView(it) }
        floatingButtons.clear()
        dictationPreviews.values.forEach { rootView?.removeView(it) }
        dictationPreviews.clear()
        notifications.values.forEach { rootView?.removeView(it) }
        notifications.clear()
        rootView = null
        activity = null
    }

    // Keep dialogs on AppCompat widgets because the host activity uses an
    // AppCompat theme; Material dialog/text-field widgets are theme-sensitive.
    @JvmStatic
    fun showAlert(
        callbackId: Int,
        title: String?,
        message: String?,
        labels: Array<String>?,
        styles: IntArray?,
    ) = onUi {
        val safeLabels = labels?.takeIf { it.isNotEmpty() } ?: arrayOf("OK")
        val safeStyles = styles?.takeIf { it.size == safeLabels.size } ?: IntArray(safeLabels.size)
        val dialog = AlertDialog.Builder(requireActivity())
            .apply {
                if (!title.isNullOrBlank()) setTitle(title)
                if (!message.isNullOrBlank()) setMessage(message)
                setOnCancelListener { MainActivity.nativeAlertDismiss(callbackId) }
                setPositiveButton(safeLabels[0]) { _, _ ->
                    MainActivity.nativeAlertResult(callbackId, 0)
                }
                if (safeLabels.size > 1) {
                    setNegativeButton(safeLabels[1]) { _, _ ->
                        MainActivity.nativeAlertResult(callbackId, 1)
                    }
                }
                if (safeLabels.size > 2) {
                    setNeutralButton(safeLabels[2]) { _, _ ->
                        MainActivity.nativeAlertResult(callbackId, 2)
                    }
                }
            }
            .create()
        dialog.setOnShowListener {
            safeStyles.forEachIndexed { index, style ->
                if (style == 2) {
                    val which = when (index) {
                        0 -> android.content.DialogInterface.BUTTON_POSITIVE
                        1 -> android.content.DialogInterface.BUTTON_NEGATIVE
                        2 -> android.content.DialogInterface.BUTTON_NEUTRAL
                        else -> 0
                    }
                    if (which != 0) dialog.getButton(which)?.setTextColor(Color.rgb(244, 97, 97))
                }
            }
        }
        dialog.show()
    }

    @JvmStatic
    fun showSelection(
        callbackId: Int,
        title: String?,
        message: String?,
        labels: Array<String>?,
        styles: IntArray?,
    ) = onUi {
        val safeLabels = labels?.takeIf { it.isNotEmpty() } ?: arrayOf("OK")
        AlertDialog.Builder(requireActivity())
            .apply {
                if (!title.isNullOrBlank()) setTitle(title)
                if (!message.isNullOrBlank()) setMessage(message)
                setItems(safeLabels) { _, which ->
                    MainActivity.nativeSelectionResult(callbackId, which)
                }
                setOnCancelListener { MainActivity.nativeSelectionDismiss(callbackId) }
            }
            .create()
            .also {
                it.setCanceledOnTouchOutside(true)
                it.show()
            }
    }

    @JvmStatic
    fun showTextInput(
        callbackId: Int,
        title: String?,
        placeholder: String?,
        initialValue: String?,
    ) = onUi {
        val input = EditText(requireActivity()).apply {
            setSingleLine(true)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
            hint = placeholder.orEmpty()
            setText(initialValue.orEmpty())
            setSelection(text?.length ?: 0)
        }
        val container = FrameLayout(requireActivity()).apply {
            setPadding(dp(20f), dp(8f), dp(20f), 0)
            addView(input, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ))
        }
        AlertDialog.Builder(requireActivity())
            .apply {
                if (!title.isNullOrBlank()) setTitle(title)
                setView(container)
                setNegativeButton("Cancel") { _, _ ->
                    MainActivity.nativeTextInputDismiss(callbackId)
                }
                setPositiveButton("OK") { _, _ ->
                    MainActivity.nativeTextInputResult(callbackId, input.text?.toString().orEmpty())
                }
                setOnCancelListener { MainActivity.nativeTextInputDismiss(callbackId) }
            }
            .show()
    }

    @JvmStatic
    fun presentCustomSheet(
        detents: IntArray?,
        initialDetent: Int,
        showsGrabber: Boolean,
        expandsOnScrollEdge: Boolean,
        modalInPresentation: Boolean,
        cornerRadius: Float,
    ) = onUi {
        val activity = requireActivity()
        sheetDialog?.dismiss()

        val screenHeight = activity.resources.displayMetrics.heightPixels
        val initialHeight = when (initialDetent) {
            0 -> (screenHeight * 0.55f).roundToInt()
            else -> (screenHeight * 0.92f).roundToInt()
        }

        val container = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            background = roundedBackground(Color.rgb(20, 22, 27), cornerRadius.takeIf { it >= 0f } ?: 18f)
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                initialHeight,
            )
        }

        if (showsGrabber) {
            container.addView(View(activity).apply {
                background = roundedBackground(Color.argb(150, 210, 214, 224), 2f)
                val width = dp(38f)
                val height = dp(4f)
                layoutParams = LinearLayout.LayoutParams(width, height).apply {
                    gravity = Gravity.CENTER_HORIZONTAL
                    topMargin = dp(8f)
                    bottomMargin = dp(8f)
                }
            })
        }

        val surface = SheetHostView(activity).apply {
            this.expandsOnScrollEdge = expandsOnScrollEdge
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f,
            )
        }
        container.addView(surface)

        val dialog = BottomSheetDialog(
            activity,
            com.google.android.material.R.style.Theme_Design_BottomSheetDialog,
        ).apply {
            setContentView(container)
            setCancelable(!modalInPresentation)
            setCanceledOnTouchOutside(!modalInPresentation)
            window?.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
            setOnDismissListener {
                if (sheetDialog === this) sheetDialog = null
            }
        }
        sheetDialog = dialog
        dialog.setOnShowListener {
            val sheet = dialog.findViewById<FrameLayout>(com.google.android.material.R.id.design_bottom_sheet)
            sheet?.background = null
            sheet?.let {
                it.layoutParams = it.layoutParams.apply {
                    height = if (detents?.contains(0) == true && detents.contains(1)) {
                        (screenHeight * 0.92f).roundToInt()
                    } else {
                        initialHeight
                    }
                }
            }
            val behavior = sheet?.let { BottomSheetBehavior.from(it) }
            behavior?.isFitToContents = false
            behavior?.halfExpandedRatio = 0.5f
            behavior?.isHideable = !modalInPresentation
            behavior?.state = if (initialDetent == 0) {
                BottomSheetBehavior.STATE_HALF_EXPANDED
            } else {
                BottomSheetBehavior.STATE_EXPANDED
            }
        }
        dialog.show()
    }

    @JvmStatic
    fun updateNativeFloatingButton(
        id: Int,
        imageName: String?,
        accessibilityLabel: String?,
        x: Float,
        y: Float,
        width: Float,
        height: Float,
        iconSize: Float,
        iconWeight: Int,
    ) = onUi {
        val root = requireRoot()
        val button = floatingButtons.getOrPut(id) {
            TextView(requireActivity()).apply {
                gravity = Gravity.CENTER
                typeface = Typeface.DEFAULT_BOLD
                setTextColor(Color.WHITE)
                background = roundedBackground(Color.argb(230, 38, 42, 51), 999f)
                elevation = dp(8f).toFloat()
                setOnClickListener { MainActivity.nativeFloatingButtonPressed(id) }
                root.addView(this)
            }
        }
        button.text = symbolFor(imageName)
        button.textSize = max(10f, iconSize)
        button.contentDescription = accessibilityLabel.orEmpty()
        button.layoutParams = FrameLayout.LayoutParams(dp(width), dp(height)).apply {
            leftMargin = dp(x)
            topMargin = dp(y)
        }
        button.visibility = View.VISIBLE
        button.bringToFront()
    }

    @JvmStatic
    fun hideNativeFloatingButton(id: Int) = onUi {
        floatingButtons.remove(id)?.let { requireRoot().removeView(it) }
    }

    @JvmStatic
    fun updateNativeDictationPreview(id: Int, text: String?, bottomOffset: Float) = onUi {
        val root = requireRoot()
        val preview = dictationPreviews.getOrPut(id) {
            TextView(requireActivity()).apply {
                gravity = Gravity.CENTER
                setTextColor(Color.WHITE)
                textSize = 15f
                maxLines = 3
                setPadding(dp(14f), dp(10f), dp(14f), dp(10f))
                background = roundedBackground(Color.argb(238, 32, 36, 44), 18f)
                elevation = dp(10f).toFloat()
                setOnClickListener {
                    MainActivity.nativeDictationPreviewDismiss(id)
                    hideNativeDictationPreview(id)
                }
                root.addView(this)
            }
        }
        preview.text = text.orEmpty()
        preview.layoutParams = FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL,
        ).apply {
            leftMargin = dp(18f)
            rightMargin = dp(18f)
            bottomMargin = dp(bottomOffset + 18f)
        }
        preview.visibility = if (text.isNullOrBlank()) View.GONE else View.VISIBLE
        preview.bringToFront()
    }

    @JvmStatic
    fun hideNativeDictationPreview(id: Int) = onUi {
        dictationPreviews.remove(id)?.let { requireRoot().removeView(it) }
    }

    @JvmStatic
    fun showNativeNotification(
        id: Int,
        title: String?,
        message: String?,
        imageName: String?,
        kind: Int,
        durationSecs: Float,
        autoClose: Boolean,
    ) = onUi {
        val root = requireRoot()
        notifications.remove(id)?.let { root.removeView(it) }
        val banner = LinearLayout(requireActivity()).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(14f), dp(10f), dp(14f), dp(10f))
            background = roundedBackground(Color.argb(242, 30, 34, 42), 16f)
            elevation = dp(12f).toFloat()
            alpha = 0f
            translationY = -dp(18f).toFloat()
            setOnClickListener {
                MainActivity.nativeNotificationAction(id)
                removeNativeNotification(id, notifyDismiss = true)
            }
        }
        banner.addView(TextView(requireActivity()).apply {
            text = symbolFor(imageName).takeIf { it != "•" } ?: kindSymbol(kind)
            setTextColor(kindColor(kind))
            textSize = 18f
            gravity = Gravity.CENTER
            layoutParams = LinearLayout.LayoutParams(dp(24f), dp(24f))
        })
        banner.addView(TextView(requireActivity()).apply {
            text = listOfNotNull(title, message?.takeIf { it.isNotBlank() }).joinToString("\n")
            setTextColor(Color.WHITE)
            textSize = 14f
            setLineSpacing(0f, 1.08f)
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply {
                leftMargin = dp(10f)
            }
        })
        notifications[id] = banner
        root.addView(banner, notificationLayoutParams(notifications.size - 1))
        banner.animate().alpha(1f).translationY(0f).setDuration(160).start()
        if (autoClose) {
            mainHandler.postDelayed(
                { removeNativeNotification(id, notifyDismiss = true) },
                max(500L, (durationSecs * 1000f).roundToInt().toLong()),
            )
        }
    }

    private fun removeNativeNotification(id: Int, notifyDismiss: Boolean) = onUi {
        val view = notifications.remove(id) ?: return@onUi
        view.animate()
            .alpha(0f)
            .translationY(-dp(12f).toFloat())
            .setDuration(120)
            .withEndAction {
                rootView?.removeView(view)
                relayoutNotifications()
                if (notifyDismiss) MainActivity.nativeNotificationDismiss(id)
            }
            .start()
    }

    private fun relayoutNotifications() {
        notifications.values.forEachIndexed { index, view ->
            view.layoutParams = notificationLayoutParams(index)
            view.requestLayout()
        }
    }

    private fun notificationLayoutParams(index: Int): FrameLayout.LayoutParams {
        return FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            Gravity.TOP or Gravity.CENTER_HORIZONTAL,
        ).apply {
            leftMargin = dp(12f)
            rightMargin = dp(12f)
            topMargin = dp(18f + index * 72f)
        }
    }

    private fun onUi(action: () -> Unit) {
        val activity = activity ?: return
        if (Looper.myLooper() == Looper.getMainLooper()) {
            action()
        } else {
            activity.runOnUiThread(action)
        }
    }

    private fun requireActivity(): MainActivity = activity ?: error("NativePresentations not registered")

    private fun requireRoot(): FrameLayout = rootView ?: error("NativePresentations root not registered")

    private fun dp(value: Float): Int {
        val density = activity?.resources?.displayMetrics?.density ?: 1f
        return (value * density).roundToInt()
    }

    private fun roundedBackground(color: Int, radiusDp: Float): GradientDrawable {
        return GradientDrawable().apply {
            setColor(color)
            cornerRadius = dp(radiusDp).toFloat()
        }
    }

    private fun symbolFor(name: String?): String {
        return when (name) {
            "arrow.down", "chevron.down", "arrow.down.circle", "arrow.down.to.line" -> "↓"
            "xmark", "xmark.circle" -> "×"
            "checkmark", "checkmark.circle" -> "✓"
            "exclamationmark.triangle" -> "!"
            else -> "•"
        }
    }

    private fun kindSymbol(kind: Int): String {
        return when (kind) {
            1 -> "✓"
            2 -> "!"
            3 -> "!"
            else -> "•"
        }
    }

    private fun kindColor(kind: Int): Int {
        return when (kind) {
            1 -> Color.rgb(111, 221, 147)
            2 -> Color.rgb(244, 188, 92)
            3 -> Color.rgb(244, 97, 97)
            else -> Color.rgb(202, 209, 222)
        }
    }
}
