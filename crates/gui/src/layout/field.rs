use crate::edit::TextField;
use crate::geometry::Bounds;
use crate::style::{Direction, ResolvedText, TextAlign};
use crate::text::TextSystem;

use super::{LayoutTree, Pending};

/// What placing a text field says of its scroll, where that is not what
/// its handle holds: clamped, or moved to show its caret
/// ([`caret_reveal`]), or a newer reveal taken ([`LayoutTree::swap_fields`]).
pub(crate) struct FieldSettled {
    pub field: TextField,
    /// Exactly ([`crate::compositor::motion`]).
    pub pos: crate::compositor::motion::Exact,
    /// The reveal generation it took.
    pub applied: u64,
}

impl LayoutTree {
    /// The fields that came to show a handle this pass claim it: a handle
    /// is shown by one field at a time (its focus, its input method and its
    /// caret are one node's). Run once the whole frame is reconciled, every
    /// layer with it, so a field moving from a closing layer to another
    /// finds the one leaving gone. A second field showing a handle while
    /// the first is mounted is a bug, a panic in a debug build; else the
    /// first keeps it, and its focus.
    pub(super) fn claim_fields(&mut self) {
        let mut mounting = std::mem::take(&mut self.mounting);
        for (f, u) in mounting.drain(..) {
            let Some(ex) = self.extra(u) else { continue };
            if !matches!(&ex.content, Pending::Input(p) if p.field.focus.id() == f) {
                continue;
            }
            match self.by_field.get(&f).copied() {
                Some(first) if first != u => {
                    debug_assert!(
                        self.detached,
                        "a TextField is shown by two fields; give each field its own TextField bound to the same model"
                    );
                    self.by_focus.insert(f, first);
                }
                _ => {
                    self.by_field.insert(f, u);
                    if let Some(ex) = self.extra_mut(u) {
                        ex.claim = Some(f);
                    }
                }
            }
        }
        self.mounting = mounting;
    }

    /// The handles claimed: one per field shown.
    #[cfg(test)]
    pub(crate) fn field_claims(&self) -> usize {
        self.by_field.len()
    }

    /// What it keeps for text fields (claims, what they settle): bounded
    /// by the fields shown, however long they are edited.
    #[cfg(test)]
    pub(crate) fn field_bookkeeping(&self) -> usize {
        self.by_field
            .len()
            .saturating_add(self.mounting.capacity())
            .saturating_add(self.fields.with(Vec::capacity))
    }

    /// What placing text fields said of their scroll since last taken
    /// (offsets it clamped or moved to show a caret), swapped into `into`
    /// (emptied first): the two buffers go back and forth, so settling
    /// allocates nothing once they have grown.
    pub(crate) fn swap_fields(&self, into: &mut Vec<FieldSettled>) {
        into.clear();
        self.fields.with_mut(|f| std::mem::swap(f, into));
    }
}

/// How a text input lays out its text: the style, the wrap width, and where
/// the text box starts in the window, moved by the field's scroll.
pub(crate) struct InputText {
    pub t: ResolvedText,
    pub width: Option<f32>,
    pub origin: (f32, f32),
}

/// A text input's text as laid out in its content box `area`, its content
/// moved by `scroll` (the field's [`Scroll::applied`](super::Scroll::applied): what the content
/// transform paint draws it under moves it by, so paint passes none). Its
/// scroll offset is its handle's (`TextField::offset`), as a
/// scroll container's is its own: caret moves bring the caret into view
/// ([`caret_reveal`]), the wheel and a held drag move it ([`field_reach`]
/// bounds it). Not being edited, a truncating single line is cut with an
/// ellipsis.
pub(crate) fn input_text(
    text: &mut TextSystem,
    content: &str,
    t: &ResolvedText,
    area: Bounds,
    multiline: bool,
    focused: bool,
    scroll: (f32, f32),
) -> InputText {
    // Being edited, it is never cut short (`ResolvedText::editing`).
    let edited;
    let t = if focused && (t.truncate || t.line_clamp.is_some()) {
        edited = t.clone().editing();
        &edited
    } else {
        t
    };
    if multiline {
        // Being edited, its box keeps the height it was measured at (a
        // clamp's, as Chrome's box does not grow on focus); its text
        // scrolls in it.
        return InputText {
            t: t.clone(),
            width: Some(area.width),
            origin: (area.x + scroll.0, area.y + scroll.1),
        };
    }
    let y = area.y + ((area.height - t.line_height) / 2.0).max(0.0) + scroll.1;
    // Not being edited, a truncating one is cut at its box with an
    // ellipsis, as Chrome's `text-overflow` cuts an unfocused input.
    if t.truncate {
        return InputText {
            t: ResolvedText {
                nowrap: true,
                ..t.clone()
            },
            width: Some(area.width),
            origin: (area.x + scroll.0, y),
        };
    }
    let line = ResolvedText {
        nowrap: true,
        truncate: false,
        ..t.clone()
    };
    // Text that fits is aligned in the box; text that does not scrolls,
    // from its inline start: a right-to-left one overflows to the left.
    let (free, _) = text.measure(content, &line, None);
    if line.physical_align() != TextAlign::Left && free <= area.width {
        return InputText {
            t: line,
            width: Some(area.width),
            origin: (area.x + scroll.0, y),
        };
    }
    let x = if line.direction == Direction::Rtl && free + 1.0 > area.width {
        area.right() - free - 1.0
    } else {
        area.x
    };
    InputText {
        t: line,
        width: None,
        origin: (x + scroll.0, y),
    }
}

/// How far a text input's content scrolls on each axis (its scroll's
/// [`Scroll::max`](super::Scroll::max)): what of its text, laid out as [`input_text`] lays it
/// out, and its caret (a pixel wide past a line's end) reach past its
/// content box `area`. A clamp not being edited fits its box: none.
pub(crate) fn field_reach(
    text: &mut TextSystem,
    content: &str,
    t: &ResolvedText,
    area: Bounds,
    multiline: bool,
    focused: bool,
) -> (f32, f32) {
    let it = input_text(text, content, t, area, multiline, focused, (0.0, 0.0));
    let (width, height) = text.measure(content, &it.t, it.width);
    // Across: a line left unwrapped (a single line's not aligned in its
    // box, or a `nowrap` textarea's), down: a textarea's lines.
    let across = it.width.is_none() || (multiline && it.t.nowrap);
    (
        if across {
            (width + 1.0 - area.width).max(0.0)
        } else {
            0.0
        },
        if multiline {
            (height - area.height).max(0.0)
        } else {
            0.0
        },
    )
}

/// The scroll offset that brings a field's caret (at byte `caret` of its
/// `content`) into view from `offset`, scrolling each axis as little as
/// it must, as Chrome does (not at all where the caret shows already),
/// within `max` ([`field_reach`]): one rule for both axes of a single
/// line and a textarea alike.
#[expect(clippy::too_many_arguments, reason = "a field's text, box and scroll")]
pub(crate) fn caret_reveal(
    text: &mut TextSystem,
    content: &str,
    caret: usize,
    t: &ResolvedText,
    area: Bounds,
    multiline: bool,
    focused: bool,
    (offset, max): ((f32, f32), (f32, f32)),
) -> (f32, f32) {
    let it = input_text(text, content, t, area, multiline, focused, (0.0, 0.0));
    let c = text.caret(content, &it.t, it.width, caret);
    // The caret unscrolled, and the offsets (lo..=hi) that show it.
    let (x, y) = (it.origin.0 + c.x, it.origin.1 + c.y);
    let into = |o: f32, lo: f32, hi: f32| if o > hi { hi.max(lo) } else { o.max(lo) };
    // A right-to-left field scrolls its content rightward ([`Scroll::applied`]).
    let ox = if t.direction == Direction::Rtl {
        into(offset.0, area.x - x, area.right() - 1.0 - x)
    } else {
        into(offset.0, x + 1.0 - area.right(), x - area.x)
    };
    let oy = into(offset.1, y + c.height - area.bottom(), y - area.y);
    (ox.clamp(0.0, max.0.max(0.0)), oy.clamp(0.0, max.1.max(0.0)))
}
