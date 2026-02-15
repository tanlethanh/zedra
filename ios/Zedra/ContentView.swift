import SwiftUI
import ZedraFFI

struct ContentView: View {
    @StateObject private var appState = AppState()

    var body: some View {
        TabView {
            NavigationStack {
                TerminalTab(appState: appState)
            }
            .tabItem {
                Label("Terminal", systemImage: "terminal")
            }

            NavigationStack {
                EditorTab()
            }
            .tabItem {
                Label("Editor", systemImage: "doc.text")
            }

            NavigationStack {
                GitTab()
            }
            .tabItem {
                Label("Git", systemImage: "arrow.triangle.branch")
            }
        }
        .tint(Color(hex: 0x61afef))
        .preferredColorScheme(.dark)
        .onReceive(appState.displayLink) { _ in
            zedra_process_frame()
            appState.pollRustState()
        }
    }
}

// MARK: - Terminal Tab

struct TerminalTab: View {
    @ObservedObject var appState: AppState

    var body: some View {
        Group {
            if appState.isConnected {
                TerminalView(appState: appState)
            } else {
                ConnectView(appState: appState)
            }
        }
        .navigationTitle("Zedra")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if !appState.transportInfo.isEmpty {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Text(appState.transportInfo)
                        .font(.caption2)
                        .foregroundColor(.secondary)
                }
            }
        }
    }
}

// MARK: - Editor Tab (placeholder)

struct EditorTab: View {
    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "doc.text.fill")
                .font(.system(size: 48))
                .foregroundColor(Color(hex: 0x61afef))
            Text("Code Editor")
                .font(.title2)
                .foregroundColor(.white)
            Text("Connect to a host to browse files")
                .font(.subheadline)
                .foregroundColor(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(hex: 0x1e1e1e))
        .navigationTitle("Editor")
        .navigationBarTitleDisplayMode(.inline)
    }
}

// MARK: - Git Tab (placeholder)

struct GitTab: View {
    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "arrow.triangle.branch")
                .font(.system(size: 48))
                .foregroundColor(Color(hex: 0x61afef))
            Text("Git")
                .font(.title2)
                .foregroundColor(.white)
            Text("Git integration coming soon")
                .font(.subheadline)
                .foregroundColor(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(hex: 0x1e1e1e))
        .navigationTitle("Git")
        .navigationBarTitleDisplayMode(.inline)
    }
}

// MARK: - Color Extension

extension Color {
    init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255.0,
            green: Double((hex >> 8) & 0xFF) / 255.0,
            blue: Double(hex & 0xFF) / 255.0
        )
    }
}
