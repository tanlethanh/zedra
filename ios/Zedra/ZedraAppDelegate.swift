import UIKit

@objc(ZedraAppDelegate)
final class ZedraAppDelegate: UIResponder, UIApplicationDelegate {
    private let runtime = GPUIRuntimeController()

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        runtime.launch()
        return true
    }

    func application(
        _ app: UIApplication,
        open url: URL,
        options: [UIApplication.OpenURLOptionsKey: Any] = [:]
    ) -> Bool {
        runtime.handleOpenURL(url)
        return true
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
