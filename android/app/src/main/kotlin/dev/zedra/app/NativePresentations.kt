package dev.zedra.app

import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AlertDialog
import com.google.android.material.bottomsheet.BottomSheetBehavior
import com.google.android.material.bottomsheet.BottomSheetDialog
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import com.google.android.material.divider.MaterialDivider
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

object NativePresentations {
    private val mainHandler = Handler(Looper.getMainLooper())
    private var activity: MainActivity? = null
    private var rootView: FrameLayout? = null
    private val floatingButtons = mutableMapOf<Int, ImageButton>()
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

    @JvmStatic
    fun showAlert(
        callbackId: Int,
        title: String?,
        message: String?,
        labels: Array<String>?,
        styles: IntArray?,
    ) = onUi {
        val safeLabels = labels?.takeIf { it.isNotEmpty() } ?: arrayOf("OK")
        val safeStyles = styles
            ?.takeIf { it.size == safeLabels.size }
            ?: IntArray(safeLabels.size)
        val dialog = MaterialAlertDialogBuilder(requireActivity())
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
                val which = when (index) {
                    0 -> android.content.DialogInterface.BUTTON_POSITIVE
                    1 -> android.content.DialogInterface.BUTTON_NEGATIVE
                    2 -> android.content.DialogInterface.BUTTON_NEUTRAL
                    else -> 0
                }
                if (which != 0) {
                    dialog.getButton(which)?.setTextColor(alertButtonColor(style))
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
        val safeStyles = styles
            ?.takeIf { it.size == safeLabels.size }
            ?: IntArray(safeLabels.size)
        val activity = requireActivity()
        lateinit var dialog: AlertDialog
        val content = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            if (!title.isNullOrBlank()) {
                addView(selectionHeader(title, primary = true))
            }
            if (!message.isNullOrBlank()) {
                addView(selectionHeader(message, primary = title.isNullOrBlank()))
            }
            safeLabels.forEachIndexed { index, label ->
                addView(TextView(activity).apply {
                    text = label
                    textSize = 16f
                    gravity = Gravity.CENTER_VERTICAL
                    minHeight = dp(56f)
                    setPadding(dp(24f), 0, dp(24f), 0)
                    setTextColor(
                        if (safeStyles[index] == 2) {
                            Color.rgb(244, 97, 97)
                        } else {
                            Color.WHITE
                        }
                    )
                    setSelectableItemBackground(this)
                    setOnClickListener {
                        if (safeStyles[index] == 1) {
                            MainActivity.nativeSelectionDismiss(callbackId)
                        } else {
                            MainActivity.nativeSelectionResult(callbackId, index)
                        }
                        dialog.dismiss()
                    }
                }, LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ))
                if (index + 1 < safeLabels.size) {
                    addView(MaterialDivider(activity), LinearLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.WRAP_CONTENT,
                    ))
                }
            }
            setPadding(0, if (title.isNullOrBlank() && message.isNullOrBlank()) dp(8f) else 0, 0, dp(8f))
        }
        dialog = MaterialAlertDialogBuilder(activity)
            .apply {
                // Keep the header and list in one Material custom view so
                // the dialog title/message panels cannot create a false gap.
                setView(content)
                setOnCancelListener { MainActivity.nativeSelectionDismiss(callbackId) }
            }
            .create()
        dialog.setCanceledOnTouchOutside(true)
        dialog.show()
    }

    @JvmStatic
    fun showListPicker(
        callbackId: Int,
        title: String?,
        message: String?,
        labels: Array<String>?,
        subtitles: Array<String?>?,
        imageNames: Array<String?>?,
    ) = onUi {
        val safeLabels = labels?.takeIf { it.isNotEmpty() } ?: run {
            MainActivity.nativeSelectionDismiss(callbackId)
            return@onUi
        }
        val activity = requireActivity()
        val sheet = BottomSheetDialog(activity)
        val content = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16f), dp(12f), dp(16f), dp(12f))
            if (!title.isNullOrBlank()) {
                addView(selectionHeader(title, primary = true))
            }
            if (!message.isNullOrBlank()) {
                addView(selectionHeader(message, primary = title.isNullOrBlank()))
            }
            val scroll = android.widget.ScrollView(activity).apply {
                layoutParams = LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    dp(420f),
                )
                val list = LinearLayout(activity).apply {
                    orientation = LinearLayout.VERTICAL
                }
                safeLabels.forEachIndexed { index, label ->
                    val subtitle = subtitles?.getOrNull(index)?.orEmpty().orEmpty()
                    list.addView(TextView(activity).apply {
                        text = if (subtitle.isBlank()) label else "$label\n$subtitle"
                        textSize = 16f
                        setLineSpacing(0f, 1.1f)
                        gravity = Gravity.CENTER_VERTICAL
                        minHeight = dp(56f)
                        setPadding(dp(8f), dp(10f), dp(8f), dp(10f))
                        setTextColor(Color.WHITE)
                        setSelectableItemBackground(this)
                        setOnClickListener {
                            MainActivity.nativeSelectionResult(callbackId, index)
                            sheet.dismiss()
                        }
                    }, LinearLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.WRAP_CONTENT,
                    ))
                    if (index + 1 < safeLabels.size) {
                        list.addView(MaterialDivider(activity), LinearLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.WRAP_CONTENT,
                        ))
                    }
                }
                addView(list)
            }
            addView(scroll)
        }
        sheet.setContentView(content)
        sheet.setOnCancelListener { MainActivity.nativeSelectionDismiss(callbackId) }
        sheet.show()
    }

    @JvmStatic
    fun showTextInput(
        callbackId: Int,
        title: String?,
        placeholder: String?,
        initialValue: String?,
    ) = onUi {
        val input = TextInputEditText(requireActivity()).apply {
            setSingleLine(true)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
            setText(initialValue.orEmpty())
            setSelection(text?.length ?: 0)
        }
        val inputLayout = TextInputLayout(requireActivity()).apply {
            hint = placeholder.orEmpty()
            addView(input, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ))
        }
        val container = FrameLayout(requireActivity()).apply {
            setPadding(dp(20f), dp(8f), dp(20f), 0)
            addView(inputLayout, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
            ))
        }
        MaterialAlertDialogBuilder(requireActivity())
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

        // Real (full-window) height — `displayMetrics.heightPixels` excludes the
        // system bars, which left the sheet short of the screen bottom.
        val realMetrics = android.util.DisplayMetrics()
        @Suppress("DEPRECATION")
        activity.windowManager.defaultDisplay.getRealMetrics(realMetrics)
        val fullHeight = realMetrics.heightPixels

        val hasTwoDetents = detents?.contains(0) == true && detents.contains(1)
        // Detents are pure top offsets: large leaves an ~8% strip, medium ~55%.
        val largeOffset = (fullHeight * 0.08f).roundToInt()
        val mediumRatio = 0.55f

        val container = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            // Transparent: the BottomSheet view carries the rounded chrome,
            // styled declaratively by the ZedraBottomSheetDialog theme.
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }

        if (showsGrabber) {
            container.addView(View(activity).apply {
                background = roundedBackground(Color.argb(150, 210, 214, 224), 2f)
                layoutParams = LinearLayout.LayoutParams(dp(38f), dp(4f)).apply {
                    gravity = Gravity.CENTER_HORIZONTAL
                    topMargin = dp(8f)
                    bottomMargin = dp(4f)
                }
            })
        }

        val surface = SheetHostView(activity).apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f,
            )
        }
        container.addView(surface)

        // No explicit theme: BottomSheetDialog resolves `bottomSheetDialogTheme`
        // from the activity theme, which points at ZedraBottomSheetDialog.
        val dialog = BottomSheetDialog(activity).apply {
            setContentView(container)
            setCancelable(!modalInPresentation)
            setCanceledOnTouchOutside(!modalInPresentation)
            // The custom sheet hosts a GPUI SurfaceView, not text input. Letting
            // the dialog follow the activity's adjustResize path can resize the
            // embedded render surface while the sheet is presenting or dragging.
            window?.setSoftInputMode(
                WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING or
                    WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN,
            )
            setOnDismissListener {
                if (sheetDialog === this) sheetDialog = null
            }
        }
        surface.bottomSheetBehavior = dialog.behavior

        // Configure the sheet view and behavior before show() so the intro is a
        // single smooth slide. The sheet view fills the window (MATCH_PARENT) so
        // it always reaches the screen bottom regardless of the detent.
        dialog.findViewById<FrameLayout>(
            com.google.android.material.R.id.design_bottom_sheet,
        )?.let { sheet ->
            sheet.layoutParams = sheet.layoutParams.apply {
                height = ViewGroup.LayoutParams.MATCH_PARENT
            }
        }
        dialog.behavior.apply {
            isFitToContents = false
            isHideable = !modalInPresentation
            skipCollapsed = true
            expandedOffset = largeOffset
            halfExpandedRatio = mediumRatio
            state = if (initialDetent == 0 && hasTwoDetents) {
                BottomSheetBehavior.STATE_HALF_EXPANDED
            } else {
                BottomSheetBehavior.STATE_EXPANDED
            }
        }
        sheetDialog = dialog
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
            ImageButton(requireActivity()).apply {
                background = roundedBackground(Color.argb(230, 38, 42, 51), 999f)
                imageTintList = ColorStateList.valueOf(Color.WHITE)
                scaleType = ImageView.ScaleType.FIT_CENTER
                elevation = dp(8f).toFloat()
                setOnClickListener { MainActivity.nativeFloatingButtonPressed(id) }
                root.addView(this)
            }
        }
        val safeIconSize = iconSize.coerceAtLeast(10f)
        val iconPadding = ((min(width, height) - safeIconSize) / 2f).coerceAtLeast(0f)
        button.setImageResource(floatingButtonIconRes(imageName))
        button.setPadding(
            dp(iconPadding),
            dp(iconPadding),
            dp(iconPadding),
            dp(iconPadding),
        )
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

    // Always post, never run inline. These actions add/remove views on the
    // shared rootView; running synchronously can land mid-traversal when the
    // caller is itself inside a view-tree walk (e.g. window inset dispatch
    // triggered by the soft keyboard), corrupting child iteration.
    private fun onUi(action: () -> Unit) {
        mainHandler.post {
            if (activity != null && rootView != null) {
                action()
            }
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

    private fun selectionHeader(text: String, primary: Boolean): TextView {
        return TextView(requireActivity()).apply {
            this.text = text
            textSize = if (primary) 20f else 14f
            setTextColor(if (primary) Color.WHITE else Color.rgb(202, 209, 222))
            setPadding(
                dp(24f),
                if (primary) dp(24f) else 0,
                dp(24f),
                if (primary) dp(16f) else dp(12f),
            )
            maxLines = 2
        }
    }

    private fun alertButtonColor(style: Int): Int {
        return if (style == 2) {
            Color.rgb(244, 97, 97)
        } else {
            Color.rgb(189, 189, 189)
        }
    }

    private fun floatingButtonIconRes(name: String?): Int {
        return when (name) {
            "arrow.down", "chevron.down", "arrow.down.circle", "arrow.down.to.line" ->
                R.drawable.ic_key_arrow_down
            else -> R.drawable.ic_key_arrow_down
        }
    }

    private fun setSelectableItemBackground(view: View) {
        val outValue = TypedValue()
        if (view.context.theme.resolveAttribute(
                android.R.attr.selectableItemBackground,
                outValue,
                true,
            )
        ) {
            view.setBackgroundResource(outValue.resourceId)
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
