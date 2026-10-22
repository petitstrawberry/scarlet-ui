use scarlet_ui_core::clipboard::{ClipboardError, ClipboardProvider, TextClipboard};

struct Provider {
    connection: sws_client::Connection,
    window_id: u32,
}
fn map_error(error: sws_client::Error) -> ClipboardError {
    match error {
        sws_client::Error::ClipboardUnsupported => ClipboardError::Unsupported,
        sws_client::Error::ClipboardDenied => ClipboardError::AccessDenied,
        sws_client::Error::ClipboardTooLarge => ClipboardError::TooLarge,
        sws_client::Error::ClipboardBusy => ClipboardError::Busy,
        other => ClipboardError::Unavailable(other.as_str().into()),
    }
}
impl ClipboardProvider for Provider {
    fn read_text(&self) -> Result<Option<alloc::string::String>, ClipboardError> {
        self.connection
            .read_clipboard_text(self.window_id)
            .map_err(map_error)
    }
    fn write_text(&self, text: &str) -> Result<(), ClipboardError> {
        self.connection
            .write_clipboard_text(self.window_id, text)
            .map_err(map_error)
    }
}
pub(super) fn service(connection: sws_client::Connection, window_id: u32) -> TextClipboard {
    TextClipboard::new(Provider {
        connection,
        window_id,
    })
}
