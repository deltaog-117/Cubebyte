//! The system clipboard, opened on first use and then kept open.
//!
//! Keeping the handle alive matters: on Wayland and X11 the text a program copied is served
//! by that program, so dropping the handle right after a copy would empty the clipboard.

use thiserror::Error;

/// Why the clipboard could not be used.
#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("no clipboard here ({0})")]
    Unavailable(String),
    #[error("the clipboard failed ({0})")]
    Failed(String),
}

/// A lazily opened handle on the system clipboard, for text only.
#[derive(Default)]
pub struct Clipboard {
    handle: Option<arboard::Clipboard>,
}

impl Clipboard {
    pub fn new() -> Self {
        Self::default()
    }

    fn open(&mut self) -> Result<&mut arboard::Clipboard, ClipboardError> {
        let handle = match self.handle.take() {
            Some(handle) => handle,
            None => arboard::Clipboard::new()
                .map_err(|error| ClipboardError::Unavailable(error.to_string()))?,
        };
        Ok(self.handle.insert(handle))
    }

    /// The text on the clipboard.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardError`] when there is no clipboard or it holds no text.
    pub fn read(&mut self) -> Result<String, ClipboardError> {
        self.open()?
            .get_text()
            .map_err(|error| ClipboardError::Failed(error.to_string()))
    }

    /// Puts `text` on the clipboard.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardError`] when there is no clipboard or it refuses the text.
    pub fn write(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.open()?
            .set_text(text)
            .map_err(|error| ClipboardError::Failed(error.to_string()))
    }
}
