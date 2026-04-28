import Foundation
import ObjectiveC.runtime
import UIKit
import ZedraFFI

@_silgen_name("gpui_ios_request_frame")
private func gpui_ios_request_frame(_ windowPtr: UnsafeMutableRawPointer?)

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

@_silgen_name("zedra_ios_native_floating_button_pressed")
private func zedra_ios_native_floating_button_pressed(_ callbackID: UInt32)

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

    static func strings(count: Int32, labels: UnsafePointer<UnsafePointer<CChar>?>?) -> [String] {
        guard let labels, count > 0 else { return [] }
        return (0..<Int(count)).map { index in
            string(labels[index]) ?? "OK"
        }
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
            button.tintColor = UIColor(white: 1.0, alpha: 0.92)
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
                effect.tintColor = UIColor(white: 0.08, alpha: 0.45)
                return effect
            }

            return UIBlurEffect(style: .systemChromeMaterialDark)
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
}

private final class NativeDictationPreviewController {
    static let shared = NativeDictationPreviewController()

    private final class Overlay {
        private static let enterDuration: TimeInterval = 0.22
        private static let exitDuration: TimeInterval = 0.16
        private static let initialScale = CGAffineTransform(scaleX: 0.94, y: 0.94)
        private static let exitScale = CGAffineTransform(scaleX: 0.96, y: 0.96)
        private static let labelInsets = UIEdgeInsets(top: 10, left: 14, bottom: 10, right: 14)
        private static let maxContentHeight: CGFloat = 72

        let effectView: UIVisualEffectView
        let label: UILabel
        weak var window: UIWindow?
        private var isVisible = false
        private var animationGeneration: UInt64 = 0
        private var lastRenderedText: String?
        private var lastBottomOffset: CGFloat?
        private var lastWindowBounds = CGRect.null

        init() {
            effectView = UIVisualEffectView(effect: nil)
            effectView.clipsToBounds = true
            effectView.transform = Self.initialScale
            effectView.isUserInteractionEnabled = false
            effectView.accessibilityLabel = "Dictation preview"

            label = UILabel()
            label.font = UIFont.monospacedSystemFont(ofSize: 13, weight: .medium)
            label.textColor = UIColor(white: 1.0, alpha: 0.92)
            label.numberOfLines = 3
            label.lineBreakMode = .byTruncatingTail
            label.alpha = 0
            effectView.contentView.addSubview(label)
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
                effect.tintColor = UIColor(white: 0.08, alpha: 0.48)
                return effect
            }

            return UIBlurEffect(style: .systemChromeMaterialDark)
        }
    }

    private var overlays: [UInt32: Overlay] = [:]

    func update(previewID: UInt32, text: String, bottomOffset: CGFloat) {
        DispatchQueue.main.async {
            let window = NativePresentationBridge.activeWindow()
            let overlay = self.overlays[previewID] ?? {
                let overlay = Overlay()
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
}

private enum PresentationCoordinator {
    private static let dismissAssociationKey = "zedra_presentation_dismiss_delegate"
    private static let alertOutsideTapAssociationKey = "zedra_alert_outside_tap_handler"

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
        buttonStyles: [AlertActionStyle]
    ) {
        DispatchQueue.main.async {
            guard let presenter = NativePresentationBridge.topViewController() else { return }

            let sheet = UIAlertController(title: title, message: message, preferredStyle: .actionSheet)
            let delegate = PresentationDismissDelegate(callbackID: callbackID, isSelection: true)
            sheet.presentationController?.delegate = delegate
            objc_setAssociatedObject(
                sheet,
                dismissAssociationKey,
                delegate,
                .OBJC_ASSOCIATION_RETAIN_NONATOMIC
            )

            for index in 0..<buttonLabels.count {
                let style = buttonStyles[safe: index] ?? .default
                sheet.addAction(UIAlertAction(title: buttonLabels[index], style: style.uiKitStyle) { _ in
                    delegate.handled = true
                    zedra_ios_selection_result(callbackID, Int32(index))
                })
            }

            presenter.present(sheet, animated: true)
        }
    }

    static func presentCustomSheet(
        configuration: CustomSheetConfiguration
    ) {
        DispatchQueue.main.async {
            guard let presenter = NativePresentationBridge.topViewController() else { return }
            let sheet = CustomSheetViewController(configuration: configuration)
            presenter.present(sheet, animated: true)
        }
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
    private var lastPanTranslationY: CGFloat = 0

    fileprivate init(configuration: CustomSheetConfiguration = .default) {
        self.configuration = configuration
        super.init(nibName: nil, bundle: nil)

        modalPresentationStyle = .pageSheet
        isModalInPresentation = configuration.isModalInPresentation
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func viewDidLoad() {
        super.viewDidLoad()

        view.backgroundColor = .systemBackground
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
        let deltaY = translation.y - lastPanTranslationY
        let velocity = gesture.velocity(in: canvasView)
        let location = gesture.location(in: canvasView)
        let contentIsAtTop = zedra_ios_sheet_content_is_at_top()

        switch gesture.state {
        case .began:
            linkedSheetPanGesture?.isEnabled = false
            lastPanTranslationY = translation.y
        case .changed:
            gpui_ios_inject_scroll(
                embeddedWindow,
                Float(location.x),
                Float(location.y),
                0,
                Float(deltaY),
                0,
                0,
                1
            )
            lastPanTranslationY = translation.y
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
            lastPanTranslationY = 0
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
            return abs(velocity.y) >= abs(velocity.x)
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

@_cdecl("ios_present_selection")
func ios_present_selection(
    _ callbackID: UInt32,
    _ title: UnsafePointer<CChar>?,
    _ message: UnsafePointer<CChar>?,
    _ buttonCount: Int32,
    _ labels: UnsafePointer<UnsafePointer<CChar>?>?,
    _ styles: UnsafePointer<Int32>?
) {
    PresentationCoordinator.presentActionSheet(
        callbackID: callbackID,
        title: NativePresentationBridge.string(title),
        message: NativePresentationBridge.string(message),
        buttonLabels: NativePresentationBridge.strings(count: buttonCount, labels: labels),
        buttonStyles: NativePresentationBridge.styles(count: buttonCount, styles: styles)
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
