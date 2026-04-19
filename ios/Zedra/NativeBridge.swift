import FirebaseAnalytics
import FirebaseCore
import FirebaseCrashlytics
import Foundation
import UIKit
import ZedraFFI

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
