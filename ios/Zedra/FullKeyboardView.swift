import UIKit

/// Host OS reported by `zedra_ios_active_host_os` — matches Rust
/// `key_encoding::HostOs`. The panel uses it to label / pick a layout that
/// matches the host the active terminal is talking to. `Unknown` is treated
/// as macOS per product direction.
enum HostOs: UInt8 {
    case unknown = 0
    case macOs = 1
    case linux = 2
    case windows = 3

    var displayName: String {
        switch self {
        case .unknown, .macOs: return "macOS"
        case .linux: return "Linux"
        case .windows: return "Windows"
        }
    }
}

/// Desktop-only key panel that replaces the system keyboard while a terminal
/// or agent session has focus. Visual chrome (background, foreground, border,
/// font size, row height) matches `KeyboardSupporter` so the panel reads as a
/// natural extension of the accessory bar above it. The supplied design mock
/// drives only position and the key set.
///
/// Layout (top → bottom):
///   Row 1: shift  Ctrl  Cmd   Home  End  PgUp  PgD   ⌫
///   Row 2: ~  @  $  *  ^  %  =  `
///   Row 3: <  >  (  )  {  }  [  ]
///   Reserved space below the key rows is intentionally left blank for
///   future additions (clipboard, snippets, agent macros, …).
final class FullKeyboardView: UIView {
    typealias SendKey = (String, UInt8) -> Void

    private struct AccessoryMods: OptionSet {
        let rawValue: UInt8

        static let shift = AccessoryMods(rawValue: 0b0001)
        static let alt = AccessoryMods(rawValue: 0b0010)
        static let ctrl = AccessoryMods(rawValue: 0b0100)
        /// Cmd / Super. Tracked so the panel can highlight an armed state;
        /// the Rust encoder ignores it because no legacy terminal byte
        /// encoding exists. Reserved for routing via a future host RPC.
        static let cmd = AccessoryMods(rawValue: 0b1000)
    }

    private enum KeyKind {
        case dispatch(name: String, fixedMods: AccessoryMods = [])
        case modifier(AccessoryMods)
    }

    private struct Key {
        let label: String
        let kind: KeyKind
        let repeats: Bool

        init(label: String, kind: KeyKind, repeats: Bool = false) {
            self.label = label
            self.kind = kind
            self.repeats = repeats
        }
    }

    private let modNavRow: [Key] = [
        Key(label: "Shift", kind: .modifier(.shift)),
        Key(label: "Ctrl", kind: .modifier(.ctrl)),
        Key(label: "Cmd", kind: .modifier(.cmd)),
        Key(label: "Home", kind: .dispatch(name: "home")),
        Key(label: "End", kind: .dispatch(name: "end")),
        Key(label: "PgUp", kind: .dispatch(name: "page_up"), repeats: true),
        Key(label: "PgD", kind: .dispatch(name: "page_down"), repeats: true),
        Key(label: "⌫", kind: .dispatch(name: "backspace"), repeats: true),
    ]

    private let symbolRow1: [Key] = [
        Key(label: "~", kind: .dispatch(name: "char:~")),
        Key(label: "@", kind: .dispatch(name: "char:@")),
        Key(label: "$", kind: .dispatch(name: "char:$")),
        Key(label: "*", kind: .dispatch(name: "char:*")),
        Key(label: "^", kind: .dispatch(name: "char:^")),
        Key(label: "%", kind: .dispatch(name: "char:%")),
        Key(label: "=", kind: .dispatch(name: "char:=")),
        Key(label: "`", kind: .dispatch(name: "char:`")),
    ]

    private let symbolRow2: [Key] = [
        Key(label: "<", kind: .dispatch(name: "char:<")),
        Key(label: ">", kind: .dispatch(name: "char:>")),
        Key(label: "(", kind: .dispatch(name: "char:(")),
        Key(label: ")", kind: .dispatch(name: "char:)")),
        Key(label: "{", kind: .dispatch(name: "char:{")),
        Key(label: "}", kind: .dispatch(name: "char:}")),
        Key(label: "[", kind: .dispatch(name: "char:[")),
        Key(label: "]", kind: .dispatch(name: "char:]")),
    ]

    private let rowHeight: CGFloat = 44
    private let repeatInitialDelay: TimeInterval = 0.35
    private let repeatInterval: TimeInterval = 0.06

    private let sendKey: SendKey
    private let hostOs: HostOs
    private weak var hostBadge: UILabel?
    private weak var topBorder: UIView?
    private var isDarkTheme = true
    private var armedMods: AccessoryMods = []
    private var rowsKeys: [[Key]] = []
    private var rowsButtons: [[UIButton]] = []
    private var keyButtons: [(button: UIButton, key: Key)] = []
    private var repeatTimer: Timer?
    private var repeatingKey: (name: String, mods: UInt8)?

    init(
        width: CGFloat,
        height: CGFloat,
        isDark: Bool,
        hostOs: HostOs,
        sendKey: @escaping SendKey
    ) {
        self.sendKey = sendKey
        self.hostOs = hostOs
        self.isDarkTheme = isDark
        super.init(frame: CGRect(x: 0, y: 0, width: width, height: height))
        autoresizingMask = [.flexibleWidth]
        clipsToBounds = false

        let border = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 0.33))
        border.autoresizingMask = [.flexibleWidth]
        addSubview(border)
        topBorder = border

        build()
        installHostBadge()
        applyTheme(isDark: isDark)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not implemented")
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

        self.backgroundColor = backgroundColor
        topBorder?.backgroundColor = borderColor

        if #available(iOS 13.0, *) {
            overrideUserInterfaceStyle = isDark ? .dark : .light
        }
        for (button, key) in keyButtons {
            styleButton(button, key: key)
        }
        applyHostBadgeColor()
    }

    private func installHostBadge() {
        let label = UILabel()
        label.text = hostOs.displayName
        label.font = .systemFont(ofSize: 10, weight: .medium)
        label.textAlignment = .right
        label.alpha = 0.55
        addSubview(label)
        hostBadge = label
        applyHostBadgeColor()
    }

    private func applyHostBadgeColor() {
        hostBadge?.textColor =
            isDarkTheme
            ? UIColor(red: 0.96, green: 0.96, blue: 0.96, alpha: 1.0)
            : UIColor(red: 0.102, green: 0.102, blue: 0.102, alpha: 1.0)
    }

    private func build() {
        for (button, _) in keyButtons {
            button.removeFromSuperview()
        }
        rowsKeys.removeAll()
        rowsButtons.removeAll()
        keyButtons.removeAll()

        for rowKeys in [modNavRow, symbolRow1, symbolRow2] {
            var rowButtons: [UIButton] = []
            for key in rowKeys {
                let button = makeButton(for: key)
                addSubview(button)
                rowButtons.append(button)
                keyButtons.append((button, key))
            }
            rowsKeys.append(rowKeys)
            rowsButtons.append(rowButtons)
        }
        setNeedsLayout()
    }

    private func makeButton(for key: Key) -> UIButton {
        let button = UIButton(type: .system)
        // Backspace glyph reads small at 16pt; bump it up while keeping the
        // button width aligned with the rest of the row.
        let fontSize: CGFloat
        if case .dispatch(let name, _) = key.kind, name == "backspace" {
            fontSize = 22
        } else {
            fontSize = 16
        }
        button.titleLabel?.font = .systemFont(ofSize: fontSize)
        button.setTitle(key.label, for: .normal)
        button.layer.cornerRadius = 6
        button.addTarget(self, action: #selector(buttonTouchDown(_:)), for: .touchDown)
        button.addTarget(self, action: #selector(buttonTouchUpInside(_:)), for: .touchUpInside)
        button.addTarget(self, action: #selector(stopRepeating), for: .touchUpOutside)
        button.addTarget(self, action: #selector(stopRepeating), for: .touchCancel)
        button.addTarget(self, action: #selector(stopRepeating), for: .touchDragExit)
        styleButton(button, key: key)
        return button
    }

    private func styleButton(_ button: UIButton, key: Key) {
        let foreground =
            isDarkTheme
            ? UIColor(red: 0.96, green: 0.96, blue: 0.96, alpha: 1.0)
            : UIColor(red: 0.102, green: 0.102, blue: 0.102, alpha: 1.0)
        button.setTitleColor(foreground, for: .normal)
        button.tintColor = foreground
        if #available(iOS 13.0, *) {
            button.overrideUserInterfaceStyle = isDarkTheme ? .dark : .light
        }

        let armed =
            isDarkTheme
            ? UIColor(white: 1.0, alpha: 0.18)
            : UIColor(white: 0.0, alpha: 0.12)
        if case .modifier(let mod) = key.kind, armedMods.contains(mod) {
            button.backgroundColor = armed
        } else {
            button.backgroundColor = .clear
        }
    }

    private func refreshModifierHighlights() {
        for (button, key) in keyButtons {
            styleButton(button, key: key)
        }
    }

    private func dispatchKey(name: String, fixedMods: AccessoryMods) {
        let combined = armedMods.union(fixedMods)
        sendKey(name, combined.rawValue)
        if !armedMods.isEmpty {
            armedMods = []
            refreshModifierHighlights()
        }
    }

    private func key(for sender: UIButton) -> Key? {
        for (button, key) in keyButtons where button === sender {
            return key
        }
        return nil
    }

    @objc
    private func buttonTouchDown(_ sender: UIButton) {
        guard let key = key(for: sender), key.repeats else {
            return
        }
        if case .dispatch(let name, let fixedMods) = key.kind {
            let combined = armedMods.union(fixedMods)
            sendKey(name, combined.rawValue)
            if !armedMods.isEmpty {
                armedMods = []
                refreshModifierHighlights()
            }
            startRepeating(name: name, mods: combined.rawValue)
        }
    }

    @objc
    private func buttonTouchUpInside(_ sender: UIButton) {
        guard let key = key(for: sender) else {
            stopRepeating()
            return
        }
        if key.repeats {
            stopRepeating()
            return
        }
        switch key.kind {
        case .dispatch(let name, let fixedMods):
            dispatchKey(name: name, fixedMods: fixedMods)
        case .modifier(let mod):
            armedMods.formSymmetricDifference(mod)
            refreshModifierHighlights()
        }
    }

    @objc
    func stopRepeating() {
        repeatTimer?.invalidate()
        repeatTimer = nil
        repeatingKey = nil
    }

    private func startRepeating(name: String, mods: UInt8) {
        stopRepeating()
        repeatingKey = (name, mods)
        let timer = Timer(timeInterval: repeatInterval, repeats: true) { [weak self] _ in
            guard let self, let target = self.repeatingKey,
                target.name == name, target.mods == mods
            else {
                return
            }
            self.sendKey(name, mods)
        }
        timer.fireDate = Date(timeIntervalSinceNow: repeatInitialDelay)
        repeatTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    override func layoutSubviews() {
        super.layoutSubviews()

        topBorder?.frame = CGRect(x: 0, y: 0, width: bounds.width, height: 0.33)

        let rowCount = rowsKeys.count
        guard rowCount > 0, bounds.width > 0, bounds.height > 0 else {
            return
        }

        // Match the accessory bar above: edge-to-edge columns, no horizontal
        // inset or gap, so each panel key column lines up with the bar button
        // directly above it.
        for (rowIndex, keys) in rowsKeys.enumerated() {
            let buttons = rowsButtons[rowIndex]
            let keyWidth = bounds.width / CGFloat(max(1, keys.count))

            let y = CGFloat(rowIndex) * rowHeight
            for (keyIndex, _) in keys.enumerated() {
                buttons[keyIndex].frame = CGRect(
                    x: CGFloat(keyIndex) * keyWidth,
                    y: y,
                    width: keyWidth,
                    height: rowHeight
                )
            }
        }

        // Pin the OS badge to the bottom of the panel (reserved region) so
        // the keys read first and the diagnostic label is unobtrusive.
        if let badge = hostBadge {
            let badgeSize = badge.sizeThatFits(CGSize(width: 120, height: 14))
            badge.frame = CGRect(
                x: bounds.width - badgeSize.width - 8,
                y: bounds.height - badgeSize.height - 4,
                width: badgeSize.width,
                height: badgeSize.height
            )
        }
    }
}
