import UIKit

@objc(ZedraAppDelegate)
final class ZedraAppDelegate: UIResponder, UIApplicationDelegate {
    private let runtime = GPUIRuntimeController()

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        ZedraDeeplink.installNotificationDelegate()
        runtime.launch()
        return true
    }

    func application(
        _ app: UIApplication,
        open url: URL,
        options: [UIApplication.OpenURLOptionsKey: Any] = [:]
    ) -> Bool {
        if ZedraDeltaGoogleSignIn.handleURL(url) {
            return true
        }
        #if DEBUG
        // `la-test` deeplinks are a debug-only on-device scaffold. Never compiled into
        // release builds, where Delta drives the aggregate Live Activity instead.
        if #available(iOS 16.2, *), LiveActivityController.handleTestURL(url) {
            return true
        }
        #endif
        runtime.handleOpenURL(url)
        return true
    }

    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        ZedraDeltaPushBridge.didRegister(deviceToken: deviceToken)
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        ZedraDeltaPushBridge.didFail(error: error)
    }

    func applicationWillEnterForeground(_ application: UIApplication) {
        runtime.applicationWillEnterForeground()
    }

    func applicationDidBecomeActive(_ application: UIApplication) {
        runtime.applicationDidBecomeActive()
    }

    func applicationWillResignActive(_ application: UIApplication) {
        runtime.applicationWillResignActive()
    }

    func applicationDidEnterBackground(_ application: UIApplication) {
        runtime.applicationDidEnterBackground()
    }

    func applicationWillTerminate(_ application: UIApplication) {
        runtime.applicationWillTerminate()
    }
}
