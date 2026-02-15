import SwiftUI
import ZedraFFI

@main
struct ZedraApp: App {
    init() {
        zedra_init()

        // Pass screen dimensions to Rust
        let screen = UIScreen.main
        zedra_init_screen(
            Float(screen.bounds.width),
            Float(screen.bounds.height),
            Float(screen.scale)
        )
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}
