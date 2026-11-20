//! Sheet (`ui/sheet`): a panel that slides over the page from one of its
//! edges, on a dimmed page, with a close button in its corner.
//!
//! It opens and closes through a [`DialogState`] and
//! [`dialog_trigger`](crate::dialog_trigger) / [`dialog_close`](crate::dialog_close),
//! as shadcn builds it on Reka's `DialogRoot`.

use gui::prelude::*;
use gui::style::Calc;
use gui::{Animation, Div, Element, Text, div, relative, text};

use crate::dialog::{DialogState, corner_close, dialog_description, dialog_overlay, modal_layer};
use crate::theme::theme;

/// The window edge a sheet sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SheetSide {
    Top,
    #[default]
    Right,
    Bottom,
    Left,
}

/// The sheet on `side`; `build` adds the classes and children a caller gives
/// `SheetContent`.
#[must_use]
pub fn sheet_content(state: &DialogState, side: SheetSide, build: impl FnOnce(Div) -> Div) -> Element {
    if !state.is_open() {
        return Element::Empty;
    }
    let t = theme();
    // `slide-in-from-right` and `slide-out-to-right` (and the other sides):
    // a whole width or height off the window, in over 500ms, out over 300ms,
    // `ease-in-out`.
    let off = match side {
        SheetSide::Right => (Calc::fraction(1.0), Calc::default()),
        SheetSide::Left => (Calc::fraction(-1.0), Calc::default()),
        SheetSide::Top => (Calc::default(), Calc::fraction(-1.0)),
        SheetSide::Bottom => (Calc::default(), Calc::fraction(1.0)),
    };
    let base = div()
        .named("sheet-content")
        .role(gui::Role::Dialog)
        .animate(slide(Animation::enter(), off).duration(0.5))
        .animate_out(slide(Animation::exit(), off).duration(0.3))
        .transition()
        .ease_in_out()
        .bg(t.background)
        .absolute()
        .flex_col()
        .gap(4.0)
        .shadow_lg()
        .border_color(t.border);
    let placed = match side {
        SheetSide::Right => base
            .top(0.0)
            .bottom(0.0)
            .right(0.0)
            .h_full()
            .w_len(relative(0.75))
            .border_l()
            .max_w(96.0),
        SheetSide::Left => base
            .top(0.0)
            .bottom(0.0)
            .left(0.0)
            .h_full()
            .w_len(relative(0.75))
            .border_r()
            .max_w(96.0),
        SheetSide::Top => base.left(0.0).right(0.0).top(0.0).h_auto().border_b(),
        SheetSide::Bottom => base.left(0.0).right(0.0).bottom(0.0).h_auto().border_t(),
    };
    modal_layer(
        state,
        div(),
        dialog_overlay(),
        build(placed).child(corner_close(state)),
        true,
    )
}

/// `a` from or to `off`, a fraction of the sheet's own size.
pub(crate) fn slide(a: Animation, (x, y): (Calc, Calc)) -> Animation {
    a.slide_x(x).slide_y(y)
}

#[must_use]
pub fn sheet_header() -> Div {
    div().named("sheet-header").flex_col().gap(1.5).p(4.0)
}

#[must_use]
pub fn sheet_footer() -> Div {
    div().named("sheet-footer").mt_auto().flex_col().gap(2.0).p(4.0)
}

#[must_use]
pub fn sheet_title(title: impl Into<String>) -> Text {
    let t = theme();
    text(title)
        .named("sheet-title")
        .text_color(t.foreground)
        .font_semibold()
}

#[must_use]
pub fn sheet_description(description: impl Into<String>) -> Text {
    dialog_description(description).named("sheet-description")
}
