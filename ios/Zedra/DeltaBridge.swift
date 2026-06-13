import AuthenticationServices
import Foundation
import GoogleSignIn
import UIKit
import UserNotifications

private final class ZedraDeltaCStringStorage {
    static let shared = ZedraDeltaCStringStorage()

    private var buffers: [String: UnsafeMutablePointer<CChar>] = [:]
    private let lock = NSLock()

    func pointer(for key: String, value: String?) -> UnsafePointer<CChar>? {
        lock.lock()
        defer { lock.unlock() }

        if let existing = buffers.removeValue(forKey: key) {
            free(existing)
        }

        guard let value, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return nil
        }

        guard let duplicated = strdup(value) else {
            return nil
        }
        buffers[key] = duplicated
        return UnsafePointer(duplicated)
    }
}

@_silgen_name("zedra_ios_delta_apple_sign_in_result")
private func zedra_ios_delta_apple_sign_in_result(
    _ callbackID: UInt32,
    _ idToken: UnsafePointer<CChar>,
    _ email: UnsafePointer<CChar>?
)

@_silgen_name("zedra_ios_delta_apple_sign_in_error")
private func zedra_ios_delta_apple_sign_in_error(
    _ callbackID: UInt32,
    _ message: UnsafePointer<CChar>
)

@_silgen_name("zedra_ios_delta_google_sign_in_result")
private func zedra_ios_delta_google_sign_in_result(
    _ callbackID: UInt32,
    _ idToken: UnsafePointer<CChar>,
    _ email: UnsafePointer<CChar>?
)

@_silgen_name("zedra_ios_delta_google_sign_in_error")
private func zedra_ios_delta_google_sign_in_error(
    _ callbackID: UInt32,
    _ message: UnsafePointer<CChar>
)

@_silgen_name("zedra_ios_delta_push_token_result")
private func zedra_ios_delta_push_token_result(
    _ callbackID: UInt32,
    _ provider: UnsafePointer<CChar>,
    _ token: UnsafePointer<CChar>,
    _ environment: UnsafePointer<CChar>?
)

@_silgen_name("zedra_ios_delta_push_token_error")
private func zedra_ios_delta_push_token_error(
    _ callbackID: UInt32,
    _ message: UnsafePointer<CChar>
)

enum ZedraDeltaGoogleSignIn {
    static func start(callbackID: UInt32) {
        DispatchQueue.main.async {
            do {
                _ = try googleConfiguration()
                guard let presenter = topViewController() else {
                    fail(callbackID, "Google sign-in requires an active window")
                    return
                }

                GIDSignIn.sharedInstance.signIn(withPresenting: presenter) { result, error in
                    if let error {
                        fail(callbackID, error.localizedDescription)
                        return
                    }
                    guard let user = result?.user else {
                        fail(callbackID, "Google sign-in was cancelled")
                        return
                    }
                    guard let idToken = user.idToken?.tokenString, !idToken.isEmpty else {
                        fail(callbackID, "Google sign-in did not return an ID token")
                        return
                    }

                    sendResult(callbackID: callbackID, idToken: idToken, email: user.profile?.email)
                }
            } catch {
                fail(callbackID, error.localizedDescription)
            }
        }
    }

    static func handleURL(_ url: URL) -> Bool {
        GIDSignIn.sharedInstance.handle(url)
    }

    private static func googleConfiguration() throws {
        guard
            let path = Bundle.main.path(forResource: "GoogleService-Info", ofType: "plist"),
            let plist = NSDictionary(contentsOfFile: path) as? [String: Any]
        else {
            throw NSError(
                domain: "dev.zedra.delta.google",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Missing ios/Zedra/GoogleService-Info.plist"]
            )
        }
        guard let clientID = plist["CLIENT_ID"] as? String, !clientID.isEmpty else {
            throw NSError(
                domain: "dev.zedra.delta.google",
                code: 2,
                userInfo: [NSLocalizedDescriptionKey: "GoogleService-Info.plist is missing CLIENT_ID"]
            )
        }
        guard let reversedClientID = plist["REVERSED_CLIENT_ID"] as? String, !reversedClientID.isEmpty else {
            throw NSError(
                domain: "dev.zedra.delta.google",
                code: 3,
                userInfo: [NSLocalizedDescriptionKey: "GoogleService-Info.plist is missing REVERSED_CLIENT_ID"]
            )
        }
        guard bundleURLSchemes().contains(reversedClientID) else {
            throw NSError(
                domain: "dev.zedra.delta.google",
                code: 4,
                userInfo: [NSLocalizedDescriptionKey: "Google URL scheme is not configured in the iOS target"]
            )
        }
        GIDSignIn.sharedInstance.configuration = GIDConfiguration(clientID: clientID)
    }

    private static func bundleURLSchemes() -> Set<String> {
        let urlTypes = Bundle.main.object(forInfoDictionaryKey: "CFBundleURLTypes") as? [[String: Any]]
        let schemes = urlTypes?.flatMap { item in
            item["CFBundleURLSchemes"] as? [String] ?? []
        } ?? []
        return Set(schemes)
    }

    private static func topViewController() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let root = scenes
            .flatMap(\.windows)
            .first(where: { $0.isKeyWindow })?
            .rootViewController
        return topViewController(from: root)
    }

    private static func topViewController(from root: UIViewController?) -> UIViewController? {
        if let navigation = root as? UINavigationController {
            return topViewController(from: navigation.visibleViewController)
        }
        if let tab = root as? UITabBarController {
            return topViewController(from: tab.selectedViewController)
        }
        if let presented = root?.presentedViewController {
            return topViewController(from: presented)
        }
        return root
    }

    private static func sendResult(callbackID: UInt32, idToken: String, email: String?) {
        idToken.withCString { tokenPtr in
            if let email, !email.isEmpty {
                email.withCString { emailPtr in
                    zedra_ios_delta_google_sign_in_result(callbackID, tokenPtr, emailPtr)
                }
            } else {
                zedra_ios_delta_google_sign_in_result(callbackID, tokenPtr, nil)
            }
        }
    }

    private static func fail(_ callbackID: UInt32, _ message: String) {
        message.withCString { zedra_ios_delta_google_sign_in_error(callbackID, $0) }
    }
}

enum ZedraDeltaPushBridge {
    private static let lock = NSLock()
    private static var pendingCallbackID: UInt32?
    private static let notificationDelegate = ZedraDeltaNotificationDelegate()

    static func configure() {
        UNUserNotificationCenter.current().delegate = notificationDelegate
    }

    static func requestToken(callbackID: UInt32) {
        DispatchQueue.main.async {
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { granted, error in
                if let error {
                    fail(callbackID, error.localizedDescription)
                    return
                }
                guard granted else {
                    fail(callbackID, "Push notification permission was not granted")
                    return
                }

                lock.lock()
                if pendingCallbackID != nil {
                    lock.unlock()
                    fail(callbackID, "Push token request already in progress")
                    return
                }
                pendingCallbackID = callbackID
                lock.unlock()

                DispatchQueue.main.async {
                    UIApplication.shared.registerForRemoteNotifications()
                }
            }
        }
    }

    static func didRegister(deviceToken: Data) {
        guard let callbackID = takePendingCallbackID() else {
            return
        }
        let token = deviceToken.map { String(format: "%02x", $0) }.joined()
        let provider = "apns"
        #if DEBUG
        let environment = "development"
        #else
        let environment = "production"
        #endif
        provider.withCString { providerPtr in
            token.withCString { tokenPtr in
                environment.withCString { environmentPtr in
                    zedra_ios_delta_push_token_result(callbackID, providerPtr, tokenPtr, environmentPtr)
                }
            }
        }
    }

    static func didFail(error: Error) {
        guard let callbackID = takePendingCallbackID() else {
            return
        }
        fail(callbackID, error.localizedDescription)
    }

    private static func takePendingCallbackID() -> UInt32? {
        lock.lock()
        defer { lock.unlock() }
        let callbackID = pendingCallbackID
        pendingCallbackID = nil
        return callbackID
    }

    private static func fail(_ callbackID: UInt32, _ message: String) {
        message.withCString { zedra_ios_delta_push_token_error(callbackID, $0) }
    }
}

private final class ZedraDeltaNotificationDelegate: NSObject, UNUserNotificationCenterDelegate {
    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        return [.banner, .list, .sound, .badge]
    }
}

private final class ZedraDeltaAppleSignInCoordinator: NSObject, ASAuthorizationControllerDelegate, ASAuthorizationControllerPresentationContextProviding {
    let callbackID: UInt32

    init(callbackID: UInt32) {
        self.callbackID = callbackID
    }

    func authorizationController(controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
        guard let credential = authorization.credential as? ASAuthorizationAppleIDCredential else {
            fail("Apple sign-in returned an unexpected credential type")
            return
        }
        guard
            let tokenData = credential.identityToken,
            let idToken = String(data: tokenData, encoding: .utf8),
            !idToken.isEmpty
        else {
            fail("Apple sign-in did not return an identity token")
            return
        }
        let email = credential.email
        idToken.withCString { tokenPtr in
            if let email, !email.isEmpty {
                email.withCString { emailPtr in
                    zedra_ios_delta_apple_sign_in_result(callbackID, tokenPtr, emailPtr)
                }
            } else {
                zedra_ios_delta_apple_sign_in_result(callbackID, tokenPtr, nil)
            }
        }
        ZedraDeltaAppleSignIn.clearCoordinator(callbackID: callbackID)
    }

    func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: Error) {
        let asError = error as? ASAuthorizationError
        if asError?.code == .canceled {
            fail("Apple sign-in was cancelled")
        } else {
            fail(error.localizedDescription)
        }
        ZedraDeltaAppleSignIn.clearCoordinator(callbackID: callbackID)
    }

    func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        return scenes.flatMap(\.windows).first(where: { $0.isKeyWindow }) ?? UIWindow()
    }

    private func fail(_ message: String) {
        message.withCString { zedra_ios_delta_apple_sign_in_error(callbackID, $0) }
    }
}

enum ZedraDeltaAppleSignIn {
    private static let lock = NSLock()
    private static var coordinators: [UInt32: ZedraDeltaAppleSignInCoordinator] = [:]

    static func start(callbackID: UInt32) {
        DispatchQueue.main.async {
            let request = ASAuthorizationAppleIDProvider().createRequest()
            request.requestedScopes = [.email]

            let coord = ZedraDeltaAppleSignInCoordinator(callbackID: callbackID)
            lock.lock()
            coordinators[callbackID] = coord
            lock.unlock()

            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = coord
            controller.presentationContextProvider = coord
            controller.performRequests()
        }
    }

    static func clearCoordinator(callbackID: UInt32) {
        lock.lock()
        coordinators.removeValue(forKey: callbackID)
        lock.unlock()
    }
}

@_cdecl("ios_start_delta_apple_sign_in")
func ios_start_delta_apple_sign_in(_ callbackID: UInt32) {
    ZedraDeltaAppleSignIn.start(callbackID: callbackID)
}

@_cdecl("ios_start_delta_google_sign_in")
func ios_start_delta_google_sign_in(_ callbackID: UInt32) {
    ZedraDeltaGoogleSignIn.start(callbackID: callbackID)
}

@_cdecl("ios_request_delta_push_token")
func ios_request_delta_push_token(_ callbackID: UInt32) {
    ZedraDeltaPushBridge.requestToken(callbackID: callbackID)
}

@_cdecl("ios_get_delta_device_name")
func ios_get_delta_device_name() -> UnsafePointer<CChar>? {
    let deviceName = UIDevice.current.name.trimmingCharacters(in: .whitespacesAndNewlines)
    if !deviceName.isEmpty {
        return ZedraDeltaCStringStorage.shared.pointer(for: "device_name", value: deviceName)
    }

    let hostname = ProcessInfo.processInfo.hostName.trimmingCharacters(in: .whitespacesAndNewlines)
    return ZedraDeltaCStringStorage.shared.pointer(for: "device_name", value: hostname)
}
