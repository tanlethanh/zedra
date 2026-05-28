import UIKit

/// Desktop-only key panel that replaces the system keyboard while a terminal
/// or agent session has focus. Surfaces keys / combos that the iOS soft
/// keyboard either doesn't have at all or buries multiple layers deep. There
/// is no QWERTY here on purpose — users tap `✕` and go back to the system
/// keyboard for prose typing, IME, dictation, autocorrect.
///
/// Every tap dispatches through `sendKey(name, mods)` using the same wire
/// format as the accessory bar (`char:<c>` for single chars, named keys, and
/// a Shift/Alt/Ctrl bitmask per `key_encoding::Mods`).
final class FullKeyboardView: UIView {
    typealias SendKey = (String, UInt8) -> Void

    private struct AccessoryMods: OptionSet {
        let rawValue: UInt8

        static let shift = AccessoryMods(rawValue: 0b001)
        static let alt = AccessoryMods(rawValue: 0b010)
        static let ctrl = AccessoryMods(rawValue: 0b100)
    }

    private enum KeyKind {
        /// Tap dispatches `(name, armedMods | fixedMods)`. The single source
        /// of truth for every non-modifier key.
        case dispatch(name: String, fixedMods: AccessoryMods = [])
        /// Sticky modifier — toggles `armedMods`, never sends bytes itself.
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

    /// Row 1 — symbols that exist on the iOS soft keyboard but are buried 1-2
    /// layers deep. Surfacing them one-tap is the highest-value cheap win.
    private let symbolRow: [Key] = [
        Key(label: "`", kind: .dispatch(name: "char:`")),
        Key(label: "~", kind: .dispatch(name: "char:~")),
        Key(label: "|", kind: .dispatch(name: "char:|")),
        Key(label: "\\", kind: .dispatch(name: "char:\\")),
        Key(label: "<", kind: .dispatch(name: "char:<")),
        Key(label: ">", kind: .dispatch(name: "char:>")),
        Key(label: "{", kind: .dispatch(name: "char:{")),
        Key(label: "}", kind: .dispatch(name: "char:}")),
        Key(label: "[", kind: .dispatch(name: "char:[")),
        Key(label: "]", kind: .dispatch(name: "char:]")),
    ]

    /// Row 2 — navigation cluster the soft keyboard has no concept of.
    private let navRow: [Key] = [
        Key(label: "Home", kind: .dispatch(name: "home")),
        Key(label: "End", kind: .dispatch(name: "end")),
        Key(label: "PgUp", kind: .dispatch(name: "page_up"), repeats: true),
        Key(label: "PgDn", kind: .dispatch(name: "page_down"), repeats: true),
        Key(label: "←", kind: .dispatch(name: "left"), repeats: true),
        Key(label: "↓", kind: .dispatch(name: "down"), repeats: true),
        Key(label: "↑", kind: .dispatch(name: "up"), repeats: true),
        Key(label: "→", kind: .dispatch(name: "right"), repeats: true),
    ]

    /// Row 3 — agent / terminal control keys + a sticky `Ctrl` for ad-hoc
    /// combos (e.g. Ctrl+\, Ctrl+PgUp).
    private let controlRow: [Key] = [
        Key(label: "Esc", kind: .dispatch(name: "escape")),
        Key(label: "Tab", kind: .dispatch(name: "tab")),
        Key(label: "⇧⇥", kind: .dispatch(name: "tab", fixedMods: .shift)),
        Key(label: "⇧⏎", kind: .dispatch(name: "enter", fixedMods: .shift)),
        Key(label: "Ctrl", kind: .modifier(.ctrl)),
        Key(label: "⌃C", kind: .dispatch(name: "char:c", fixedMods: .ctrl)),
        Key(label: "⌃D", kind: .dispatch(name: "char:d", fixedMods: .ctrl)),
        Key(label: "⌃R", kind: .dispatch(name: "char:r", fixedMods: .ctrl)),
    ]

    private let sendKey: SendKey
    private var isDarkTheme = true
    private var armedMods: AccessoryMods = []
    private var rowsKeys: [[Key]] = []
    private var rowsButtons: [[UIButton]] = []
    private var keyButtons: [(button: UIButton, key: Key)] = []
    private var repeatTimer: Timer?
    private var repeatingKey: (name: String, mods: UInt8)?
    private let repeatInitialDelay: TimeInterval = 0.35
    private let repeatInterval: TimeInterval = 0.06

    init(width: CGFloat, height: CGFloat, isDark: Bool, sendKey: @escaping SendKey) {
        self.sendKey = sendKey
        self.isDarkTheme = isDark
        super.init(frame: CGRect(x: 0, y: 0, width: width, height: height))
        autoresizingMask = [.flexibleWidth]
        clipsToBounds = true
        applyTheme(isDark: isDark)
        build()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) not implemented")
    }

    func applyTheme(isDark: Bool) {
        isDarkTheme = isDark
        backgroundColor =
            isDark
            ? UIColor(red: 0.12, green: 0.12, blue: 0.13, alpha: 1.0)
            : UIColor(red: 0.82, green: 0.83, blue: 0.85, alpha: 1.0)
        for (button, key) in keyButtons {
            styleButton(button, key: key)
        }
    }

    private func build() {
        for (button, _) in keyButtons {
            button.removeFromSuperview()
        }
        rowsKeys.removeAll()
        rowsButtons.removeAll()
        keyButtons.removeAll()

        for rowKeys in [symbolRow, navRow, controlRow] {
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
        button.titleLabel?.font = .systemFont(ofSize: 18.0, weight: .regular)
        button.setTitle(key.label, for: .normal)
        button.layer.cornerRadius = 6.0
        button.addTarget(self, action: #selector(buttonTouchDown(_:)), for: .touchDown)
        button.addTarget(self, action: #selector(buttonTouchUpInside(_:)), for: .touchUpInside)
        button.addTarget(self, action: #selector(stopRepeating), for: .touchUpOutside)
        button.addTarget(self, action: #selector(stopRepeating), for: .touchCancel)
        button.addTarget(self, action: #selector(stopRepeating), for: .touchDragExit)
        styleButton(button, key: key)
        return button
    }

    private func styleButton(_ button: UIButton, key: Key) {
        let isModifier: Bool
        switch key.kind {
        case .modifier: isModifier = true
        case .dispatch: isModifier = false
        }
        let baseBg =
            isDarkTheme
            ? (isModifier
                ? UIColor(red: 0.20, green: 0.20, blue: 0.22, alpha: 1.0)
                : UIColor(red: 0.30, green: 0.30, blue: 0.33, alpha: 1.0))
            : (isModifier
                ? UIColor(red: 0.65, green: 0.66, blue: 0.69, alpha: 1.0)
                : UIColor.white)
        let foreground =
            isDarkTheme
            ? UIColor(white: 1.0, alpha: 1.0)
            : UIColor(red: 0.07, green: 0.07, blue: 0.10, alpha: 1.0)
        button.backgroundColor = baseBg
        button.setTitleColor(foreground, for: .normal)
        button.tintColor = foreground

        if case .modifier(let mod) = key.kind, armedMods.contains(mod) {
            button.backgroundColor =
                isDarkTheme
                ? UIColor(white: 1.0, alpha: 0.25)
                : UIColor(white: 0.0, alpha: 0.20)
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
        let rowCount = rowsKeys.count
        guard rowCount > 0, bounds.width > 0, bounds.height > 0 else {
            return
        }
        let horizontalInset: CGFloat = 4
        let keyGap: CGFloat = 4
        let rowGap: CGFloat = 4

        let rowHeight = (bounds.height - rowGap * CGFloat(rowCount + 1)) / CGFloat(rowCount)
        let availableWidth = bounds.width - horizontalInset * 2

        for (rowIndex, keys) in rowsKeys.enumerated() {
            let buttons = rowsButtons[rowIndex]
            let totalGap = keyGap * CGFloat(max(0, keys.count - 1))
            let keyWidth = (availableWidth - totalGap) / CGFloat(max(1, keys.count))

            var x: CGFloat = horizontalInset
            let y = rowGap + CGFloat(rowIndex) * (rowHeight + rowGap)
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
