import UIKit

/// Modifier-key bitmask matching Rust `key_encoding::Mods`
/// (shift = bit 0, alt = bit 1, ctrl = bit 2).
struct AccessoryMods: OptionSet {
    let rawValue: UInt8

    static let shift = AccessoryMods(rawValue: 0b001)
    static let alt = AccessoryMods(rawValue: 0b010)
    static let ctrl = AccessoryMods(rawValue: 0b100)
}

/// Resizing container for the keyboard accessory bar. Overriding
/// `intrinsicContentSize` is what makes UIKit re-layout the keyboard when the
/// detail row is toggled.
private final class AccessoryContainer: UIView {
    var contentHeight: CGFloat = 44.0 {
        didSet {
            invalidateIntrinsicContentSize()
        }
    }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: contentHeight)
    }
}

@objcMembers
final class KeyboardSupporter: NSObject {
    private enum KeySpecKind {
        /// Plain key: sends `(name, armedMods | fixedMods)` and clears any armed sticky mods.
        case key(name: String, fixedMods: AccessoryMods = [])
        /// Sticky modifier: toggles the armed state, never sends bytes.
        case modifier(AccessoryMods)
        /// Expands or collapses the detail row.
        case toggleDetail
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
        KeySpec(label: "•••", kind: .toggleDetail, repeats: false),
    ]

    private let detailRow: [KeySpec] = [
        KeySpec(label: "Ctrl", kind: .modifier(.ctrl), repeats: false),
        KeySpec(label: "Alt", kind: .modifier(.alt), repeats: false),
        KeySpec(label: "Shift", kind: .modifier(.shift), repeats: false),
        KeySpec(label: "⇧⇥", kind: .key(name: "tab", fixedMods: .shift), repeats: false),
        KeySpec(label: "⌃C", kind: .key(name: "char:c", fixedMods: .ctrl), repeats: false),
        KeySpec(label: "⌃D", kind: .key(name: "char:d", fixedMods: .ctrl), repeats: false),
        KeySpec(label: "⌃R", kind: .key(name: "char:r", fixedMods: .ctrl), repeats: false),
        KeySpec(label: "Home", kind: .key(name: "home"), repeats: false),
        KeySpec(label: "End", kind: .key(name: "end"), repeats: false),
        KeySpec(label: "PgUp", kind: .key(name: "page_up"), repeats: true),
        KeySpec(label: "PgDn", kind: .key(name: "page_down"), repeats: true),
    ]

    private let rowHeight: CGFloat = 44.0
    private let repeatInitialDelay: TimeInterval = 0.35
    private let repeatInterval: TimeInterval = 0.06

    private(set) var accessoryView: UIView?
    private weak var container: AccessoryContainer?
    private weak var topBorder: UIView?
    private weak var detailSeparator: UIView?
    private weak var leftKeyboardCornerFill: UIView?
    private weak var rightKeyboardCornerFill: UIView?
    private weak var detailRowView: UIView?
    private var primaryButtons: [(button: UIButton, spec: KeySpec)] = []
    private var detailButtons: [(button: UIButton, spec: KeySpec)] = []
    private var sendKey: ((String, UInt8) -> Void)?
    private var repeatTimer: Timer?
    private var repeatingKey: (name: String, mods: UInt8)?
    private var isDarkTheme = true
    private var detailExpanded = false
    private var armedMods: AccessoryMods = []

    func makeAccessoryView(width: CGFloat, sendKey: @escaping (String, UInt8) -> Void) -> UIView {
        stopRepeating()
        self.sendKey = sendKey
        primaryButtons.removeAll()
        detailButtons.removeAll()
        armedMods = []
        detailExpanded = false

        let container = AccessoryContainer(
            frame: CGRect(x: 0, y: 0, width: width, height: rowHeight)
        )
        container.clipsToBounds = false
        container.contentHeight = rowHeight
        self.container = container

        let border = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 0.33))
        container.addSubview(border)
        topBorder = border

        // The system keyboard has rounded top corners, which can expose the window
        // background beside an inputAccessoryView. Fill only those side gaps.
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

        layoutRow(primaryRow, into: container, atY: 0, width: width, storeIn: &primaryButtons)

        let detail = UIView(frame: CGRect(x: 0, y: rowHeight, width: width, height: rowHeight))
        detail.isHidden = true
        container.addSubview(detail)
        detailRowView = detail

        let sep = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 0.33))
        detail.addSubview(sep)
        detailSeparator = sep

        layoutRow(detailRow, into: detail, atY: 0, width: width, storeIn: &detailButtons)

        accessoryView = container
        applyTheme(isDark: isDarkTheme)
        refreshModifierHighlights()
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
        detailSeparator?.backgroundColor = borderColor
        leftKeyboardCornerFill?.backgroundColor = backgroundColor
        rightKeyboardCornerFill?.backgroundColor = backgroundColor

        let interfaceStyle: UIUserInterfaceStyle = isDark ? .dark : .light
        if #available(iOS 13.0, *) {
            container?.overrideUserInterfaceStyle = interfaceStyle
            detailRowView?.overrideUserInterfaceStyle = interfaceStyle
        }
        for (button, _) in primaryButtons + detailButtons {
            applyButtonTheme(button)
        }
        refreshModifierHighlights()
    }

    @objc
    func stopRepeating() {
        repeatTimer?.invalidate()
        repeatTimer = nil
        repeatingKey = nil
    }

    private func layoutRow(
        _ specs: [KeySpec],
        into parent: UIView,
        atY y: CGFloat,
        width: CGFloat,
        storeIn buttons: inout [(button: UIButton, spec: KeySpec)]
    ) {
        let buttonWidth = width / CGFloat(specs.count)
        for (index, spec) in specs.enumerated() {
            let button = UIButton(type: .system)
            button.frame = CGRect(
                x: buttonWidth * CGFloat(index),
                y: y,
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
            parent.addSubview(button)
            buttons.append((button, spec))
        }
    }

    private func applyButtonTheme(_ button: UIButton) {
        let foregroundColor =
            isDarkTheme
            ? UIColor(red: 0.96, green: 0.96, blue: 0.96, alpha: 1.0)
            : UIColor(red: 0.102, green: 0.102, blue: 0.102, alpha: 1.0)
        let interfaceStyle: UIUserInterfaceStyle = isDarkTheme ? .dark : .light
        button.setTitleColor(foregroundColor, for: .normal)
        button.tintColor = foregroundColor
        button.backgroundColor = .clear
        if #available(iOS 13.0, *) {
            button.overrideUserInterfaceStyle = interfaceStyle
        }
    }

    private func refreshModifierHighlights() {
        let highlight =
            isDarkTheme
            ? UIColor(white: 1.0, alpha: 0.18)
            : UIColor(white: 0.0, alpha: 0.12)
        for (button, spec) in detailButtons {
            switch spec.kind {
            case .modifier(let mod):
                button.backgroundColor = armedMods.contains(mod) ? highlight : .clear
            case .toggleDetail:
                button.backgroundColor = detailExpanded ? highlight : .clear
            case .key:
                button.backgroundColor = .clear
            }
        }
        for (button, spec) in primaryButtons {
            if case .toggleDetail = spec.kind {
                button.backgroundColor = detailExpanded ? highlight : .clear
            }
        }
    }

    private func setDetailExpanded(_ expanded: Bool) {
        guard detailExpanded != expanded else {
            return
        }
        detailExpanded = expanded
        detailRowView?.isHidden = !expanded
        container?.contentHeight = expanded ? rowHeight * 2 : rowHeight
        if !expanded {
            armedMods = []
        }
        refreshModifierHighlights()
    }

    private func handleSpec(_ spec: KeySpec) {
        switch spec.kind {
        case .toggleDetail:
            setDetailExpanded(!detailExpanded)
        case .modifier(let mod):
            armedMods.formSymmetricDifference(mod)
            refreshModifierHighlights()
        case .key(let name, let fixedMods):
            let mods = armedMods.union(fixedMods)
            sendKey?(name, mods.rawValue)
            if !armedMods.isEmpty {
                armedMods = []
                refreshModifierHighlights()
            }
        }
    }

    private func spec(for sender: UIButton) -> KeySpec? {
        for (button, spec) in primaryButtons where button === sender {
            return spec
        }
        for (button, spec) in detailButtons where button === sender {
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
            let mods = armedMods.union(fixedMods)
            sendKey?(name, mods.rawValue)
            let cleared = !armedMods.isEmpty
            if cleared {
                armedMods = []
                refreshModifierHighlights()
            }
            startRepeating(name: name, mods: mods.rawValue)
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
        } else {
            handleSpec(spec)
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
