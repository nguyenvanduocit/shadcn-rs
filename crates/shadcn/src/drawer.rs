//! Drawer (`ui/drawer`): a panel that slides in from a window edge and is
//! dismissed by swiping it back toward that edge, by Escape, or by a press on
//! the dimmed page. From the bottom edge it carries a grab handle.
//!
//! The behaviour is `primitives::drawer`'s (reka's `Drawer`): every change
//! asks the caller first, a swipe follows the pointer and closes past 40 px
//! or faster than 0.3 px/ms, closing gives focus back to the trigger. It
//! opens and closes through [`drawer_trigger`] and [`drawer_close`] (or the
//! dialog's), and keeps its open state as a dialog does. Snap points
//! (`snap_points`, a two-way `snap_point`), `sequential_snap` and
//! `on_open_complete` are vaul's. The content follows the swipe and its
//! snap point, and the overlay fades as the swipe goes. This module draws
//! it.

use gui::prelude::*;
use gui::style::Calc;
use gui::{
    Animation, Bind, Div, Easing, Element, IntoElement, Model, Signal, component, div, provide, text, use_model,
};
use primitives::context::use_root;
use primitives::drawer::{ChangeReason, Drawer, DrawerOptions};
use primitives::presence::Presence;

pub use primitives::drawer::{DrawerModal, SnapPoint, SwipeDirection as DrawerDirection};

use crate::dialog::{
    Modal, ModalLook, ModalText, dialog_close, dialog_description, dialog_trigger, modal_layer_with, modal_title,
};
use crate::part::{Part, PartProps};
use crate::sheet::slide;
use crate::theme::theme;

/// `duration-500`: how long the drawer and its overlay move.
const DURATION_MS: f32 = 500.0;

/// What [`drawer`] is given.
#[derive(Clone, PartialEq, Default)]
pub struct DrawerProps {
    open: Bind<bool>,
    snap: Bind<Option<SnapPoint>>,
    snap_points: Vec<SnapPoint>,
    sequential_snap: bool,
    modal: DrawerModal,
    veto: Option<gui::Callback<(bool, ChangeReason), gui::Dismiss>>,
    settled: Option<gui::Callback<bool>>,
}

/// The drawer of a [`drawer`] root.
type DrawerState = Drawer<PartProps<DrawerProps>>;

/// What the content part reads from its root: the drawer, and the edge the
/// content tells it it sits on.
#[derive(Clone, Copy)]
struct DrawerContext {
    drawer: DrawerState,
    direction: Signal<DrawerDirection>,
}

/// The drawer's root: give it a [`drawer_trigger`] and a [`drawer_content`].
#[must_use]
pub fn drawer() -> Part<DrawerProps> {
    Part::new(DrawerProps::default(), div().named("drawer"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let direction = Signal::labeled("drawer direction", DrawerDirection::Down);
            let d = Drawer::new(
                use_model(props, |p: &PartProps<DrawerProps>| &p.props.open),
                Some(use_model(props, |p: &PartProps<DrawerProps>| &p.props.snap)),
                move || {
                    let direction = direction.get();
                    props.with(|p| DrawerOptions {
                        modal: p.props.modal,
                        direction,
                        snap_points: p.props.snap_points.clone(),
                        sequential_snap: p.props.sequential_snap,
                        veto: p.props.veto.clone(),
                        on_settled: p.props.settled.clone(),
                        ..DrawerOptions::default()
                    })
                },
            );
            provide(DrawerContext { drawer: d, direction });
            provide(Modal::Drawer(d));
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl Part<DrawerProps> {
    /// Open or not, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn open(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_model(open))
    }

    /// Open from the start, when no model is given (`default-open`).
    #[must_use]
    pub fn default_open(self, open: bool) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_default(open))
    }

    /// Call `f` each time it opens or closes (`@update:open`).
    #[must_use]
    pub fn on_open_change(self, f: impl Into<gui::Callback<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().on_change(f))
    }

    /// Call `f` once it has finished opening (`true`) or closing, its
    /// animation run (`@update:open-complete`).
    #[must_use]
    pub fn on_open_complete(self, f: impl Into<gui::Callback<bool>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.settled = Some(f))
    }

    /// How modal it is (`:modal`; [`DrawerModal::On`] by default).
    #[must_use]
    pub fn modal(self, modal: DrawerModal) -> Self {
        self.with_props(|p| p.modal = modal)
    }

    /// The heights it rests at (`:snap-points`): fractions of the window's
    /// height, pixels, or rems.
    #[must_use]
    pub fn snap_points(self, points: impl IntoIterator<Item = SnapPoint>) -> Self {
        self.with_props(|p| p.snap_points = points.into_iter().collect())
    }

    /// The snap point it rests at, kept by the caller (`v-model:snap-point`;
    /// `None`: fully open).
    #[must_use]
    pub fn snap_point(self, point: impl Into<Model<Option<SnapPoint>>>) -> Self {
        self.with_props(|p| p.snap = p.snap.clone().with_model(point))
    }

    /// The snap point it rests at first, when no model is given
    /// (`default-snap-point`).
    #[must_use]
    pub fn default_snap_point(self, point: Option<SnapPoint>) -> Self {
        self.with_props(|p| p.snap = p.snap.clone().with_default(point))
    }

    /// Let go, it moves one snap point at most (`snap-to-sequential-points`).
    #[must_use]
    pub fn sequential_snap(self, sequential: bool) -> Self {
        self.with_props(|p| p.sequential_snap = sequential)
    }

    /// Answer each request to open or close, with its reason
    /// (`@update:open` with `details.cancel()`).
    #[must_use]
    pub fn on_request(self, f: impl Into<gui::Callback<(bool, ChangeReason), gui::Dismiss>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.veto = Some(f))
    }
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
/// toward, which it tells its root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DrawerContentProps {
    direction: DrawerDirection,
}

/// The drawer on the edge of `direction` (`DrawerContent`; `Down`, at the
/// bottom edge with a grab handle, is the default); style and fill it as a
/// `Div`. Dragged toward its edge it follows the pointer, and with snap
/// points it rests at the active one.
#[must_use]
pub fn drawer_content(direction: DrawerDirection) -> Part<DrawerContentProps> {
    let t = theme();
    let root = div()
        .named("drawer-content")
        .background(t.background)
        .position(Position::Absolute)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .height(Length::Auto)
        // `duration-500 ease-[cubic-bezier(0.32,0.72,0,1)]`.
        .transition(Properties::TRANSFORM)
        .ease(Easing::CubicBezier(0.32, 0.72, 0.0, 1.0));
    Part::new(DrawerContentProps { direction }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let root = use_root::<DrawerContext>("drawer_content", "drawer");
            if let Some(r) = root {
                // The edge is the root's: the content tells it, now and as
                // it changes, from an effect (setup, as a render, writes
                // nothing it did not make).
                gui::Effect::new(move || r.direction.set(props.with(|p| p.props.direction)));
            }
            move || {
                let Some(DrawerContext { drawer: d, .. }) = root else {
                    return Element::Empty;
                };
                let presence = d.presence();
                if presence == Presence::Gone {
                    return Element::Empty;
                }
                let (direction, root) = props.with(|p| (p.props.direction, p.slot.0.clone()));
                let content = d.content(placed(root, direction, d));
                let mut content = content.into_element();
                if direction == DrawerDirection::Down
                    && let Some(children) = content.children_mut()
                {
                    children.insert(0, d.handle(drawer_handle()).into_element());
                }
                // `DrawerOverlay`: the dialog's scrim, fading over 500ms,
                // and with the swipe (`1 - --drawer-swipe-progress`).
                let look = ModalLook {
                    centred: false,
                    overlay_ms: Some(DURATION_MS),
                    close_button: false,
                };
                modal_layer_with(
                    presence,
                    d.has_overlay(),
                    look,
                    content,
                    |frame| d.viewport(frame),
                    move |o| o.opacity(move || 1.0 - d.progress()).duration_ms(move || duration(d)),
                )
            }
        })
    })
}

/// How long the drawer and its overlay move: nothing while dragged (the
/// pointer leads, `data-swiping:duration-0`), the release scalar's share of
/// 500 ms as a flung drawer finishes (`--drawer-swipe-strength`).
fn duration<P>(d: Drawer<P>) -> f32 {
    if d.dragging() {
        0.0
    } else if d.swiped() {
        DURATION_MS * d.release_scalar().unwrap_or(1.0)
    } else {
        DURATION_MS
    }
}

/// `root` on the edge of `direction`, moved by `d`'s snap point and swipe.
fn placed<P>(root: Div, direction: DrawerDirection, d: Drawer<P>) -> Div {
    let off = match direction {
        DrawerDirection::Up => (Calc::default(), Calc::fraction(-1.0)),
        DrawerDirection::Down => (Calc::default(), Calc::fraction(1.0)),
        DrawerDirection::Right => (Calc::fraction(1.0), Calc::default()),
        DrawerDirection::Left => (Calc::fraction(-1.0), Calc::default()),
    };
    let vertical = direction.vertical();
    // Dragged it moves with the pointer (`--drawer-swipe-movement-*`) from
    // its snap point (`--drawer-snap-point-offset`); let go short of
    // dismissing, it eases back. Bound to the node: a move renders nothing.
    let base = root
        .animation(slide(Animation::enter(), off))
        .exit_animation(slide(Animation::exit(), off))
        .duration_ms(move || duration(d))
        .translate_x(move || Calc::px(d.offset().0 + if vertical { 0.0 } else { d.snap_offset() }))
        .translate_y(move || Calc::px(d.offset().1 + if vertical { d.snap_offset() } else { 0.0 }));
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

/// The drawer's title, which names it to a screen reader.
pub fn drawer_title(title: impl Into<String>) -> ModalText {
    let t = theme();
    modal_title(
        text(title)
            .named("drawer-title")
            .text_color(t.foreground)
            .font_weight(600),
    )
}

/// The drawer's description, which describes it to a screen reader.
pub fn drawer_description(description: impl Into<String>) -> ModalText {
    dialog_description(description).named("drawer-description")
}
