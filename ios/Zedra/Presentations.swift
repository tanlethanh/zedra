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

private enum PresentationCoordinator {
    private static let dismissAssociationKey = "zedra_presentation_dismiss_delegate"

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

            for index in 0..<buttonLabels.count {
                let style = buttonStyles[safe: index] ?? .default
                alert.addAction(UIAlertAction(title: buttonLabels[index], style: style.uiKitStyle) { _ in
                    zedra_ios_alert_result(callbackID, Int32(index))
                })
            }

            presenter.present(alert, animated: true)
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
