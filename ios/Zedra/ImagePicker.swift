import ImageIO
import PhotosUI
import UIKit
import UniformTypeIdentifiers

@_silgen_name("zedra_ios_image_acquire_result")
private func zedra_ios_image_acquire_result(_ callbackID: UInt32, _ data: UnsafePointer<UInt8>?, _ len: UInt, _ ext: UnsafePointer<CChar>?)

@_silgen_name("zedra_ios_image_acquire_cancel")
private func zedra_ios_image_acquire_cancel(_ callbackID: UInt32)

@_silgen_name("zedra_ios_image_acquire_error")
private func zedra_ios_image_acquire_error(_ callbackID: UInt32, _ message: UnsafePointer<CChar>?)

private let maxPixelSize = 2048
private let passthroughMaxBytes = 3_500_000

// PHPickerViewController.delegate is weak; hold the delegate alive until the pick completes.
private var pickerDelegates: [UInt32: ImagePickerDelegate] = [:]

private func fireError(_ callbackID: UInt32, _ message: String) {
    message.withCString { zedra_ios_image_acquire_error(callbackID, $0) }
}

private func fireResult(_ callbackID: UInt32, _ data: Data, _ ext: String) {
    data.withUnsafeBytes { raw in
        let base = raw.bindMemory(to: UInt8.self).baseAddress
        ext.withCString { extPtr in
            zedra_ios_image_acquire_result(callbackID, base, UInt(data.count), extPtr)
        }
    }
}

private final class ImagePickerDelegate: NSObject, PHPickerViewControllerDelegate {
    let callbackID: UInt32

    init(callbackID: UInt32) {
        self.callbackID = callbackID
    }

    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        let callbackID = self.callbackID
        picker.dismiss(animated: true)
        pickerDelegates[callbackID] = nil

        guard let provider = results.first?.itemProvider else {
            zedra_ios_image_acquire_cancel(callbackID)
            return
        }
        // `url` is valid only inside this handler — read the bytes before returning.
        provider.loadFileRepresentation(forTypeIdentifier: UTType.image.identifier) { url, error in
            if let error {
                fireError(callbackID, error.localizedDescription)
                return
            }
            guard let url, let data = try? Data(contentsOf: url) else {
                fireError(callbackID, "couldn't read picked image")
                return
            }
            processImageData(data, callbackID: callbackID)
        }
    }
}

private func acquireFromPhotoLibrary(_ callbackID: UInt32) {
    guard let presenter = NativePresentationBridge.topViewController() else {
        zedra_ios_image_acquire_cancel(callbackID)
        return
    }
    var config = PHPickerConfiguration()
    config.filter = .images
    config.selectionLimit = 1
    config.preferredAssetRepresentationMode = .current

    let picker = PHPickerViewController(configuration: config)
    let delegate = ImagePickerDelegate(callbackID: callbackID)
    pickerDelegates[callbackID] = delegate
    picker.delegate = delegate
    presenter.present(picker, animated: true)
}

private func acquireFromClipboard(_ callbackID: UInt32) {
    let pasteboard = UIPasteboard.general
    guard pasteboard.hasImages else {
        zedra_ios_image_acquire_cancel(callbackID)
        return
    }
    let data = pasteboard.data(forPasteboardType: UTType.png.identifier)
        ?? pasteboard.data(forPasteboardType: UTType.jpeg.identifier)
        ?? pasteboard.data(forPasteboardType: UTType.heic.identifier)
        ?? pasteboard.image?.pngData()
    guard let data else {
        fireError(callbackID, "couldn't read clipboard image")
        return
    }
    processImageData(data, callbackID: callbackID)
}

// Decode, optionally pass PNG through untouched, otherwise downscale + re-encode as JPEG.
private func processImageData(_ data: Data, callbackID: UInt32) {
    DispatchQueue.global(qos: .userInitiated).async {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else {
            fireError(callbackID, "couldn't decode image")
            return
        }

        // Screenshots and text survive PNG better than JPEG — keep small PNGs as-is.
        if isPNG(data), let (width, height) = pixelSize(source),
            max(width, height) <= maxPixelSize, data.count <= passthroughMaxBytes {
            fireResult(callbackID, data, "png")
            return
        }

        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize,
        ]
        guard let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            fireError(callbackID, "couldn't downscale image")
            return
        }
        guard let jpeg = encodeJPEG(cgImage, quality: 0.80) else {
            fireError(callbackID, "couldn't encode image")
            return
        }
        fireResult(callbackID, jpeg, "jpg")
    }
}

private func isPNG(_ data: Data) -> Bool {
    let signature: [UInt8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
    guard data.count >= signature.count else { return false }
    return data.prefix(signature.count).elementsEqual(signature)
}

private func pixelSize(_ source: CGImageSource) -> (Int, Int)? {
    guard let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
        let width = props[kCGImagePropertyPixelWidth] as? Int,
        let height = props[kCGImagePropertyPixelHeight] as? Int
    else {
        return nil
    }
    return (width, height)
}

private func encodeJPEG(_ image: CGImage, quality: CGFloat) -> Data? {
    guard let mutableData = CFDataCreateMutable(nil, 0),
        let dest = CGImageDestinationCreateWithData(mutableData, UTType.jpeg.identifier as CFString, 1, nil)
    else {
        return nil
    }
    let props = [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary
    CGImageDestinationAddImage(dest, image, props)
    guard CGImageDestinationFinalize(dest) else { return nil }
    return mutableData as Data
}

@_cdecl("ios_acquire_image")
func ios_acquire_image(_ callbackID: UInt32, _ source: Int32) {
    DispatchQueue.main.async {
        if source == 1 {
            acquireFromClipboard(callbackID)
        } else {
            acquireFromPhotoLibrary(callbackID)
        }
    }
}

@_cdecl("ios_clipboard_has_image")
func ios_clipboard_has_image() -> Bool {
    return UIPasteboard.general.hasImages
}
