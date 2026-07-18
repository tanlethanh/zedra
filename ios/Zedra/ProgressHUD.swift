import UIKit

// Non-blocking upload-progress toast: a top pill with a spinner + short message.
// One HUD at a time; a new id retargets the same pill instead of stacking.
enum ProgressHUD {
    private static var currentID: UInt32?
    private static var container: UIVisualEffectView?
    private static var label: UILabel?
    private static var spinner: UIActivityIndicatorView?

    static func show(id: UInt32, message: String) {
        currentID = id
        if container == nil {
            makeContainer()
        } else {
            container?.layer.removeAllAnimations()
            container?.alpha = 1
        }

        let text = message.isEmpty ? "Working…" : message
        label?.text = text
        container?.effect = overlayEffect()
        container?.accessibilityLabel = text
        label?.textColor = NativePresentationTheme.primaryTextColor
        spinner?.color = NativePresentationTheme.secondaryTextColor
        spinner?.startAnimating()
    }

    static func dismiss(id: UInt32) {
        // Ignore stale ids: only the currently-shown upload may hide the pill.
        guard id == currentID, let container else { return }
        currentID = nil
        UIView.animate(
            withDuration: 0.18,
            animations: { container.alpha = 0 },
            completion: { _ in
                // A newer show() may have re-adopted the pill mid-fade.
                guard currentID == nil else { return }
                spinner?.stopAnimating()
                container.removeFromSuperview()
                self.container = nil
                self.label = nil
                self.spinner = nil
            }
        )
    }

    private static func makeContainer() {
        let window = NativePresentationBridge.activeWindow()

        let effectView = UIVisualEffectView(effect: nil)
        effectView.isUserInteractionEnabled = false
        effectView.clipsToBounds = true
        effectView.alpha = 0
        effectView.translatesAutoresizingMaskIntoConstraints = false
        effectView.isAccessibilityElement = true
        effectView.accessibilityTraits.insert(.updatesFrequently)

        let spinner = UIActivityIndicatorView(style: .medium)
        spinner.hidesWhenStopped = false
        spinner.translatesAutoresizingMaskIntoConstraints = false

        let label = UILabel()
        label.font = .systemFont(ofSize: 14, weight: .medium)
        label.numberOfLines = 1
        label.lineBreakMode = .byTruncatingTail
        label.translatesAutoresizingMaskIntoConstraints = false

        let stack = UIStackView(arrangedSubviews: [spinner, label])
        stack.axis = .horizontal
        stack.alignment = .center
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false
        effectView.contentView.addSubview(stack)
        window.addSubview(effectView)

        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: effectView.contentView.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: effectView.contentView.bottomAnchor, constant: -10),
            stack.leadingAnchor.constraint(equalTo: effectView.contentView.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: effectView.contentView.trailingAnchor, constant: -16),
            label.widthAnchor.constraint(lessThanOrEqualToConstant: 240),
            effectView.centerXAnchor.constraint(equalTo: window.centerXAnchor),
            effectView.bottomAnchor.constraint(
                equalTo: window.safeAreaLayoutGuide.bottomAnchor, constant: -16),
            effectView.leadingAnchor.constraint(greaterThanOrEqualTo: window.leadingAnchor, constant: 16),
            effectView.trailingAnchor.constraint(lessThanOrEqualTo: window.trailingAnchor, constant: -16),
        ])
        effectView.layoutIfNeeded()
        effectView.layer.cornerRadius = effectView.bounds.height / 2
        effectView.layer.cornerCurve = .continuous

        container = effectView
        self.label = label
        self.spinner = spinner

        UIView.animate(withDuration: 0.22) { effectView.alpha = 1 }
    }

    private static func overlayEffect() -> UIVisualEffect {
        if #available(iOS 26.0, *) {
            let effect = UIGlassEffect(style: .regular)
            effect.isInteractive = false
            return effect
        }
        return NativePresentationTheme.blurEffect()
    }
}

@_cdecl("ios_present_native_progress")
func ios_present_native_progress(_ id: UInt32, _ message: UnsafePointer<CChar>?) {
    let text = NativePresentationBridge.string(message) ?? ""
    NativePresentationBridge.onMain { ProgressHUD.show(id: id, message: text) }
}

@_cdecl("ios_dismiss_native_progress")
func ios_dismiss_native_progress(_ id: UInt32) {
    NativePresentationBridge.onMain { ProgressHUD.dismiss(id: id) }
}
