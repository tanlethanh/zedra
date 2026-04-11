import UIKit

@objcMembers
final class KeyboardSupporter: NSObject {
    private(set) var accessoryView: UIView?

    func makeAccessoryView(width: CGFloat, target: Any, action: Selector) -> UIView {
        let height: CGFloat = 44.0
        let bar = UIView(frame: CGRect(x: 0, y: 0, width: width, height: height))
        bar.backgroundColor = UIColor(red: 0.055, green: 0.047, blue: 0.047, alpha: 0.96)
        if #available(iOS 13.0, *) {
            bar.overrideUserInterfaceStyle = .dark
        }

        let border = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 0.33))
        border.backgroundColor = UIColor(white: 1.0, alpha: 0.12)
        bar.addSubview(border)

        let labels = ["Esc", "Tab", "←", "↓", "↑", "→", "⏎"]
        let buttonWidth = width / CGFloat(labels.count)

        for (index, label) in labels.enumerated() {
            let button = UIButton(type: .system)
            button.frame = CGRect(x: buttonWidth * CGFloat(index), y: 0, width: buttonWidth, height: height)
            button.setTitle(label, for: .normal)
            button.titleLabel?.font = .systemFont(ofSize: 16.0)
            let color = UIColor(red: 0.96, green: 0.96, blue: 0.96, alpha: 1.0)
            button.setTitleColor(color, for: .normal)
            button.tintColor = color
            if #available(iOS 13.0, *) {
                button.overrideUserInterfaceStyle = .dark
            }
            button.tag = index
            button.addTarget(target, action: action, for: .touchUpInside)
            bar.addSubview(button)
        }

        accessoryView = bar
        return bar
    }
}
