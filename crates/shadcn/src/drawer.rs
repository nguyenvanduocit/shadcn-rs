//! Drawer (`ui/drawer`): a panel that slides in from a window edge and is
//! dismissed by swiping it back toward that edge, by Escape, or by a press on
//! the dimmed page. From the bottom edge it carries a grab handle.
//!
//! It is a modal like a dialog: [`DrawerState`] holds a [`DialogState`] plus
//! the swipe in progress.

use std::cell::Cell;
use std::rc::Rc;

use gui::prelude::*;
use gui::style::Calc;
use gui::{Animation, Div, DragEvent, DragPhase, Easing, Element, Radius, Signal, Text, div, relative, text};

use crate::dialog::{DialogState, dialog_description, dialog_overlay, modal_layer};
use crate::sheet::slide;
use crate::theme::theme;

/// How far a swipe must carry the drawer toward its edge to dismiss it
/// (Reka's `DEFAULT_SWIPE_THRESHOLD`).
const SWIPE_THRESHOLD: f32 = 40.0;

/// The direction a drawer is swiped to dismiss it, which is also the edge it
/// sits on (`swipe-direction`; `down` by default, at the bottom edge).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DrawerDirection {
    Up,
    #[default]
    Down,
    Left,
    Right,
}

/// Whether a drawer is open, the edge it sits on, and how far it is being swiped.
#[derive(Clone)]
pub struct DrawerState {
    open: DialogState,
    direction: DrawerDirection,
    /// Pixels the drawer has been dragged toward its edge.
    swipe: Signal<f32>,
    /// Where the drag started.
    origin: Rc<Cell<(f32, f32)>>,
}

impl Default for DrawerState {
    fn default() -> Self {
        Self::new(DrawerDirection::default())
    }
}

impl DrawerState {
    /// A closed drawer dismissed by swiping in `direction`.
    #[must_use]
    pub fn new(direction: DrawerDirection) -> Self {
        Self {
            open: DialogState::new(),
            direction,
            swipe: Signal::labeled("drawer-swipe", 0.0),
            origin: Rc::new(Cell::new((0.0, 0.0))),
        }
    }

    /// Whether the drawer shows (reactive).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.is_open()
    }

    pub fn open(&self) {
        self.swipe.set(0.0);
        self.open.open();
    }

    pub fn close(&self) {
        self.open.close();
    }

    /// Follow a drag on the drawer: it moves only toward its edge, and a
    /// release past [`SWIPE_THRESHOLD`] dismisses it.
    fn dragged(&self, e: &DragEvent) {
        match e.phase {
            DragPhase::Start => {
                self.origin.set((e.x, e.y));
                self.swipe.set(0.0);
            }
            DragPhase::Move => {
                let (x, y) = self.origin.get();
                let toward_edge = match self.direction {
                    DrawerDirection::Up => y - e.y,
                    DrawerDirection::Down => e.y - y,
                    DrawerDirection::Left => x - e.x,
                    DrawerDirection::Right => e.x - x,
                };
                self.swipe.set(toward_edge.max(0.0));
            }
            DragPhase::End => {
                if self.swipe.get() >= SWIPE_THRESHOLD {
                    self.close();
                }
                self.swipe.set(0.0);
            }
        }
    }
}

/// Make `trigger` open the drawer when pressed (`DrawerTrigger as-child`).
#[must_use]
pub fn drawer_trigger(state: &DrawerState, trigger: Div) -> Div {
    let state = state.clone();
    trigger.on_click(move |_| state.open())
}

/// Make `close` close the drawer when pressed (`DrawerClose as-child`).
#[must_use]
pub fn drawer_close(state: &DrawerState, close: Div) -> Div {
    let state = state.clone();
    close.on_click(move |_| state.close())
}

/// The drawer on its edge; `build` adds the classes and children a caller
/// gives `DrawerContent`.
#[must_use]
pub fn drawer_content(state: &DrawerState, build: impl FnOnce(Div) -> Div) -> Element {
    if !state.is_open() {
        return Element::Empty;
    }
    let t = theme();
    let on_drag = state.clone();
    let swipe = state.swipe.get();
    // Dragged toward its edge it moves with the pointer
    // (`--drawer-swipe-movement-*`); let go short of dismissing, it eases back.
    let (along, off) = match state.direction {
        DrawerDirection::Up => ((0.0, -swipe), (Calc::default(), Calc::fraction(-1.0))),
        DrawerDirection::Down => ((0.0, swipe), (Calc::default(), Calc::fraction(1.0))),
        DrawerDirection::Right => ((swipe, 0.0), (Calc::fraction(1.0), Calc::default())),
        DrawerDirection::Left => ((-swipe, 0.0), (Calc::fraction(-1.0), Calc::default())),
    };
    let base = div()
        .named("drawer-content")
        .bg(t.background)
        .absolute()
        .flex_col()
        .h_auto()
        // `slide-in-from-bottom` and its siblings, `duration-500
        // ease-[cubic-bezier(0.32,0.72,0,1)]`; `data-swiping:duration-0`.
        .animate(slide(Animation::enter(), off))
        .animate_out(slide(Animation::exit(), off))
        .transition_transform()
        .duration_ms(if swipe > 0.0 { 0.0 } else { 500.0 })
        .ease(Easing::CubicBezier(0.32, 0.72, 0.0, 1.0))
        .translate_x(Calc::px(along.0))
        .translate_y(Calc::px(along.1))
        .on_drag(move |e| on_drag.dragged(e));
    let mut placed = match state.direction {
        DrawerDirection::Up => base.left(0.0).right(0.0).top(0.0).mb(24.0).rounded_b(Radius::LG),
        DrawerDirection::Down => base
            .left(0.0)
            .right(0.0)
            .bottom(0.0)
            .mt(24.0)
            .rounded_t(Radius::LG)
            .child(drawer_handle()),
        DrawerDirection::Right => base.top(0.0).bottom(0.0).right(0.0).w_len(relative(0.75)).max_w(96.0),
        DrawerDirection::Left => base.top(0.0).bottom(0.0).left(0.0).w_len(relative(0.75)).max_w(96.0),
    };
    if matches!(state.direction, DrawerDirection::Up | DrawerDirection::Down) {
        // `max-h-[80vh]`: the layer is the window's height.
        placed.style_mut().max_height = relative(0.8);
    }
    // `DrawerOverlay`: the dialog's scrim, fading over 500ms.
    modal_layer(
        &state.open,
        div(),
        dialog_overlay().duration_ms(500.0),
        build(placed),
        true,
    )
}

/// The grab bar atop a bottom drawer.
fn drawer_handle() -> Div {
    let t = theme();
    div()
        .named("drawer-handle")
        .bg(t.muted)
        .mx_auto()
        .mt(4.0)
        .h(2.0)
        .w_px(100.0)
        .shrink_0()
        .rounded_full()
}

#[must_use]
pub fn drawer_header() -> Div {
    div().named("drawer-header").flex_col().gap(1.5).p(4.0)
}

#[must_use]
pub fn drawer_footer() -> Div {
    div().named("drawer-footer").mt_auto().flex_col().gap(2.0).p(4.0)
}

#[must_use]
pub fn drawer_title(title: impl Into<String>) -> Text {
    let t = theme();
    text(title)
        .named("drawer-title")
        .text_color(t.foreground)
        .font_semibold()
}

#[must_use]
pub fn drawer_description(description: impl Into<String>) -> Text {
    dialog_description(description).named("drawer-description")
}
