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
/// or agent session has focus. Surfaces keys / combos the iOS soft keyboard
/// doesn't expose as a single tap. There is no QWERTY here on purpose —
/// `✕` hands focus back to the system IME for prose typing, dictation,
/// Vietnamese / CJK input.
///
/// Layout (top → bottom):
///   Row 1 (chips):  Shift  Ctrl  Cmd   Home  End  PgUp  PgDn   ⌫
///   Row 2 (flat):   ~  @  $  *  ^  %  =  `
///   Row 3 (flat):   <  >  (  )  {  }  [  ]
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
        /// Tap dispatches `(name, armedMods | fixedMods)`.
        case dispatch(name: String, fixedMods: AccessoryMods = [])
        /// Sticky modifier — toggles `armedMods`, never sends bytes itself.
        case modifier(AccessoryMods)
    }

    private enum KeyStyle {
        /// Raised pill / chip — modifiers, nav cluster, backspace.
        case chip
        /// Flat glyph — symbols. No background unless pressed.
        case flat
    }

    private struct Key {
        let label: String
        let kind: KeyKind
        let style: KeyStyle
        let repeats: Bool

        init(label: String, kind: KeyKind, style: KeyStyle, repeats: Bool = false) {
            self.label = label
            self.kind = kind
            self.style = style
            self.repeats = repeats
        }
    }

    private let chipRow: [Key] = [
        Key(label: "shift", kind: .modifier(.shift), style: .chip),
        Key(label: "Ctrl", kind: .modifier(.ctrl), style: .chip),
        Key(label: "Cmd", kind: .modifier(.cmd), style: .chip),
        Key(label: "Home", kind: .dispatch(name: "home"), style: .chip),
        Key(label: "End", kind: .dispatch(name: "end"), style: .chip),
        Key(label: "PgUp", kind: .dispatch(name: "page_up"), style: .chip, repeats: true),
        Key(label: "PgD", kind: .dispatch(name: "page_down"), style: .chip, repeats: true),
        Key(label: "⌫", kind: .dispatch(name: "backspace"), style: .chip, repeats: true),
    ]

    private let symbolRow1: [Key] = [
        Key(label: "~", kind: .dispatch(name: "char:~"), style: .flat),
        Key(label: "@", kind: .dispatch(name: "char:@"), style: .flat),
        Key(label: "$", kind: .dispatch(name: "char:$"), style: .flat),
        Key(label: "*", kind: .dispatch(name: "char:*"), style: .flat),
        Key(label: "^", kind: .dispatch(name: "char:^"), style: .flat),
        Key(label: "%", kind: .dispatch(name: "char:%"), style: .flat),
        Key(label: "=", kind: .dispatch(name: "char:="), style: .flat),
        Key(label: "`", kind: .dispatch(name: "char:`"), style: .flat),
    ]

    private let symbolRow2: [Key] = [
        Key(label: "<", kind: .dispatch(name: "char:<"), style: .flat),
        Key(label: ">", kind: .dispatch(name: "char:>"), style: .flat),
        Key(label: "(", kind: .dispatch(name: "char:("), style: .flat),
        Key(label: ")", kind: .dispatch(name: "char:)"), style: .flat),
        Key(label: "{", kind: .dispatch(name: "char:{"), style: .flat),
        Key(label: "}", kind: .dispatch(name: "char:}"), style: .flat),
        Key(label: "[", kind: .dispatch(name: "char:["), style: .flat),
        Key(label: "]", kind: .dispatch(name: "char:]"), style: .flat),
    ]

    private let sendKey: SendKey
    private let hostOs: HostOs
    private weak var hostBadge: UILabel?
    private var isDarkTheme = true
    private var armedMods: AccessoryMods = []
    private var rowsKeys: [[Key]] = []
    private var rowsButtons: [[UIButton]] = []
    private var keyButtons: [(button: UIButton, key: Key)] = []
    private var repeatTimer: Timer?
    private var repeatingKey: (name: String, mods: UInt8)?
    private let repeatInitialDelay: TimeInterval = 0.35
    private let repeatInterval: TimeInterval = 0.06

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
        clipsToBounds = true
        applyTheme(isDark: isDark)
        build()
        installHostBadge()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not implemented")
    }

    func applyTheme(isDark: Bool) {
        isDarkTheme = isDark
        backgroundColor =
            isDark
            ? UIColor(red: 0.08, green: 0.08, blue: 0.10, alpha: 1.0)
            : UIColor(red: 0.82, green: 0.83, blue: 0.85, alpha: 1.0)
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
            ? UIColor(white: 1.0, alpha: 1.0)
            : UIColor(red: 0.07, green: 0.07, blue: 0.10, alpha: 1.0)
    }

    private func build() {
        for (button, _) in keyButtons {
            button.removeFromSuperview()
        }
        rowsKeys.removeAll()
        rowsButtons.removeAll()
        keyButtons.removeAll()

        for rowKeys in [chipRow, symbolRow1, symbolRow2] {
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
        let fontSize: CGFloat = key.style == .chip ? 15 : 22
        button.titleLabel?.font = .systemFont(ofSize: fontSize, weight: .regular)
        button.setTitle(key.label, for: .normal)
        button.layer.cornerRadius = key.style == .chip ? 6 : 4
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
            ? UIColor(white: 1.0, alpha: 1.0)
            : UIColor(red: 0.07, green: 0.07, blue: 0.10, alpha: 1.0)
        button.setTitleColor(foreground, for: .normal)
        button.tintColor = foreground

        switch key.style {
        case .chip:
            let base =
                isDarkTheme
                ? UIColor(red: 0.30, green: 0.30, blue: 0.33, alpha: 1.0)
                : UIColor(red: 0.65, green: 0.66, blue: 0.69, alpha: 1.0)
            let armed =
                isDarkTheme
                ? UIColor(white: 1.0, alpha: 0.28)
                : UIColor(white: 0.0, alpha: 0.22)
            if case .modifier(let mod) = key.kind, armedMods.contains(mod) {
                button.backgroundColor = armed
            } else {
                button.backgroundColor = base
            }
        case .flat:
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
        if let badge = hostBadge {
            let badgeSize = badge.sizeThatFits(CGSize(width: 120, height: 14))
            badge.frame = CGRect(
                x: bounds.width - badgeSize.width - 8,
                y: 2,
                width: badgeSize.width,
                height: badgeSize.height
            )
        }
        let rowCount = rowsKeys.count
        guard rowCount > 0, bounds.width > 0, bounds.height > 0 else {
            return
        }
        let horizontalInset: CGFloat = 6
        let keyGap: CGFloat = 6
        let topInset: CGFloat = 14
        // Reserve roughly the bottom 40% of the panel for future content
        // (clipboard, snippets, agent macros). Lay the key rows out in the
        // upper region only.
        let keysRegionHeight = bounds.height * 0.60
        let rowGap: CGFloat = 8
        let rowHeight = (keysRegionHeight - topInset - rowGap * CGFloat(rowCount - 1)) / CGFloat(rowCount)
        let availableWidth = bounds.width - horizontalInset * 2

        for (rowIndex, keys) in rowsKeys.enumerated() {
            let buttons = rowsButtons[rowIndex]
            let totalGap = keyGap * CGFloat(max(0, keys.count - 1))
            let keyWidth = (availableWidth - totalGap) / CGFloat(max(1, keys.count))

            var x: CGFloat = horizontalInset
            let y = topInset + CGFloat(rowIndex) * (rowHeight + rowGap)
            for (keyIndex, _) in keys.enumerated() {
                buttons[keyIndex].frame = CGRect(
                    x: x,
                    y: y,
                    width: keyWidth,
                    height: rowHeight
                )
                x += keyWidth + keyGap
            }
        }
    }
}
