import SwiftUI
import ZedraFFI

/// Terminal view — displays terminal output and accepts keyboard input
struct TerminalView: View {
    @ObservedObject var appState: AppState
    @State private var inputText = ""
    @FocusState private var isInputFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            // Terminal output
            ScrollViewReader { proxy in
                ScrollView {
                    Text(appState.terminalOutput)
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundColor(Color(hex: 0xabb2bf))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(8)
                        .id("terminal-bottom")
                }
                .background(Color(hex: 0x1e1e1e))
                .onChange(of: appState.terminalOutput) { _ in
                    withAnimation {
                        proxy.scrollTo("terminal-bottom", anchor: .bottom)
                    }
                }
            }

            // Input bar
            HStack(spacing: 8) {
                // Quick-action keys
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 4) {
                        QuickKey(label: "ESC") { zedra_send_key("escape") }
                        QuickKey(label: "TAB") { zedra_send_key("tab") }
                        QuickKey(label: "↑") { zedra_send_key("up") }
                        QuickKey(label: "↓") { zedra_send_key("down") }
                        QuickKey(label: "←") { zedra_send_key("left") }
                        QuickKey(label: "→") { zedra_send_key("right") }
                        QuickKey(label: "CTRL-C") { zedra_send_input("\u{03}") }
                        QuickKey(label: "CTRL-D") { zedra_send_input("\u{04}") }
                    }
                    .padding(.horizontal, 4)
                }
                .frame(height: 32)
            }
            .background(Color(hex: 0x21252b))

            // Text input field
            HStack(spacing: 8) {
                TextField("Type here...", text: $inputText)
                    .textFieldStyle(.plain)
                    .font(.system(size: 14, design: .monospaced))
                    .foregroundColor(.white)
                    .autocapitalization(.none)
                    .disableAutocorrection(true)
                    .focused($isInputFocused)
                    .onSubmit {
                        sendInput()
                    }

                Button(action: sendInput) {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.title3)
                        .foregroundColor(Color(hex: 0x61afef))
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color(hex: 0x282c34))
        }
        .toolbar {
            ToolbarItem(placement: .navigationBarTrailing) {
                Button("Disconnect") {
                    zedra_disconnect()
                }
                .foregroundColor(.red)
            }
        }
        .onAppear {
            isInputFocused = true
        }
    }

    private func sendInput() {
        guard !inputText.isEmpty else { return }
        zedra_send_input(inputText + "\r")
        inputText = ""
    }
}

/// A quick-action key button in the terminal toolbar
struct QuickKey: View {
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.system(size: 11, weight: .medium, design: .monospaced))
                .foregroundColor(Color(hex: 0xabb2bf))
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(Color(hex: 0x3e4451))
                .cornerRadius(4)
        }
    }
}
