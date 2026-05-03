import UIKit

@objcMembers
final class KeyboardSupporter: NSObject {
    private struct KeySpec {
        let label: String
        let key: String
        let repeats: Bool
    }

    private let keySpecs = [
        KeySpec(label: "Esc", key: "escape", repeats: false),
        KeySpec(label: "Tab", key: "tab", repeats: false),
        KeySpec(label: "←", key: "left", repeats: true),
        KeySpec(label: "↓", key: "down", repeats: true),
        KeySpec(label: "↑", key: "up", repeats: true),
        KeySpec(label: "→", key: "right", repeats: true),
        KeySpec(label: "⏎", key: "enter", repeats: false),
    ]

    private let repeatInitialDelay: TimeInterval = 0.35
    private let repeatInterval: TimeInterval = 0.06

    private(set) var accessoryView: UIView?
    private var sendKey: ((String) -> Void)?
    private var repeatTimer: Timer?
    private var repeatingKey: String?

    func makeAccessoryView(width: CGFloat, sendKey: @escaping (String) -> Void) -> UIView {
        stopRepeating()
        self.sendKey = sendKey

        let height: CGFloat = 44.0
        let bar = UIView(frame: CGRect(x: 0, y: 0, width: width, height: height))
        bar.backgroundColor = UIColor(red: 0.055, green: 0.047, blue: 0.047, alpha: 0.96)
        if #available(iOS 13.0, *) {
            bar.overrideUserInterfaceStyle = .dark
        }

        let border = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 0.33))
        border.backgroundColor = UIColor(white: 1.0, alpha: 0.12)
        bar.addSubview(border)

        let buttonWidth = width / CGFloat(keySpecs.count)

        for (index, spec) in keySpecs.enumerated() {
            let button = UIButton(type: .system)
            button.frame = CGRect(x: buttonWidth * CGFloat(index), y: 0, width: buttonWidth, height: height)
            button.setTitle(spec.label, for: .normal)
            button.titleLabel?.font = .systemFont(ofSize: 16.0)
            let color = UIColor(red: 0.96, green: 0.96, blue: 0.96, alpha: 1.0)
            button.setTitleColor(color, for: .normal)
            button.tintColor = color
            if #available(iOS 13.0, *) {
                button.overrideUserInterfaceStyle = .dark
            }
            button.tag = index
            button.addTarget(self, action: #selector(buttonTouchDown(_:)), for: .touchDown)
            button.addTarget(self, action: #selector(buttonTouchUpInside(_:)), for: .touchUpInside)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchUpOutside)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchCancel)
            button.addTarget(self, action: #selector(stopRepeating), for: .touchDragExit)
            bar.addSubview(button)
        }

        accessoryView = bar
        return bar
    }

    func stopRepeating() {
        repeatTimer?.invalidate()
        repeatTimer = nil
        repeatingKey = nil
    }

    private func keySpec(for sender: UIButton) -> KeySpec? {
        guard keySpecs.indices.contains(sender.tag) else {
            return nil
        }
        return keySpecs[sender.tag]
    }

    @objc
    private func buttonTouchDown(_ sender: UIButton) {
        guard let spec = keySpec(for: sender), spec.repeats else {
            return
        }
        sendKey?(spec.key)
        startRepeating(spec.key)
    }

    @objc
    private func buttonTouchUpInside(_ sender: UIButton) {
        guard let spec = keySpec(for: sender) else {
            stopRepeating()
            return
        }

        if spec.repeats {
            stopRepeating()
        } else {
            sendKey?(spec.key)
        }
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
            self.sendKey?(key)
        }
        timer.fireDate = Date(timeIntervalSinceNow: repeatInitialDelay)
        repeatTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }
}
