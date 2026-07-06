/// Unified acquire-then-upload flow shared by iOS and Android: bridges
/// `platform_bridge::acquire_image`'s callback into an async call, then uploads.
use zedra_session::SessionHandle;

use crate::platform_bridge::{self, ImageAcquireSource};

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

/// Acquires an image (native picker/clipboard → processed bytes) then
/// uploads it to the host. Returns the workspace-relative path to paste.
pub async fn acquire_and_upload(
    source: ImageAcquireSource,
    session: SessionHandle,
) -> Result<String, ImageUploadError> {
    let (tx, rx) = futures::channel::oneshot::channel();
    platform_bridge::acquire_image(source, move |result| {
        let _ = tx.send(result);
    });
    let image = match rx.await {
        Ok(Some(Ok(image))) => image,
        Ok(Some(Err(msg))) => return Err(ImageUploadError::Acquire(msg)),
        Ok(None) | Err(_) => return Err(ImageUploadError::Cancelled),
    };

    session
        .fs_upload(image.data, &image.extension)
        .await
        .map_err(|e| ImageUploadError::Upload(e.to_string()))
}
