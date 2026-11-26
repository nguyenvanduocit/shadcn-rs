//! The system clipboard, for text (ADR-0028, "The outbox").
//!
//! Writing is an effect with an outcome, so it is confirm-then-act: the
//! answer comes back through a port with the caller's context, and nothing
//! depends on a returned `bool` (a cut deletes only what the answer says was
//! copied, and only if it is still there). Reading is an input, not an
//! effect: [`paste`] stays a synchronous read.

use crate::port::Port;

/// How a write ended: `Ok`, or the system's error, said.
pub type Wrote = Result<(), String>;

/// Put `text` on the clipboard; `answer` gets how it ended with `context`
/// (live, in the same turn). Inside a fork (a check) nothing is written and
/// no answer comes: a check has no effect on the outside world.
pub fn write<C: 'static>(text: String, context: C, answer: Port<(Wrote, C)>) {
    answered(text, context, move |wrote, context| answer.post((wrote, context)));
}

/// [`write`], its answer given to `answer` rather than posted: the engine's
/// own text field (copy and cut, `interact.rs`), which has no scope to own
/// a port. The same rule: inside a fork nothing is written and no answer
/// comes; live, the answer comes now, after the write.
pub(crate) fn answered<C>(text: String, context: C, answer: impl FnOnce(Wrote, C)) {
    if crate::engine::forking() {
        return;
    }
    let wrote = arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string());
    answer(wrote, context);
}

/// The clipboard's text, if it holds any: an input read.
#[must_use]
pub fn paste() -> Option<String> {
    arboard::Clipboard::new().and_then(|mut c| c.get_text()).ok()
}
