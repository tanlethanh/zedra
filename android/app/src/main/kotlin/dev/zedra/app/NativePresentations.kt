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
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.FrameLayout
import android.widget.EditText
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.PopupWindow
import android.widget.TextView
import androidx.appcompat.app.AlertDialog
import androidx.core.content.res.ResourcesCompat
import androidx.webkit.ProxyConfig
import androidx.webkit.ProxyController
import androidx.webkit.WebViewFeature
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.widget.NestedScrollView
import com.google.android.material.bottomsheet.BottomSheetBehavior
import com.google.android.material.bottomsheet.BottomSheetDialog
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import com.google.android.material.divider.MaterialDivider
import dev.zed.gpui.SelectionController
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
    private var editMenuPopup: PopupWindow? = null
    private var webViewOverlay: View? = null
    private var activeWebView: WebView? = null
    private var activeWebViewCallbackId: Int = 0
    private var webViewProxyActive: Boolean = false
    private var webViewUrlField: EditText? = null
    private var webViewFaviconIcon: ImageView? = null
    private var webViewBackButton: TextView? = null
    private var webViewForwardButton: TextView? = null
    private var nativeTheme = NativeTheme.dark()

    private data class NativeTheme(
        val background: Int,
        val card: Int,
        val overlay: Int,
        val textPrimary: Int,
        val textSecondary: Int,
        val textMuted: Int,
        val border: Int,
        val accentRed: Int,
        val accentGreen: Int,
        val accentYellow: Int,
    ) {
        companion object {
            fun dark() = NativeTheme(
                background = Color.rgb(14, 12, 12),
                card = Color.rgb(19, 19, 19),
                overlay = Color.rgb(19, 19, 19),
                textPrimary = Color.WHITE,
                textSecondary = Color.rgb(202, 202, 202),
                textMuted = Color.rgb(80, 80, 80),
                border = Color.rgb(44, 44, 44),
                accentRed = Color.rgb(224, 108, 117),
                accentGreen = Color.rgb(152, 195, 121),
                accentYellow = Color.rgb(229, 192, 123),
            )

            fun light() = NativeTheme(
                background = Color.rgb(245, 245, 245),
                card = Color.WHITE,
                overlay = Color.WHITE,
                textPrimary = Color.rgb(26, 26, 26),
                textSecondary = Color.rgb(74, 74, 74),
                textMuted = Color.rgb(138, 138, 138),
                border = Color.rgb(216, 216, 216),
                accentRed = Color.rgb(207, 34, 46),
                accentGreen = Color.rgb(26, 127, 55),
                accentYellow = Color.rgb(154, 103, 0),
            )
        }
    }

    @JvmStatic
    fun register(activity: MainActivity, rootView: FrameLayout) {
        this.activity = activity
        this.rootView = rootView
    }

    @JvmStatic
    fun unregister() {
        sheetDialog?.dismiss()
        sheetDialog = null
        editMenuPopup?.dismiss()
        editMenuPopup = null
        closeWebViewNow()
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
    fun setDarkTheme(isDark: Boolean) {
        nativeTheme = if (isDark) NativeTheme.dark() else NativeTheme.light()
        onUi {
            floatingButtons.values.forEach { applyFloatingButtonTheme(it) }
            dictationPreviews.values.forEach { applyDictationPreviewTheme(it) }
            relayoutNotifications()
        }
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
                setCancelable(true)
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
        dialog.setCanceledOnTouchOutside(false)
        dialog.show()
        applyDialogTheme(dialog)
        safeStyles.forEachIndexed { index, style ->
            val which = when (index) {
                0 -> android.content.DialogInterface.BUTTON_POSITIVE
                1 -> android.content.DialogInterface.BUTTON_NEGATIVE
                2 -> android.content.DialogInterface.BUTTON_NEUTRAL
                else -> return@forEachIndexed
            }
            dialog.getButton(which)?.setTextColor(alertButtonColor(style))
        }
    }

    @JvmStatic
    fun showSelection(
        callbackId: Int,
        title: String?,
        message: String?,
        labels: Array<String>?,
        styles: IntArray?,
        imageNames: Array<String?>,
    ) = onUi {
        val safeLabels = labels?.takeIf { it.isNotEmpty() } ?: arrayOf("OK")
        val safeStyles = styles
            ?.takeIf { it.size == safeLabels.size }
            ?: IntArray(safeLabels.size)
        val activity = requireActivity()
        lateinit var dialog: AlertDialog
        val content = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(nativeTheme.overlay)
            if (!title.isNullOrBlank()) {
                addView(selectionHeader(title, primary = true))
            }
            if (!message.isNullOrBlank()) {
                addView(selectionHeader(message, primary = title.isNullOrBlank()))
            }
            safeLabels.forEachIndexed { index, label ->
                val imageName = imageNames?.getOrNull(index)
                val iconRes = selectionIconRes(imageName)
                val row = LinearLayout(activity).apply {
                    orientation = LinearLayout.HORIZONTAL
                    gravity = Gravity.CENTER_VERTICAL
                    minimumHeight = dp(56f)
                    setPadding(dp(24f), 0, dp(24f), 0)
                    setSelectableItemBackground(this)
                    setOnClickListener {
                        if (safeStyles[index] == 1) {
                            MainActivity.nativeSelectionDismiss(callbackId)
                        } else {
                            MainActivity.nativeSelectionResult(callbackId, index)
                        }
                        dialog.dismiss()
                    }
                }
                if (iconRes != 0) {
                    row.addView(ImageView(activity).apply {
                        layoutParams = LinearLayout.LayoutParams(dp(20f), dp(20f)).apply {
                            marginEnd = dp(14f)
                        }
                        setImageResource(iconRes)
                        imageTintList = ColorStateList.valueOf(nativeTheme.textPrimary)
                    })
                }
                row.addView(TextView(activity).apply {
                    text = label
                    textSize = 16f
                    setTextColor(
                        if (safeStyles[index] == 2) nativeTheme.accentRed
                        else nativeTheme.textPrimary
                    )
                }, LinearLayout.LayoutParams(
                    0,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    1f,
                ))
                addView(row, LinearLayout.LayoutParams(
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
                setView(content)
                setOnCancelListener { MainActivity.nativeSelectionDismiss(callbackId) }
            }
            .create()
        dialog.setCanceledOnTouchOutside(true)
        dialog.show()
        applyDialogTheme(dialog)
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
            setBackgroundColor(nativeTheme.background)
            setPadding(0, 0, 0, dp(8f))
            addView(dragHandle())
            if (!title.isNullOrBlank()) {
                addView(pickerHeader(title, message))
            }
            val scroll = NestedScrollView(activity).apply {
                layoutParams = LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    dp(420f),
                )
                isNestedScrollingEnabled = true
                val list = LinearLayout(activity).apply {
                    orientation = LinearLayout.VERTICAL
                }
                safeLabels.forEachIndexed { index, label ->
                    val subtitle = subtitles?.getOrNull(index)?.orEmpty().orEmpty()
                    val imageName = imageNames?.getOrNull(index)
                    val row = LinearLayout(activity).apply {
                        orientation = LinearLayout.HORIZONTAL
                        gravity = Gravity.CENTER_VERTICAL
                        minimumHeight = dp(48f)
                        setPadding(dp(20f), dp(8f), dp(20f), dp(8f))
                        setSelectableItemBackground(this)
                        setOnClickListener {
                            MainActivity.nativeSelectionResult(callbackId, index)
                            sheet.dismiss()
                        }
                    }
                    val iconRes = agentIconRes(imageName)
                    val iconView = ImageView(activity).apply {
                        layoutParams = LinearLayout.LayoutParams(dp(20f), dp(20f)).apply {
                            marginEnd = dp(14f)
                        }
                        if (iconRes != 0) {
                            setImageResource(iconRes)
                            imageTintList = ColorStateList.valueOf(nativeTheme.textPrimary)
                        }
                    }
                    row.addView(iconView)
                    val textCol = LinearLayout(activity).apply {
                        orientation = LinearLayout.VERTICAL
                        layoutParams = LinearLayout.LayoutParams(
                            0,
                            ViewGroup.LayoutParams.WRAP_CONTENT,
                            1f,
                        )
                    }
                    textCol.addView(TextView(activity).apply {
                        text = label
                        textSize = 15f
                        setTextColor(nativeTheme.textPrimary)
                        typeface = loraTypeface(activity)
                        includeFontPadding = false
                    })
                    if (subtitle.isNotBlank()) {
                        textCol.addView(TextView(activity).apply {
                            text = subtitle
                            textSize = 12f
                            setTextColor(nativeTheme.textSecondary)
                            typeface = loraTypeface(activity)
                            includeFontPadding = false
                            setPadding(0, dp(2f), 0, 0)
                        })
                    }
                    row.addView(textCol)
                    list.addView(row, LinearLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.WRAP_CONTENT,
                    ))
                }
                addView(list)
            }
            addView(scroll)
        }
        sheet.setContentView(content)
        sheet.setOnCancelListener { MainActivity.nativeSelectionDismiss(callbackId) }
        sheet.show()
        sheet.findViewById<FrameLayout>(
            com.google.android.material.R.id.design_bottom_sheet,
        )?.background = roundedBackground(nativeTheme.background, 18f)
    }

    // Floating contextual edit menu (e.g. Paste) anchored at a window point.
    // Mirrors the iOS UIEditMenuInteraction path; x/y arrive as GPUI logical
    // points already shifted above the touch by the Rust caller.
    @JvmStatic
    fun showNativeEditMenu(
        callbackId: Int,
        x: Float,
        y: Float,
        labels: Array<String>,
        imageNames: Array<String>,
    ) = onUi {
        if (labels.isEmpty()) {
            MainActivity.nativeEditMenuDismiss(callbackId)
            return@onUi
        }
        val root = requireRoot()
        editMenuPopup?.dismiss()

        val row = LinearLayout(requireActivity()).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            background = roundedBackground(withAlpha(nativeTheme.overlay, 242), 12f)
            elevation = dp(12f).toFloat()
            setPadding(dp(4f), dp(4f), dp(4f), dp(4f))
        }

        // A single flag guards the result/dismiss callbacks so the popup's
        // onDismiss (which also fires after a selection) never double-reports.
        var resolved = false
        val popup = PopupWindow(
            row,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            true,
        ).apply {
            setBackgroundDrawable(android.graphics.drawable.ColorDrawable(Color.TRANSPARENT))
            isOutsideTouchable = true
        }

        labels.forEachIndexed { index, label ->
            if (index > 0) {
                row.addView(View(requireActivity()).apply {
                    setBackgroundColor(withAlpha(nativeTheme.border, 200))
                    layoutParams = LinearLayout.LayoutParams(dp(1f), dp(20f))
                })
            }
            row.addView(TextView(requireActivity()).apply {
                text = label
                setTextColor(nativeTheme.textPrimary)
                textSize = 15f
                gravity = Gravity.CENTER
                setPadding(dp(16f), dp(10f), dp(16f), dp(10f))
                isClickable = true
                setOnClickListener {
                    resolved = true
                    popup.dismiss()
                    MainActivity.nativeEditMenuResult(callbackId, index)
                }
            })
        }

        popup.setOnDismissListener {
            if (editMenuPopup === popup) editMenuPopup = null
            if (!resolved) {
                resolved = true
                MainActivity.nativeEditMenuDismiss(callbackId)
            }
        }

        // Measure so the bubble sits above the anchor and stays on-screen.
        row.measure(View.MeasureSpec.UNSPECIFIED, View.MeasureSpec.UNSPECIFIED)
        val anchorX = dp(x)
        val anchorY = dp(y)
        val left = (anchorX - row.measuredWidth / 2)
            .coerceIn(dp(8f), max(dp(8f), root.width - row.measuredWidth - dp(8f)))
        val top = (anchorY - row.measuredHeight).coerceAtLeast(dp(8f))

        editMenuPopup = popup
        popup.showAtLocation(root, Gravity.NO_GRAVITY, left, top)
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
            setText(initialValue.orEmpty())
            setSelection(text?.length ?: 0)
            setTextColor(nativeTheme.textPrimary)
            setHintTextColor(nativeTheme.textMuted)
            hint = placeholder.orEmpty()
            backgroundTintList = ColorStateList.valueOf(nativeTheme.border)
        }
        val container = FrameLayout(requireActivity()).apply {
            setBackgroundColor(nativeTheme.overlay)
            setPadding(dp(20f), dp(8f), dp(20f), 0)
            addView(input, FrameLayout.LayoutParams(
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
            .also { applyDialogTheme(it) }
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
                background = roundedBackground(withAlpha(nativeTheme.textSecondary, 150), 2f)
                layoutParams = LinearLayout.LayoutParams(dp(38f), dp(4f)).apply {
                    gravity = Gravity.CENTER_HORIZONTAL
                    topMargin = dp(8f)
                    bottomMargin = dp(4f)
                }
            })
        }

        val surface = SheetHostView(activity)
        // Wrap the surface so the native selection overlay (added by the
        // SelectionController) can sit above only the GPUI sheet content.
        val surfaceWrap = FrameLayout(activity).apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f,
            )
            addView(
                surface,
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.MATCH_PARENT,
                ),
            )
        }
        container.addView(surfaceWrap)
        surface.selectionController = SelectionController(surfaceWrap, surface)

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
                surface.selectionController?.destroy()
                surface.selectionController = null
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
    fun dismissCustomSheet() = onUi {
        sheetDialog?.dismiss()
        sheetDialog = null
    }

    @JvmStatic
    fun openWebView(callbackId: Int, configJson: String?) = onUi {
        val config = parseWebViewConfig(configJson) ?: return@onUi
        if (config.url.isBlank()) return@onUi
        closeWebViewNow()

        val activity = requireActivity()
        val root = requireRoot()
        val container = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(nativeTheme.background)
            elevation = dp(18f).toFloat()
        }
        val title = config.title
        val url = config.url
        if (BuildConfig.DEBUG) {
            android.util.Log.i(
                "zedra",
                "[debug:webview] openWebView cb=$callbackId handler=${config.messageHandlerName} " +
                    "intercept=${config.interceptNavigation} url=${config.url}",
            )
            WebView.setWebContentsDebuggingEnabled(true)
        }
        val webView = WebView(activity).apply {
            setBackgroundColor(nativeTheme.background)
            webViewClient = WebViewControllerClient(callbackId, config)
            webChromeClient = object : android.webkit.WebChromeClient() {
                override fun onReceivedIcon(view: WebView, icon: android.graphics.Bitmap?) {
                    super.onReceivedIcon(view, icon)
                    if (icon != null) updateWebViewFavicon(icon)
                }

                override fun onConsoleMessage(m: android.webkit.ConsoleMessage): Boolean {
                    if (BuildConfig.DEBUG) {
                        android.util.Log.i("zedra", "[debug:webview] console: ${m.message()}")
                    }
                    return true
                }
            }
            @Suppress("SetJavaScriptEnabled")
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            val handlerName = config.messageHandlerName
            if (!handlerName.isNullOrBlank()) {
                addJavascriptInterface(WebViewMessageBridge(callbackId), handlerName)
            }
        }
        // Safari-style chrome: a top bar (back / forward / URL capsule with
        // favicon + reload / share / close) above the page.
        container.addView(buildWebViewTopBar(activity, webView, title, url))
        container.addView(webView, LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            0,
            1f,
        ))

        activeWebView = webView
        activeWebViewCallbackId = callbackId
        webViewOverlay = container
        root.addView(container, FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT,
        ))
        container.bringToFront()
        applyWebViewProxy(activity, config.socksProxy) { loadWebView(webView, url) }
    }

    private fun buildWebViewTopBar(
        activity: MainActivity,
        webView: WebView,
        title: String,
        url: String,
    ): View {
        fun controlButton(glyph: String, size: Float, onClick: () -> Unit) = TextView(activity).apply {
            text = glyph
            textSize = size
            setTextColor(nativeTheme.textPrimary)
            gravity = Gravity.CENTER
            setPadding(dp(8f), dp(8f), dp(8f), dp(8f))
            isClickable = true
            setSelectableItemBackground(this)
            setOnClickListener { onClick() }
        }
        // Padding shrinks the visible glyph within the fixed 36dp-tall slot below, so system
        // icon drawables read at the same visual weight as the text glyphs (e.g. close's "✕").
        fun controlImageButton(resId: Int, padding: Float, onClick: () -> Unit) = ImageButton(activity).apply {
            setImageResource(resId)
            imageTintList = ColorStateList.valueOf(nativeTheme.textPrimary)
            background = null
            scaleType = ImageView.ScaleType.CENTER_INSIDE
            setPadding(dp(padding), dp(padding), dp(padding), dp(padding))
            isClickable = true
            setSelectableItemBackground(this)
            setOnClickListener { onClick() }
        }

        val backButton = controlButton("‹", 22f) {
            if (webView.canGoBack()) webView.goBack()
        }.apply { isEnabled = false; alpha = 0.35f }
        val forwardButton = controlButton("›", 22f) {
            if (webView.canGoForward()) webView.goForward()
        }.apply { isEnabled = false; alpha = 0.35f }
        val shareButton = controlImageButton(android.R.drawable.ic_menu_share, padding = 9f) {
            shareCurrentWebViewUrl(activity, webView)
        }
        val closeButton = controlButton("✕", 17f) { closeWebViewNow() }

        // URL capsule: favicon + editable URL + reload, sized to match the reload button.
        val faviconIcon = ImageView(activity).apply {
            scaleType = ImageView.ScaleType.FIT_CENTER
            visibility = View.GONE
        }
        val reloadButton = controlButton("↻", 16f) { webView.reload() }
        val clearButton = controlImageButton(android.R.drawable.ic_menu_close_clear_cancel, padding = 9f) {
            webViewUrlField?.setText("")
        }.apply { visibility = View.GONE }

        // Back stays visible in edit mode; only forward/share collapse (their weighted-row
        // space is reclaimed by the pill).
        fun setEditingChromeHidden(hidden: Boolean) {
            val visibility = if (hidden) View.GONE else View.VISIBLE
            forwardButton.visibility = visibility
            shareButton.visibility = visibility
            faviconIcon.visibility = if (hidden) View.GONE else if (faviconIcon.drawable != null) View.VISIBLE else View.GONE
            reloadButton.visibility = visibility
            clearButton.visibility = if (hidden) View.VISIBLE else View.GONE
        }

        val urlField = EditText(activity).apply {
            setSingleLine(true)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
            imeOptions = android.view.inputmethod.EditorInfo.IME_ACTION_GO
            setText(android.net.Uri.parse(url).host ?: title.takeIf { it.isNotBlank() } ?: url)
            textSize = 15f
            setTextColor(nativeTheme.textPrimary)
            gravity = Gravity.CENTER
            maxLines = 1
            setPadding(dp(4f), 0, dp(4f), 0)
            setBackgroundColor(Color.TRANSPARENT)
            ellipsize = android.text.TextUtils.TruncateAt.MIDDLE
            setOnFocusChangeListener { view, hasFocus ->
                val field = view as EditText
                setEditingChromeHidden(hasFocus)
                if (hasFocus) {
                    field.gravity = Gravity.CENTER_VERTICAL or Gravity.START
                    field.setText(webView.url ?: url)
                    field.setSelection(field.text?.length ?: 0)
                    field.selectAll()
                } else {
                    field.gravity = Gravity.CENTER
                    val uri = android.net.Uri.parse(webView.url ?: "")
                    field.setText(uri.host ?: webView.url)
                }
            }
            setOnEditorActionListener { view, actionId, _ ->
                if (actionId == android.view.inputmethod.EditorInfo.IME_ACTION_GO) {
                    navigateWebViewTo(webView, (view as EditText).text?.toString())
                    view.clearFocus()
                    val imm = activity.getSystemService(android.content.Context.INPUT_METHOD_SERVICE)
                        as android.view.inputmethod.InputMethodManager
                    imm.hideSoftInputFromWindow(view.windowToken, 0)
                    true
                } else {
                    false
                }
            }
        }
        val pill = LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            background = GradientDrawable().apply {
                cornerRadius = dp(18f).toFloat()
                setColor(nativeTheme.card)
            }
            // Sized to match the reload glyph's visual weight, not the reload button's tap box.
            addView(faviconIcon, LinearLayout.LayoutParams(dp(22f), dp(22f)).apply {
                marginStart = dp(9f)
            })
            addView(urlField, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f))
            addView(reloadButton, LinearLayout.LayoutParams(dp(30f), ViewGroup.LayoutParams.MATCH_PARENT).apply {
                marginEnd = dp(3f)
            })
            addView(clearButton, LinearLayout.LayoutParams(dp(30f), ViewGroup.LayoutParams.MATCH_PARENT).apply {
                marginEnd = dp(3f)
            })
        }

        webViewUrlField = urlField
        webViewFaviconIcon = faviconIcon
        webViewBackButton = backButton
        webViewForwardButton = forwardButton

        // Single row: back | forward | URL pill (flex) | share | close. Every button gets the
        // same fixed height as the pill so glyphs of different sizes stay vertically centered.
        fun fixed(view: View) = view.apply {
            minimumWidth = dp(40f)
        }
        return LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(nativeTheme.background)
            clipToPadding = false
            addView(LinearLayout(activity).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                addView(fixed(backButton), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, dp(36f)))
                addView(fixed(forwardButton), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, dp(36f)))
                addView(pill, LinearLayout.LayoutParams(0, dp(36f), 1f).apply {
                    setMargins(dp(6f), 0, dp(6f), 0)
                })
                addView(fixed(shareButton), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, dp(36f)))
                addView(fixed(closeButton), LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, dp(36f)))
            }, LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT,
            ).apply { setMargins(dp(8f), dp(8f), dp(8f), dp(8f)) })
            addView(View(activity).apply {
                setBackgroundColor(nativeTheme.border)
            }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, max(1, dp(0.5f))))
            // Keep the row below the status bar.
            ViewCompat.setOnApplyWindowInsetsListener(this) { v, insets ->
                val sys = insets.getInsets(WindowInsetsCompat.Type.systemBars()).top
                v.setPadding(0, sys, 0, 0)
                insets
            }
            ViewCompat.requestApplyInsets(this)
        }
    }

    private fun navigateWebViewTo(webView: WebView, raw: String?) {
        val trimmed = raw?.trim().orEmpty()
        if (trimmed.isEmpty()) return
        val uri = android.net.Uri.parse(trimmed)
        webView.loadUrl(if (uri.scheme != null) trimmed else "https://$trimmed")
    }

    private fun shareCurrentWebViewUrl(activity: MainActivity, webView: WebView) {
        val url = webView.url ?: return
        val intent = android.content.Intent(android.content.Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(android.content.Intent.EXTRA_TEXT, url)
        }
        activity.startActivity(android.content.Intent.createChooser(intent, null))
    }

    /** Dismiss the currently presented webview, if any. */
    fun closeWebView() = onUi { closeWebViewNow() }

    /** Evaluate JavaScript in the currently presented webview. */
    fun evalWebView(js: String?) = onUi {
        if (js.isNullOrBlank()) return@onUi
        activeWebView?.evaluateJavascript(js, null)
    }

    @JvmStatic
    fun handleBackPressed(): Boolean {
        val webView = activeWebView ?: return false
        if (webView.canGoBack()) {
            webView.goBack()
        } else {
            closeWebViewNow()
        }
        return true
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
                applyFloatingButtonTheme(this)
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
                applyDictationPreviewTheme(this)
                textSize = 15f
                maxLines = 3
                setPadding(dp(14f), dp(10f), dp(14f), dp(10f))
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
            background = roundedBackground(withAlpha(nativeTheme.overlay, 242), 16f)
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
            setTextColor(nativeTheme.textPrimary)
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
            view.background = roundedBackground(withAlpha(nativeTheme.overlay, 242), 16f)
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

    // Route the webview through the in-app SOCKS proxy (the web tunnel), then load.
    // ProxyController.setProxyOverride is process-global while set, so it's cleared
    // on close; socks5:// carries the page's localhost:* traffic to the proxy.
    private fun applyWebViewProxy(activity: MainActivity, socksProxy: String?, load: () -> Unit) {
        if (socksProxy.isNullOrBlank() || !WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE)) {
            load()
            return
        }
        val proxyConfig = ProxyConfig.Builder().addProxyRule("socks5://$socksProxy").build()
        webViewProxyActive = true
        ProxyController.getInstance().setProxyOverride(
            proxyConfig,
            { command -> activity.runOnUiThread(command) },
            { load() },
        )
    }

    private fun clearWebViewProxy() {
        if (!webViewProxyActive) return
        webViewProxyActive = false
        if (WebViewFeature.isFeatureSupported(WebViewFeature.PROXY_OVERRIDE)) {
            ProxyController.getInstance().clearProxyOverride({ command -> command.run() }, {})
        }
    }

    private fun closeWebViewNow() {
        clearWebViewProxy()
        val root = rootView
        val overlay = webViewOverlay
        val webView = activeWebView
        val callbackId = activeWebViewCallbackId
        webViewOverlay = null
        activeWebView = null
        activeWebViewCallbackId = 0
        webViewUrlField = null
        webViewFaviconIcon = null
        webViewBackButton = null
        webViewForwardButton = null
        if (overlay != null && root != null) {
            root.removeView(overlay)
        }
        webView?.destroy()
        if (callbackId > 0) {
            MainActivity.nativeWebViewDismiss(callbackId)
        }
    }

    /** Refresh the top bar (URL, favicon, back/forward) to match the webview. */
    private fun updateWebViewChrome(webView: WebView, configTitle: String) {
        webViewBackButton?.let {
            val canGoBack = webView.canGoBack()
            it.isEnabled = canGoBack
            it.alpha = if (canGoBack) 1f else 0.35f
        }
        webViewForwardButton?.let {
            val canGoForward = webView.canGoForward()
            it.isEnabled = canGoForward
            it.alpha = if (canGoForward) 1f else 0.35f
        }
        // Don't stomp on text the user is actively typing.
        val field = webViewUrlField
        if (field != null && !field.hasFocus()) {
            val uri = android.net.Uri.parse(webView.url ?: "")
            val display = uri.host ?: configTitle.takeIf { it.isNotBlank() } ?: webView.url
            if (!display.isNullOrBlank()) {
                field.setText(display)
            }
        }
    }

    /** Hide the previous page's favicon while a new page loads. */
    private fun resetWebViewFavicon() {
        webViewFaviconIcon?.setImageDrawable(null)
        webViewFaviconIcon?.visibility = View.GONE
    }

    private fun updateWebViewFavicon(icon: android.graphics.Bitmap) {
        webViewFaviconIcon?.setImageBitmap(icon)
        webViewFaviconIcon?.visibility = View.VISIBLE
    }

    private data class WebViewConfig(
        val url: String,
        val title: String,
        val messageHandlerName: String?,
        val interceptNavigation: Boolean,
        val socksProxy: String?,
    )

    private fun parseWebViewConfig(configJson: String?): WebViewConfig? {
        if (configJson.isNullOrBlank()) return null
        return try {
            val obj = org.json.JSONObject(configJson)
            WebViewConfig(
                url = obj.optString("url"),
                title = obj.optString("title", ""),
                messageHandlerName = obj.optString("messageHandlerName", "").takeIf { it.isNotBlank() },
                interceptNavigation = obj.optBoolean("interceptNavigation", false),
                socksProxy = obj.optString("socksProxy", "").takeIf { it.isNotBlank() },
            )
        } catch (e: Throwable) {
            null
        }
    }

    /** Exposed as `window.<name>` so the page can post messages to Rust. */
    private class WebViewMessageBridge(private val callbackId: Int) {
        @android.webkit.JavascriptInterface
        fun postMessage(message: String?) {
            if (BuildConfig.DEBUG) {
                android.util.Log.i("zedra", "[debug:webview] bridge postMessage cb=$callbackId msg=$message")
            }
            MainActivity.nativeWebViewMessage(callbackId, message ?: "")
        }
    }

    private class WebViewControllerClient(
        private val callbackId: Int,
        private val config: WebViewConfig,
    ) : WebViewClient() {
        override fun shouldOverrideUrlLoading(
            view: WebView,
            request: android.webkit.WebResourceRequest,
        ): Boolean {
            if (!config.interceptNavigation) return false
            val allow = MainActivity.nativeWebViewNavigate(
                callbackId,
                request.url.toString(),
            )
            // Returning true means we handled (i.e. blocked) the navigation.
            return !allow
        }

        override fun onPageStarted(view: WebView, url: String?, favicon: android.graphics.Bitmap?) {
            super.onPageStarted(view, url, favicon)
            if (BuildConfig.DEBUG) {
                android.util.Log.i("zedra", "[debug:webview] onPageStarted url=$url")
            }
            NativePresentations.resetWebViewFavicon()
            NativePresentations.updateWebViewChrome(view, config.title)
        }

        override fun onPageFinished(view: WebView, url: String?) {
            super.onPageFinished(view, url)
            if (BuildConfig.DEBUG) {
                android.util.Log.i("zedra", "[debug:webview] onPageFinished url=$url")
            }
            NativePresentations.updateWebViewChrome(view, config.title)
        }

        // Previously unimplemented, so a blocked/failed load (cleartext policy, host
        // unreachable) silently rendered a blank page with no signal anywhere in the logs.
        override fun onReceivedError(
            view: WebView,
            request: android.webkit.WebResourceRequest,
            error: android.webkit.WebResourceError,
        ) {
            super.onReceivedError(view, request, error)
            if (BuildConfig.DEBUG) {
                android.util.Log.w(
                    "zedra",
                    "[debug:webview] onReceivedError url=${request.url} " +
                        "isMainFrame=${request.isForMainFrame} code=${error.errorCode} " +
                        "description=${error.description}",
                )
            }
        }

        override fun onReceivedHttpError(
            view: WebView,
            request: android.webkit.WebResourceRequest,
            errorResponse: android.webkit.WebResourceResponse,
        ) {
            super.onReceivedHttpError(view, request, errorResponse)
            if (BuildConfig.DEBUG) {
                android.util.Log.w(
                    "zedra",
                    "[debug:webview] onReceivedHttpError url=${request.url} " +
                        "isMainFrame=${request.isForMainFrame} status=${errorResponse.statusCode}",
                )
            }
        }

        override fun doUpdateVisitedHistory(view: WebView, url: String?, isReload: Boolean) {
            super.doUpdateVisitedHistory(view, url, isReload)
            NativePresentations.updateWebViewChrome(view, config.title)
        }
    }

    private fun loadWebView(webView: WebView, url: String) {
        if (activeWebView === webView) {
            webView.loadUrl(url)
        }
    }

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

    private fun withAlpha(color: Int, alpha: Int): Int {
        return Color.argb(
            alpha.coerceIn(0, 255),
            Color.red(color),
            Color.green(color),
            Color.blue(color),
        )
    }

    private fun applyDialogTheme(dialog: AlertDialog) {
        dialog.window?.setBackgroundDrawable(roundedBackground(nativeTheme.overlay, 28f))
        dialog.getButton(android.content.DialogInterface.BUTTON_POSITIVE)
            ?.setTextColor(nativeTheme.textPrimary)
        dialog.getButton(android.content.DialogInterface.BUTTON_NEGATIVE)
            ?.setTextColor(nativeTheme.textSecondary)
        dialog.getButton(android.content.DialogInterface.BUTTON_NEUTRAL)
            ?.setTextColor(nativeTheme.textSecondary)
        dialog.findViewById<View>(android.R.id.content)?.let { applyTextColors(it) }
    }

    private fun applyTextColors(view: View) {
        when (view) {
            // Buttons keep the color set explicitly by the call site (positive/negative/neutral),
            // and EditText keeps its own text/hint colors. Walking past them would re-tint
            // user-typed content and override per-button colors.
            is Button, is EditText -> {}
            is TextView -> {
                val text = view.text?.toString().orEmpty()
                val color = when (text) {
                    "✓" -> nativeTheme.accentGreen
                    "!" -> nativeTheme.accentYellow
                    else -> if (view.textSize <= 13f * view.resources.displayMetrics.scaledDensity) {
                        nativeTheme.textSecondary
                    } else {
                        nativeTheme.textPrimary
                    }
                }
                view.setTextColor(color)
            }
            is ImageView -> view.imageTintList = ColorStateList.valueOf(nativeTheme.textPrimary)
        }
        if (view is ViewGroup) {
            for (index in 0 until view.childCount) {
                applyTextColors(view.getChildAt(index))
            }
        }
    }

    private fun applyFloatingButtonTheme(button: ImageButton) {
        button.background = roundedBackground(withAlpha(nativeTheme.overlay, 230), 999f)
        button.imageTintList = ColorStateList.valueOf(nativeTheme.textPrimary)
    }

    private fun applyDictationPreviewTheme(preview: TextView) {
        preview.setTextColor(nativeTheme.textPrimary)
        preview.background = roundedBackground(withAlpha(nativeTheme.overlay, 238), 18f)
    }

    private fun dragHandle(): View {
        val activity = requireActivity()
        val handle = View(activity).apply {
            background = GradientDrawable().apply {
                shape = GradientDrawable.RECTANGLE
                cornerRadius = dp(2f).toFloat()
                setColor(nativeTheme.textMuted)
            }
        }
        val wrap = FrameLayout(activity).apply {
            setPadding(0, dp(8f), 0, dp(8f))
            addView(handle, FrameLayout.LayoutParams(dp(36f), dp(4f)).apply {
                gravity = Gravity.CENTER_HORIZONTAL
            })
        }
        return wrap
    }

    private fun pickerHeader(title: String, message: String?): LinearLayout {
        val activity = requireActivity()
        return LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20f), dp(12f), dp(20f), dp(12f))
            addView(TextView(activity).apply {
                text = title
                textSize = 18f
                setTextColor(nativeTheme.textPrimary)
                typeface = loraTypeface(activity)
                includeFontPadding = false
            })
            if (!message.isNullOrBlank()) {
                addView(TextView(activity).apply {
                    text = message
                    textSize = 13f
                    setTextColor(nativeTheme.textSecondary)
                    typeface = loraTypeface(activity)
                    includeFontPadding = false
                    setPadding(0, dp(8f), 0, 0)
                })
            }
        }
    }

    private fun selectionHeader(text: String, primary: Boolean): TextView {
        val activity = requireActivity()
        return TextView(activity).apply {
            this.text = text
            textSize = if (primary) 20f else 14f
            setTextColor(if (primary) nativeTheme.textPrimary else nativeTheme.textSecondary)
            typeface = loraTypeface(activity)
            setPadding(
                dp(24f),
                if (primary) dp(24f) else dp(8f),
                dp(24f),
                if (primary) dp(4f) else dp(16f),
            )
            maxLines = 2
        }
    }

    private var cachedLora: android.graphics.Typeface? = null
    private fun loraTypeface(ctx: android.content.Context): android.graphics.Typeface? {
        cachedLora?.let { return it }
        val tf = ResourcesCompat.getFont(ctx, R.font.lora)
        cachedLora = tf
        return tf
    }

    // Icon names are pipeline slugs (`assets/icons/<slug>.svg`); the matching Android
    // drawable is `ic_<slug>` with hyphens swapped for underscores — identical to
    // `androidDrawableName` in android/build.gradle (the `ic_` prefix keeps the name
    // identifier-safe regardless of the slug).
    private fun iconRes(name: String?): Int {
        if (name.isNullOrBlank()) return 0
        val activity = activity ?: return 0
        return activity.resources.getIdentifier(drawableName(name), "drawable", activity.packageName)
    }

    private fun drawableName(slug: String): String = "ic_${slug.replace('-', '_')}"

    private fun agentIconRes(name: String?): Int = iconRes(name)

    private fun selectionIconRes(name: String?): Int = iconRes(name)

    private fun alertButtonColor(style: Int): Int = when (style) {
        2 -> nativeTheme.accentRed       // Destructive
        1 -> nativeTheme.textSecondary   // Cancel
        else -> nativeTheme.textPrimary  // Default
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
            1 -> nativeTheme.accentGreen
            2 -> nativeTheme.accentYellow
            3 -> nativeTheme.accentRed
            else -> nativeTheme.textSecondary
        }
    }
}
