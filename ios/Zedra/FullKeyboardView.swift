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
    private var layer: Layer = .letters
    private var isDarkTheme = true
    private var rows: [UIStackView] = []
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

    /// Lay the keyboard out for the current `layer` + `shift` state. Builds
    /// from scratch every time so cap state, layer switches, and width
    /// changes all share the same code path.
    private func rebuild() {
        for row in rows {
            row.removeFromSuperview()
        }
        rows.removeAll()
        keyButtons.removeAll()

        let rowsSpecs = keyboardLayout()
        let rowCount = rowsSpecs.count
        let rowHeight = bounds.height / CGFloat(rowCount)
        let verticalPadding: CGFloat = 4.0

        for (rowIndex, rowKeys) in rowsSpecs.enumerated() {
            let stack = UIStackView()
            stack.axis = .horizontal
            stack.spacing = 4
            stack.distribution = .fill
            stack.alignment = .fill
            stack.translatesAutoresizingMaskIntoConstraints = false
            stack.frame = CGRect(
                x: 4,
                y: CGFloat(rowIndex) * rowHeight + verticalPadding / 2,
                width: bounds.width - 8,
                height: rowHeight - verticalPadding
            )
            stack.autoresizingMask = [.flexibleWidth]
            addSubview(stack)
            rows.append(stack)

            for key in rowKeys {
                let button = makeButton(for: key)
                stack.addArrangedSubview(button)
                keyButtons.append((button, key))
            }
        }
    }

    private func keyboardLayout() -> [[Key]] {
        switch layer {
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
        button.setContentHuggingPriority(.defaultLow, for: .horizontal)
        button.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.addTarget(self, action: #selector(keyTapped(_:)), for: .touchUpInside)
        styleButton(button, key: key)

        // Constrain width by stretching one "unit" key as the baseline; non-1.0
        // keys are multiples of that base via constraint priorities — UIStackView
        // distributes remaining space proportionally when we set a width anchor
        // referencing the first button.
        if key.widthHint != 1.0, let anchor = keyButtons.first?.button.widthAnchor {
            button.widthAnchor.constraint(equalTo: anchor, multiplier: key.widthHint).isActive = true
        }
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
            layer = target
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
        // Re-layout rows when the host resizes us (rotation, split view).
        let rowCount = rows.count
        guard rowCount > 0 else {
            return
        }
        let rowHeight = bounds.height / CGFloat(rowCount)
        let verticalPadding: CGFloat = 4.0
        for (index, row) in rows.enumerated() {
            row.frame = CGRect(
                x: 4,
                y: CGFloat(index) * rowHeight + verticalPadding / 2,
                width: bounds.width - 8,
                height: rowHeight - verticalPadding
            )
        }
    }
}
