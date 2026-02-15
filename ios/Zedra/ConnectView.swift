import SwiftUI
import ZedraFFI

/// Connection form — SwiftUI equivalent of the GPUI ConnectView
struct ConnectView: View {
    @ObservedObject var appState: AppState
    @State private var host = ""
    @State private var port = "2123"

    var body: some View {
        VStack(spacing: 24) {
            Spacer()

            // Card
            VStack(spacing: 16) {
                // Title
                Text("Zedra")
                    .font(.title)
                    .foregroundColor(Color(hex: 0x61afef))

                Text("Connect to zedra-host daemon")
                    .font(.subheadline)
                    .foregroundColor(.secondary)

                // Host field
                VStack(alignment: .leading, spacing: 4) {
                    Text("Host")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    TextField("127.0.0.1", text: $host)
                        .textFieldStyle(.roundedBorder)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                        .keyboardType(.URL)
                }

                // Port field
                VStack(alignment: .leading, spacing: 4) {
                    Text("Port")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    TextField("2123", text: $port)
                        .textFieldStyle(.roundedBorder)
                        .keyboardType(.numberPad)
                }

                // Connect button
                Button(action: connect) {
                    if appState.connectionStatus == 1 {
                        ProgressView()
                            .tint(.white)
                            .frame(maxWidth: .infinity)
                    } else {
                        Text("Connect")
                            .frame(maxWidth: .infinity)
                    }
                }
                .buttonStyle(.borderedProminent)
                .tint(Color(hex: 0x61afef))
                .disabled(appState.connectionStatus == 1)

                // Error message
                if appState.connectionStatus == 3 {
                    Text(appState.connectionError)
                        .font(.caption)
                        .foregroundColor(.red)
                        .multilineTextAlignment(.center)
                }

                // Divider
                HStack {
                    Rectangle()
                        .fill(Color.secondary.opacity(0.3))
                        .frame(height: 1)
                    Text("or")
                        .font(.caption)
                        .foregroundColor(.secondary)
                    Rectangle()
                        .fill(Color.secondary.opacity(0.3))
                        .frame(height: 1)
                }

                // Scan QR button
                Button(action: scanQR) {
                    Label("Scan QR Code", systemImage: "qrcode.viewfinder")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .tint(Color(hex: 0x61afef))
            }
            .padding(24)
            .background(Color(hex: 0x282c34))
            .cornerRadius(12)
            .overlay(
                RoundedRectangle(cornerRadius: 12)
                    .stroke(Color(hex: 0x3e4451), lineWidth: 1)
            )
            .padding(.horizontal, 32)

            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(hex: 0x1e1e1e))
    }

    private func connect() {
        let h = host.isEmpty ? "127.0.0.1" : host
        let p = UInt16(port) ?? 2123
        zedra_connect(h, p)
    }

    private func scanQR() {
        // QR scanning will be implemented with AVFoundation
        // For now, log the intent
        print("QR scan requested")
    }
}
