//! Popover (`ui/popover`): a floating panel next to its trigger, opened and
//! closed by pressing the trigger, dismissed by Escape or a press outside.
//!
//! The parts nest as in Vue: [`popover`] is the anchor the panel is placed
//! against; it holds the [`popover_trigger`] and the [`popover_content`].
//!
//! ```ignore
//! popover()
//!     .child(popover_trigger(&state, button(ButtonVariant::Outline, ButtonSize::Default).child("Open")))
//!     .child(popover_content(&state, |c| c.w(80.0).child("…")))
//! ```

use gui::prelude::*;
use gui::{Div, Element, NamedKey, Placement, Side, SideAlign, deferred, div};

use gui::style::Origin;

use crate::dialog::DialogState;
use crate::motion::popper;
use crate::theme::theme;

/// Whether a popover is open; like a dialog, it takes focus when it opens and
/// gives it back to the trigger when it closes, unless made
/// [`PopoverState::without_auto_focus`].
#[derive(Clone)]
pub struct PopoverState {
    open: DialogState,
    auto_focus: bool,
}

impl Default for PopoverState {
    fn default() -> Self {
        Self::new()
    }
}

impl PopoverState {
    /// A closed popover.
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: DialogState::new(),
            auto_focus: true,
        }
    }

    /// A closed popover that leaves focus where it is when it opens
    /// (`@open-auto-focus.prevent`), so typing in the field that opened it
    /// goes on.
    #[must_use]
    pub fn without_auto_focus() -> Self {
        Self {
            open: DialogState::new(),
            auto_focus: false,
        }
    }

    /// Whether the popover shows (reactive).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.is_open()
    }

    pub fn open(&self) {
        if self.auto_focus {
            self.open.open();
        } else {
            self.open.show();
        }
    }

    pub fn close(&self) {
        self.open.close();
    }

    pub fn toggle(&self) {
        if self.is_open() {
            self.close();
        } else {
            self.open();
        }
    }
}

/// The anchor the content is placed against: wrap the trigger (and the
/// content) in it.
#[must_use]
pub fn popover() -> Div {
    div().named("popover").inline_flex()
}

/// Make `trigger` open and close the popover when pressed (`PopoverTrigger as-child`).
#[must_use]
pub fn popover_trigger(state: &PopoverState, trigger: Div) -> Div {
    let state = state.clone();
    trigger.on_click(move |_| state.toggle())
}

/// The panel, below its anchor and centred on it, 4px away; `build` adds the
/// classes and children a caller gives `PopoverContent`.
#[must_use]
pub fn popover_content(state: &PopoverState, build: impl FnOnce(Div) -> Div) -> Element {
    popover_content_placed(state, Side::Bottom, SideAlign::Center, build)
}

/// [`popover_content`] on `side` of its anchor, lined up by `align`
/// (`<PopoverContent side="…" align="…">`).
#[must_use]
pub fn popover_content_placed(
    state: &PopoverState,
    side: Side,
    align: SideAlign,
    build: impl FnOnce(Div) -> Div,
) -> Element {
    if !state.is_open() {
        return Element::Empty;
    }
    let t = theme();
    let on_key = state.clone();
    let on_outside = state.clone();
    let content = popper(div(), Origin::Anchor)
        .named("popover-content")
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .w(72.0)
        .rounded_md()
        .border()
        .border_color(t.border)
        .p(4.0)
        .shadow_md()
        .on_key_down(move |e| {
            let escape = e.is(NamedKey::Escape);
            if escape {
                on_key.close();
            }
            escape
        })
        .on_click_outside(move || on_outside.close());
    deferred(
        Placement::Anchored {
            side,
            align,
            offset: 4.0,
            match_width: false,
            fit_height: false,
        },
        build(content),
    )
    .into_element()
}
