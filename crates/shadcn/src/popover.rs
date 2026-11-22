//! Popover (`ui/popover`): a floating panel next to its trigger, opened and
//! closed by pressing the trigger, dismissed by Escape or a press outside.
//!
//! The parts nest as in Vue: [`popover`] is the anchor the panel is placed
//! against and keeps whether it is open, its own (`default_open`) or the
//! caller's (`open`, `v-model:open`); it holds the [`popover_trigger`] and
//! the [`popover_content`]. Like a dialog the content takes focus when it
//! opens and gives it back when it closes, unless told
//! [`open_auto_focus(false)`](Part::open_auto_focus).
//!
//! ```ignore
//! popover()
//!     .child(popover_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child("Open")))
//!     .child(popover_content().width(320.0).child("…"))
//! ```

use gui::prelude::*;
use gui::{
    Bind, Div, Element, Model, NamedKey, Positioning, Side, SideAlign, UseModel, component, deferred, div, provide,
    use_context, use_model,
};

use gui::style::Origin;

use crate::dialog::return_focus;
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
}

/// What the parts below a [`popover`] read: whether it is open.
#[derive(Clone, Copy)]
pub(crate) struct PopoverContext {
    open: UseModel<PartProps<PopoverProps>, bool>,
}

impl PopoverContext {
    pub(crate) fn find() -> Option<Self> {
        use_context::<Self>()
    }

    /// Whether the popover shows (tracked).
    pub(crate) fn is_open(self) -> bool {
        self.open.get()
    }

    pub(crate) fn set_open(self, open: bool) {
        self.open.set(open);
    }
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
                provide(PopoverContext {
                    open: use_model(props, |p| &p.props.open),
                });
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
    pub fn on_open_change(self, f: impl Fn(bool) + 'static) -> Self {
        self.with_props(|p| p.open = p.open.clone().on_change(f))
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
            let popover = PopoverContext::find();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match popover {
                    Some(o) => root
                        .aria_expanded(o.is_open())
                        .on_click(move |_| o.set_open(!o.open.get_untracked())),
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
/// the popover is closed, unless it is kept mounted (`.force_mount()`);
/// Escape or a press outside closes it.
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
    let props = PopoverContentProps {
        side: Side::Bottom,
        align: SideAlign::Center,
        auto_focus: true,
        force_mount: false,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let popover = PopoverContext::find();
            let auto_focus = props.with_untracked(|p| p.props.auto_focus);
            if let Some(o) = popover {
                return_focus(move || o.is_open(), auto_focus, None);
            }
            move || {
                let Some(o) = popover else {
                    return Element::Empty;
                };
                let (p, root) = props.with(|p| (p.props, p.slot.0.clone()));
                if !o.is_open() {
                    // Kept, hidden where it would float from: a layer is no
                    // step of its instances' identity, so they stay.
                    return if p.force_mount {
                        root.display(Display::None).into_element()
                    } else {
                        Element::Empty
                    };
                }
                let content = root
                    .on_key_down(move |e| {
                        let escape = e.is(NamedKey::Escape);
                        if escape {
                            o.set_open(false);
                        }
                        escape
                    })
                    .on_click_outside(move || o.set_open(false));
                deferred(
                    Positioning::Anchored {
                        side: p.side,
                        align: p.align,
                        offset: 4.0,
                        match_width: false,
                        fit_height: false,
                    },
                    content,
                )
                .into_element()
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
                gui::layout::Content::Text { content, .. } if content.starts_with("n ") => Some(content.clone()),
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
}
