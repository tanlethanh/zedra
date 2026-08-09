import Foundation
import UIKit
import ZedraFFI

@_silgen_name("gpui_ios_set_keyboard_accessory_view")
private func gpui_ios_set_keyboard_accessory_view(_ viewPtr: UnsafeMutableRawPointer?)

@_silgen_name("gpui_ios_handle_keyboard_accessory_action")
private func gpui_ios_handle_keyboard_accessory_action(
    _ windowPtr: UnsafeMutableRawPointer?, _ action: UnsafePointer<CChar>?
) -> Bool

@_silgen_name("gpui_ios_handle_key_bar_action")
private func gpui_ios_handle_key_bar_action(
    _ windowPtr: UnsafeMutableRawPointer?, _ action: UnsafePointer<CChar>?
) -> Bool

@_silgen_name("gpui_ios_handle_key_bar_text")
private func gpui_ios_handle_key_bar_text(
    _ windowPtr: UnsafeMutableRawPointer?, _ text: UnsafePointer<CChar>?
) -> Bool

@_silgen_name("gpui_ios_hide_keyboard")
private func gpui_ios_hide_keyboard(_ windowPtr: UnsafeMutableRawPointer?)

@_silgen_name("gpui_ios_show_keyboard")
private func gpui_ios_show_keyboard(_ windowPtr: UnsafeMutableRawPointer?)

@_silgen_name("gpui_ios_request_frame_forced")
private func gpui_ios_request_frame_forced(_ windowPtr: UnsafeMutableRawPointer?)

@_silgen_name("gpui_ios_handle_view_resize")
private func gpui_ios_handle_view_resize(
    _ windowPtr: UnsafeMutableRawPointer?, _ widthPts: Float, _ heightPts: Float)

@_silgen_name("gpui_ios_set_software_keyboard_visible")
private func gpui_ios_set_software_keyboard_visible(_ visible: Bool)

#if !ZEDRA_NO_TELEMETRY
@_silgen_name("zedra_firebase_initialize")
private func zedra_firebase_initialize()
#endif

@_silgen_name("zedra_ios_app_will_terminate")
private func zedra_ios_app_will_terminate()

@_silgen_name("zedra_ios_app_will_enter_foreground")
private func zedra_ios_app_will_enter_foreground()

final class GPUIRuntimeController: NSObject {
    private static weak var activeController: GPUIRuntimeController?

    private var gpuiApp: UnsafeMutableRawPointer?
    private var gpuiWindow: UnsafeMutableRawPointer?
    private var displayLink: CADisplayLink?
    private let keyboardAccessoryController = KeyboardSupporter()
    /// Second instance of the same bar, hosted above the safe area while the
    /// keyboard is down. Separate instance because each one owns its own buttons
    /// and repeat timer.
    private let pinnedKeyBarController = KeyboardSupporter()
    private var pinnedKeyBarView: UIView?
    private var pinnedKeyBarVisible = false
    // Keep the keys clear of the home indicator's swipe region without spending the
    // whole 34pt safe-area inset on empty space.
    private static let maxPinnedKeyBarBottomPadding: CGFloat = 22.0
    private var extendedKeypad = false
    private var keypadCmdSlot = false
    private var keyboardHeightPts: CGFloat = 0
    /// Full-screen presentations covering the GPUI window. Counted because the
    /// webview and the QR scanner can hand off to each other.
    private var occludingPresentations = 0
    private var mainWindowOccluded: Bool { occludingPresentations > 0 }

    /// Dismiss the main GPUI window's software keyboard. Manual-focus surfaces
    /// (the terminal) keep `keyboard_session_requested` set, so a native sheet
    /// presented over them leaves the keyboard up and re-shows it each frame.
    /// Clearing the request here lets a presentation resign it cleanly.
    static func dismissMainWindowKeyboard() {
        guard let window = activeController?.gpuiWindow else { return }
        gpui_ios_hide_keyboard(window)
    }

    /// Called by full-screen presentations on appear/disappear. While the GPUI
    /// window is covered it renders nothing and ignores keyboard notifications —
    /// otherwise it keeps painting behind the presentation and re-lays out around
    /// the presentation's own keyboard.
    static func beginMainWindowOcclusion() {
        DispatchQueue.main.async { activeController?.updateOcclusion(delta: 1) }
    }

    static func endMainWindowOcclusion() {
        DispatchQueue.main.async { activeController?.updateOcclusion(delta: -1) }
    }

    private func updateOcclusion(delta: Int) {
        let wasOccluded = mainWindowOccluded
        occludingPresentations = max(0, occludingPresentations + delta)
        guard mainWindowOccluded != wasOccluded else { return }
        if mainWindowOccluded {
            keyboardAccessoryController.stopRepeating()
            hidePinnedKeyBar()
            // Zero the keyboard before gating the notifications, or the hide that
            // follows is swallowed and GPUI keeps the stale inset.
            keyboardHeightPts = 0
            zedra_ios_set_keyboard_height(0)
            gpui_ios_set_software_keyboard_visible(false)
            Self.dismissMainWindowKeyboard()
            stopDisplayLink()
        } else {
            if displayLink == nil, gpuiWindow != nil {
                startDisplayLink()
            }
            updatePinnedKeyBar(visible: pinnedKeyBarVisible)
        }
    }

    func launch() {
        Self.activeController = self
#if !ZEDRA_NO_TELEMETRY
        zedra_firebase_initialize()
#endif

        gpuiApp = gpui_ios_initialize()
        zedra_launch_gpui()
        gpui_ios_did_finish_launching(gpuiApp)
        gpuiWindow = gpui_ios_get_window()
        if gpuiWindow != nil {
            setupKeyboardAccessoryView()
            startDisplayLink()
        }

        pushScreenScale()
        DispatchQueue.main.async { [weak self] in
            self?.pushSafeAreaInsets()
        }

        NotificationCenter.default.addObserver(
            self,
            selector: #selector(orientationDidChange),
            name: UIDevice.orientationDidChangeNotification,
            object: nil
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(keyboardWillShow(_:)),
            name: UIResponder.keyboardWillShowNotification,
            object: nil
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(keyboardWillHide(_:)),
            name: UIResponder.keyboardWillHideNotification,
            object: nil
        )
    }

    func handleOpenURL(_ url: URL) {
        ZedraDeeplink.route(url: url)
    }

    func applicationWillEnterForeground() {
        zedra_ios_app_will_enter_foreground()
        gpui_ios_will_enter_foreground(gpuiApp)
        if displayLink == nil, gpuiWindow != nil, !mainWindowOccluded {
            startDisplayLink()
        }
    }

    func applicationDidBecomeActive() {
        gpui_ios_did_become_active(gpuiApp)
        pushSafeAreaInsets()
        // Rust pushes bar visibility once; re-apply in case the window was not
        // attached yet when it did.
        updatePinnedKeyBar(visible: pinnedKeyBarVisible)
    }

    func applicationWillResignActive() {
        keyboardAccessoryController.stopRepeating()
        pinnedKeyBarController.stopRepeating()
        gpui_ios_will_resign_active(gpuiApp)
    }

    func applicationDidEnterBackground() {
        keyboardAccessoryController.stopRepeating()
        pinnedKeyBarController.stopRepeating()
        keyboardAccessoryController.cancelComposing()
        pinnedKeyBarController.cancelComposing()
        gpui_ios_did_enter_background(gpuiApp)
        zedra_ios_app_did_enter_background()
        stopDisplayLink()
    }

    func applicationWillTerminate() {
        keyboardAccessoryController.stopRepeating()
        pinnedKeyBarController.stopRepeating()
        stopDisplayLink()
        zedra_ios_app_will_terminate()
        gpui_ios_will_terminate(gpuiApp)
    }

    @objc
    func pushWindowSize() {
        guard let gpuiWindow else { return }
        let size = UIScreen.main.bounds.size
        gpui_ios_handle_view_resize(gpuiWindow, Float(size.width), Float(size.height))
    }

    @objc
    func pushSafeAreaInsets() {
        guard let window = uiWindow else { return }
        let scale = UIScreen.main.scale
        let insets = window.safeAreaInsets
        zedra_ios_set_safe_area_insets(
            Float(insets.top * scale),
            Float(insets.bottom * scale),
            Float(insets.left * scale),
            Float(insets.right * scale)
        )
    }

    @objc
    func keyboardWillShow(_ notification: Notification) {
        // The keyboard belongs to whatever covers the window, not to the terminal.
        guard !mainWindowOccluded else { return }
        guard
            let info = notification.userInfo,
            let endFrame = (info[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue
        else {
            return
        }

        let heightPx = UInt32(endFrame.height * UIScreen.main.scale)
        keyboardHeightPts = endFrame.height
        zedra_ios_set_keyboard_height(heightPx)
        gpui_ios_set_software_keyboard_visible(heightPx > 0)
        // The composer lives in the bar, so the bar has to clear its own keyboard.
        if pinnedKeyBarController.isComposing {
            layoutPinnedKeyBar()
        }
    }

    @objc
    func keyboardWillHide(_ notification: Notification) {
        guard !mainWindowOccluded else { return }
        keyboardAccessoryController.stopRepeating()
        keyboardHeightPts = 0
        zedra_ios_set_keyboard_height(0)
        gpui_ios_set_software_keyboard_visible(false)
    }

    @objc
    private func orientationDidChange() {
        pushSafeAreaInsets()
        pushWindowSize()
        // The bar is laid out with frames for a fixed width; rebuild it on rotation.
        pinnedKeyBarView?.removeFromSuperview()
        pinnedKeyBarView = nil
        updatePinnedKeyBar(visible: pinnedKeyBarVisible)
    }

    private func sendKeyboardAccessoryKey(_ key: String) {
        guard let gpuiWindow else { return }
        key.withCString { action in
            _ = gpui_ios_handle_keyboard_accessory_action(gpuiWindow, action)
        }
        if key == "dismiss_keyboard" {
            Self.dismissMainWindowKeyboard()
        }
    }

    @objc
    func renderFrame() {
        guard let gpuiWindow else { return }
        // Modifiers are consumed by keys from either the bar or the software
        // keyboard, so the highlight cannot be refreshed from bar presses alone.
        refreshKeypadModifiers()
        if zedra_ios_check_pending_frame() {
            gpui_ios_request_frame_forced(gpuiWindow)
        } else {
            gpui_ios_request_frame(gpuiWindow)
        }
    }

    private var uiWindow: UIWindow? {
        guard let gpuiWindow, let windowPtr = gpui_ios_get_ui_window(gpuiWindow) else {
            return nil
        }
        return Unmanaged<UIWindow>.fromOpaque(windowPtr).takeUnretainedValue()
    }

    private func setupKeyboardAccessoryView() {
        let width = UIScreen.main.bounds.width
        let bar = keyboardAccessoryController.makeAccessoryView(
            width: width,
            extended: extendedKeypad,
            cmdSlot: keypadCmdSlot,
            sendKey: { [weak self] key in
                self?.sendKeyboardAccessoryKey(key)
            },
            sendComposedText: { [weak self] text in
                self?.sendComposedText(text)
            },
            requestTerminalKeyboard: { [weak self] in
                self?.requestTerminalKeyboard()
            }
        )
        gpui_ios_set_keyboard_accessory_view(Unmanaged.passUnretained(bar).toOpaque())
    }

    /// Rust pushes this when the setting changes; both bars rebuild their rows.
    static func setKeypadLayout(extended: Bool, cmdSlot: Bool) {
        DispatchQueue.main.async {
            activeController?.applyKeypadLayout(extended: extended, cmdSlot: cmdSlot)
        }
    }

    private func applyKeypadLayout(extended enabled: Bool, cmdSlot useCmdSlot: Bool) {
        guard extendedKeypad != enabled || keypadCmdSlot != useCmdSlot else { return }
        extendedKeypad = enabled
        keypadCmdSlot = useCmdSlot
        // UIKit sizes the keyboard-attached bar from the view it is handed, so
        // that one is rebuilt rather than resized in place.
        setupKeyboardAccessoryView()
        pinnedKeyBarView?.removeFromSuperview()
        pinnedKeyBarView = nil
        updatePinnedKeyBar(visible: pinnedKeyBarVisible)
    }

    /// Give first responder back to the terminal, keeping the keyboard on screen.
    private func requestTerminalKeyboard() {
        guard let gpuiWindow else { return }
        gpui_ios_show_keyboard(gpuiWindow)
    }

    /// Straight into the terminal's input handler. The generic text-input entry
    /// point dispatches key events through GPUI's keymap instead, which never
    /// reached the terminal from a bar hosted outside the keyboard.
    private func sendComposedText(_ text: String) {
        guard let gpuiWindow else {
            NSLog("key-bar: composed text dropped, no GPUI window")
            return
        }
        let handled = text.withCString { gpui_ios_handle_key_bar_text(gpuiWindow, $0) }
        if !handled {
            NSLog("key-bar: composed text rejected by the input handler")
        }
    }

    static func setKeyboardAccessoryTheme(isDark: Bool) {
        DispatchQueue.main.async {
            activeController?.keyboardAccessoryController.applyTheme(isDark: isDark)
            activeController?.pinnedKeyBarController.applyTheme(isDark: isDark)
        }
    }

    /// Tapping the terminal surface drops the composer and its keyboard.
    static func cancelKeypadComposer() {
        DispatchQueue.main.async {
            activeController?.keyboardAccessoryController.cancelComposing()
            activeController?.pinnedKeyBarController.cancelComposing()
        }
    }

    static func setPinnedKeyBarVisible(_ visible: Bool) {
        DispatchQueue.main.async {
            activeController?.updatePinnedKeyBar(visible: visible)
        }
    }

    private func updatePinnedKeyBar(visible: Bool) {
        pinnedKeyBarVisible = visible
        guard !mainWindowOccluded else {
            hidePinnedKeyBar()
            return
        }
        // Composing raises a keyboard, which is exactly when Rust hides the rows;
        // the bar has to stay up or the composer would vanish behind it.
        guard visible || pinnedKeyBarController.isComposing else {
            hidePinnedKeyBar()
            return
        }
        guard let window = uiWindow else { return }

        let width = window.bounds.width
        // The full safe-area inset (34pt) leaves a large empty band under the keys;
        // clear the home indicator and no more.
        let bottomPadding = min(window.safeAreaInsets.bottom, Self.maxPinnedKeyBarBottomPadding)
        let bar =
            pinnedKeyBarView
            ?? {
                let bar = pinnedKeyBarController.makeAccessoryView(
                    width: width,
                    bottomPadding: bottomPadding,
                    extended: extendedKeypad,
                    cmdSlot: keypadCmdSlot,
                    sendKey: { [weak self] key in
                        self?.sendPinnedKeyBarKey(key)
                    },
                    sendComposedText: { [weak self] text in
                        self?.sendComposedText(text)
                    },
                    requestTerminalKeyboard: { [weak self] in
                        self?.requestTerminalKeyboard()
                    },
                    needsLayout: { [weak self] in
                        self?.layoutPinnedKeyBar()
                    }
                )
                window.addSubview(bar)
                pinnedKeyBarView = bar
                return bar
            }()
        bar.isHidden = false
        window.bringSubviewToFront(bar)
        layoutPinnedKeyBar()
    }

    /// The pinned bar runs with no keyboard session, so the accessory entry point
    /// rejects it; `handle_key_bar_action` reaches the same input handler, which
    /// encodes keys against the terminal's live mode.
    private func sendPinnedKeyBarKey(_ key: String) {
        let handled =
            key.withCString { action in
                gpui_ios_handle_key_bar_action(gpuiWindow, action)
            }
        if !handled {
            key.withCString { zedra_ios_send_key_input($0) }
        }
    }

    /// Sticky modifier state lives with the terminal; mirror it into both bars so
    /// the armed and locked highlights match what the next keystroke will carry.
    private func refreshKeypadModifiers() {
        let mask = zedra_ios_key_bar_modifier_mask()
        keyboardAccessoryController.setModifierMask(mask)
        pinnedKeyBarController.setModifierMask(mask)
    }

    /// Re-place the pinned bar after its own layout changed (row count, composer).
    private func layoutPinnedKeyBar() {
        guard !mainWindowOccluded else {
            hidePinnedKeyBar()
            return
        }
        guard let window = uiWindow, let bar = pinnedKeyBarView else { return }
        let composing = pinnedKeyBarController.isComposing
        // While composing the bar rides its own keyboard; otherwise it sits above
        // the home indicator.
        let bottomPadding = composing
            ? 0
            : min(window.safeAreaInsets.bottom, Self.maxPinnedKeyBarBottomPadding)
        let height = pinnedKeyBarController.keysHeight + bottomPadding
        let bottom = composing ? window.bounds.height - keyboardHeightPts : window.bounds.height
        bar.frame = CGRect(
            x: 0,
            y: bottom - height,
            width: window.bounds.width,
            height: height
        )
        window.bringSubviewToFront(bar)
        zedra_ios_set_pinned_key_bar_height(UInt32(height * UIScreen.main.scale))
    }

    private func hidePinnedKeyBar() {
        pinnedKeyBarController.stopRepeating()
        pinnedKeyBarView?.isHidden = true
        zedra_ios_set_pinned_key_bar_height(0)
    }

    private func startDisplayLink() {
        let displayLink = CADisplayLink(target: self, selector: #selector(renderFrame))
        displayLink.add(to: .main, forMode: .common)
        self.displayLink = displayLink
    }

    private func stopDisplayLink() {
        displayLink?.invalidate()
        displayLink = nil
    }

    private func pushScreenScale() {
        zedra_ios_set_screen_scale(Float(UIScreen.main.scale))
    }
}
