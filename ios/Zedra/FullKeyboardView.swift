import UIKit

/// In-app QWERTY keyboard that replaces the system software keyboard while the
/// user is driving a terminal / agent session. Each tap is forwarded to
/// `sendKey(name, mods)` using the same wire format as the accessory bar
/// (`char:<c>`, named keys, mod bitmask).
///
/// The view is intended to be installed via `gpui_ios_set_keyboard_input_view`.
/// UIKit reads `frame.size` for the keyboard slot, so callers must size this
/// to the system keyboard's last known height before installing it.
final class FullKeyboardView: UIView {
    /// Closure invoked for every keystroke. Matches the FFI used by
    /// `KeyboardSupporter` so both surfaces share one transport.
    typealias SendKey = (String, UInt8) -> Void

    private enum Layer {
        case letters
        case numbers
        case symbols
    }

    private enum ShiftState {
        case off
        case oneShot
        case locked
    }

    private enum KeyKind {
        case char(String)
        case named(String)
        case shift
        case backspace
        case toggleLayer(Layer)
        case space
        case enter
    }

    private struct Key {
        let label: String
        let kind: KeyKind
        let widthHint: CGFloat
    }

    private let sendKey: SendKey
    private var shift: ShiftState = .off
    private var activeLayer: Layer = .letters
    private var isDarkTheme = true
    private var rowsKeys: [[Key]] = []
    private var rowsButtons: [[UIButton]] = []
    private var keyButtons: [(button: UIButton, key: Key)] = []
    private var lastShiftTapDate: Date?

    init(width: CGFloat, height: CGFloat, isDark: Bool, sendKey: @escaping SendKey) {
        self.sendKey = sendKey
        self.isDarkTheme = isDark
        super.init(frame: CGRect(x: 0, y: 0, width: width, height: height))
        autoresizingMask = [.flexibleWidth]
        clipsToBounds = true
        applyTheme(isDark: isDark)
        rebuild()
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

    /// Rebuild the button set for the current `activeLayer` + `shift` state.
    /// Per-row frames are computed in `layoutSubviews` so this method stays
    /// independent of the view's current size — the same code path runs on
    /// first install, on rotation, and on every layer / shift toggle.
    private func rebuild() {
        for (button, _) in keyButtons {
            button.removeFromSuperview()
        }
        rowsKeys.removeAll()
        rowsButtons.removeAll()
        keyButtons.removeAll()

        let specs = keyboardLayout()
        for rowKeys in specs {
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

    private func keyboardLayout() -> [[Key]] {
        switch activeLayer {
        case .letters:
            let row1 = "qwertyuiop".map { Key(label: shiftedLetter(String($0)), kind: .char(String($0)), widthHint: 1.0) }
            let row2 = "asdfghjkl".map { Key(label: shiftedLetter(String($0)), kind: .char(String($0)), widthHint: 1.0) }
            var row3: [Key] = [
                Key(label: shiftLabel(), kind: .shift, widthHint: 1.5)
            ]
            row3.append(contentsOf: "zxcvbnm".map { Key(label: shiftedLetter(String($0)), kind: .char(String($0)), widthHint: 1.0) })
            row3.append(Key(label: "⌫", kind: .backspace, widthHint: 1.5))
            let row4: [Key] = [
                Key(label: "123", kind: .toggleLayer(.numbers), widthHint: 1.5),
                Key(label: "space", kind: .space, widthHint: 5.0),
                Key(label: "return", kind: .enter, widthHint: 2.0),
            ]
            return [row1, row2, row3, row4]
        case .numbers:
            let row1 = "1234567890".map { Key(label: String($0), kind: .char(String($0)), widthHint: 1.0) }
            let row2 = ["-", "/", ":", ";", "(", ")", "$", "&", "@", "\""].map { Key(label: $0, kind: .char($0), widthHint: 1.0) }
            var row3: [Key] = [
                Key(label: "#+=", kind: .toggleLayer(.symbols), widthHint: 1.5)
            ]
            row3.append(contentsOf: [".", ",", "?", "!", "'"].map { Key(label: $0, kind: .char($0), widthHint: 1.0) })
            row3.append(Key(label: "⌫", kind: .backspace, widthHint: 1.5))
            let row4: [Key] = [
                Key(label: "ABC", kind: .toggleLayer(.letters), widthHint: 1.5),
                Key(label: "space", kind: .space, widthHint: 5.0),
                Key(label: "return", kind: .enter, widthHint: 2.0),
            ]
            return [row1, row2, row3, row4]
        case .symbols:
            let row1 = ["[", "]", "{", "}", "#", "%", "^", "*", "+", "="].map { Key(label: $0, kind: .char($0), widthHint: 1.0) }
            let row2 = ["_", "\\", "|", "~", "<", ">", "€", "£", "¥", "•"].map { Key(label: $0, kind: .char($0), widthHint: 1.0) }
            var row3: [Key] = [
                Key(label: "123", kind: .toggleLayer(.numbers), widthHint: 1.5)
            ]
            row3.append(contentsOf: [".", ",", "?", "!", "'"].map { Key(label: $0, kind: .char($0), widthHint: 1.0) })
            row3.append(Key(label: "⌫", kind: .backspace, widthHint: 1.5))
            let row4: [Key] = [
                Key(label: "ABC", kind: .toggleLayer(.letters), widthHint: 1.5),
                Key(label: "space", kind: .space, widthHint: 5.0),
                Key(label: "return", kind: .enter, widthHint: 2.0),
            ]
            return [row1, row2, row3, row4]
        }
    }

    private func shiftedLetter(_ c: String) -> String {
        return shift == .off ? c : c.uppercased()
    }

    private func shiftLabel() -> String {
        switch shift {
        case .off: return "⇧"
        case .oneShot: return "⬆︎"
        case .locked: return "⇪"
        }
    }

    private func makeButton(for key: Key) -> UIButton {
        let button = UIButton(type: .system)
        button.titleLabel?.font = .systemFont(ofSize: 18.0, weight: .regular)
        button.setTitle(key.label, for: .normal)
        button.layer.cornerRadius = 6.0
        button.addTarget(self, action: #selector(keyTapped(_:)), for: .touchUpInside)
        styleButton(button, key: key)
        return button
    }

    private func styleButton(_ button: UIButton, key: Key) {
        let isSpecial: Bool
        switch key.kind {
        case .char: isSpecial = false
        default: isSpecial = true
        }
        let baseBg =
            isDarkTheme
            ? (isSpecial
                ? UIColor(red: 0.20, green: 0.20, blue: 0.22, alpha: 1.0)
                : UIColor(red: 0.30, green: 0.30, blue: 0.33, alpha: 1.0))
            : (isSpecial
                ? UIColor(red: 0.65, green: 0.66, blue: 0.69, alpha: 1.0)
                : UIColor.white)
        let foreground =
            isDarkTheme
            ? UIColor(white: 1.0, alpha: 1.0)
            : UIColor(red: 0.07, green: 0.07, blue: 0.10, alpha: 1.0)
        button.backgroundColor = baseBg
        button.setTitleColor(foreground, for: .normal)
        button.tintColor = foreground

        // Highlight a locked / armed shift so the user can see the state.
        if case .shift = key.kind, shift == .locked {
            button.backgroundColor =
                isDarkTheme
                ? UIColor(white: 1.0, alpha: 0.25)
                : UIColor(white: 0.0, alpha: 0.20)
        }
    }

    @objc
    private func keyTapped(_ sender: UIButton) {
        guard let (_, key) = keyButtons.first(where: { $0.button === sender }) else {
            return
        }
        switch key.kind {
        case .char(let c):
            let outgoing = shift == .off ? c : c.uppercased()
            sendKey("char:\(outgoing)", 0)
            if shift == .oneShot {
                shift = .off
                rebuild()
            }
        case .named(let name):
            sendKey(name, 0)
        case .shift:
            handleShiftTap()
        case .backspace:
            sendKey("backspace", 0)
        case .toggleLayer(let target):
            activeLayer = target
            // Switching layers also resets shift; the system keyboard does the
            // same so the symbol layer doesn't capslock through letters.
            shift = .off
            rebuild()
        case .space:
            sendKey("char: ", 0)
        case .enter:
            sendKey("enter", 0)
        }
    }

    private func handleShiftTap() {
        let now = Date()
        let isDoubleTap =
            lastShiftTapDate.map { now.timeIntervalSince($0) < 0.35 } ?? false
        lastShiftTapDate = now

        if isDoubleTap {
            shift = .locked
        } else {
            switch shift {
            case .off: shift = .oneShot
            case .oneShot: shift = .off
            case .locked: shift = .off
            }
        }
        rebuild()
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
            let totalWeight = keys.reduce(0.0 as CGFloat) { $0 + $1.widthHint }
            let totalGap = keyGap * CGFloat(max(0, keys.count - 1))
            let unit = (availableWidth - totalGap) / max(0.0001, totalWeight)

            var x: CGFloat = horizontalInset
            let y = rowGap + CGFloat(rowIndex) * (rowHeight + rowGap)
            for (keyIndex, key) in keys.enumerated() {
                let width = unit * key.widthHint
                buttons[keyIndex].frame = CGRect(
                    x: x,
                    y: y,
                    width: width,
                    height: rowHeight
                )
                x += width + keyGap
            }
        }
    }
}
