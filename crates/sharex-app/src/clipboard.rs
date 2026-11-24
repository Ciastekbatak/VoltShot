use arboard::{Clipboard, ImageData};
use sharex_core::{CapturedImage, ClipboardSink, CoreError};
use std::borrow::Cow;

/// OS clipboard adapter backed by `arboard`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClipboard;

impl SystemClipboard {
    pub const fn new() -> Self {
        Self
    }
}

impl ClipboardSink for SystemClipboard {
    fn copy_image(&self, image: &CapturedImage) -> Result<(), CoreError> {
        let mut clipboard =
            Clipboard::new().map_err(|err| CoreError::ClipboardFailed(err.to_string()))?;
        let data = ImageData {
            width: image.width() as usize,
            height: image.height() as usize,
            bytes: Cow::Borrowed(image.rgba()),
        };
        clipboard
            .set_image(data)
            .map_err(|err| CoreError::ClipboardFailed(err.to_string()))
    }

    fn copy_text(&self, text: &str) -> Result<(), CoreError> {
        let mut clipboard =
            Clipboard::new().map_err(|err| CoreError::ClipboardFailed(err.to_string()))?;
        clipboard
            .set_text(text.to_owned())
            .map_err(|err| CoreError::ClipboardFailed(err.to_string()))
    }
}
