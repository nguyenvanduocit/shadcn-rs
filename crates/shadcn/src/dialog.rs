//! Dialog (`ui/dialog`): a modal window over a dimmed page, centred, with a
//! header, footer and a close button in its corner.
//!
//! [`DialogState`] is also the state of every other modal built on Reka's
//! `DialogRoot` (alert dialog, sheet, drawer): they open through
//! [`dialog_trigger`], close through [`dialog_close`] and share
//! [`modal_layer`], so Escape, the overlay and focus behave the same in all.

use gui::focus::{self, FocusId};
use gui::prelude::*;
use gui::{Animation, Color, Div, Element, Icon, NamedKey, Placement, Signal, Text, deferred, div, icon, text};

use crate::theme::theme;

/// Whether a dialog is open, and where focus goes back to when it closes.
#[derive(Clone)]
pub struct DialogState {
    open: Signal<bool>,
    /// The focus held when the dialog opened, and whether it was by keyboard.
    return_focus: Signal<Option<(FocusId, bool)>>,
}

impl Default for DialogState {
    fn default() -> Self {
        Self::new()
    }
}

impl DialogState {
    /// A closed dialog.
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: Signal::labeled("dialog-open", false),
            return_focus: Signal::labeled("dialog-return-focus", None),
        }
    }

    /// Whether the dialog shows (reactive).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Show the dialog, taking focus off the page so keys and Tab go to it.
    pub fn open(&self) {
        if self.open.get() {
            return;
        }
        self.return_focus
            .set(focus::focused().map(|id| (id, focus::focus_visible())));
        focus::set_focus(None, false);
        self.open.set(true);
    }

    /// Show it and leave focus where it is (`@open-auto-focus.prevent`):
    /// closing then gives nothing back.
    pub(crate) fn show(&self) {
        if !self.open.get() {
            self.open.set(true);
        }
    }

    /// Hide the dialog and give focus back to what held it before.
    pub fn close(&self) {
        if !self.open.get() {
            return;
        }
        self.open.set(false);
        if let Some((id, by_keyboard)) = self.return_focus.get() {
            focus::set_focus(Some(id), by_keyboard);
        }
        self.return_focus.set(None);
    }

    /// Open when closed, close when open.
    pub fn toggle(&self) {
        if self.open.get() { self.close() } else { self.open() }
    }
}

/// Make `trigger` open the dialog when pressed (`DialogTrigger as-child`).
#[must_use]
pub fn dialog_trigger(state: &DialogState, trigger: Div) -> Div {
    let state = state.clone();
    trigger.on_click(move |_| state.open())
}

/// Make `close` close the dialog when pressed (`DialogClose as-child`).
#[must_use]
pub fn dialog_close(state: &DialogState, close: Div) -> Div {
    let state = state.clone();
    close.on_click(move |_| state.close())
}

/// The page dimmer behind a modal: `fixed inset-0 z-50 bg-black/80`, fading
/// in and out with it.
#[must_use]
pub fn dialog_overlay() -> Div {
    crate::motion::fade(
        div()
            .named("dialog-overlay")
            .absolute()
            .inset(0.0)
            .bg(Color::BLACK.opacity(0.8)),
    )
}

/// A modal on its own window-sized layer: `overlay`, then `content` placed
/// by `frame` (the layer root's layout). Escape closes it; so does a press on
/// the overlay when `dismiss_outside` holds (an alert dialog keeps it open).
/// Nothing renders while `state` is closed.
pub(crate) fn modal_layer(
    state: &DialogState,
    frame: Div,
    overlay: Div,
    content: Div,
    dismiss_outside: bool,
) -> Element {
    if !state.is_open() {
        return Element::Empty;
    }
    let t = theme();
    let on_key = state.clone();
    let on_overlay = state.clone();
    let root = frame
        .named("dialog-portal")
        .size_full()
        .text_color(t.foreground)
        .on_key_down(move |e| {
            let escape = e.is(NamedKey::Escape);
            if escape {
                on_key.close();
            }
            escape
        })
        .child(overlay.when(dismiss_outside, |o| o.on_mouse_down(move |_| on_overlay.close())))
        .child(content);
    deferred(Placement::Fill, root).into_element()
}

/// The close button in a modal's top-right corner: an X at 70% opacity.
pub(crate) fn corner_close(state: &DialogState) -> Div {
    let t = theme();
    dialog_close(
        state,
        div()
            .named("dialog-close")
            .absolute()
            .top(4.0)
            .right(4.0)
            .rounded_xs()
            .opacity(0.7)
            .transition_opacity()
            .cursor_pointer()
            .focusable()
            .hover(|s| s.opacity(1.0))
            .focus(move |s| s.opacity(1.0).ring(2.0, t.ring))
            .aria_label("Close")
            .child(icon(Icon::X)),
    )
}

/// The dialog itself, centred on a dimmed page with a close button in its
/// corner; `build` adds the classes and children a caller gives
/// `DialogContent` (`|c| c.max_w_px(425.0).child(…)`).
#[must_use]
pub fn dialog_content(state: &DialogState, build: impl FnOnce(Div) -> Div) -> Element {
    if !state.is_open() {
        return Element::Empty;
    }
    let content = build(centred_surface().named("dialog-content").role(gui::Role::Dialog)).child(corner_close(state));
    centred_modal(state, content, true)
}

/// The box of a centred modal (dialog, alert dialog), fading and zooming in
/// and out over 200ms (`fade-in-0 zoom-in-95 duration-200` and their `-out`).
pub(crate) fn centred_surface() -> Div {
    let t = theme();
    div()
        .animate(Animation::enter().fade(0.0).zoom(0.95))
        .animate_out(Animation::exit().fade(0.0).zoom(0.95))
        .duration_ms(200.0)
        .bg(t.background)
        .grid()
        .w_full()
        .gap(4.0)
        .rounded_lg()
        .border()
        .border_color(t.border)
        .p(6.0)
        .shadow_lg()
        .max_w(128.0)
}

/// `content` centred over the overlay: `fixed top-1/2 left-1/2 -translate-1/2
/// max-w-[calc(100%-2rem)]`, so 1rem clear of the window's sides.
pub(crate) fn centred_modal(state: &DialogState, content: Div, dismiss_outside: bool) -> Element {
    let frame = div().flex().items_center().justify_center().px(4.0);
    modal_layer(state, frame, dialog_overlay(), content, dismiss_outside)
}

#[must_use]
pub fn dialog_header() -> Div {
    div().named("dialog-header").flex_col().gap(2.0).text_left()
}

#[must_use]
pub fn dialog_footer() -> Div {
    div().named("dialog-footer").flex_row().gap(2.0).justify_end()
}

#[must_use]
pub fn dialog_title(title: impl Into<String>) -> Text {
    text(title)
        .named("dialog-title")
        .text_lg()
        .leading_none()
        .font_semibold()
}

#[must_use]
pub fn dialog_description(description: impl Into<String>) -> Text {
    let t = theme();
    text(description)
        .named("dialog-description")
        .text_color(t.muted_foreground)
        .text_sm()
}
