import UIKit

/// Modifier-key bitmask matching Rust `key_encoding::Mods`
/// (shift = bit 0, alt = bit 1, ctrl = bit 2).
struct AccessoryMods: OptionSet {
    let rawValue: UInt8

    static let shift = AccessoryMods(rawValue: 0b001)
    static let alt = AccessoryMods(rawValue: 0b010)
    static let ctrl = AccessoryMods(rawValue: 0b100)
}

/// Accessory bar shown above whichever keyboard is active. The bar itself only
/// owns the primary row (Esc / Tab / arrows / Enter) plus a `•••` toggle. When
/// the toggle is tapped, the host swaps the system keyboard for `FullKeyboardView`
/// and updates the toggle label to `✕`.
@objcMembers
final class KeyboardSupporter: NSObject {
    private enum KeySpecKind {
        case key(name: String, fixedMods: AccessoryMods = [])
        case togglePanel
    }

    private struct KeySpec {
        let label: String
        let kind: KeySpecKind
        let repeats: Bool
    }

    private let primaryRow: [KeySpec] = [
        KeySpec(label: "Esc", kind: .key(name: "escape"), repeats: false),
        KeySpec(label: "Tab", kind: .key(name: "tab"), repeats: false),
        KeySpec(label: "←", kind: .key(name: "left"), repeats: true),
        KeySpec(label: "↓", kind: .key(name: "down"), repeats: true),
        KeySpec(label: "↑", kind: .key(name: "up"), repeats: true),
        KeySpec(label: "→", kind: .key(name: "right"), repeats: true),
        KeySpec(label: "⏎", kind: .key(name: "enter"), repeats: false),
        KeySpec(label: "•••", kind: .togglePanel, repeats: false),
    ]

    private let rowHeight: CGFloat = 44.0
    private let repeatInitialDelay: TimeInterval = 0.35
    private let repeatInterval: TimeInterval = 0.06

    private(set) var accessoryView: UIView?
    private weak var container: UIView?
    private weak var topBorder: UIView?
    private weak var leftKeyboardCornerFill: UIView?
    private weak var rightKeyboardCornerFill: UIView?
    private var buttons: [(button: UIButton, spec: KeySpec)] = []
    private var sendKey: ((String, UInt8) -> Void)?
    private var togglePanel: (() -> Void)?
    private var repeatTimer: Timer?
    private var repeatingKey: (name: String, mods: UInt8)?
    private var isDarkTheme = true
    private(set) var isPanelOpen = false

    func makeAccessoryView(
        width: CGFloat,
        sendKey: @escaping (String, UInt8) -> Void,
        togglePanel: @escaping () -> Void
    ) -> UIView {
        stopRepeating()
        self.sendKey = sendKey
        self.togglePanel = togglePanel
        buttons.removeAll()

        let container = UIView(frame: CGRect(x: 0, y: 0, width: width, height: rowHeight))
        container.clipsToBounds = false
        self.container = container

        let border = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 0.33))
        container.addSubview(border)
        topBorder = border

        let cornerFillWidth: CGFloat = 18.0
        let cornerFillHeight: CGFloat = 12.0
        let leftFill = UIView(
            frame: CGRect(x: 0, y: rowHeight, width: cornerFillWidth, height: cornerFillHeight)
        )
        let rightFill = UIView(
            frame: CGRect(
                x: width - cornerFillWidth,
                y: rowHeight,
                width: cornerFillWidth,
                height: cornerFillHeight
            )
        )
        container.addSubview(leftFill)
        container.addSubview(rightFill)
        leftKeyboardCornerFill = leftFill
        rightKeyboardCornerFill = rightFill

        let buttonWidth = width / CGFloat(primaryRow.count)
        for (index, spec) in primaryRow.enumerated() {
            let button = UIButton(type: .system)
            button.frame = CGRect(
                x: buttonWidth * CGFloat(index),
                y: 0,
                width: buttonWidth,
                height: rowHeight
            )
            button.setTitle(spec.label, for: .normal)
            button.titleLabel?.font = .systemFont(ofSize: 16.0)
            button.tag = index
            button.layer.cornerRadius = 6.0
            button.addTarget(self, action: #selector(buttonTouchDown(_:)), for: .touchDown)
            button.addTarget(self, action: #selector(buttonTouchUpInside(_:)), for: .touchUpInside)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchUpOutside)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchCancel)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchDragExit)
            container.addSubview(button)
            buttons.append((button, spec))
        }

        accessoryView = container
        applyTheme(isDark: isDarkTheme)
        refreshToggleLabel()
        return container
    }

    func applyTheme(isDark: Bool) {
        isDarkTheme = isDark

        let backgroundColor =
            isDark
            ? UIColor(red: 0.055, green: 0.047, blue: 0.047, alpha: 0.96)
            : UIColor(red: 0.961, green: 0.961, blue: 0.961, alpha: 0.98)
        let borderColor =
            isDark
            ? UIColor(white: 1.0, alpha: 0.12)
            : UIColor(white: 0.0, alpha: 0.10)

        container?.backgroundColor = backgroundColor
        topBorder?.backgroundColor = borderColor
        leftKeyboardCornerFill?.backgroundColor = backgroundColor
        rightKeyboardCornerFill?.backgroundColor = backgroundColor

        let interfaceStyle: UIUserInterfaceStyle = isDark ? .dark : .light
        if #available(iOS 13.0, *) {
            container?.overrideUserInterfaceStyle = interfaceStyle
        }
        let foregroundColor =
            isDark
            ? UIColor(red: 0.96, green: 0.96, blue: 0.96, alpha: 1.0)
            : UIColor(red: 0.102, green: 0.102, blue: 0.102, alpha: 1.0)
        for (button, _) in buttons {
            button.setTitleColor(foregroundColor, for: .normal)
            button.tintColor = foregroundColor
            if #available(iOS 13.0, *) {
                button.overrideUserInterfaceStyle = interfaceStyle
            }
        }
    }

    /// Set whether the host currently displays the in-app full keyboard. The
    /// `•••` button flips to `✕` so users have a way back to the system
    /// keyboard, and any pending key repeat is cancelled.
    func setPanelOpen(_ open: Bool) {
        isPanelOpen = open
        refreshToggleLabel()
        if open {
            stopRepeating()
        }
    }

    private func refreshToggleLabel() {
        for (button, spec) in buttons {
            if case .togglePanel = spec.kind {
                button.setTitle(isPanelOpen ? "✕" : "•••", for: .normal)
            }
        }
    }

    @objc
    func stopRepeating() {
        repeatTimer?.invalidate()
        repeatTimer = nil
        repeatingKey = nil
    }

    private func spec(for sender: UIButton) -> KeySpec? {
        for (button, spec) in buttons where button === sender {
            return spec
        }
        return nil
    }

    @objc
    private func buttonTouchDown(_ sender: UIButton) {
        guard let spec = spec(for: sender), spec.repeats else {
            return
        }
        if case .key(let name, let fixedMods) = spec.kind {
            sendKey?(name, fixedMods.rawValue)
            startRepeating(name: name, mods: fixedMods.rawValue)
        }
    }

    @objc
    private func buttonTouchUpInside(_ sender: UIButton) {
        guard let spec = spec(for: sender) else {
            stopRepeating()
            return
        }
        if spec.repeats {
            stopRepeating()
            return
        }
        switch spec.kind {
        case .key(let name, let fixedMods):
            sendKey?(name, fixedMods.rawValue)
        case .togglePanel:
            togglePanel?()
        }
    }

    private func startRepeating(name: String, mods: UInt8) {
        stopRepeating()
        repeatingKey = (name, mods)

        // Accessory arrow keys should behave like held hardware keys: one immediate
        // keystroke, then repeat until UIKit reports any release or cancellation.
        let timer = Timer(timeInterval: repeatInterval, repeats: true) { [weak self] _ in
            guard let self, let target = self.repeatingKey,
                target.name == name, target.mods == mods
            else {
                return
            }
            self.sendKey?(name, mods)
        }
        timer.fireDate = Date(timeIntervalSinceNow: repeatInitialDelay)
        repeatTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }
}
