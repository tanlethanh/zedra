import Foundation
import ObjectiveC.runtime
import UIKit
import ZedraFFI

@_silgen_name("gpui_ios_request_frame_forced")
private func gpui_ios_request_frame_forced(_ windowPtr: UnsafeMutableRawPointer?)

@_silgen_name("gpui_ios_handle_view_resize")
private func gpui_ios_handle_view_resize(
    _ windowPtr: UnsafeMutableRawPointer?, _ widthPts: Float, _ heightPts: Float
)

@_silgen_name("gpui_ios_inject_scroll")
private func gpui_ios_inject_scroll(
    _ windowPtr: UnsafeMutableRawPointer?,
    _ originX: Float,
    _ originY: Float,
    _ deltaX: Float,
    _ deltaY: Float,
    _ velocityX: Float,
    _ velocityY: Float,
    _ phase: Int32
)

@_silgen_name("zedra_ios_mount_custom_sheet_content")
private func zedra_ios_mount_custom_sheet_content(
    _ parentViewPtr: UnsafeMutableRawPointer?, _ widthPts: Float, _ heightPts: Float
) -> UnsafeMutableRawPointer?

@_silgen_name("zedra_ios_unmount_custom_sheet_content")
private func zedra_ios_unmount_custom_sheet_content()

@_silgen_name("zedra_ios_sheet_content_is_at_top")
private func zedra_ios_sheet_content_is_at_top() -> Bool

@_silgen_name("zedra_ios_text_input_result")
private func zedra_ios_text_input_result(_ callbackID: UInt32, _ value: UnsafePointer<CChar>?)

@_silgen_name("zedra_ios_text_input_dismiss")
private func zedra_ios_text_input_dismiss(_ callbackID: UInt32)

@_silgen_name("zedra_ios_native_floating_button_pressed")
private func zedra_ios_native_floating_button_pressed(_ callbackID: UInt32)

@_silgen_name("zedra_ios_dictation_preview_dismiss")
private func zedra_ios_dictation_preview_dismiss(_ previewID: UInt32)

@_silgen_name("zedra_ios_native_edit_menu_result")
private func zedra_ios_native_edit_menu_result(_ callbackID: UInt32, _ itemIndex: Int32)

@_silgen_name("zedra_ios_native_edit_menu_dismiss")
private func zedra_ios_native_edit_menu_dismiss(_ callbackID: UInt32)

@_silgen_name("zedra_ios_native_notification_action")
private func zedra_ios_native_notification_action(_ callbackID: UInt32)

@_silgen_name("zedra_ios_native_notification_dismiss")
private func zedra_ios_native_notification_dismiss(_ callbackID: UInt32)

fileprivate enum AlertActionStyle: Int32 {
    case `default` = 0
    case cancel = 1
    case destructive = 2

    var uiKitStyle: UIAlertAction.Style {
        switch self {
        case .default:
            return .default
        case .cancel:
            return .cancel
        case .destructive:
            return .destructive
        }
    }
}

fileprivate enum CustomSheetDetent: Int32 {
    case medium = 0
    case large = 1

    var identifier: UISheetPresentationController.Detent.Identifier {
        switch self {
        case .medium:
            return .medium
        case .large:
            return .large
        }
    }

    var uiKitDetent: UISheetPresentationController.Detent {
        switch self {
        case .medium:
            return .medium()
        case .large:
            return .large()
        }
    }
}

fileprivate enum NativeNotificationKind: Int32 {
    case info = 0
    case success = 1
    case warning = 2
    case error = 3

    var defaultSystemImageName: String {
        switch self {
        case .info:
            return "info.circle"
        case .success:
            return "checkmark.circle.fill"
        case .warning:
            return "exclamationmark.triangle.fill"
        case .error:
            return "xmark.octagon.fill"
        }
    }
}

enum NativePresentationTheme {
    // Default to the current system trait so early presentations (before the
    // Rust ThemeState pushes the saved preference) don't hard-snap to dark on
    // a light device.
    private(set) static var isDark: Bool = {
        UITraitCollection.current.userInterfaceStyle == .dark
    }()

    static func setDark(_ value: Bool) {
        isDark = value
        NativeNotificationBannerController.shared.applyTheme()
        NativeFloatingButtonController.shared.applyTheme()
        NativeDictationPreviewController.shared.applyTheme()
    }

    static var interfaceStyle: UIUserInterfaceStyle {
        isDark ? .dark : .light
    }

    static var backgroundColor: UIColor {
        isDark ? UIColor(red: 0.055, green: 0.047, blue: 0.047, alpha: 1) : UIColor(red: 0.961, green: 0.961, blue: 0.961, alpha: 1)
    }

    static var surfaceColor: UIColor {
        isDark ? UIColor(red: 0.075, green: 0.075, blue: 0.075, alpha: 1) : .white
    }

    static var overlayColor: UIColor {
        isDark ? UIColor(red: 0.075, green: 0.075, blue: 0.075, alpha: 1) : .white
    }

    static var primaryTextColor: UIColor {
        isDark ? .white : UIColor(red: 0.102, green: 0.102, blue: 0.102, alpha: 1)
    }

    static var secondaryTextColor: UIColor {
        isDark ? UIColor(red: 0.792, green: 0.792, blue: 0.792, alpha: 1) : UIColor(red: 0.29, green: 0.29, blue: 0.29, alpha: 1)
    }

    static var mutedTextColor: UIColor {
        isDark ? UIColor(red: 0.314, green: 0.314, blue: 0.314, alpha: 1) : UIColor(red: 0.541, green: 0.541, blue: 0.541, alpha: 1)
    }

    static var borderColor: UIColor {
        isDark ? UIColor(red: 0.173, green: 0.173, blue: 0.173, alpha: 1) : UIColor(red: 0.847, green: 0.847, blue: 0.847, alpha: 1)
    }

    static var accentGreen: UIColor {
        isDark ? UIColor(red: 0.596, green: 0.765, blue: 0.475, alpha: 1) : UIColor(red: 0.102, green: 0.498, blue: 0.216, alpha: 1)
    }

    static var accentYellow: UIColor {
        isDark ? UIColor(red: 0.898, green: 0.753, blue: 0.482, alpha: 1) : UIColor(red: 0.604, green: 0.404, blue: 0, alpha: 1)
    }

    static var accentRed: UIColor {
        isDark ? UIColor(red: 0.878, green: 0.424, blue: 0.459, alpha: 1) : UIColor(red: 0.812, green: 0.133, blue: 0.18, alpha: 1)
    }

    static func blurEffect() -> UIVisualEffect {
        UIBlurEffect(style: isDark ? .systemChromeMaterialDark : .systemChromeMaterialLight)
    }
}

private final class PresentationDismissDelegate: NSObject, UIAdaptivePresentationControllerDelegate {
    let callbackID: UInt32
    let isSelection: Bool
    var handled = false

    init(callbackID: UInt32, isSelection: Bool) {
        self.callbackID = callbackID
        self.isSelection = isSelection
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        guard !handled else { return }
        handled = true
        if isSelection {
            zedra_ios_selection_dismiss(callbackID)
        } else {
            zedra_ios_alert_dismiss(callbackID)
        }
    }
}

private final class AgentListPickerViewController: UITableViewController {
    private let callbackID: UInt32
    private let pickerTitle: String?
    private let pickerMessage: String?
    private let itemLabels: [String]
    private let itemSubtitles: [String?]
    private let itemImageNames: [String?]
    init(
        callbackID: UInt32,
        title: String?,
        message: String?,
        itemLabels: [String],
        itemSubtitles: [String?],
        itemImageNames: [String?]
    ) {
        self.callbackID = callbackID
        self.pickerTitle = title
        self.pickerMessage = message
        self.itemLabels = itemLabels
        self.itemSubtitles = itemSubtitles
        self.itemImageNames = itemImageNames
        super.init(style: .insetGrouped)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        overrideUserInterfaceStyle = NativePresentationTheme.interfaceStyle
        view.backgroundColor = NativePresentationTheme.backgroundColor
        navigationController?.navigationBar.tintColor = NativePresentationTheme.primaryTextColor
        navigationItem.leftBarButtonItem = UIBarButtonItem(
            barButtonSystemItem: .cancel,
            target: self,
            action: #selector(cancelTapped)
        )
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "AgentListPickerCell")
        if #available(iOS 15.0, *) {
            tableView.sectionHeaderTopPadding = 0
        }
        // Suppress the default top gap insetGrouped adds before the first section.
        tableView.tableHeaderView = UIView(frame: CGRect(x: 0, y: 0, width: 0, height: CGFloat.leastNormalMagnitude))

        let stack = UIStackView()
        stack.axis = .vertical
        stack.alignment = .center
        stack.spacing = 2

        let titleLabel = UILabel()
        titleLabel.text = pickerTitle
        titleLabel.font = .systemFont(ofSize: 16, weight: .semibold)
        titleLabel.textColor = NativePresentationTheme.primaryTextColor
        stack.addArrangedSubview(titleLabel)

        if let message = pickerMessage, !message.isEmpty {
            let subtitleLabel = UILabel()
            subtitleLabel.text = message
            subtitleLabel.font = .systemFont(ofSize: 12, weight: .regular)
            subtitleLabel.textColor = NativePresentationTheme.secondaryTextColor
            subtitleLabel.numberOfLines = 1
            stack.addArrangedSubview(subtitleLabel)
        }

        stack.sizeToFit()
        navigationItem.titleView = stack
    }

    @objc private func cancelTapped() {
        dismiss(animated: true) {
            zedra_ios_selection_dismiss(self.callbackID)
        }
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        itemLabels.count
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "AgentListPickerCell", for: indexPath)
        var content = UIListContentConfiguration.subtitleCell()
        content.text = itemLabels[indexPath.row]
        content.textProperties.color = NativePresentationTheme.primaryTextColor
        content.secondaryTextProperties.color = NativePresentationTheme.secondaryTextColor
        if let subtitle = itemSubtitles[safe: indexPath.row].flatMap({ $0 }), !subtitle.isEmpty {
            content.secondaryText = subtitle
        }
        if let imageName = itemImageNames[safe: indexPath.row].flatMap({ $0 }),
           let image = NativePresentationBridge.actionListImage(named: imageName) {
            content.image = image
            content.imageProperties.reservedLayoutSize = CGSize(width: 28, height: 28)
        }
        cell.contentConfiguration = content
        cell.selectionStyle = .default
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        dismiss(animated: true) {
            zedra_ios_selection_result(self.callbackID, Int32(indexPath.row))
        }
    }
}

private final class AlertOutsideTapDismissHandler: NSObject, UIGestureRecognizerDelegate {
    private let callbackID: UInt32
    private weak var alert: UIAlertController?
    private weak var gestureHost: UIView?
    private var tapGesture: UITapGestureRecognizer?
    private var handled = false

    init(callbackID: UInt32, alert: UIAlertController) {
        self.callbackID = callbackID
        self.alert = alert
    }

    func install() {
        guard let host = alert?.view.window else { return }

        let recognizer = UITapGestureRecognizer(
            target: self,
            action: #selector(handleOutsideTap(_:))
        )
        recognizer.cancelsTouchesInView = false
        recognizer.delegate = self
        host.addGestureRecognizer(recognizer)

        gestureHost = host
        tapGesture = recognizer
    }

    func markHandled() {
        guard !handled else { return }
        handled = true
        cleanup()
    }

    @objc
    private func handleOutsideTap(_ gesture: UITapGestureRecognizer) {
        guard gesture.state == .ended, !handled, let alert else { return }

        markHandled()
        alert.dismiss(animated: true) {
            zedra_ios_alert_dismiss(self.callbackID)
        }
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldReceive touch: UITouch
    ) -> Bool {
        guard let alertView = alert?.view else { return false }
        let point = touch.location(in: alertView)
        return !alertView.bounds.contains(point)
    }

    private func cleanup() {
        if let tapGesture {
            gestureHost?.removeGestureRecognizer(tapGesture)
        }
        tapGesture = nil
        gestureHost = nil
    }
}

enum NativePresentationBridge {
    /// Returns the active key window, preferring foreground-active scenes.
    /// Falls back progressively to any visible window, then a last-resort empty window.
    static func activeWindow() -> UIWindow {
        let scenes = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .filter { $0.activationState == .foregroundActive }

        for scene in scenes {
            if let keyWindow = scene.windows.first(where: \.isKeyWindow) { return keyWindow }
        }
        for scene in scenes {
            if let visibleWindow = scene.windows.first(where: { !$0.isHidden }) { return visibleWindow }
        }
        return UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first ?? UIWindow(frame: UIScreen.main.bounds)
    }

    static func topViewController() -> UIViewController? {
        let keyWindow = activeWindow()
        var controller = keyWindow.rootViewController
        while let presented = controller?.presentedViewController {
            controller = presented
        }
        return controller
    }

    static func string(_ pointer: UnsafePointer<CChar>?) -> String? {
        guard let pointer else { return nil }
        return String(cString: pointer)
    }

    /// Run `work` on the main thread, synchronously if already there. GPUI drives the
    /// main thread via CADisplayLink, so a queued main-async block can be starved when
    /// invoked from a native-menu action re-entering GPUI; running inline avoids the hop.
    static func onMain(_ work: @escaping () -> Void) {
        if Thread.isMainThread {
            work()
        } else {
            DispatchQueue.main.async(execute: work)
        }
    }

    static func strings(count: Int32, labels: UnsafePointer<UnsafePointer<CChar>?>?) -> [String] {
        guard let labels, count > 0 else { return [] }
        return (0..<Int(count)).map { index in
            string(labels[index]) ?? "OK"
        }
    }

    static func optionalStrings(
        count: Int32,
        labels: UnsafePointer<UnsafePointer<CChar>?>?
    ) -> [String?] {
        guard let labels, count > 0 else { return [] }
        return (0..<Int(count)).map { index in
            guard let value = string(labels[index]), !value.isEmpty else { return nil }
            return value
        }
    }

    private static let actionIconPointSize: CGFloat = 22
    private static let iconInsets: [String: CGFloat] = [
        "google": 2,
    ]

    static func actionImage(named imageName: String) -> UIImage? {
        var name = (imageName as NSString).lastPathComponent
        if name.hasSuffix(".svg") { name = String(name.dropLast(4)) }
        guard let raw = UIImage(named: name) else { return nil }
        let inset = iconInsets[name] ?? 0
        let canvas = CGSize(width: actionIconPointSize, height: actionIconPointSize)
        let renderer = UIGraphicsImageRenderer(size: canvas)
        let rendered = renderer.image { _ in
            raw.draw(in: CGRect(x: inset, y: inset, width: canvas.width - inset * 2, height: canvas.height - inset * 2))
        }
        return rendered.withRenderingMode(.alwaysTemplate)
    }

    private static let actionListIconPointSize: CGFloat = 22
    static func actionListImage(named imageName: String) -> UIImage? {
        guard let image = actionImage(named: imageName) else { return nil }
        let size = CGSize(width: actionListIconPointSize, height: actionListIconPointSize)
        let tinted = image.withTintColor(NativePresentationTheme.primaryTextColor, renderingMode: .alwaysTemplate)
        let format = UIGraphicsImageRendererFormat.default()
        format.scale = UIScreen.main.scale
        let renderer = UIGraphicsImageRenderer(size: size, format: format)
        return renderer.image { _ in
            tinted.draw(in: CGRect(origin: .zero, size: size))
        }.withRenderingMode(.alwaysOriginal)
    }

    static func notificationImage(named imageName: String) -> UIImage? {
        let image = UIImage(named: imageName) ?? UIImage(systemName: imageName)
        return image?.withRenderingMode(.alwaysTemplate)
    }

    fileprivate static func styles(count: Int32, styles: UnsafePointer<Int32>?) -> [AlertActionStyle] {
        guard count > 0 else { return [] }
        return (0..<Int(count)).map { index in
            let rawValue = styles?[index] ?? AlertActionStyle.default.rawValue
            return AlertActionStyle(rawValue: rawValue) ?? .default
        }
    }

    fileprivate static func detents(
        count: Int32,
        detents: UnsafePointer<Int32>?
    ) -> [CustomSheetDetent] {
        guard count > 0 else { return [.medium, .large] }
        let parsed = (0..<Int(count)).compactMap { index -> CustomSheetDetent? in
            guard let rawValue = detents?[index] else { return nil }
            return CustomSheetDetent(rawValue: rawValue)
        }
        return parsed.isEmpty ? [.medium, .large] : parsed
    }
}

private let nativeFloatingButtonDefaultIconPointSize: CGFloat = 16.0
private let nativeFloatingButtonDefaultIconWeightRawValue: Int32 = 5

private final class NativeFloatingButtonController {
    static let shared = NativeFloatingButtonController()

    private final class Control {
        private static let enterDuration: TimeInterval = 0.22
        private static let exitDuration: TimeInterval = 0.18
        private static let initialScale = CGAffineTransform(scaleX: 0.78, y: 0.78)
        private static let exitScale = CGAffineTransform(scaleX: 0.86, y: 0.86)

        let effectView: UIVisualEffectView
        let button: UIButton
        weak var window: UIWindow?
        private var isVisible = false
        private var animationGeneration: UInt64 = 0

        init(callbackID: UInt32, owner: NativeFloatingButtonController) {
            effectView = UIVisualEffectView(effect: nil)
            effectView.clipsToBounds = true
            effectView.transform = Self.initialScale

            button = UIButton(type: .system)
            button.tintColor = NativePresentationTheme.primaryTextColor
            button.alpha = 0
            button.addAction(
                UIAction { [weak owner] _ in
                    owner?.buttonTapped(callbackID: callbackID)
                },
                for: .touchUpInside
            )
            effectView.contentView.addSubview(button)
        }

        func update(
            in window: UIWindow,
            frame: CGRect,
            systemImageName: String,
            accessibilityLabel: String,
            iconPointSize: CGFloat,
            iconWeight: UIImage.SymbolWeight
        ) {
            let wasDetached = effectView.superview == nil || self.window !== window
            if self.window !== window {
                effectView.removeFromSuperview()
                window.addSubview(effectView)
                self.window = window
            } else if effectView.superview == nil {
                window.addSubview(effectView)
            }

            let imageConfig = UIImage.SymbolConfiguration(
                pointSize: Self.resolvedIconPointSize(iconPointSize),
                weight: iconWeight
            )
            button.setImage(
                UIImage(systemName: systemImageName, withConfiguration: imageConfig),
                for: .normal
            )
            button.accessibilityLabel = accessibilityLabel
            effectView.frame = frame
            effectView.layer.cornerRadius = min(frame.width, frame.height) / 2
            if #available(iOS 13.0, *) {
                effectView.layer.cornerCurve = .continuous
            }
            button.frame = effectView.bounds
            effectView.isHidden = false
            effectView.isUserInteractionEnabled = true
            window.bringSubviewToFront(effectView)

            if wasDetached || !isVisible {
                materialize()
            }
        }

        func dematerialize(completion: @escaping () -> Void) {
            guard effectView.superview != nil else {
                completion()
                return
            }

            animationGeneration &+= 1
            let generation = animationGeneration
            isVisible = false
            effectView.isUserInteractionEnabled = false
            effectView.layer.removeAllAnimations()
            button.layer.removeAllAnimations()

            UIView.animate(
                withDuration: Self.exitDuration,
                delay: 0,
                options: [.beginFromCurrentState, .curveEaseInOut],
                animations: {
                    self.effectView.effect = nil
                    self.effectView.transform = Self.exitScale
                    self.button.alpha = 0
                },
                completion: { _ in
                    guard self.animationGeneration == generation else { return }
                    self.effectView.removeFromSuperview()
                    self.effectView.transform = Self.initialScale
                    self.effectView.effect = nil
                    completion()
                }
            )
        }

        private func materialize() {
            animationGeneration &+= 1
            isVisible = true
            effectView.layer.removeAllAnimations()
            button.layer.removeAllAnimations()

            effectView.effect = nil
            effectView.transform = Self.initialScale
            button.alpha = 0

            UIView.animate(
                withDuration: Self.enterDuration,
                delay: 0,
                usingSpringWithDamping: 0.78,
                initialSpringVelocity: 0.2,
                options: [.beginFromCurrentState, .allowUserInteraction],
                animations: {
                    self.effectView.effect = Self.buttonEffect()
                    self.effectView.transform = .identity
                    self.button.alpha = 1
                }
            )
        }

        private static func buttonEffect() -> UIVisualEffect {
            if #available(iOS 26.0, *) {
                let effect = UIGlassEffect(style: .regular)
                effect.isInteractive = true
                effect.tintColor = NativePresentationTheme.overlayColor.withAlphaComponent(0.45)
                return effect
            }

            return NativePresentationTheme.blurEffect()
        }

        static func symbolWeight(_ rawValue: Int32) -> UIImage.SymbolWeight {
            switch rawValue {
            case 0:
                return .unspecified
            case 1:
                return .ultraLight
            case 2:
                return .thin
            case 3:
                return .light
            case 4:
                return .regular
            case 5:
                return .medium
            case 6:
                return .semibold
            case 7:
                return .bold
            case 8:
                return .heavy
            case 9:
                return .black
            default:
                return .semibold
            }
        }

        private static func resolvedIconPointSize(_ pointSize: CGFloat) -> CGFloat {
            guard pointSize.isFinite && pointSize > 0 else {
                return nativeFloatingButtonDefaultIconPointSize
            }

            return pointSize
        }
    }

    private var controls: [UInt32: Control] = [:]

    func update(
        callbackID: UInt32,
        systemImageName: String,
        accessibilityLabel: String,
        frame: CGRect,
        iconPointSize: CGFloat,
        iconWeight: Int32
    ) {
        DispatchQueue.main.async {
            let window = NativePresentationBridge.activeWindow()
            let control = self.controls[callbackID] ?? {
                let control = Control(callbackID: callbackID, owner: self)
                self.controls[callbackID] = control
                return control
            }()
            control.update(
                in: window,
                frame: frame.integral,
                systemImageName: systemImageName,
                accessibilityLabel: accessibilityLabel,
                iconPointSize: iconPointSize,
                iconWeight: Control.symbolWeight(iconWeight)
            )
        }
    }

    func hide(callbackID: UInt32) {
        DispatchQueue.main.async {
            guard let control = self.controls[callbackID] else { return }
            control.dematerialize { [weak self, weak control] in
                guard let self, let control, self.controls[callbackID] === control else { return }
                self.controls.removeValue(forKey: callbackID)
            }
        }
    }

    private func buttonTapped(callbackID: UInt32) {
        zedra_ios_native_floating_button_pressed(callbackID)
    }

    func applyTheme() {
        DispatchQueue.main.async {
            for control in self.controls.values {
                control.button.tintColor = NativePresentationTheme.primaryTextColor
            }
        }
    }
}

private final class NativeDictationPreviewController {
    static let shared = NativeDictationPreviewController()

    private final class Overlay: NSObject {
        private static let enterDuration: TimeInterval = 0.22
        private static let exitDuration: TimeInterval = 0.16
        private static let initialScale = CGAffineTransform(scaleX: 0.94, y: 0.94)
        private static let exitScale = CGAffineTransform(scaleX: 0.96, y: 0.96)
        private static let labelInsets = UIEdgeInsets(top: 10, left: 14, bottom: 10, right: 14)
        private static let maxContentHeight: CGFloat = 72

        let previewID: UInt32
        let effectView: UIVisualEffectView
        let label: UILabel
        weak var owner: NativeDictationPreviewController?
        weak var window: UIWindow?
        private var isVisible = false
        private var animationGeneration: UInt64 = 0
        private var lastRenderedText: String?
        private var lastBottomOffset: CGFloat?
        private var lastWindowBounds = CGRect.null

        init(previewID: UInt32, owner: NativeDictationPreviewController) {
            self.previewID = previewID
            self.owner = owner
            effectView = UIVisualEffectView(effect: nil)
            label = UILabel()

            super.init()

            effectView.clipsToBounds = true
            effectView.transform = Self.initialScale
            effectView.isUserInteractionEnabled = true
            effectView.isAccessibilityElement = true
            effectView.accessibilityLabel = "Dictation preview"
            effectView.accessibilityHint = "Double tap to dismiss"
            effectView.accessibilityTraits.insert(.button)

            label.font = UIFont.monospacedSystemFont(ofSize: 13, weight: .medium)
            label.textColor = NativePresentationTheme.primaryTextColor
            label.numberOfLines = 3
            label.lineBreakMode = .byTruncatingTail
            label.alpha = 0
            effectView.contentView.addSubview(label)

            let tapGesture = UITapGestureRecognizer(
                target: self,
                action: #selector(dismissFromTap)
            )
            effectView.addGestureRecognizer(tapGesture)
        }

        @objc private func dismissFromTap() {
            owner?.dismissFromTap(previewID: previewID)
        }

        func update(in window: UIWindow, text: String, bottomOffset: CGFloat) {
            let wasDetached = effectView.superview == nil || self.window !== window
            let displayText = text.isEmpty ? "Listening..." : text
            if self.window !== window {
                effectView.removeFromSuperview()
                window.addSubview(effectView)
                self.window = window
            } else if effectView.superview == nil {
                window.addSubview(effectView)
            }

            if !wasDetached,
               isVisible,
               lastRenderedText == displayText,
               lastBottomOffset == bottomOffset,
               lastWindowBounds == window.bounds
            {
                window.bringSubviewToFront(effectView)
                return
            }

            label.text = displayText
            effectView.accessibilityValue = displayText

            let frame = Self.frame(
                in: window.bounds,
                fitting: label,
                bottomOffset: bottomOffset
            )
            effectView.frame = frame.integral
            effectView.layer.cornerRadius = min(22, frame.height / 2)
            if #available(iOS 13.0, *) {
                effectView.layer.cornerCurve = .continuous
            }
            label.frame = effectView.bounds.inset(by: Self.labelInsets)
            effectView.isHidden = false
            window.bringSubviewToFront(effectView)
            lastRenderedText = displayText
            lastBottomOffset = bottomOffset
            lastWindowBounds = window.bounds

            if wasDetached || !isVisible {
                materialize()
            }
        }

        func dematerialize(completion: @escaping () -> Void) {
            guard effectView.superview != nil else {
                completion()
                return
            }

            animationGeneration &+= 1
            let generation = animationGeneration
            isVisible = false
            effectView.layer.removeAllAnimations()
            label.layer.removeAllAnimations()

            UIView.animate(
                withDuration: Self.exitDuration,
                delay: 0,
                options: [.beginFromCurrentState, .curveEaseInOut],
                animations: {
                    self.effectView.effect = nil
                    self.effectView.transform = Self.exitScale
                    self.label.alpha = 0
                },
                completion: { _ in
                    guard self.animationGeneration == generation else { return }
                    self.effectView.removeFromSuperview()
                    self.effectView.transform = Self.initialScale
                    self.effectView.effect = nil
                    self.lastRenderedText = nil
                    self.lastBottomOffset = nil
                    self.lastWindowBounds = .null
                    completion()
                }
            )
        }

        private func materialize() {
            animationGeneration &+= 1
            isVisible = true
            effectView.layer.removeAllAnimations()
            label.layer.removeAllAnimations()

            effectView.effect = nil
            effectView.transform = Self.initialScale
            label.alpha = 0

            UIView.animate(
                withDuration: Self.enterDuration,
                delay: 0,
                usingSpringWithDamping: 0.84,
                initialSpringVelocity: 0.12,
                options: [.beginFromCurrentState, .allowUserInteraction],
                animations: {
                    self.effectView.effect = Self.overlayEffect()
                    self.effectView.transform = .identity
                    self.label.alpha = 1
                }
            )
        }

        private static func frame(
            in bounds: CGRect,
            fitting label: UILabel,
            bottomOffset: CGFloat
        ) -> CGRect {
            let horizontalMargin = min(24, max(12, bounds.width * 0.08))
            let maxWidth = max(80, min(bounds.width - (horizontalMargin * 2), 420))
            let minWidth = min(maxWidth, 140)
            let labelMaxWidth = max(1, maxWidth - labelInsets.left - labelInsets.right)
            let fittingSize = label.sizeThatFits(
                CGSize(width: labelMaxWidth, height: maxContentHeight)
            )
            let width = max(
                minWidth,
                min(maxWidth, ceil(fittingSize.width + labelInsets.left + labelInsets.right))
            )
            let height = max(
                42,
                min(96, ceil(fittingSize.height + labelInsets.top + labelInsets.bottom))
            )
            let bottom = max(16, bottomOffset.isFinite ? bottomOffset : 16)
            let x = bounds.midX - (width / 2)
            let y = max(12, bounds.height - bottom - height)
            return CGRect(x: x, y: y, width: width, height: height)
        }

        private static func overlayEffect() -> UIVisualEffect {
            if #available(iOS 26.0, *) {
                let effect = UIGlassEffect(style: .regular)
                effect.isInteractive = false
                effect.tintColor = NativePresentationTheme.overlayColor.withAlphaComponent(0.48)
                return effect
            }

            return NativePresentationTheme.blurEffect()
        }
    }

    private var overlays: [UInt32: Overlay] = [:]

    func update(previewID: UInt32, text: String, bottomOffset: CGFloat) {
        DispatchQueue.main.async {
            let window = NativePresentationBridge.activeWindow()
            let overlay = self.overlays[previewID] ?? {
                let overlay = Overlay(previewID: previewID, owner: self)
                self.overlays[previewID] = overlay
                return overlay
            }()
            overlay.update(in: window, text: text, bottomOffset: bottomOffset)
        }
    }

    func hide(previewID: UInt32) {
        DispatchQueue.main.async {
            guard let overlay = self.overlays[previewID] else { return }
            overlay.dematerialize { [weak self, weak overlay] in
                guard let self, let overlay, self.overlays[previewID] === overlay else { return }
                self.overlays.removeValue(forKey: previewID)
            }
        }
    }

    private func dismissFromTap(previewID: UInt32) {
        hide(previewID: previewID)
        zedra_ios_dictation_preview_dismiss(previewID)
    }

    func applyTheme() {
        DispatchQueue.main.async {
            for overlay in self.overlays.values {
                overlay.label.textColor = NativePresentationTheme.primaryTextColor
            }
        }
    }
}

private struct NativeNotificationConfiguration {
    var callbackID: UInt32
    var title: String
    var message: String?
    var imageName: String?
    var kind: NativeNotificationKind
    var duration: TimeInterval
    var autoClose: Bool
}

private final class NativeNotificationBannerController {
    static let shared = NativeNotificationBannerController()

    private final class Banner: NSObject {
        private static let enterDuration: TimeInterval = 0.48
        private static let enterFadeDuration: TimeInterval = 0.14
        private static let exitDuration: TimeInterval = 0.24
        private static let exitFadeDuration: TimeInterval = 0.1
        private static let enterInitialScale: CGFloat = 0.01
        private static let exitScale: CGFloat = 0.01
        private static let contentInsets = UIEdgeInsets(top: 12, left: 14, bottom: 12, right: 14)
        private static let iconSize: CGFloat = 24
        private static let textGap: CGFloat = 10

        let callbackID: UInt32
        let effectView: UIVisualEffectView
        private let iconView: UIImageView
        private let titleLabel: UILabel
        private let messageLabel: UILabel
        private weak var owner: NativeNotificationBannerController?
        private weak var window: UIWindow?
        private var isVisible = false
        private var animationGeneration: UInt64 = 0

        init(callbackID: UInt32, owner: NativeNotificationBannerController) {
            self.callbackID = callbackID
            effectView = UIVisualEffectView(effect: nil)
            iconView = UIImageView()
            titleLabel = UILabel()
            messageLabel = UILabel()

            super.init()

            self.owner = owner
            configureViewHierarchy()
            configureGestures()
        }

        func prepare(in window: UIWindow, configuration: NativeNotificationConfiguration) {
            if self.window !== window {
                effectView.removeFromSuperview()
                window.addSubview(effectView)
                self.window = window
            } else if effectView.superview == nil {
                window.addSubview(effectView)
            }

            apply(configuration: configuration)
            effectView.isHidden = false
            effectView.isUserInteractionEnabled = true
            window.bringSubviewToFront(effectView)
        }

        func preferredFrame(in window: UIWindow, top: CGFloat) -> CGRect {
            Self.frame(in: window, top: top, titleLabel: titleLabel, messageLabel: messageLabel)
        }

        func setStackFrame(_ frame: CGRect, depth: Int, isExpanded: Bool, animated: Bool) {
            let resolvedFrame = frame.integral
            let clampedDepth = min(max(depth, 0), 3)
            let targetAlpha: CGFloat
            if isExpanded {
                targetAlpha = 1
            } else {
                switch clampedDepth {
                case 0:
                    targetAlpha = 1
                case 1:
                    targetAlpha = 0.84
                case 2:
                    targetAlpha = 0.64
                default:
                    targetAlpha = 0
                }
            }

            let applyFrame = {
                let contentVisible = isExpanded || clampedDepth == 0
                self.effectView.frame = resolvedFrame
                self.effectView.alpha = targetAlpha
                self.effectView.contentView.alpha = contentVisible ? 1 : 0
                self.effectView.isUserInteractionEnabled = contentVisible
                self.effectView.accessibilityElementsHidden = !contentVisible
                self.effectView.layer.cornerRadius = min(24, resolvedFrame.height / 2)
                self.effectView.layer.zPosition = CGFloat(100 - clampedDepth)
                if #available(iOS 13.0, *) {
                    self.effectView.layer.cornerCurve = .continuous
                }
                self.layoutContent()
            }

            if animated && isVisible {
                UIView.animate(
                    withDuration: 0.2,
                    delay: 0,
                    options: [.beginFromCurrentState, .allowUserInteraction, .curveEaseInOut],
                    animations: applyFrame
                )
            } else {
                applyFrame()
            }
        }

        func materializeIfNeeded() {
            guard !isVisible else { return }
            materialize()
        }

        func dematerialize(completion: @escaping () -> Void) {
            guard effectView.superview != nil else {
                completion()
                return
            }

            animationGeneration &+= 1
            let generation = animationGeneration
            isVisible = false
            effectView.isUserInteractionEnabled = false
            effectView.layer.removeAllAnimations()
            effectView.contentView.layer.removeAllAnimations()

            UIView.animate(
                withDuration: Self.exitFadeDuration,
                delay: 0,
                options: [.beginFromCurrentState, .curveEaseOut],
                animations: {
                    self.effectView.effect = nil
                    self.effectView.alpha = 0
                }
            )

            UIView.animate(
                withDuration: Self.exitDuration,
                delay: 0,
                options: [.beginFromCurrentState, .curveEaseInOut],
                animations: {
                    self.effectView.transform = Self.offscreenTransform(
                        for: self.effectView,
                        scale: Self.exitScale
                    )
                },
                completion: { _ in
                    guard self.animationGeneration == generation else { return }
                    self.effectView.removeFromSuperview()
                    self.effectView.transform = .identity
                    self.effectView.alpha = 1
                    self.effectView.effect = nil
                    completion()
                }
            )
        }

        private func configureViewHierarchy() {
            effectView.clipsToBounds = true
            effectView.alpha = 0
            effectView.layer.borderWidth = 1 / UIScreen.main.scale
            effectView.layer.borderColor = NativePresentationTheme.borderColor.withAlphaComponent(0.7).cgColor
            effectView.accessibilityIdentifier = "zedra-native-notification"

            iconView.contentMode = .scaleAspectFit

            titleLabel.font = UIFont.systemFont(ofSize: 14, weight: .semibold)
            titleLabel.textColor = NativePresentationTheme.primaryTextColor
            titleLabel.lineBreakMode = .byTruncatingTail
            titleLabel.numberOfLines = 1

            messageLabel.font = UIFont.systemFont(ofSize: 12.5, weight: .regular)
            messageLabel.textColor = NativePresentationTheme.secondaryTextColor
            messageLabel.lineBreakMode = .byTruncatingTail
            messageLabel.numberOfLines = 2

            effectView.contentView.addSubview(iconView)
            effectView.contentView.addSubview(titleLabel)
            effectView.contentView.addSubview(messageLabel)
        }

        func applyCurrentTheme() {
            effectView.layer.borderColor = NativePresentationTheme.borderColor.withAlphaComponent(0.7).cgColor
            titleLabel.textColor = NativePresentationTheme.primaryTextColor
            messageLabel.textColor = NativePresentationTheme.secondaryTextColor
            iconView.tintColor = NativePresentationTheme.primaryTextColor
            if isVisible {
                effectView.effect = Self.bannerEffect()
            }
        }

        private func configureGestures() {
            let tapGesture = UITapGestureRecognizer(target: self, action: #selector(activateFromTap))
            effectView.addGestureRecognizer(tapGesture)

            let swipeGesture = UISwipeGestureRecognizer(target: self, action: #selector(dismissFromSwipe))
            swipeGesture.direction = .up
            effectView.addGestureRecognizer(swipeGesture)

            let expandGesture = UISwipeGestureRecognizer(target: self, action: #selector(expandFromSwipe))
            expandGesture.direction = .down
            effectView.addGestureRecognizer(expandGesture)
        }

        private func apply(configuration: NativeNotificationConfiguration) {
            let imageName = configuration.imageName?.isEmpty == false
                ? configuration.imageName!
                : configuration.kind.defaultSystemImageName
            iconView.image = NativePresentationBridge.notificationImage(named: imageName)
                ?? NativePresentationBridge.notificationImage(
                    named: configuration.kind.defaultSystemImageName
                )
            iconView.tintColor = NativePresentationTheme.primaryTextColor
            titleLabel.text = configuration.title.isEmpty ? "Zedra" : configuration.title
            let message = configuration.message?.trimmingCharacters(in: .whitespacesAndNewlines)
            messageLabel.text = message
            messageLabel.isHidden = message?.isEmpty ?? true
            effectView.accessibilityLabel = titleLabel.text
            effectView.accessibilityValue = message
        }

        private func layoutContent() {
            let bounds = effectView.bounds
            let content = bounds.inset(by: Self.contentInsets)
            let iconFrame = CGRect(
                x: content.minX,
                y: bounds.midY - (Self.iconSize / 2),
                width: Self.iconSize,
                height: Self.iconSize
            ).integral
            iconView.frame = iconFrame

            let textX = iconFrame.maxX + Self.textGap
            let textWidth = max(1, content.maxX - textX)
            let hasMessage = !messageLabel.isHidden
            let titleHeight = ceil(
                titleLabel.sizeThatFits(CGSize(width: textWidth, height: 24)).height
            )
            let messageHeight = hasMessage
                ? ceil(messageLabel.sizeThatFits(CGSize(width: textWidth, height: 42)).height)
                : 0
            let totalTextHeight = titleHeight + (hasMessage ? 3 + messageHeight : 0)
            var y = bounds.midY - (totalTextHeight / 2)

            titleLabel.frame = CGRect(
                x: textX,
                y: y,
                width: textWidth,
                height: titleHeight
            ).integral
            y = titleLabel.frame.maxY + (hasMessage ? 3 : 0)
            messageLabel.frame = CGRect(
                x: textX,
                y: y,
                width: textWidth,
                height: messageHeight
            ).integral
        }

        private func materialize() {
            animationGeneration &+= 1
            isVisible = true
            effectView.layer.removeAllAnimations()
            effectView.contentView.layer.removeAllAnimations()

            effectView.effect = nil
            effectView.alpha = 0
            effectView.contentView.alpha = 1
            effectView.transform = Self.offscreenTransform(
                for: effectView,
                scale: Self.enterInitialScale
            )

            UIView.animate(
                withDuration: Self.enterFadeDuration,
                delay: 0,
                options: [.beginFromCurrentState, .allowUserInteraction, .curveEaseOut],
                animations: {
                    self.effectView.effect = Self.bannerEffect()
                    self.effectView.alpha = 1
                    self.effectView.contentView.alpha = 1
                }
            )

            UIView.animate(
                withDuration: Self.enterDuration,
                delay: 0,
                usingSpringWithDamping: 0.76,
                initialSpringVelocity: 0.24,
                options: [.beginFromCurrentState, .allowUserInteraction],
                animations: {
                    self.effectView.transform = .identity
                }
            )
        }

        @objc
        private func activateFromTap() {
            owner?.activate(callbackID: callbackID)
        }

        @objc
        private func dismissFromSwipe() {
            owner?.dismiss(callbackID: callbackID, notifyRust: true)
        }

        @objc
        private func expandFromSwipe() {
            owner?.expandStack()
        }

        private static func frame(
            in window: UIWindow,
            top: CGFloat,
            titleLabel: UILabel,
            messageLabel: UILabel
        ) -> CGRect {
            let horizontalMargin = min(18, max(12, window.bounds.width * 0.04))
            let width = max(1, min(window.bounds.width - (horizontalMargin * 2), 430))
            let textWidth = max(
                1,
                width - contentInsets.left - contentInsets.right - iconSize - textGap
            )
            let titleHeight = ceil(
                titleLabel.sizeThatFits(CGSize(width: textWidth, height: 24)).height
            )
            let hasMessage = !messageLabel.isHidden
            let messageHeight = hasMessage
                ? ceil(messageLabel.sizeThatFits(CGSize(width: textWidth, height: 42)).height)
                : 0
            let contentHeight = contentInsets.top
                + titleHeight
                + (hasMessage ? 3 + messageHeight : 0)
                + contentInsets.bottom
            let height = max(58, min(104, ceil(contentHeight)))
            let x = window.bounds.midX - (width / 2)
            return CGRect(x: x, y: top, width: width, height: height)
        }

        private static func offscreenTransform(for view: UIView, scale: CGFloat) -> CGAffineTransform {
            CGAffineTransform(
                a: scale,
                b: 0,
                c: 0,
                d: scale,
                tx: 0,
                ty: -(view.frame.maxY + 18)
            )
        }

        private static func bannerEffect() -> UIVisualEffect {
            if #available(iOS 26.0, *) {
                let effect = UIGlassEffect(style: .regular)
                effect.isInteractive = true
                effect.tintColor = NativePresentationTheme.overlayColor.withAlphaComponent(0.42)
                return effect
            }

            return NativePresentationTheme.blurEffect()
        }
    }

    private static let collapsedStackYOffset: CGFloat = 9
    private static let collapsedStackWidthInset: CGFloat = 10
    private static let expandedStackGap: CGFloat = 8
    private static let maxVisibleBubbles = 3

    private var banners: [UInt32: Banner] = [:]
    private var order: [UInt32] = []
    private var dismissWorkItems: [UInt32: DispatchWorkItem] = [:]
    private var isExpanded = false

    func present(configuration: NativeNotificationConfiguration) {
        DispatchQueue.main.async {
            let window = NativePresentationBridge.activeWindow()
            let banner = self.banners[configuration.callbackID] ?? {
                let banner = Banner(callbackID: configuration.callbackID, owner: self)
                self.banners[configuration.callbackID] = banner
                self.order.append(configuration.callbackID)
                return banner
            }()

            banner.prepare(in: window, configuration: configuration)
            self.relayout(in: window, animated: true)
            banner.materializeIfNeeded()
            self.scheduleDismissIfNeeded(for: configuration)
        }
    }

    func applyTheme() {
        DispatchQueue.main.async {
            for banner in self.banners.values {
                banner.applyCurrentTheme()
            }
        }
    }

    fileprivate func activate(callbackID: UInt32) {
        zedra_ios_native_notification_action(callbackID)
        dismiss(callbackID: callbackID, notifyRust: false)
    }

    fileprivate func dismiss(callbackID: UInt32, notifyRust: Bool) {
        dismissWorkItems[callbackID]?.cancel()
        dismissWorkItems.removeValue(forKey: callbackID)
        guard let banner = banners.removeValue(forKey: callbackID) else { return }
        order.removeAll { $0 == callbackID }
        if order.count <= 1 {
            isExpanded = false
        }

        if notifyRust {
            zedra_ios_native_notification_dismiss(callbackID)
        }

        banner.dematerialize {}
        relayout(in: NativePresentationBridge.activeWindow(), animated: true)
    }

    fileprivate func expandStack() {
        guard !isExpanded, order.count > 1 else { return }

        isExpanded = true
        relayout(in: NativePresentationBridge.activeWindow(), animated: true)
    }

    private func scheduleDismissIfNeeded(for configuration: NativeNotificationConfiguration) {
        let callbackID = configuration.callbackID
        dismissWorkItems[callbackID]?.cancel()
        dismissWorkItems.removeValue(forKey: callbackID)

        guard configuration.autoClose else { return }

        let resolvedDuration: TimeInterval
        if configuration.duration.isFinite && configuration.duration > 0 {
            resolvedDuration = min(max(configuration.duration, 1.2), 12)
        } else {
            resolvedDuration = 3.2
        }

        let workItem = DispatchWorkItem { [weak self] in
            self?.dismiss(callbackID: callbackID, notifyRust: true)
        }
        dismissWorkItems[callbackID] = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + resolvedDuration, execute: workItem)
    }

    private func relayout(in window: UIWindow, animated: Bool) {
        let top = max(8, window.safeAreaInsets.top + 8)

        if isExpanded {
            relayoutExpanded(order, in: window, top: top, animated: animated)
        } else {
            relayoutCollapsed(Array(order.reversed()), in: window, top: top, animated: animated)
        }
    }

    private func relayoutCollapsed(
        _ topToBottom: [UInt32],
        in window: UIWindow,
        top: CGFloat,
        animated: Bool
    ) {
        let visibleBackDepth = min(max(topToBottom.count - 1, 0), Self.maxVisibleBubbles - 1)
        let frontTop = top + (CGFloat(visibleBackDepth) * Self.collapsedStackYOffset)
        let stackFrame = topToBottom
            .first
            .flatMap { banners[$0]?.preferredFrame(in: window, top: frontTop) }

        for (depth, callbackID) in topToBottom.enumerated().reversed() {
            guard let banner = banners[callbackID], let stackFrame else { continue }
            let visibleDepth = min(depth, Self.maxVisibleBubbles)
            let horizontalInset = CGFloat(visibleDepth) * Self.collapsedStackWidthInset
            let yOffset = -(CGFloat(visibleDepth) * Self.collapsedStackYOffset)
            let frame = stackFrame
                .insetBy(dx: horizontalInset, dy: 0)
                .offsetBy(dx: 0, dy: yOffset)

            banner.setStackFrame(frame, depth: visibleDepth, isExpanded: false, animated: animated)
            window.bringSubviewToFront(banner.effectView)
        }
    }

    private func relayoutExpanded(
        _ topToBottom: [UInt32],
        in window: UIWindow,
        top: CGFloat,
        animated: Bool
    ) {
        var y = top
        var frames: [(depth: Int, callbackID: UInt32, frame: CGRect)] = []

        for (depth, callbackID) in topToBottom.enumerated() {
            guard let banner = banners[callbackID] else { continue }
            let frame = banner.preferredFrame(in: window, top: y)
            frames.append((depth, callbackID, frame))
            y = frame.maxY + Self.expandedStackGap
        }

        for item in frames.reversed() {
            guard let banner = banners[item.callbackID] else { continue }
            banner.setStackFrame(
                item.frame,
                depth: item.depth,
                isExpanded: true,
                animated: animated
            )
            window.bringSubviewToFront(banner.effectView)
        }
    }

}

@available(iOS 16.0, *)
private final class NativeEditMenuPresenter: NSObject, UIEditMenuInteractionDelegate {
    private let callbackID: UInt32
    private let itemLabels: [String]
    private let itemImageNames: [String?]
    private let targetRect: CGRect
    private let onFinish: (UInt32) -> Void
    private var hostView: UIView?
    private weak var interaction: UIEditMenuInteraction?
    private var handled = false
    private var finished = false

    init(
        callbackID: UInt32,
        itemLabels: [String],
        itemImageNames: [String?],
        targetRect: CGRect,
        onFinish: @escaping (UInt32) -> Void
    ) {
        self.callbackID = callbackID
        self.itemLabels = itemLabels
        self.itemImageNames = itemImageNames
        self.targetRect = targetRect
        self.onFinish = onFinish
    }

    func attach(interaction: UIEditMenuInteraction, to view: UIView) {
        self.interaction = interaction
        self.hostView = view
        view.addInteraction(interaction)
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        targetRectFor configuration: UIEditMenuConfiguration
    ) -> CGRect {
        targetRect
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        menuFor configuration: UIEditMenuConfiguration,
        suggestedActions: [UIMenuElement]
    ) -> UIMenu? {
        let actions = itemLabels.enumerated().map { index, label in
            let image = itemImageNames[safe: index]
                .flatMap { $0 }
                .flatMap { NativePresentationBridge.actionImage(named: $0) }
            return UIAction(title: label, image: image) { [weak self] _ in
                guard let self else { return }
                self.handled = true
                zedra_ios_native_edit_menu_result(self.callbackID, Int32(index))
                self.finish()
            }
        }
        return UIMenu(children: actions)
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        willDismissMenuFor configuration: UIEditMenuConfiguration,
        animator: UIEditMenuInteractionAnimating
    ) {
        animator.addCompletion { [weak self] in
            guard let self else { return }
            if !self.handled {
                zedra_ios_native_edit_menu_dismiss(self.callbackID)
            }
            self.finish()
        }
    }

    private func finish() {
        if finished { return }
        finished = true
        if let interaction {
            hostView?.removeInteraction(interaction)
        }
        hostView?.removeFromSuperview()
        hostView = nil
        onFinish(callbackID)
    }
}

private enum PresentationCoordinator {
    private static let dismissAssociationKey = "zedra_presentation_dismiss_delegate"
    private static let alertOutsideTapAssociationKey = "zedra_alert_outside_tap_handler"
    private static weak var activeCustomSheet: CustomSheetViewController?
    @available(iOS 16.0, *)
    private static var nativeEditMenuPresenters: [UInt32: NativeEditMenuPresenter] = [:]

    static func presentAlert(
        callbackID: UInt32,
        title: String?,
        message: String?,
        buttonLabels: [String],
        buttonStyles: [AlertActionStyle]
    ) {
        DispatchQueue.main.async {
            guard let presenter = NativePresentationBridge.topViewController() else { return }

            let alert = UIAlertController(title: title, message: message, preferredStyle: .alert)
            Self.applyTheme(to: alert)
            let outsideTapHandler = AlertOutsideTapDismissHandler(callbackID: callbackID, alert: alert)
            objc_setAssociatedObject(
                alert,
                alertOutsideTapAssociationKey,
                outsideTapHandler,
                .OBJC_ASSOCIATION_RETAIN_NONATOMIC
            )

            for index in 0..<buttonLabels.count {
                let style = buttonStyles[safe: index] ?? .default
                alert.addAction(UIAlertAction(title: buttonLabels[index], style: style.uiKitStyle) { _ in
                    outsideTapHandler.markHandled()
                    zedra_ios_alert_result(callbackID, Int32(index))
                })
            }

            presenter.present(alert, animated: true) {
                outsideTapHandler.install()
            }
        }
    }

    static func presentActionSheet(
        callbackID: UInt32,
        title: String?,
        message: String?,
        buttonLabels: [String],
        buttonStyles: [AlertActionStyle],
        buttonImageNames: [String?]
    ) {
        DispatchQueue.main.async {
            guard let presenter = NativePresentationBridge.topViewController() else { return }

            let sheet = UIAlertController(title: title, message: message, preferredStyle: .actionSheet)
            Self.applyTheme(to: sheet)
            let delegate = PresentationDismissDelegate(callbackID: callbackID, isSelection: true)
            sheet.presentationController?.delegate = delegate
            objc_setAssociatedObject(sheet, dismissAssociationKey, delegate, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)

            let hasCancelAction = buttonStyles.prefix(buttonLabels.count).contains(.cancel)
            for index in 0..<buttonLabels.count {
                let style = buttonStyles[safe: index] ?? .default
                let action = UIAlertAction(title: buttonLabels[index], style: style.uiKitStyle) { _ in
                    delegate.handled = true
                    zedra_ios_selection_result(callbackID, Int32(index))
                }
                if let imageName = buttonImageNames[safe: index].flatMap({ $0 }),
                   let image = NativePresentationBridge.actionImage(named: imageName) {
                    action.setValue(image, forKey: "image")
                }
                sheet.addAction(action)
            }

            if !hasCancelAction {
                // UIKit allows one cancel action; add a dismiss affordance only when callers omit it.
                sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in
                    delegate.handled = true
                    zedra_ios_selection_dismiss(callbackID)
                })
            }

            presenter.present(sheet, animated: true)
        }
    }

    static func presentListPicker(
        callbackID: UInt32,
        title: String?,
        message: String?,
        itemLabels: [String],
        itemSubtitles: [String?],
        itemImageNames: [String?]
    ) {
        DispatchQueue.main.async {
            guard !itemLabels.isEmpty else {
                zedra_ios_selection_dismiss(callbackID)
                return
            }
            guard let presenter = NativePresentationBridge.topViewController() else {
                zedra_ios_selection_dismiss(callbackID)
                return
            }

            let controller = AgentListPickerViewController(
                callbackID: callbackID,
                title: title,
                message: message,
                itemLabels: itemLabels,
                itemSubtitles: itemSubtitles,
                itemImageNames: itemImageNames
            )
            let navigation = UINavigationController(rootViewController: controller)
            navigation.overrideUserInterfaceStyle = NativePresentationTheme.interfaceStyle
            navigation.view.backgroundColor = NativePresentationTheme.backgroundColor
            navigation.navigationBar.tintColor = NativePresentationTheme.primaryTextColor
            navigation.modalPresentationStyle = .formSheet
            if let sheet = navigation.sheetPresentationController {
                sheet.detents = [.medium(), .large()]
                sheet.prefersGrabberVisible = true
            }
            let delegate = PresentationDismissDelegate(callbackID: callbackID, isSelection: true)
            navigation.presentationController?.delegate = delegate
            objc_setAssociatedObject(navigation, dismissAssociationKey, delegate, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
            presenter.present(navigation, animated: true)
        }
    }

    static func presentNativeEditMenu(
        callbackID: UInt32,
        sourcePoint: CGPoint,
        itemLabels: [String],
        itemImageNames: [String?]
    ) {
        DispatchQueue.main.async {
            guard !itemLabels.isEmpty else {
                zedra_ios_native_edit_menu_dismiss(callbackID)
                return
            }
            guard #available(iOS 16.0, *) else {
                zedra_ios_native_edit_menu_dismiss(callbackID)
                return
            }

            let window = NativePresentationBridge.activeWindow()
            guard let anchor = nativeEditMenuAnchor(sourcePoint: sourcePoint, in: window.bounds) else {
                zedra_ios_native_edit_menu_dismiss(callbackID)
                return
            }
            let hostView = UIView(frame: window.bounds)
            hostView.backgroundColor = .clear
            hostView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            window.addSubview(hostView)

            let presenter = NativeEditMenuPresenter(
                callbackID: callbackID,
                itemLabels: itemLabels,
                itemImageNames: itemImageNames,
                targetRect: anchor.targetRect,
                onFinish: { callbackID in
                    nativeEditMenuPresenters.removeValue(forKey: callbackID)
                }
            )
            let interaction = UIEditMenuInteraction(delegate: presenter)
            presenter.attach(interaction: interaction, to: hostView)
            nativeEditMenuPresenters[callbackID] = presenter
            interaction.presentEditMenu(
                with: UIEditMenuConfiguration(identifier: nil, sourcePoint: anchor.sourcePoint)
            )
        }
    }

    @available(iOS 16.0, *)
    private static func nativeEditMenuAnchor(
        sourcePoint: CGPoint,
        in bounds: CGRect
    ) -> (sourcePoint: CGPoint, targetRect: CGRect)? {
        guard sourcePoint.x.isFinite,
              sourcePoint.y.isFinite,
              bounds.origin.x.isFinite,
              bounds.origin.y.isFinite,
              bounds.width.isFinite,
              bounds.height.isFinite,
              bounds.width > 0,
              bounds.height > 0
        else {
            return nil
        }

        let x = min(max(sourcePoint.x, bounds.minX), bounds.maxX)
        let y = min(max(sourcePoint.y, bounds.minY), bounds.maxY)
        let rectWidth = min(CGFloat(2), bounds.width)
        let rectHeight = min(CGFloat(2), bounds.height)
        let rectX = min(max(x - rectWidth / 2, bounds.minX), bounds.maxX - rectWidth)
        let rectY = min(max(y - rectHeight / 2, bounds.minY), bounds.maxY - rectHeight)
        return (
            CGPoint(x: x, y: y),
            CGRect(x: rectX, y: rectY, width: rectWidth, height: rectHeight)
        )
    }

    static func presentTextInput(
        callbackID: UInt32,
        title: String?,
        placeholder: String?,
        initialValue: String?
    ) {
        DispatchQueue.main.async {
            guard let presenter = NativePresentationBridge.topViewController() else { return }

            let alert = UIAlertController(
                title: title?.isEmpty == false ? title : nil,
                message: nil,
                preferredStyle: .alert
            )
            Self.applyTheme(to: alert)
            alert.addTextField { field in
                field.placeholder = placeholder
                field.text = initialValue
                field.textColor = NativePresentationTheme.primaryTextColor
                field.tintColor = NativePresentationTheme.primaryTextColor
                field.overrideUserInterfaceStyle = NativePresentationTheme.interfaceStyle
                field.clearButtonMode = .whileEditing
                field.autocapitalizationType = .words
                field.returnKeyType = .done
            }
            alert.addAction(UIAlertAction(title: "Save", style: .default) { _ in
                let value = alert.textFields?.first?.text ?? ""
                value.withCString { ptr in
                    zedra_ios_text_input_result(callbackID, ptr)
                }
            })
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel) { _ in
                zedra_ios_text_input_dismiss(callbackID)
            })
            presenter.present(alert, animated: true) {
                alert.textFields?.first?.selectAll(nil)
            }
        }
    }

    static func presentCustomSheet(
        configuration: CustomSheetConfiguration
    ) {
        DispatchQueue.main.async {
            // Resign the terminal keyboard or it re-shows each frame over the sheet.
            GPUIRuntimeController.dismissMainWindowKeyboard()
            // Replacing a live sheet: present from its presenter, not the dismissing sheet.
            let presenter = activeCustomSheet?.presentingViewController
                ?? NativePresentationBridge.topViewController()
            guard let presenter else { return }
            let present = {
                let sheet = CustomSheetViewController(configuration: configuration)
                activeCustomSheet = sheet
                presenter.present(sheet, animated: true)
            }
            // Present only after the old sheet's dismissal completes, or UIKit's
            // in-flight transition state can fail the new presentation.
            if let existing = activeCustomSheet {
                existing.dismiss(animated: false, completion: present)
            } else {
                present()
            }
        }
    }

    static func dismissCustomSheet() {
        DispatchQueue.main.async {
            activeCustomSheet?.dismiss(animated: true)
            activeCustomSheet = nil
        }
    }

    private static func applyTheme(to alert: UIAlertController) {
        alert.overrideUserInterfaceStyle = NativePresentationTheme.interfaceStyle
        alert.view.tintColor = NativePresentationTheme.primaryTextColor
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        guard indices.contains(index) else { return nil }
        return self[index]
    }
}

fileprivate struct CustomSheetConfiguration {
    var detents: [CustomSheetDetent]
    var initialDetent: CustomSheetDetent
    var prefersGrabberVisible: Bool
    var prefersScrollingExpandsWhenScrolledToEdge: Bool
    var prefersEdgeAttachedInCompactHeight: Bool
    var widthFollowsPreferredContentSizeWhenEdgeAttached: Bool
    var preferredCornerRadius: CGFloat?
    var isModalInPresentation: Bool

    static let `default` = CustomSheetConfiguration(
        detents: [.medium, .large],
        initialDetent: .medium,
        prefersGrabberVisible: true,
        prefersScrollingExpandsWhenScrolledToEdge: true,
        prefersEdgeAttachedInCompactHeight: false,
        widthFollowsPreferredContentSizeWhenEdgeAttached: false,
        preferredCornerRadius: nil,
        isModalInPresentation: false
    )
}

final class CustomSheetViewController: UIViewController, UIGestureRecognizerDelegate {
    private let configuration: CustomSheetConfiguration
    private let canvasView = UIView()
    private lazy var contentPanGesture = UIPanGestureRecognizer(
        target: self,
        action: #selector(handleCanvasPan(_:))
    )
    private var embeddedWindow: UnsafeMutableRawPointer?
    private var displayLink: CADisplayLink?
    private var sheetPanLinked = false
    private weak var linkedSheetPanGesture: UIPanGestureRecognizer?
    private var lastPanTranslation = CGPoint.zero

    fileprivate init(configuration: CustomSheetConfiguration = .default) {
        self.configuration = configuration
        super.init(nibName: nil, bundle: nil)

        modalPresentationStyle = .pageSheet
        isModalInPresentation = configuration.isModalInPresentation
        overrideUserInterfaceStyle = NativePresentationTheme.interfaceStyle
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func viewDidLoad() {
        super.viewDidLoad()

        view.backgroundColor = NativePresentationTheme.backgroundColor
        configureCanvasLayout()
        configureSheetPresentation()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        forceInitialEmbeddedFrameIfNeeded()
        linkSheetPanGestureIfNeeded()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        mountGpuiCanvasIfNeeded()
        resizeEmbeddedCanvasIfNeeded()
        linkSheetPanGestureIfNeeded()
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        startDisplayLinkIfNeeded()
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        stopDisplayLink()
        if presentedViewController == nil && presentingViewController == nil {
            unmountEmbeddedCanvas()
        }
    }

    deinit {
        stopDisplayLink()
        unmountEmbeddedCanvas()
    }

    private func configureCanvasLayout() {
        canvasView.translatesAutoresizingMaskIntoConstraints = false
        canvasView.backgroundColor = .clear
        canvasView.accessibilityIdentifier = "zedra-custom-sheet-canvas"
        contentPanGesture.cancelsTouchesInView = true
        contentPanGesture.delaysTouchesBegan = false
        contentPanGesture.delaysTouchesEnded = false
        contentPanGesture.delegate = self
        canvasView.addGestureRecognizer(contentPanGesture)
        view.addSubview(canvasView)

        NSLayoutConstraint.activate([
            canvasView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            canvasView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            canvasView.topAnchor.constraint(equalTo: view.topAnchor),
            canvasView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            canvasView.heightAnchor.constraint(greaterThanOrEqualToConstant: 240),
        ])

        // Reserved integration seam: GPUI content should attach into `canvasView`
        // so UIKit provides only sheet chrome, gestures, and animation.
    }

    @objc
    private func handleCanvasPan(_ gesture: UIPanGestureRecognizer) {
        guard let embeddedWindow else {
            return
        }

        let translation = gesture.translation(in: canvasView)
        let delta = CGPoint(
            x: translation.x - lastPanTranslation.x,
            y: translation.y - lastPanTranslation.y
        )
        let velocity = gesture.velocity(in: canvasView)
        let location = gesture.location(in: canvasView)

        switch gesture.state {
        case .began:
            linkedSheetPanGesture?.isEnabled = false
            lastPanTranslation = translation
        case .changed:
            gpui_ios_inject_scroll(
                embeddedWindow,
                Float(location.x),
                Float(location.y),
                Float(delta.x),
                Float(delta.y),
                0,
                0,
                1
            )
            lastPanTranslation = translation
        case .ended, .cancelled, .failed:
            gpui_ios_inject_scroll(
                embeddedWindow,
                Float(location.x),
                Float(location.y),
                0,
                0,
                Float(velocity.x),
                Float(velocity.y),
                2
            )
            linkedSheetPanGesture?.isEnabled = true
            gesture.setTranslation(.zero, in: canvasView)
            lastPanTranslation = .zero
        default:
            break
        }
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        if gestureRecognizer === contentPanGesture {
            let velocity = contentPanGesture.velocity(in: canvasView)
            if zedra_ios_sheet_content_is_at_top(), velocity.y > abs(velocity.x), velocity.y > 0 {
                return false
            }
            return true
        }

        return true
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        false
    }

    private func linkSheetPanGestureIfNeeded() {
        guard !sheetPanLinked else { return }
        guard let container = sheetGestureContainerView() else { return }

        let panRecognizers = container.gestureRecognizers?.compactMap { $0 as? UIPanGestureRecognizer } ?? []
        guard !panRecognizers.isEmpty else { return }

        for recognizer in panRecognizers where recognizer !== contentPanGesture {
            recognizer.require(toFail: contentPanGesture)
            if linkedSheetPanGesture == nil {
                linkedSheetPanGesture = recognizer
            }
        }

        sheetPanLinked = true
    }

    private func sheetGestureContainerView() -> UIView? {
        var current: UIView? = view
        while let candidate = current {
            if let recognizers = candidate.gestureRecognizers,
               recognizers.contains(where: { $0 is UIPanGestureRecognizer && $0 !== contentPanGesture }) {
                return candidate
            }
            current = candidate.superview
        }
        return presentationController?.presentedView
    }

    private func mountGpuiCanvasIfNeeded() {
        guard embeddedWindow == nil else { return }
        let bounds = canvasView.bounds.integral
        guard bounds.width > 0, bounds.height > 0 else { return }
        embeddedWindow = zedra_ios_mount_custom_sheet_content(
            Unmanaged.passUnretained(canvasView).toOpaque(),
            Float(bounds.width),
            Float(bounds.height)
        )
        resizeEmbeddedCanvasIfNeeded()
        forceInitialEmbeddedFrameIfNeeded()
    }

    private func resizeEmbeddedCanvasIfNeeded() {
        guard let embeddedWindow else { return }
        let bounds = canvasView.bounds.integral
        guard bounds.width > 0, bounds.height > 0 else { return }
        gpui_ios_handle_view_resize(embeddedWindow, Float(bounds.width), Float(bounds.height))
    }

    private func unmountEmbeddedCanvas() {
        guard embeddedWindow != nil else { return }
        embeddedWindow = nil
        zedra_ios_unmount_custom_sheet_content()
    }

    private func startDisplayLinkIfNeeded() {
        guard displayLink == nil else { return }
        let displayLink = CADisplayLink(target: self, selector: #selector(renderEmbeddedFrame))
        displayLink.add(to: .main, forMode: .common)
        self.displayLink = displayLink
    }

    private func stopDisplayLink() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc
    private func renderEmbeddedFrame() {
        guard let embeddedWindow else { return }
        gpui_ios_request_frame(embeddedWindow)
    }

    private func forceInitialEmbeddedFrameIfNeeded() {
        guard let embeddedWindow else { return }
        gpui_ios_request_frame_forced(embeddedWindow)
    }

    private func configureSheetPresentation() {
        guard let sheet = sheetPresentationController else { return }

        sheet.detents = configuration.detents.map(\.uiKitDetent)
        sheet.selectedDetentIdentifier = configuration.initialDetent.identifier
        sheet.prefersGrabberVisible = configuration.prefersGrabberVisible
        sheet.prefersScrollingExpandsWhenScrolledToEdge =
            configuration.prefersScrollingExpandsWhenScrolledToEdge
        sheet.prefersEdgeAttachedInCompactHeight =
            configuration.prefersEdgeAttachedInCompactHeight
        sheet.widthFollowsPreferredContentSizeWhenEdgeAttached =
            configuration.widthFollowsPreferredContentSizeWhenEdgeAttached
        sheet.preferredCornerRadius = configuration.preferredCornerRadius
    }

}

@_cdecl("ios_present_alert")
func ios_present_alert(
    _ callbackID: UInt32,
    _ title: UnsafePointer<CChar>?,
    _ message: UnsafePointer<CChar>?,
    _ buttonCount: Int32,
    _ labels: UnsafePointer<UnsafePointer<CChar>?>?,
    _ styles: UnsafePointer<Int32>?
) {
    PresentationCoordinator.presentAlert(
        callbackID: callbackID,
        title: NativePresentationBridge.string(title),
        message: NativePresentationBridge.string(message),
        buttonLabels: NativePresentationBridge.strings(count: buttonCount, labels: labels),
        buttonStyles: NativePresentationBridge.styles(count: buttonCount, styles: styles)
    )
}

@_cdecl("ios_present_list_picker")
func ios_present_list_picker(
    _ callbackID: UInt32,
    _ title: UnsafePointer<CChar>?,
    _ message: UnsafePointer<CChar>?,
    _ itemCount: Int32,
    _ labels: UnsafePointer<UnsafePointer<CChar>?>?,
    _ subtitles: UnsafePointer<UnsafePointer<CChar>?>?,
    _ imageNames: UnsafePointer<UnsafePointer<CChar>?>?
) {
    PresentationCoordinator.presentListPicker(
        callbackID: callbackID,
        title: NativePresentationBridge.string(title),
        message: NativePresentationBridge.string(message),
        itemLabels: NativePresentationBridge.strings(count: itemCount, labels: labels),
        itemSubtitles: NativePresentationBridge.optionalStrings(
            count: itemCount,
            labels: subtitles
        ),
        itemImageNames: NativePresentationBridge.optionalStrings(
            count: itemCount,
            labels: imageNames
        )
    )
}

@_cdecl("ios_present_selection")
func ios_present_selection(
    _ callbackID: UInt32,
    _ title: UnsafePointer<CChar>?,
    _ message: UnsafePointer<CChar>?,
    _ buttonCount: Int32,
    _ labels: UnsafePointer<UnsafePointer<CChar>?>?,
    _ styles: UnsafePointer<Int32>?,
    _ imageNames: UnsafePointer<UnsafePointer<CChar>?>?
) {
    PresentationCoordinator.presentActionSheet(
        callbackID: callbackID,
        title: NativePresentationBridge.string(title),
        message: NativePresentationBridge.string(message),
        buttonLabels: NativePresentationBridge.strings(count: buttonCount, labels: labels),
        buttonStyles: NativePresentationBridge.styles(count: buttonCount, styles: styles),
        buttonImageNames: NativePresentationBridge.optionalStrings(
            count: buttonCount,
            labels: imageNames
        )
    )
}

@_cdecl("ios_present_native_edit_menu")
func ios_present_native_edit_menu(
    _ callbackID: UInt32,
    _ xPts: Float,
    _ yPts: Float,
    _ itemCount: Int32,
    _ labels: UnsafePointer<UnsafePointer<CChar>?>?,
    _ imageNames: UnsafePointer<UnsafePointer<CChar>?>?
) {
    PresentationCoordinator.presentNativeEditMenu(
        callbackID: callbackID,
        sourcePoint: CGPoint(x: CGFloat(xPts), y: CGFloat(yPts)),
        itemLabels: NativePresentationBridge.strings(count: itemCount, labels: labels),
        itemImageNames: NativePresentationBridge.optionalStrings(
            count: itemCount,
            labels: imageNames
        )
    )
}

@_cdecl("ios_present_text_input")
func ios_present_text_input(
    _ callbackID: UInt32,
    _ title: UnsafePointer<CChar>?,
    _ placeholder: UnsafePointer<CChar>?,
    _ initialValue: UnsafePointer<CChar>?
) {
    PresentationCoordinator.presentTextInput(
        callbackID: callbackID,
        title: NativePresentationBridge.string(title),
        placeholder: NativePresentationBridge.string(placeholder),
        initialValue: NativePresentationBridge.string(initialValue)
    )
}

@_cdecl("ios_present_custom_sheet")
func ios_present_custom_sheet(
    _ detentCount: Int32,
    _ detents: UnsafePointer<Int32>?,
    _ initialDetent: Int32,
    _ showsGrabber: Bool,
    _ expandsOnScrollEdge: Bool,
    _ edgeAttachedInCompactHeight: Bool,
    _ widthFollowsPreferredContentSizeWhenEdgeAttached: Bool,
    _ hasCornerRadius: Bool,
    _ cornerRadius: Float,
    _ modalInPresentation: Bool
) {
    PresentationCoordinator.presentCustomSheet(
        configuration: CustomSheetConfiguration(
            detents: NativePresentationBridge.detents(count: detentCount, detents: detents),
            initialDetent: CustomSheetDetent(rawValue: initialDetent) ?? .medium,
            prefersGrabberVisible: showsGrabber,
            prefersScrollingExpandsWhenScrolledToEdge: expandsOnScrollEdge,
            prefersEdgeAttachedInCompactHeight: edgeAttachedInCompactHeight,
            widthFollowsPreferredContentSizeWhenEdgeAttached:
                widthFollowsPreferredContentSizeWhenEdgeAttached,
            preferredCornerRadius: hasCornerRadius ? CGFloat(cornerRadius) : nil,
            isModalInPresentation: modalInPresentation
        )
    )
}

@_cdecl("ios_dismiss_custom_sheet")
func ios_dismiss_custom_sheet() {
    PresentationCoordinator.dismissCustomSheet()
}

@_cdecl("ios_update_native_floating_button")
func ios_update_native_floating_button(
    _ callbackID: UInt32,
    _ systemImageName: UnsafePointer<CChar>?,
    _ accessibilityLabel: UnsafePointer<CChar>?,
    _ xPts: Float,
    _ yPts: Float,
    _ widthPts: Float,
    _ heightPts: Float
) {
    NativeFloatingButtonController.shared.update(
        callbackID: callbackID,
        systemImageName: NativePresentationBridge.string(systemImageName) ?? "circle",
        accessibilityLabel: NativePresentationBridge.string(accessibilityLabel) ?? "",
        frame: CGRect(
            x: CGFloat(xPts),
            y: CGFloat(yPts),
            width: CGFloat(widthPts),
            height: CGFloat(heightPts)
        ),
        iconPointSize: nativeFloatingButtonDefaultIconPointSize,
        iconWeight: nativeFloatingButtonDefaultIconWeightRawValue
    )
}

@_cdecl("ios_update_native_floating_button_with_icon")
func ios_update_native_floating_button_with_icon(
    _ callbackID: UInt32,
    _ systemImageName: UnsafePointer<CChar>?,
    _ accessibilityLabel: UnsafePointer<CChar>?,
    _ xPts: Float,
    _ yPts: Float,
    _ widthPts: Float,
    _ heightPts: Float,
    _ iconSizePts: Float,
    _ iconWeight: Int32
) {
    NativeFloatingButtonController.shared.update(
        callbackID: callbackID,
        systemImageName: NativePresentationBridge.string(systemImageName) ?? "circle",
        accessibilityLabel: NativePresentationBridge.string(accessibilityLabel) ?? "",
        frame: CGRect(
            x: CGFloat(xPts),
            y: CGFloat(yPts),
            width: CGFloat(widthPts),
            height: CGFloat(heightPts)
        ),
        iconPointSize: CGFloat(iconSizePts),
        iconWeight: iconWeight
    )
}

@_cdecl("ios_hide_native_floating_button")
func ios_hide_native_floating_button(_ callbackID: UInt32) {
    NativeFloatingButtonController.shared.hide(callbackID: callbackID)
}

@_cdecl("ios_update_native_dictation_preview")
func ios_update_native_dictation_preview(
    _ previewID: UInt32,
    _ text: UnsafePointer<CChar>?,
    _ bottomOffsetPts: Float
) {
    NativeDictationPreviewController.shared.update(
        previewID: previewID,
        text: NativePresentationBridge.string(text) ?? "",
        bottomOffset: CGFloat(bottomOffsetPts)
    )
}

@_cdecl("ios_hide_native_dictation_preview")
func ios_hide_native_dictation_preview(_ previewID: UInt32) {
    NativeDictationPreviewController.shared.hide(previewID: previewID)
}

@_cdecl("ios_present_native_notification")
func ios_present_native_notification(
    _ callbackID: UInt32,
    _ title: UnsafePointer<CChar>?,
    _ message: UnsafePointer<CChar>?,
    _ imageName: UnsafePointer<CChar>?,
    _ kind: Int32,
    _ durationSecs: Float,
    _ autoClose: Bool
) {
    let messageText = NativePresentationBridge.string(message)?
        .trimmingCharacters(in: .whitespacesAndNewlines)
    let imageText = NativePresentationBridge.string(imageName)?
        .trimmingCharacters(in: .whitespacesAndNewlines)
    NativeNotificationBannerController.shared.present(
        configuration: NativeNotificationConfiguration(
            callbackID: callbackID,
            title: NativePresentationBridge.string(title) ?? "Zedra",
            message: messageText?.isEmpty == true ? nil : messageText,
            imageName: imageText?.isEmpty == true ? nil : imageText,
            kind: NativeNotificationKind(rawValue: kind) ?? .info,
            duration: TimeInterval(durationSecs),
            autoClose: autoClose
        )
    )
}
