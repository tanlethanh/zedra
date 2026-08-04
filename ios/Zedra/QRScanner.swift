import AVFoundation
import CoreImage
import PhotosUI
import UIKit
import UniformTypeIdentifiers
import Vision
import ZedraFFI

private final class QRScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate,
    PHPickerViewControllerDelegate {
    private var session: AVCaptureSession?
    private var previewLayer: AVCaptureVideoPreviewLayer?
    // The scan result must fire once — camera frames and a picked image can race.
    private var handled = false
    private let overlay = ViewfinderOverlay()
    private var photoButtonTop: NSLayoutConstraint?

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        configureOverlay()
        configureCancelButton()
        configurePhotoButton()
        requestCameraAndStart()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
        // Keep the button just below the viewfinder square, whose size follows the bounds.
        photoButtonTop?.constant = ViewfinderOverlay.side(in: view.bounds) / 2 + 32
    }

    override func viewWillTransition(to size: CGSize, with coordinator: UIViewControllerTransitionCoordinator) {
        super.viewWillTransition(to: size, with: coordinator)
        coordinator.animate(alongsideTransition: nil) { [weak self] _ in
            self?.updatePreviewOrientation()
        }
    }

    private func configureOverlay() {
        overlay.translatesAutoresizingMaskIntoConstraints = false
        overlay.backgroundColor = .clear
        overlay.contentMode = .redraw
        // The cutout is punched with a clear blend mode — an opaque view would render it black.
        overlay.isOpaque = false
        overlay.isUserInteractionEnabled = false
        view.addSubview(overlay)

        NSLayoutConstraint.activate([
            overlay.topAnchor.constraint(equalTo: view.topAnchor),
            overlay.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            overlay.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            overlay.trailingAnchor.constraint(equalTo: view.trailingAnchor),
        ])
    }

    private func configureCancelButton() {
        let button = UIButton(type: .system)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.setTitle("Cancel", for: .normal)
        button.setTitleColor(.white, for: .normal)
        button.titleLabel?.font = .systemFont(ofSize: 17, weight: .semibold)
        button.addTarget(self, action: #selector(cancelTapped), for: .touchUpInside)
        view.addSubview(button)

        NSLayoutConstraint.activate([
            button.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            button.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
        ])
    }

    private func configurePhotoButton() {
        var config = UIButton.Configuration.plain()
        config.image = UIImage(systemName: "photo.on.rectangle")
        config.title = "Upload Image"
        config.imagePadding = 8
        config.baseForegroundColor = .white
        config.attributedTitle?.font = .systemFont(ofSize: 15, weight: .medium)

        let button = UIButton(configuration: config)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.addTarget(self, action: #selector(photoTapped), for: .touchUpInside)
        view.addSubview(button)

        let top = button.topAnchor.constraint(equalTo: view.centerYAnchor)
        photoButtonTop = top
        NSLayoutConstraint.activate([
            top,
            button.centerXAnchor.constraint(equalTo: view.centerXAnchor),
        ])
    }

    @objc
    private func photoTapped() {
        guard presentedViewController == nil else { return }
        var config = PHPickerConfiguration()
        config.filter = .images
        config.selectionLimit = 1
        config.preferredAssetRepresentationMode = .current

        let picker = PHPickerViewController(configuration: config)
        picker.delegate = self
        present(picker, animated: true)
    }

    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        guard let provider = results.first?.itemProvider else { return }

        // `url` is valid only inside this handler — read the bytes before returning.
        provider.loadFileRepresentation(forTypeIdentifier: UTType.image.identifier) { [weak self] url, _ in
            let value = url.flatMap { try? Data(contentsOf: $0) }.flatMap(decodeQR)
            DispatchQueue.main.async {
                guard let self else { return }
                if let value {
                    self.finish(with: value)
                } else {
                    self.showNoCodeFound()
                }
            }
        }
    }

    private func showNoCodeFound() {
        let alert = UIAlertController(
            title: "No QR Code Found",
            message: "That image doesn't contain a QR code. Try another one.",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        present(alert, animated: true)
    }

    private func requestCameraAndStart() {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            setupCamera()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                DispatchQueue.main.async {
                    if granted {
                        self?.setupCamera()
                    } else {
                        self?.showPermissionDenied()
                    }
                }
            }
        default:
            showPermissionDenied()
        }
    }

    private func setupCamera() {
        guard
            let device = AVCaptureDevice.default(for: .video),
            let input = try? AVCaptureDeviceInput(device: device)
        else {
            cancelTapped()
            return
        }

        let session = AVCaptureSession()
        guard session.canAddInput(input) else {
            cancelTapped()
            return
        }
        session.addInput(input)

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else {
            cancelTapped()
            return
        }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]

        let previewLayer = AVCaptureVideoPreviewLayer(session: session)
        previewLayer.frame = view.bounds
        previewLayer.videoGravity = .resizeAspectFill
        view.layer.insertSublayer(previewLayer, at: 0)

        self.session = session
        self.previewLayer = previewLayer
        updatePreviewOrientation()

        DispatchQueue.global(qos: .userInteractive).async {
            session.startRunning()
        }
    }

    private func updatePreviewOrientation() {
        guard let connection = previewLayer?.connection else { return }
        let interfaceOrientation = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first?.effectiveGeometry.interfaceOrientation ?? .portrait

        let videoOrientation: AVCaptureVideoOrientation
        switch interfaceOrientation {
        case .landscapeLeft:
            videoOrientation = .landscapeLeft
        case .landscapeRight:
            videoOrientation = .landscapeRight
        case .portraitUpsideDown:
            videoOrientation = .portraitUpsideDown
        default:
            videoOrientation = .portrait
        }

        guard connection.isVideoOrientationSupported else { return }
        connection.videoOrientation = videoOrientation
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        for metadata in metadataObjects {
            guard let code = metadata as? AVMetadataMachineReadableCodeObject, let value = code.stringValue else {
                continue
            }

            finish(with: value)
            return
        }
    }

    private func finish(with value: String) {
        guard !handled else { return }
        handled = true
        if session?.isRunning == true {
            session?.stopRunning()
        }
        value.withCString { zedra_qr_scanner_result($0) }
        dismiss(animated: true)
    }

    @objc
    private func cancelTapped() {
        if session?.isRunning == true {
            session?.stopRunning()
        }
        dismiss(animated: true)
    }

    private func showPermissionDenied() {
        let alert = UIAlertController(
            title: "Camera Access Required",
            message: "Please enable camera access in Settings to scan QR codes.",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "OK", style: .default) { [weak self] _ in
            self?.cancelTapped()
        })
        present(alert, animated: true)
    }
}

/// Dark scrim with a clear square cutout and rounded corner brackets. Mirrors the Android overlay.
private final class ViewfinderOverlay: UIView {
    // 75% of the narrower dimension, capped so the guide stays a viewfinder on iPad.
    static func side(in bounds: CGRect) -> CGFloat {
        min(min(bounds.width, bounds.height) * 0.75, 420)
    }

    override func draw(_ rect: CGRect) {
        guard let ctx = UIGraphicsGetCurrentContext() else { return }
        let side = Self.side(in: bounds)
        let square = CGRect(
            x: bounds.midX - side / 2,
            y: bounds.midY - side / 2,
            width: side,
            height: side
        )

        ctx.setFillColor(UIColor.black.withAlphaComponent(0.53).cgColor)
        ctx.fill(bounds)
        ctx.setBlendMode(.clear)
        ctx.fill(square)
        ctx.setBlendMode(.normal)

        let arm: CGFloat = 32
        let radius: CGFloat = 8
        let corners: [(CGPoint, CGPoint, CGPoint)] = [
            (CGPoint(x: square.minX, y: square.minY + arm), CGPoint(x: square.minX, y: square.minY),
             CGPoint(x: square.minX + arm, y: square.minY)),
            (CGPoint(x: square.maxX - arm, y: square.minY), CGPoint(x: square.maxX, y: square.minY),
             CGPoint(x: square.maxX, y: square.minY + arm)),
            (CGPoint(x: square.maxX, y: square.maxY - arm), CGPoint(x: square.maxX, y: square.maxY),
             CGPoint(x: square.maxX - arm, y: square.maxY)),
            (CGPoint(x: square.minX + arm, y: square.maxY), CGPoint(x: square.minX, y: square.maxY),
             CGPoint(x: square.minX, y: square.maxY - arm)),
        ]

        ctx.setStrokeColor(UIColor.white.cgColor)
        ctx.setLineWidth(4)
        ctx.setLineCap(.round)
        for (start, corner, end) in corners {
            ctx.move(to: start)
            ctx.addArc(tangent1End: corner, tangent2End: end, radius: radius)
            ctx.addLine(to: end)
            ctx.strokePath()
        }
    }
}

private func decodeQR(_ data: Data) -> String? {
    guard let image = CIImage(data: data) else { return nil }
    let request = VNDetectBarcodesRequest()
    request.symbologies = [.qr]
    let handler = VNImageRequestHandler(ciImage: image, options: [:])
    try? handler.perform([request])
    return request.results?.compactMap { $0.payloadStringValue }.first
}

@_cdecl("ios_present_qr_scanner")
func ios_present_qr_scanner() {
    DispatchQueue.main.async {
        guard let presenter = NativePresentationBridge.topViewController() else { return }
        let scanner = QRScannerViewController()
        scanner.modalPresentationStyle = .fullScreen
        presenter.present(scanner, animated: true)
    }
}
