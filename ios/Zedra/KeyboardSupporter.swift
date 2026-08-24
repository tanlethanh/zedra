import UIKit

/// Terminal key bar. Renders either the compact single row or the extended
/// two-row keypad, and swaps in an IME composing field on a left swipe or a tap
/// on the ⌨ key.
///
/// Modifier state is owned by Rust (`zedra_terminal::keyboard_accessory`) because
/// it must also apply to characters committed by the software keyboard; this view
/// only renders the mask it is given.
@objcMembers
final class KeyboardSupporter: NSObject, UITextFieldDelegate, UIGestureRecognizerDelegate {
    private struct KeySpec {
        let label: String
        let key: String
        var repeats: Bool = false
        /// Set for Shift/Ctrl/Alt: the bit this key highlights from the mask.
        var modifierBit: UInt32?
        /// Spoken name for keys whose glyph does not read as a word.
        var accessibilityLabel: String?
    }

    /// Opens the composer instead of reaching the terminal. Handled locally in
    /// `buttonTouchUpInside`, so it is never forwarded as a key to Rust.
    private static let composerKey = "zedra:composer"

    private let compactRow = [
        KeySpec(label: "Esc", key: "escape"),
        KeySpec(label: "Tab", key: "tab"),
        KeySpec(label: "←", key: "left", repeats: true),
        KeySpec(label: "↓", key: "down", repeats: true),
        KeySpec(label: "↑", key: "up", repeats: true),
        KeySpec(label: "→", key: "right", repeats: true),
        KeySpec(label: "⏎", key: "enter"),
        KeySpec(label: "⌨", key: Self.composerKey, accessibilityLabel: "Open composer"),
    ]

    private let extendedTopRow = [
        KeySpec(label: "Esc", key: "escape"),
        KeySpec(label: "Shift", key: "mod:shift", modifierBit: 1),
        KeySpec(label: "Tab", key: "tab"),
        KeySpec(label: "/", key: "char:/"),
        KeySpec(label: "-", key: "char:-"),
        KeySpec(label: "↑", key: "up", repeats: true),
        KeySpec(label: "⏎", key: "enter"),
    ]

    /// Cmd only means anything to a macOS host; every other host gets a pipe,
    /// which is real shell syntax and awkward to reach on a phone keyboard.
    private var platformSlot: KeySpec {
        cmdSlot
            ? KeySpec(label: "Cmd", key: "mod:cmd", modifierBit: 8)
            : KeySpec(label: "|", key: "char:|")
    }

    private var extendedBottomRow: [KeySpec] {
        [
            KeySpec(label: "⌫", key: "backspace", repeats: true),
            KeySpec(label: "Ctrl", key: "mod:ctrl", modifierBit: 2),
            KeySpec(label: "Alt", key: "mod:alt", modifierBit: 4),
            platformSlot,
            KeySpec(label: "←", key: "left", repeats: true),
            KeySpec(label: "↓", key: "down", repeats: true),
            KeySpec(label: "→", key: "right", repeats: true),
        ]
    }

    private let repeatInitialDelay: TimeInterval = 0.35
    private let repeatInterval: TimeInterval = 0.06

    static let rowHeight: CGFloat = 44.0
    /// Two stacked rows would eat too much of the screen at full row height.
    static let extendedRowHeight: CGFloat = 32.0
    private var currentRowHeight: CGFloat { extended && !composing ? Self.extendedRowHeight : Self.rowHeight }
    /// Height of the key area for the current layout, before safe-area padding.
    var keysHeight: CGFloat { extended && !composing ? Self.extendedRowHeight * 2 : Self.rowHeight }

    private(set) var accessoryView: UIView?
    private weak var topBorder: UIView?
    private weak var leftKeyboardCornerFill: UIView?
    private weak var rightKeyboardCornerFill: UIView?
    private var buttons: [UIButton] = []
    private var specsByTag: [Int: KeySpec] = [:]
    private var sendKey: ((String) -> Void)?
    private var sendComposedText: ((String) -> Void)?
    private var repeatTimer: Timer?
    private var repeatingKey: String?
    private var repeatFired = false
    private var isDarkTheme = true
    private var extended = false
    private var cmdSlot = false
    private var composing = false
    private var modifierMask: UInt32 = 0
    private var barWidth: CGFloat = 0
    private var barBottomPadding: CGFloat = 0
    private weak var composeField: UITextField?
    private weak var keysPage: UIView?
    private weak var composePage: UIView?
    /// The bar's geometry changed (rows, composer): the host re-places it.
    private var needsLayout: (() -> Void)?
    /// Hand the keyboard back to the terminal when the user leaves the composer.
    private var requestTerminalKeyboard: (() -> Void)?
    private weak var panRecognizer: UIPanGestureRecognizer?

    func setModifierMask(_ mask: UInt32) {
        guard modifierMask != mask else { return }
        modifierMask = mask
        applyModifierHighlights()
    }

    /// Builds the key bar. `bottomPadding` > 0 pins it above the safe area instead
    /// of riding the keyboard, so the background extends under the home indicator
    /// and the keyboard corner fills are left out.
    func makeAccessoryView(
        width: CGFloat,
        bottomPadding: CGFloat = 0.0,
        extended: Bool = false,
        cmdSlot: Bool = false,
        sendKey: @escaping (String) -> Void,
        sendComposedText: ((String) -> Void)? = nil,
        requestTerminalKeyboard: (() -> Void)? = nil,
        needsLayout: (() -> Void)? = nil
    ) -> UIView {
        stopRepeating()
        self.sendKey = sendKey
        self.sendComposedText = sendComposedText
        self.requestTerminalKeyboard = requestTerminalKeyboard
        self.needsLayout = needsLayout
        self.extended = extended
        self.cmdSlot = cmdSlot
        self.barWidth = width
        self.barBottomPadding = bottomPadding
        composing = false

        let bar = UIView(frame: CGRect(x: 0, y: 0, width: width, height: keysHeight + bottomPadding))
        bar.clipsToBounds = false

        let border = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 0.33))
        bar.addSubview(border)
        topBorder = border

        if bottomPadding == 0.0 {
            // The system keyboard has rounded top corners, which can expose the window
            // background beside an inputAccessoryView. Fill only those side gaps.
            let cornerFillWidth: CGFloat = 18.0
            let cornerFillHeight: CGFloat = 12.0
            let leftFill = UIView(
                frame: CGRect(x: 0, y: keysHeight, width: cornerFillWidth, height: cornerFillHeight)
            )
            let rightFill = UIView(
                frame: CGRect(
                    x: width - cornerFillWidth,
                    y: keysHeight,
                    width: cornerFillWidth,
                    height: cornerFillHeight
                )
            )
            bar.addSubview(leftFill)
            bar.addSubview(rightFill)
            leftKeyboardCornerFill = leftFill
            rightKeyboardCornerFill = rightFill
        }

        accessoryView = bar

        // The two pages sit side by side and follow the finger, so the composer is
        // visibly dragged in rather than swapped at the end of a gesture.
        let keys = UIView(frame: CGRect(x: 0, y: 0, width: width, height: keysHeight))
        let compose = UIView(frame: CGRect(x: width, y: 0, width: width, height: keysHeight))
        compose.clipsToBounds = true
        bar.addSubview(keys)
        bar.addSubview(compose)
        keysPage = keys
        composePage = compose

        let pan = UIPanGestureRecognizer(target: self, action: #selector(handlePan(_:)))
        pan.delegate = self
        bar.addGestureRecognizer(pan)
        panRecognizer = pan

        rebuildRows()
        return bar
    }

    private func rebuildRows() {
        guard let bar = accessoryView, let keys = keysPage, let compose = composePage else { return }
        stopRepeating()
        for button in buttons {
            button.removeFromSuperview()
        }
        buttons.removeAll()
        specsByTag.removeAll()
        composeField?.removeFromSuperview()
        composeField = nil
        bar.frame = CGRect(
            x: bar.frame.origin.x,
            y: bar.frame.origin.y,
            width: barWidth,
            height: keysHeight + barBottomPadding
        )
        keys.frame = CGRect(x: pageOffset(forComposing: composing), y: 0, width: barWidth, height: keysHeight)
        compose.frame = CGRect(x: keys.frame.origin.x + barWidth, y: 0, width: barWidth, height: keysHeight)

        if extended {
            buildRow(extendedTopRow, in: keys, atY: 0)
            buildRow(extendedBottomRow, in: keys, atY: Self.extendedRowHeight)
        } else {
            buildRow(compactRow, in: keys, atY: 0)
        }
        buildComposeRow(in: compose)

        applyTheme(isDark: isDarkTheme)
        applyModifierHighlights()
        needsLayout?()
    }

    private func buildRow(_ specs: [KeySpec], in bar: UIView, atY y: CGFloat) {
        let buttonWidth = barWidth / CGFloat(specs.count)
        for (index, spec) in specs.enumerated() {
            let button = UIButton(type: .system)
            button.frame = CGRect(
                x: buttonWidth * CGFloat(index),
                y: y,
                width: buttonWidth,
                height: currentRowHeight
            )
            button.setTitle(spec.label, for: .normal)
            button.titleLabel?.font = .systemFont(ofSize: extended ? 14.0 : 16.0)
            button.accessibilityLabel = spec.accessibilityLabel
            button.tag = buttons.count
            specsByTag[button.tag] = spec
            button.addTarget(self, action: #selector(buttonTouchDown(_:)), for: .touchDown)
            button.addTarget(self, action: #selector(buttonTouchUpInside(_:)), for: .touchUpInside)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchUpOutside)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchCancel)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchDragExit)
            bar.addSubview(button)
            buttons.append(button)
        }
    }

    private func buildComposeRow(in page: UIView) {
        let sendWidth: CGFloat = 64.0
        let fieldHeight = Self.rowHeight
        let fieldY = (keysHeight - fieldHeight) / 2

        let field = UITextField(
            frame: CGRect(
                x: 12.0,
                y: fieldY,
                width: barWidth - sendWidth - 20.0,
                height: fieldHeight
            )
        )
        field.placeholder = "Compose, then send"
        field.font = .systemFont(ofSize: 15.0)
        field.returnKeyType = .send
        field.delegate = self
        page.addSubview(field)
        composeField = field

        // The keyboard's own return key submits, so this is the way out, not a
        // second submit control.
        let close = UIButton(type: .system)
        close.frame = CGRect(x: barWidth - sendWidth, y: fieldY, width: sendWidth, height: fieldHeight)
        close.setTitle("✕", for: .normal)
        close.titleLabel?.font = .systemFont(ofSize: 17.0)
        close.accessibilityLabel = "Close composer"
        close.addTarget(self, action: #selector(closeComposer), for: .touchUpInside)
        page.addSubview(close)
        buttons.append(close)
    }

    @objc
    private func handlePan(_ recognizer: UIPanGestureRecognizer) {
        let dx = recognizer.translation(in: recognizer.view).x
        switch recognizer.state {
        case .began:
            stopRepeating()
        case .changed:
            applyDragOffset(dx)
        case .ended, .cancelled:
            // Past halfway, or thrown hard enough, the page flips.
            let velocity = recognizer.velocity(in: recognizer.view).x
            let flipped = abs(dx) > barWidth / 2 || abs(velocity) > 600
            setComposing(composing != flipped, animated: true, keepKeyboard: true)
        default:
            break
        }
    }

    private func pageOffset(forComposing composing: Bool) -> CGFloat {
        composing ? -barWidth : 0
    }

    private func applyDragOffset(_ dx: CGFloat) {
        let base = pageOffset(forComposing: composing)
        let offset = min(max(base + dx, -barWidth), 0)
        keysPage?.frame.origin.x = offset
        composePage?.frame.origin.x = offset + barWidth
    }

    /// `keepKeyboard` distinguishes the user stepping back to the keys — where the
    /// keyboard should stay up, now typing into the terminal — from a forced cancel
    /// (drawer, navigation, terminal tap), where it must go away with the composer.
    private func setComposing(_ enabled: Bool, animated: Bool, keepKeyboard: Bool = false) {
        let changed = composing != enabled
        composing = enabled
        let offset = pageOffset(forComposing: enabled)
        let settle = { [weak self] in
            guard let self else { return }
            self.keysPage?.frame.origin.x = offset
            self.composePage?.frame.origin.x = offset + self.barWidth
        }
        if animated {
            UIView.animate(withDuration: 0.18, animations: settle)
        } else {
            settle()
        }
        guard changed else { return }

        if enabled {
            composeField?.becomeFirstResponder()
        } else if keepKeyboard {
            // Handing first responder straight to the terminal keeps the keyboard
            // up; resigning first would drop it and animate a new one back in.
            requestTerminalKeyboard?()
        } else {
            composeField?.resignFirstResponder()
        }
        needsLayout?()
    }

    /// Horizontal drags belong to the pager. The composing field fills most of its
    /// page, so refusing drags that start on it would strand the user there.
    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard let pan = gestureRecognizer as? UIPanGestureRecognizer else { return true }
        let velocity = pan.velocity(in: pan.view)
        return abs(velocity.x) > abs(velocity.y)
    }

    /// Drop the composer and its keyboard. Also the exit from the accessory's Keys
    /// button, since the bar's own pager is not on screen while composing.
    @objc
    var isComposing: Bool { composing }

    /// The composer's ✕: step back to the keys, keeping the keyboard for the terminal.
    @objc
    private func closeComposer() {
        setComposing(false, animated: true, keepKeyboard: true)
    }

    @objc
    func cancelComposing() {
        composeField?.resignFirstResponder()
        guard composing else { return }
        setComposing(false, animated: false)
    }

    @objc
    private func submitComposedText() {
        guard let text = composeField?.text, !text.isEmpty else { return }
        NSLog("key-bar: submitting composed text (%d chars)", text.count)
        // Text only: the composer fills the prompt, and the user decides when to run it.
        sendComposedText?(text)
        composeField?.text = ""
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        submitComposedText()
        return false
    }

    private func applyModifierHighlights() {
        // Accent blue marks an active modifier; a locked one is fully opaque, an
        // armed one dimmer, so the two stay distinguishable without a fill.
        let accent = NativePresentationTheme.accentBlue
        let foreground = foregroundColor

        for button in buttons {
            guard let bit = specsByTag[button.tag]?.modifierBit else { continue }
            let armed = modifierMask & bit != 0
            let locked = modifierMask & (bit << 4) != 0
            let color =
                locked
                ? accent
                : (armed ? accent.withAlphaComponent(0.6) : foreground)
            button.setTitleColor(color, for: .normal)
            button.tintColor = color
        }
    }

    private var foregroundColor: UIColor {
        isDarkTheme
            ? UIColor(red: 0.96, green: 0.96, blue: 0.96, alpha: 1.0)
            : UIColor(red: 0.102, green: 0.102, blue: 0.102, alpha: 1.0)
    }

    func applyTheme(isDark: Bool) {
        isDarkTheme = isDark

        let backgroundColor = isDark
            ? UIColor(red: 0.055, green: 0.047, blue: 0.047, alpha: 0.96)
            : UIColor(red: 0.961, green: 0.961, blue: 0.961, alpha: 0.98)
        let foregroundColor = self.foregroundColor
        let borderColor = isDark
            ? UIColor(white: 1.0, alpha: 0.12)
            : UIColor(white: 0.0, alpha: 0.10)

        accessoryView?.backgroundColor = backgroundColor
        topBorder?.backgroundColor = borderColor
        leftKeyboardCornerFill?.backgroundColor = backgroundColor
        rightKeyboardCornerFill?.backgroundColor = backgroundColor

        composeField?.textColor = foregroundColor
        let interfaceStyle: UIUserInterfaceStyle = isDark ? .dark : .light
        if #available(iOS 13.0, *) {
            accessoryView?.overrideUserInterfaceStyle = interfaceStyle
        }
        if #available(iOS 13.0, *) {
            composeField?.overrideUserInterfaceStyle = interfaceStyle
        }
        for button in buttons {
            button.setTitleColor(foregroundColor, for: .normal)
            button.tintColor = foregroundColor
            if #available(iOS 13.0, *) {
                button.overrideUserInterfaceStyle = interfaceStyle
            }
        }
        applyModifierHighlights()
    }

    func stopRepeating() {
        repeatTimer?.invalidate()
        repeatTimer = nil
        repeatingKey = nil
        repeatFired = false
    }

    private func keySpec(for sender: UIButton) -> KeySpec? {
        specsByTag[sender.tag]
    }

    @objc
    private func buttonTouchDown(_ sender: UIButton) {
        // Nothing is sent yet: a horizontal drag starting on this key belongs to
        // the pager, and a key already sent cannot be taken back. Holds emit from
        // the repeat timer, taps on release.
        guard let spec = keySpec(for: sender), spec.repeats else {
            return
        }
        startRepeating(spec.key)
    }

    @objc
    private func buttonTouchUpInside(_ sender: UIButton) {
        guard let spec = keySpec(for: sender) else {
            stopRepeating()
            return
        }

        let repeated = repeatFired
        stopRepeating()
        guard !repeated else { return }

        if spec.key == Self.composerKey {
            setComposing(true, animated: true)
            return
        }
        sendKey?(spec.key)
    }

    private func startRepeating(_ key: String) {
        stopRepeating()
        repeatingKey = key

        // Accessory arrow keys should behave like held hardware keys: one immediate
        // keystroke, then repeat until UIKit reports any release or cancellation.
        let timer = Timer(timeInterval: repeatInterval, repeats: true) { [weak self] _ in
            guard let self, self.repeatingKey == key else {
                return
            }
            self.repeatFired = true
            self.sendKey?(key)
        }
        timer.fireDate = Date(timeIntervalSinceNow: repeatInitialDelay)
        repeatTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }
}
