import FirebaseAnalytics
import FirebaseCore
import FirebaseCrashlytics
import Foundation
import UIKit
import ZedraFFI
import ObjectiveC.runtime

private final class CStringStorage {
    static let shared = CStringStorage()

    private var buffers: [String: UnsafeMutablePointer<CChar>] = [:]
    private let lock = NSLock()

    func pointer(for key: String, value: String?) -> UnsafePointer<CChar>? {
        lock.lock()
        defer { lock.unlock() }

        if let existing = buffers.removeValue(forKey: key) {
            free(existing)
        }

        guard let value else {
            return nil
        }

        guard let duplicated = strdup(value) else {
            return nil
        }
        buffers[key] = duplicated
        return UnsafePointer(duplicated)
    }
}

private final class PresentationDismissDelegate: NSObject, UIAdaptivePresentationControllerDelegate {
    let callbackID: UInt32
    let isSelection: Bool
    var handled = false

    init(callbackID: UInt32, isSelection: Bool) {
        self.callbackID = callbackID
        self.isSelection = isSelection
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        guard !handled else { return }
        handled = true
        if isSelection {
            zedra_ios_selection_dismiss(callbackID)
        } else {
            zedra_ios_alert_dismiss(callbackID)
        }
    }
}

enum NativePresentationBridge {
    /// Returns the active key window, preferring foreground-active scenes.
    /// Falls back progressively to any visible window, then a last-resort empty window.
    static func activeWindow() -> UIWindow {
        let scenes = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .filter { $0.activationState == .foregroundActive }

        for scene in scenes {
            if let keyWindow = scene.windows.first(where: \.isKeyWindow) { return keyWindow }
        }
        for scene in scenes {
            if let visibleWindow = scene.windows.first(where: { !$0.isHidden }) { return visibleWindow }
        }
        return UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first ?? UIWindow(frame: UIScreen.main.bounds)
    }

    static func topViewController() -> UIViewController? {
        let keyWindow = activeWindow()
        var controller = keyWindow.rootViewController
        while let presented = controller?.presentedViewController {
            controller = presented
        }
        return controller
    }

    static func string(_ pointer: UnsafePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        return String(cString: pointer)
    }

    static func strings(count: Int32, labels: UnsafePointer<UnsafePointer<CChar>?>?) -> [String] {
        guard let labels, count > 0 else { return [] }
        return (0..<Int(count)).map { index in
            string(labels[index]) ?? "OK"
        }
    }

    static func styles(count: Int32, styles: UnsafePointer<Int32>?) -> [Int32] {
        guard let styles, count > 0 else { return Array(repeating: 0, count: Int(count)) }
        return (0..<Int(count)).map { styles[$0] }
    }
}

@_cdecl("ios_get_app_version")
func ios_get_app_version() -> UnsafePointer<CChar>? {
    let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
    return CStringStorage.shared.pointer(for: "app_version", value: version)
}

@_cdecl("ios_get_app_build_number")
func ios_get_app_build_number() -> UnsafePointer<CChar>? {
    let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String
    return CStringStorage.shared.pointer(for: "app_build", value: build)
}

@_cdecl("ios_get_documents_directory")
func ios_get_documents_directory() -> UnsafePointer<CChar>? {
    let path = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first?.path
    return CStringStorage.shared.pointer(for: "documents_directory", value: path)
}

@_cdecl("zedra_firebase_initialize")
func zedra_firebase_initialize_bridge() {
    FirebaseApp.configure()
}

@_cdecl("ios_present_alert")
func ios_present_alert(
    _ callbackID: UInt32,
    _ title: UnsafePointer<CChar>?,
    _ message: UnsafePointer<CChar>?,
    _ buttonCount: Int32,
    _ labels: UnsafePointer<UnsafePointer<CChar>?>?,
    _ styles: UnsafePointer<Int32>?
) {
    let titleString = NativePresentationBridge.string(title)
    let messageString = NativePresentationBridge.string(message)
    let buttonLabels = NativePresentationBridge.strings(count: buttonCount, labels: labels)
    let buttonStyles = NativePresentationBridge.styles(count: buttonCount, styles: styles)

    DispatchQueue.main.async {
        guard let presenter = NativePresentationBridge.topViewController() else { return }

        let alert = UIAlertController(title: titleString, message: messageString, preferredStyle: .alert)

        for index in 0..<buttonLabels.count {
            let style: UIAlertAction.Style
            switch buttonStyles[index] {
            case 1: style = .cancel
            case 2: style = .destructive
            default: style = .default
            }
            alert.addAction(UIAlertAction(title: buttonLabels[index], style: style) { _ in
                zedra_ios_alert_result(callbackID, Int32(index))
            })
        }

        presenter.present(alert, animated: true)
    }
}

@_cdecl("ios_present_selection")
func ios_present_selection(
    _ callbackID: UInt32,
    _ title: UnsafePointer<CChar>?,
    _ message: UnsafePointer<CChar>?,
    _ buttonCount: Int32,
    _ labels: UnsafePointer<UnsafePointer<CChar>?>?,
    _ styles: UnsafePointer<Int32>?
) {
    let titleString = NativePresentationBridge.string(title)
    let messageString = NativePresentationBridge.string(message)
    let buttonLabels = NativePresentationBridge.strings(count: buttonCount, labels: labels)
    let buttonStyles = NativePresentationBridge.styles(count: buttonCount, styles: styles)

    DispatchQueue.main.async {
        guard let presenter = NativePresentationBridge.topViewController() else { return }

        let sheet = UIAlertController(title: titleString, message: messageString, preferredStyle: .actionSheet)
        let delegate = PresentationDismissDelegate(callbackID: callbackID, isSelection: true)
        sheet.presentationController?.delegate = delegate
        objc_setAssociatedObject(sheet, "zedra_selection_delegate", delegate, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)

        if let popover = sheet.popoverPresentationController {
            popover.sourceView = presenter.view
            popover.sourceRect = CGRect(x: presenter.view.bounds.midX, y: presenter.view.bounds.midY, width: 1, height: 1)
            popover.permittedArrowDirections = []
        }

        for index in 0..<buttonLabels.count {
            let style: UIAlertAction.Style = buttonStyles[index] == 2 ? .destructive : .default
            sheet.addAction(UIAlertAction(title: buttonLabels[index], style: style) { _ in
                delegate.handled = true
                zedra_ios_selection_result(callbackID, Int32(index))
            })
        }

        presenter.present(sheet, animated: true)
    }
}

@_cdecl("ios_trigger_haptic")
func ios_trigger_haptic(_ kind: Int32) {
    DispatchQueue.main.async {
        switch kind {
        case 0: // ImpactLight
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        case 1: // ImpactMedium
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        case 2: // ImpactHeavy
            UIImpactFeedbackGenerator(style: .heavy).impactOccurred()
        case 3: // ImpactSoft
            UIImpactFeedbackGenerator(style: .soft).impactOccurred()
        case 4: // ImpactRigid
            UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
        case 5: // SelectionChanged
            UISelectionFeedbackGenerator().selectionChanged()
        case 6: // NotificationSuccess
            UINotificationFeedbackGenerator().notificationOccurred(.success)
        case 7: // NotificationWarning
            UINotificationFeedbackGenerator().notificationOccurred(.warning)
        case 8: // NotificationError
            UINotificationFeedbackGenerator().notificationOccurred(.error)
        default:
            break
        }
    }
}

@_cdecl("ios_open_url")
func ios_open_url(_ url: UnsafePointer<CChar>?) {
    guard let urlString = NativePresentationBridge.string(url), let nsURL = URL(string: urlString) else {
        return
    }

    DispatchQueue.main.async {
        UIApplication.shared.open(nsURL)
    }
}

@_cdecl("zedra_log_event")
func zedra_log_event(
    _ name: UnsafePointer<CChar>?,
    _ keys: UnsafePointer<UnsafePointer<CChar>?>?,
    _ values: UnsafePointer<UnsafePointer<CChar>?>?,
    _ count: Int32
) {
    guard let name = NativePresentationBridge.string(name) else { return }

    var params: [String: String] = [:]
    if let keys, let values, count > 0 {
        for index in 0..<Int(count) {
            guard let key = NativePresentationBridge.string(keys[index]), let value = NativePresentationBridge.string(values[index]) else {
                continue
            }
            params[key] = value
        }
    }

    Analytics.logEvent(name, parameters: params)
}

@_cdecl("zedra_record_error")
func zedra_record_error(
    _ message: UnsafePointer<CChar>?,
    _ file: UnsafePointer<CChar>?,
    _ line: Int32
) {
    guard let message = NativePresentationBridge.string(message) else { return }
    let fileString = NativePresentationBridge.string(file) ?? "unknown"
    let fullMessage = "[\(fileString):\(line)] \(message)"
    let error = NSError(domain: "dev.zedra.rust", code: 1, userInfo: [NSLocalizedDescriptionKey: fullMessage])
    Crashlytics.crashlytics().record(error: error)
}

@_cdecl("zedra_record_panic")
func zedra_record_panic(_ message: UnsafePointer<CChar>?, _ location: UnsafePointer<CChar>?) {
    guard let message = NativePresentationBridge.string(message) else { return }
    let locationString = NativePresentationBridge.string(location) ?? "unknown"
    let fullMessage = "Rust panic at \(locationString): \(message)"
    Crashlytics.crashlytics().log(fullMessage)
    let error = NSError(domain: "dev.zedra.rust.panic", code: 2, userInfo: [NSLocalizedDescriptionKey: fullMessage])
    Crashlytics.crashlytics().record(error: error)
}

@_cdecl("zedra_set_user_id")
func zedra_set_user_id(_ userID: UnsafePointer<CChar>?) {
    guard let userID = NativePresentationBridge.string(userID) else { return }
    Analytics.setUserID(userID)
    Crashlytics.crashlytics().setUserID(userID)
}

@_cdecl("zedra_set_custom_key")
func zedra_set_custom_key(_ key: UnsafePointer<CChar>?, _ value: UnsafePointer<CChar>?) {
    guard let key = NativePresentationBridge.string(key), let value = NativePresentationBridge.string(value) else {
        return
    }
    Crashlytics.crashlytics().setCustomValue(value, forKey: key)
}

@_cdecl("zedra_set_collection_enabled")
func zedra_set_collection_enabled(_ enabled: Int32) {
    let isEnabled = enabled != 0
    Analytics.setAnalyticsCollectionEnabled(isEnabled)
    Crashlytics.crashlytics().setCrashlyticsCollectionEnabled(isEnabled)
}
