//! The system clipboard, for text.

/// Put `text` on the clipboard; returns whether it worked.
pub fn copy(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text.to_owned()))
        .is_ok()
}

/// The clipboard's text, if it holds any.
#[must_use]
pub fn paste() -> Option<String> {
    arboard::Clipboard::new().and_then(|mut c| c.get_text()).ok()
}
