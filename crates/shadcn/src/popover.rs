//! Popover (`ui/popover`): a floating panel next to its trigger, opened and
//! closed by pressing the trigger, dismissed by Escape or a press outside.
//!
//! The parts nest as in Vue: [`popover`] is the anchor the panel is placed
//! against and keeps whether it is open, its own (`default_open`) or the
//! caller's (`open`, `v-model:open`); it holds the [`popover_trigger`] and
//! the [`popover_content`]. The behaviour is `primitives::popover`'s: the
//! content takes focus when it opens (unless told
//! [`open_auto_focus(false)`](Part::open_auto_focus)) and gives it to the
//! trigger when it closes; Escape, a press outside it or focus going
//! outside it close it, and its trigger is not outside it. This module
//! draws it.
//!
//! ```ignore
//! popover()
//!     .child(popover_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child("Open")))
//!     .child(popover_content().width(320.0).child("…"))
//! ```

use gui::prelude::*;
use gui::{Bind, Div, Element, Model, Side, SideAlign, component, div, provide, use_model};
use primitives::context::use_root;
use primitives::popover::{Popover, PopoverOptions};
use primitives::popper::Popper;
use primitives::presence::Presence;

use gui::style::Origin;

use crate::dialog::Veto;
use crate::motion::popper;
use crate::part::{Part, PartProps};
use crate::theme::theme;
/// A popup's shadow (shadcn's `shadow-md`): menu, popover and hover card
/// content.
pub(crate) const POPUP_SHADOW: [Shadow; 2] = [
    Shadow {
        x: 0.0,
        y: 4.0,
        blur: 6.0,
        spread: -1.0,
        color: SHADE,
    },
    Shadow {
        x: 0.0,
        y: 2.0,
        blur: 4.0,
        spread: -2.0,
        color: SHADE,
    },
];

/// The shadow of what is lifted over a page or a popup (shadcn's
/// `shadow-lg`): a sub-menu, a dialog, a sheet.
pub(crate) const LIFTED_SHADOW: [Shadow; 2] = [
    Shadow {
        x: 0.0,
        y: 10.0,
        blur: 15.0,
        spread: -3.0,
        color: SHADE,
    },
    Shadow {
        x: 0.0,
        y: 4.0,
        blur: 6.0,
        spread: -4.0,
        color: SHADE,
    },
];

/// Black at 10%, the colour of both shadows (`rgb(0 0 0 / 0.1)`).
const SHADE: gui::Color = gui::Color {
    a: 0.1,
    ..gui::Color::BLACK
};

/// What [`popover`] is given.
#[derive(Clone, PartialEq, Default)]
pub struct PopoverProps {
    open: Bind<bool>,
    modal: bool,
    veto: Option<Veto>,
}

/// The popover of a [`popover`] root.
type PopoverState = Popover<PartProps<PopoverProps>>;

/// What the content's options come from: the root's props and the
/// content's own, which the content writes as it renders.
#[derive(Clone, Copy)]
struct PopoverContext {
    popover: PopoverState,
    content: gui::Signal<PopoverContentProps>,
}

/// The anchor the content is placed against: wrap the trigger (and the
/// content) in it. Closed, and kept by itself, until told otherwise.
#[must_use]
pub fn popover() -> Part<PopoverProps> {
    Part::new(
        PopoverProps::default(),
        div().named("popover").display(Display::Flex).inline(true),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let content = gui::Signal::labeled("popover content", PopoverContentProps::DEFAULT);
                let popover = Popover::new(
                    use_model(props, |p: &PartProps<PopoverProps>| &p.props.open),
                    move || {
                        let c = content.get();
                        props.with(|p| PopoverOptions {
                            modal: p.props.modal,
                            keep_mounted: c.force_mount,
                            open_auto_focus: c.auto_focus,
                            veto: p.props.veto.clone(),
                            ..PopoverOptions::default()
                        })
                    },
                );
                provide(PopoverContext { popover, content });
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

impl Part<PopoverProps> {
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

    /// Modal (`:modal`, off by default): focus trapped, the page inert and
    /// hidden from screen readers.
    #[must_use]
    pub fn modal(self, modal: bool) -> Self {
        self.with_props(|p| p.modal = modal)
    }

    /// Answer each close a user asks for (Escape, a press or focus outside).
    #[must_use]
    pub fn on_dismiss(self, f: impl Into<gui::Callback<gui::DismissReason, gui::Dismiss>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.veto = Some(f))
    }
}

/// Make `trigger` open and close the popover when pressed (`PopoverTrigger as-child`).
/// Give `trigger` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
pub fn popover_trigger(trigger: impl Into<Div>) -> Part<()> {
    Part::new((), trigger.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let popover = use_root::<PopoverContext>("popover_trigger", "popover");
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match popover {
                    Some(o) => o.popover.trigger(root),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// Place the content against `anchor` instead of the whole [`popover`]
/// (`PopoverAnchor as-child`): render it inside the popover; the trigger
/// still opens and closes it.
#[must_use]
pub fn popover_anchor(anchor: impl Into<Div>) -> Part<()> {
    Part::new((), anchor.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let popover = use_root::<PopoverContext>("popover_anchor", "popover");
            if let Some(o) = popover {
                o.popover.register_anchor();
            }
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match popover {
                    Some(o) => o.popover.anchor(root),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// Make `close` close the popover when pressed (`PopoverClose as-child`).
#[must_use]
pub fn popover_close(close: impl Into<Div>) -> Part<()> {
    Part::new((), close.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let popover = use_root::<PopoverContext>("popover_close", "popover");
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match popover {
                    Some(o) => o.popover.close(root),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// What [`popover_content`] is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopoverContentProps {
    side: Side,
    align: SideAlign,
    auto_focus: bool,
    force_mount: bool,
}

impl PopoverContentProps {
    const DEFAULT: Self = Self {
        side: Side::Bottom,
        align: SideAlign::Center,
        auto_focus: true,
        force_mount: false,
    };
}

impl Part<PopoverContentProps> {
    /// On `side` of its anchor (`side="…"`; below by default).
    #[must_use]
    pub fn side(self, side: Side) -> Self {
        self.with_props(|p| p.side = side)
    }

    /// Lined up with its anchor by `align` (`align="…"`; centred by default).
    #[must_use]
    pub fn align(self, align: SideAlign) -> Self {
        self.with_props(|p| p.align = align)
    }

    /// Whether opening takes focus off the page (`@open-auto-focus.prevent`
    /// is `false`): without it, typing in the field that opened it goes on,
    /// and closing gives nothing back.
    #[must_use]
    pub fn open_auto_focus(self, auto_focus: bool) -> Self {
        self.with_props(|p| p.auto_focus = auto_focus)
    }

    /// Kept mounted while closed (`force-mount`): hidden in its place, the
    /// same instances and state when it opens again.
    #[must_use]
    pub fn force_mount(self) -> Self {
        self.with_props(|p| p.force_mount = true)
    }
}

/// The panel, below its anchor and centred on it, 4px away
/// (`PopoverContent`); style and fill it as a `Div`. Nothing renders while
/// the popover is closed, unless it is kept mounted (`.force_mount()`).
#[must_use]
pub fn popover_content() -> Part<PopoverContentProps> {
    let t = theme();
    let root = popper(div(), Origin::Anchor)
        .named("popover-content")
        .background(t.popover)
        .text_color(t.popover_foreground)
        .width(288.0)
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .padding(16.0)
        .shadow(POPUP_SHADOW);
    Part::new(PopoverContentProps::DEFAULT, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let popover = use_root::<PopoverContext>("popover_content", "popover");
            if let Some(o) = popover {
                // The content's own props are options of the popover: an
                // effect hands them up, now and as they change (setup, as a
                // render, writes nothing it did not make).
                gui::Effect::new(move || o.content.set(props.with(|p| p.props)));
            }
            move || {
                let Some(o) = popover else {
                    return Element::Empty;
                };
                let (p, root) = props.with(|p| (p.props, p.slot.0.clone()));
                match o.popover.presence() {
                    Presence::Gone => Element::Empty,
                    // Kept, hidden where it would float from: a layer is no
                    // step of its instances' identity, so they stay.
                    Presence::Kept => root.display(Display::None).into_element(),
                    Presence::Open => {
                        let at = Popper::DEFAULT.side(p.side).align(p.align).side_offset(4.0);
                        o.popover.floating(root, &at).into_element()
                    }
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui::{Scenario, Signal, text};

    /// A component counting the bumps it hears.
    fn counter(bump: Signal<u32>) -> Element {
        component((), move |_| {
            let n = Signal::new(0_u32);
            let _bumps = gui::watch(move || bump.get(), move |_, _| n.update(|v| *v = v.saturating_add(1)));
            move || text(format!("n {}", n.get())).into_element()
        })
        .into_element()
    }

    fn shown(s: &Scenario) -> Vec<String> {
        s.last()
            .nodes
            .iter()
            .filter_map(|n| match &n.content {
                gui::layout::Content::Text { content, .. } if content.starts_with("n ") => Some(content.to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn content_kept_mounted_keeps_its_state_across_open_and_close() {
        let open = Signal::new(true);
        let bump = Signal::new(0_u32);
        let mut s = Scenario::measuring(
            move || {
                popover()
                    .open(open)
                    .child(popover_trigger(div().child("Open")))
                    .child(popover_content().force_mount().child(counter(bump)))
                    .into_element()
            },
            400,
            400,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        s.act("bump", || bump.set(1));
        s.settle().expect("settled");
        assert_eq!(shown(&s), ["n 1"]);
        let setups = gui::component::stats().setups;
        s.act("close", || open.set(false));
        s.settle().expect("settled");
        assert!(shown(&s).is_empty(), "closed, nothing shows");
        s.act("open", || open.set(true));
        s.settle().expect("settled");
        assert_eq!(shown(&s), ["n 1"], "the same counter, its count kept");
        assert_eq!(gui::component::stats().setups, setups, "nothing set up again");
    }

    fn bounds_of(s: &Scenario, name: &str) -> gui::Bounds {
        s.last()
            .nodes
            .iter()
            .find(|n| n.name == Some(name))
            .map(|n| n.bounds)
            .expect("the node on screen")
    }

    /// An explicit anchor tells the popover from its setup that it is there
    /// (through an effect: a setup writes nothing it did not make), and the
    /// content opens against it, in the first frame.
    #[test]
    fn an_explicit_anchor_mounts_and_places_the_content_below_it() {
        let open = Signal::new(true);
        let mut s = Scenario::measuring(
            move || {
                popover()
                    .open(open)
                    .child(popover_trigger(div().child("Open")))
                    .child(popover_anchor(
                        div().named("the-anchor").margin_top(200.0).width(40.0).height(20.0),
                    ))
                    .child(popover_content().child("Inside"))
                    .into_element()
            },
            400,
            400,
            1.0,
        )
        .expect("a scenario");
        s.frame("first").expect("a frame");
        let anchor = bounds_of(&s, "the-anchor");
        let content = bounds_of(&s, "popover-content");
        assert!(
            content.y >= anchor.y + anchor.height,
            "the content below the anchor: {content:?} against {anchor:?}"
        );
    }

    /// An anchor replaced in one update (keyed anew): the new one joins
    /// before the old one leaves, and the old one's leaving does not take
    /// the anchor away from the new one, so the content stays below it.
    #[test]
    fn an_anchor_replaced_in_one_update_keeps_the_content_below_it() {
        let open = Signal::new(true);
        let which = Signal::new(false);
        let mut s = Scenario::measuring(
            move || {
                popover()
                    .open(open)
                    .child(popover_trigger(div().child("Open")))
                    .child(
                        div()
                            .key(gui::key_of(if which.get() { "second" } else { "first" }))
                            .child(popover_anchor(
                                div().named("the-anchor").margin_top(200.0).width(40.0).height(20.0),
                            )),
                    )
                    // Below the anchor: content placed against the whole
                    // popover would sit under this, far from the anchor.
                    .child(div().height(100.0))
                    .child(popover_content().child("Inside"))
                    .into_element()
            },
            400,
            400,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        s.act("replace the anchor", || which.set(true));
        s.settle().expect("settled");
        let anchor = bounds_of(&s, "the-anchor");
        let content = bounds_of(&s, "popover-content");
        let gap = content.y - (anchor.y + anchor.height);
        assert!(
            (0.0..50.0).contains(&gap),
            "the content just below the new anchor: {content:?} against {anchor:?}"
        );
    }
}
