import Foundation
import UserNotifications
import ZedraFFI

/// App-level deeplink handling. Routes `zedra://` URLs from any source (system URL
/// intents, notification taps) into the Rust core via `zedra_deeplink_received`.
///
/// This is an app feature, independent of Delta. Delta owns auth and backend
/// interaction (push-token registration, sign-in); it does not handle deeplinks.
enum ZedraDeeplink {
    /// Route a `zedra://` URL into the Rust core.
    static func route(url: URL) {
        route(urlString: url.absoluteString)
    }

    /// Route a raw `zedra://` URL string into the Rust core.
    static func route(urlString: String) {
        guard !urlString.isEmpty else { return }
        NSLog("deeplink: routing url %@", urlString)
        urlString.withCString { zedra_deeplink_received($0) }
    }

    /// Extract the `deeplink` value from an APNS payload and route it. The host
    /// embeds the URL under the top-level `deeplink` key of the push payload.
    static func route(notificationUserInfo userInfo: [AnyHashable: Any]) {
        guard let deeplink = userInfo["deeplink"] as? String, !deeplink.isEmpty else {
            NSLog("deeplink: notification tap: no deeplink in userInfo")
            return
        }
        NSLog("deeplink: notification tap: routing deeplink %@", deeplink)
        route(urlString: deeplink)
    }

    /// Install the shared notification-center delegate. Call once at launch.
    static func installNotificationDelegate() {
        UNUserNotificationCenter.current().delegate = notificationDelegate
    }

    private static let notificationDelegate = ZedraNotificationDelegate()
}

/// Notification presentation + tap routing. Presentation behavior and deeplink
/// routing are app concerns; push-token registration stays with Delta.
private final class ZedraNotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    // Use the completion-handler delegate variants, not the `async` ones. The system
    // delivers these on the main thread; the `async` variants run on a background
    // cooperative executor, where UIKit's post-tap state-restoration snapshot trips a
    // "must be made on main thread" assertion (SIGABRT). Routing the deeplink drives the
    // GPUI runtime, which touches UIKit, so it must run on the main thread regardless.
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list, .sound, .badge])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        ZedraDeeplink.route(notificationUserInfo: response.notification.request.content.userInfo)
        completionHandler()
    }
}
