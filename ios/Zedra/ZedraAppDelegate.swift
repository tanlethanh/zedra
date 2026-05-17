import UIKit

@objc(ZedraAppDelegate)
final class ZedraAppDelegate: UIResponder, UIApplicationDelegate {
    private let runtime = GPUIRuntimeController()

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        ZedraDeltaPushBridge.configure()
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
