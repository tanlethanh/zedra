import AVFoundation
import UIKit
import ZedraFFI

private final class QRScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    private var session: AVCaptureSession?
    private var previewLayer: AVCaptureVideoPreviewLayer?

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        configureCancelButton()
        requestCameraAndStart()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    override func viewWillTransition(to size: CGSize, with coordinator: UIViewControllerTransitionCoordinator) {
        super.viewWillTransition(to: size, with: coordinator)
        coordinator.animate(alongsideTransition: nil) { [weak self] _ in
            self?.updatePreviewOrientation()
        }
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

            session?.stopRunning()
            value.withCString { zedra_qr_scanner_result($0) }
            dismiss(animated: true)
            return
        }
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

@_cdecl("ios_present_qr_scanner")
func ios_present_qr_scanner() {
    DispatchQueue.main.async {
        guard let presenter = NativePresentationBridge.topViewController() else { return }
        let scanner = QRScannerViewController()
        scanner.modalPresentationStyle = .fullScreen
        presenter.present(scanner, animated: true)
    }
}
