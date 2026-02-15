import SwiftUI
import Combine
import ZedraFFI

/// Observable app state that bridges Rust backend → SwiftUI
///
/// Polls the Rust backend each frame (via CADisplayLink) and publishes
/// state changes to SwiftUI views.
class AppState: ObservableObject {
    /// Connection status: 0=disconnected, 1=connecting, 2=connected, 3=error
    @Published var connectionStatus: Int32 = 0
    @Published var connectionError: String = ""
    @Published var transportInfo: String = ""
    @Published var terminalOutput: String = ""

    /// CADisplayLink timer for 60 FPS frame processing
    let displayLink: AnyPublisher<Date, Never>

    var isConnected: Bool { connectionStatus == 2 }

    private var displayLinkInstance: CADisplayLink?
    private let frameSubject = PassthroughSubject<Date, Never>()

    init() {
        displayLink = frameSubject.eraseToAnyPublisher()
        setupDisplayLink()
    }

    deinit {
        displayLinkInstance?.invalidate()
    }

    private func setupDisplayLink() {
        let link = CADisplayLink(target: self, selector: #selector(onFrame))
        link.add(to: .main, forMode: .common)
        displayLinkInstance = link
    }

    @objc private func onFrame(_ link: CADisplayLink) {
        frameSubject.send(Date())
    }

    /// Poll Rust state each frame and update published properties
    func pollRustState() {
        // Connection status
        let status = zedra_get_connection_status()
        if status != connectionStatus {
            connectionStatus = status
        }

        // Error message
        if status == 3 {
            if let ptr = zedra_get_connection_error() {
                connectionError = String(cString: ptr)
                zedra_free_string(ptr)
            }
        } else {
            if !connectionError.isEmpty {
                connectionError = ""
            }
        }

        // Transport info
        if let ptr = zedra_get_transport_info() {
            let info = String(cString: ptr)
            zedra_free_string(ptr)
            if info != transportInfo {
                transportInfo = info
            }
        } else if !transportInfo.isEmpty {
            transportInfo = ""
        }

        // Terminal output
        if let ptr = zedra_get_terminal_output() {
            let output = String(cString: ptr)
            zedra_free_string(ptr)
            terminalOutput += output
        }
    }
}
