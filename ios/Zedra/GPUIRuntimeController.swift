import Foundation
import UIKit
import ZedraFFI

@_silgen_name("gpui_ios_set_keyboard_accessory_view")
private func gpui_ios_set_keyboard_accessory_view(_ viewPtr: UnsafeMutableRawPointer?)

@_silgen_name("gpui_ios_request_frame_forced")
private func gpui_ios_request_frame_forced(_ windowPtr: UnsafeMutableRawPointer?)

@_silgen_name("gpui_ios_handle_view_resize")
private func gpui_ios_handle_view_resize(
    _ windowPtr: UnsafeMutableRawPointer?, _ widthPts: Float, _ heightPts: Float)

@_silgen_name("gpui_ios_set_software_keyboard_visible")
private func gpui_ios_set_software_keyboard_visible(_ visible: Bool)

@_silgen_name("zedra_firebase_initialize")
private func zedra_firebase_initialize()

@_silgen_name("zedra_ios_send_key_input")
private func zedra_ios_send_key_input(_ key: UnsafePointer<CChar>)

@_silgen_name("zedra_ios_app_will_terminate")
private func zedra_ios_app_will_terminate()

final class GPUIRuntimeController: NSObject {
    private var gpuiApp: UnsafeMutableRawPointer?
    private var gpuiWindow: UnsafeMutableRawPointer?
    private var displayLink: CADisplayLink?
    private let keyboardAccessoryController = KeyboardSupporter()

    func launch() {
        zedra_firebase_initialize()

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
        url.absoluteString.withCString { zedra_deeplink_received($0) }
    }

    func applicationWillEnterForeground() {
        gpui_ios_will_enter_foreground(gpuiApp)
        if displayLink == nil, gpuiWindow != nil {
            startDisplayLink()
        }
    }

    func applicationDidBecomeActive() {
        gpui_ios_did_become_active(gpuiApp)
        pushSafeAreaInsets()
    }

    func applicationWillResignActive() {
        keyboardAccessoryController.stopRepeating()
        gpui_ios_will_resign_active(gpuiApp)
    }

    func applicationDidEnterBackground() {
        keyboardAccessoryController.stopRepeating()
        gpui_ios_did_enter_background(gpuiApp)
        zedra_ios_app_did_enter_background()
        stopDisplayLink()
    }

    func applicationWillTerminate() {
        keyboardAccessoryController.stopRepeating()
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
        guard
            let info = notification.userInfo,
            let endFrame = (info[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue
        else {
            return
        }

        let heightPx = UInt32(endFrame.height * UIScreen.main.scale)
        zedra_ios_set_keyboard_height(heightPx)
        gpui_ios_set_software_keyboard_visible(heightPx > 0)
    }

    @objc
    func keyboardWillHide(_ notification: Notification) {
        keyboardAccessoryController.stopRepeating()
        zedra_ios_set_keyboard_height(0)
        gpui_ios_set_software_keyboard_visible(false)
    }

    @objc
    private func orientationDidChange() {
        pushSafeAreaInsets()
        pushWindowSize()
    }

    private func sendKeyboardAccessoryKey(_ key: String) {
        key.withCString { zedra_ios_send_key_input($0) }
    }

    @objc
    func renderFrame() {
        guard let gpuiWindow else { return }
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
            width: width
        ) { [weak self] key in
            self?.sendKeyboardAccessoryKey(key)
        }
        gpui_ios_set_keyboard_accessory_view(Unmanaged.passUnretained(bar).toOpaque())
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
