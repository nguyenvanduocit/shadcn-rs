//! Drawer (`ui/drawer`): a panel that slides in from a window edge and is
//! dismissed by swiping it back toward that edge, by Escape, or by a press on
//! the dimmed page. From the bottom edge it carries a grab handle.
//!
//! As vaul builds it on Reka's `DialogRoot`, [`drawer`] is a
//! [`dialog`](crate::dialog) root: it opens and closes through
//! [`drawer_trigger`] and [`drawer_close`] (or the dialog's), and keeps its
//! open state the same way. Its content keeps the swipe in progress.

use std::cell::Cell;

use gui::prelude::*;
use gui::style::Calc;
use gui::{Animation, Div, DragEvent, DragPhase, Easing, Element, Signal, Text, component, div, text};

use crate::dialog::{
    DialogContext, DialogProps, ModalLook, dialog_close, dialog_description, dialog_trigger, modal_layer, modal_root,
    return_focus,
};
use crate::part::Part;
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

/// The drawer's root: give it a [`drawer_trigger`] and a [`drawer_content`].
#[must_use]
pub fn drawer() -> Part<DialogProps> {
    modal_root(div().named("drawer"))
}

/// Make `trigger` open the drawer when pressed (`DrawerTrigger as-child`).
/// Give `trigger` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
pub fn drawer_trigger(trigger: impl Into<Div>) -> Part<()> {
    dialog_trigger(trigger)
}

/// Make `close` close the drawer when pressed (`DrawerClose as-child`).
/// Give `close` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
pub fn drawer_close(close: impl Into<Div>) -> Part<()> {
    dialog_close(close)
}

/// What [`drawer_content`] is given: the edge it sits on and is swiped
/// toward (vaul's `direction`, given to the content, the one part that reads it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DrawerContentProps {
    direction: DrawerDirection,
}

/// The drawer on the edge of `direction` (`DrawerContent`; `Down`, at the
/// bottom edge with a grab handle, is vaul's default); style and fill it as
/// a `Div`. Dragged toward its edge it follows the pointer, and let go past
/// [`SWIPE_THRESHOLD`] it closes.
#[must_use]
pub fn drawer_content(direction: DrawerDirection) -> Part<DrawerContentProps> {
    let t = theme();
    let root = div()
        .named("drawer-content")
        .role(gui::Role::Dialog)
        .background(t.background)
        .position(Position::Absolute)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .height(Length::Auto)
        // `duration-500 ease-[cubic-bezier(0.32,0.72,0,1)]`.
        .transition(Properties::TRANSFORM)
        .ease(Easing::CubicBezier(0.32, 0.72, 0.0, 1.0))
        .when(direction == DrawerDirection::Down, |d| d.child(drawer_handle()));
    Part::new(DrawerContentProps { direction }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let dialog = DialogContext::find();
            // Pixels the drawer has been dragged toward its edge, and where the drag started.
            let swipe = Signal::labeled("drawer-swipe", 0.0_f32);
            let origin = std::rc::Rc::new(Cell::new((0.0_f32, 0.0_f32)));
            if let Some(d) = dialog {
                return_focus(move || d.is_open(), true, Some(d.focus));
                // Each opening starts unswiped.
                gui::watch(
                    move || d.is_open(),
                    move |open, _| {
                        if *open {
                            swipe.set(0.0);
                        }
                    },
                );
            }
            move || {
                let Some(d) = dialog else {
                    return Element::Empty;
                };
                if !d.is_open() {
                    return Element::Empty;
                }
                let (direction, root) = props.with(|p| (p.props.direction, p.slot.0.clone()));
                let origin = std::rc::Rc::clone(&origin);
                let dragged = move |e: &DragEvent| match e.phase {
                    DragPhase::Start => {
                        origin.set((e.x, e.y));
                        swipe.set(0.0);
                    }
                    DragPhase::Move => {
                        let (x, y) = origin.get();
                        let toward_edge = match direction {
                            DrawerDirection::Up => y - e.y,
                            DrawerDirection::Down => e.y - y,
                            DrawerDirection::Left => x - e.x,
                            DrawerDirection::Right => e.x - x,
                        };
                        swipe.set(toward_edge.max(0.0));
                    }
                    DragPhase::End => {
                        if swipe.get_untracked() >= SWIPE_THRESHOLD {
                            d.set_open(false);
                        }
                        swipe.set(0.0);
                    }
                };
                let content = placed(root, direction, swipe.get()).on_drag(dragged);
                // `DrawerOverlay`: the dialog's scrim, fading over 500ms.
                modal_layer(
                    d,
                    ModalLook {
                        centred: false,
                        overlay_ms: Some(500.0),
                        dismiss_outside: true,
                        close_button: false,
                    },
                    content,
                )
            }
        })
    })
}

/// `root` on the edge of `direction`, `swipe` pixels toward it.
fn placed(root: Div, direction: DrawerDirection, swipe: f32) -> Div {
    // Dragged toward its edge it moves with the pointer
    // (`--drawer-swipe-movement-*`); let go short of dismissing, it eases back.
    let (along, off) = match direction {
        DrawerDirection::Up => ((0.0, -swipe), (Calc::default(), Calc::fraction(-1.0))),
        DrawerDirection::Down => ((0.0, swipe), (Calc::default(), Calc::fraction(1.0))),
        DrawerDirection::Right => ((swipe, 0.0), (Calc::fraction(1.0), Calc::default())),
        DrawerDirection::Left => ((-swipe, 0.0), (Calc::fraction(-1.0), Calc::default())),
    };
    // `slide-in-from-bottom` and its siblings; `data-swiping:duration-0`.
    let base = root
        .animation(slide(Animation::enter(), off))
        .exit_animation(slide(Animation::exit(), off))
        .duration_ms(if swipe > 0.0 { 0.0 } else { 500.0 })
        .translate_x(Calc::px(along.0))
        .translate_y(Calc::px(along.1));
    let mut placed = match direction {
        DrawerDirection::Up => base
            .left(0.0)
            .right(0.0)
            .top(0.0)
            .margin_bottom(96.0)
            .radius_bottom(10.0),
        DrawerDirection::Down => base.left(0.0).right(0.0).bottom(0.0).margin_top(96.0).radius_top(10.0),
        DrawerDirection::Right => base.top(0.0).bottom(0.0).right(0.0).width(pct(75.0)).max_width(384.0),
        DrawerDirection::Left => base.top(0.0).bottom(0.0).left(0.0).width(pct(75.0)).max_width(384.0),
    };
    if matches!(direction, DrawerDirection::Up | DrawerDirection::Down) {
        // `max-h-[80vh]`: the layer is the window's height.
        placed.style_mut().max_height = Length::Fraction(0.8);
    }
    placed
}

/// The grab bar atop a bottom drawer.
fn drawer_handle() -> Div {
    let t = theme();
    div()
        .named("drawer-handle")
        .background(t.muted)
        .margin_x(Length::Auto)
        .margin_top(16.0)
        .height(8.0)
        .width(100.0)
        .flex_shrink(0.0)
        .radius(9999.0)
}

#[must_use]
pub fn drawer_header() -> Div {
    div()
        .named("drawer-header")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(6.0)
        .padding(16.0)
}

#[must_use]
pub fn drawer_footer() -> Div {
    div()
        .named("drawer-footer")
        .margin_top(Length::Auto)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .padding(16.0)
}

#[must_use]
pub fn drawer_title(title: impl Into<String>) -> Text {
    let t = theme();
    text(title)
        .named("drawer-title")
        .text_color(t.foreground)
        .font_weight(600)
}

#[must_use]
pub fn drawer_description(description: impl Into<String>) -> Text {
    dialog_description(description).named("drawer-description")
}
