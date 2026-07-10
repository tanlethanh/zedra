/// Acquire-then-upload flow shared by iOS and Android: bridges
/// `platform_bridge::acquire_image`'s callback into an async call, then uploads.
/// The two phases are separate so the caller can show upload progress only once
/// acquisition succeeds — never during the picker or after a cancel.
use zedra_session::SessionHandle;

use crate::platform_bridge::{self, ImageAcquireSource, PickedImage};

#[derive(Debug)]
pub enum ImageUploadError {
    /// User dismissed the picker, or the clipboard held no image. Silent — no alert.
    Cancelled,
    /// Native acquisition failed (corrupt/undecodable image, platform error).
    Acquire(String),
    /// The host rejected or failed to store the upload.
    Upload(String),
}

impl ImageUploadError {
    /// Message suitable for a user-facing alert. Callers should skip the
    /// alert entirely for `Cancelled`.
    pub fn user_message(&self) -> String {
        match self {
            ImageUploadError::Cancelled => String::new(),
            ImageUploadError::Acquire(msg) => format!("Couldn't read image: {msg}"),
            ImageUploadError::Upload(msg) => format!("Upload failed: {msg}"),
        }
    }
}

/// Acquires an image via the native picker/clipboard → processed bytes.
/// `Cancelled` when the user dismisses the picker or the clipboard has no image.
pub async fn acquire(source: ImageAcquireSource) -> Result<PickedImage, ImageUploadError> {
    let (tx, rx) = futures::channel::oneshot::channel();
    platform_bridge::acquire_image(source, move |result| {
        let _ = tx.send(result);
    });
    match rx.await {
        Ok(Some(Ok(image))) => Ok(image),
        Ok(Some(Err(msg))) => Err(ImageUploadError::Acquire(msg)),
        Ok(None) | Err(_) => Err(ImageUploadError::Cancelled),
    }
}

/// Uploads already-acquired image bytes to the host. Returns the path to paste.
pub async fn upload(
    image: PickedImage,
    session: SessionHandle,
) -> Result<String, ImageUploadError> {
    session
        .fs_upload(image.data, &image.extension)
        .await
        .map_err(|e| ImageUploadError::Upload(e.to_string()))
}
