//! Dialog (`ui/dialog`): a modal window over a dimmed page, centred, with a
//! header, footer and a close button in its corner.
//!
//! The parts nest as in Vue: [`dialog`] keeps whether it is open, its own
//! (`default_open`) or the caller's (`open`, `v-model:open`); the
//! [`dialog_trigger`], [`dialog_content`] and each [`dialog_close`] inside
//! read it. Every other modal built on Reka's `DialogRoot` (alert dialog,
//! sheet, drawer) is a [`dialog`] too, so Escape, the overlay and focus
//! behave the same in all: opening takes focus off the page, and closing,
//! however it happens (a part, Escape, or the caller writing the model),
//! gives it back (Reka's `getReturnFocusElement`): to the trigger when its
//! press opened the dialog, else to what held focus as it opened, else to
//! the trigger; with none of them, nothing keeps focus.
//!
//! ```ignore
//! dialog()
//!     .child(dialog_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child("Open")))
//!     .child(dialog_content().max_width(425.0).child(dialog_header().child(dialog_title("Edit profile"))))
//! ```

use gui::focus::{self, FocusId};
use gui::prelude::*;
use gui::{
    Animation, Bind, Color, Div, Element, FocusHandle, Icon, Length, Model, NamedKey, Positioning, Signal, Text,
    UseModel, component, deferred, div, icon, on_mounted, on_unmounted, provide, text, use_context, use_model, watch,
};

use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`dialog`] (and every modal on it) is given.
#[derive(Clone, PartialEq, Default)]
pub struct DialogProps {
    open: Bind<bool>,
}

/// What the parts below a [`dialog`] read: whether it is open.
#[derive(Clone, Copy)]
pub(crate) struct DialogContext {
    open: UseModel<PartProps<DialogProps>, bool>,
    pub(crate) focus: FocusReturn,
}

impl DialogContext {
    /// The nearest dialog's, from a part's setup.
    pub(crate) fn find() -> Option<Self> {
        use_context::<Self>()
    }

    /// Whether the dialog shows (tracked).
    pub(crate) fn is_open(self) -> bool {
        self.open.get()
    }

    pub(crate) fn set_open(self, open: bool) {
        self.open.set(open);
    }
}

/// A modal's root: give it a [`dialog_trigger`] and a [`dialog_content`].
/// Closed, and kept by itself, until told otherwise. Reka's root has no box
/// of its own; this one is a plain `div` its caller lays out (the content
/// floats on its own layer).
#[must_use]
pub fn dialog() -> Part<DialogProps> {
    modal_root(div().named("dialog"))
}

/// A modal root on `root` (dialog, alert dialog, sheet, drawer).
pub(crate) fn modal_root(root: Div) -> Part<DialogProps> {
    Part::new(DialogProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(DialogContext {
                open: use_model(props, |p| &p.props.open),
                focus: FocusReturn::new(),
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl Part<DialogProps> {
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

/// Make `trigger` open the dialog when pressed (`DialogTrigger as-child`).
/// Give `trigger` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
pub fn dialog_trigger(trigger: impl Into<Div>) -> Part<()> {
    Part::new((), trigger.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let dialog = DialogContext::find();
            // Reka's `triggerElement`: where focus returns when nothing
            // else is to have it.
            let handle = FocusHandle::new();
            if let Some(d) = dialog {
                on_mounted(move || d.focus.trigger.set(Some(handle.id())));
                on_unmounted(move || {
                    if d.focus.trigger.get_untracked() == Some(handle.id()) {
                        d.focus.trigger.set(None);
                    }
                });
            }
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match dialog {
                    Some(d) => root.track_focus(handle).aria_expanded(d.is_open()).on_click(move |_| {
                        if !d.open.get_untracked() {
                            d.focus.pressed(handle.id());
                        }
                        d.set_open(true);
                    }),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// Make `close` close the dialog when pressed (`DialogClose as-child`).
/// Give `close` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
pub fn dialog_close(close: impl Into<Div>) -> Part<()> {
    Part::new((), close.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let dialog = DialogContext::find();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match dialog {
                    Some(d) => root.on_click(move |_| d.set_open(false)),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// The page dimmer behind a modal: `fixed inset-0 z-50 bg-black/80`, fading
/// in and out with it.
#[must_use]
pub fn dialog_overlay() -> Div {
    crate::motion::fade(
        div()
            .named("dialog-overlay")
            .position(Position::Absolute)
            .inset(0.0)
            .background(Color::BLACK.opacity(0.8)),
    )
}

/// How a modal's content sits on its layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ModalLook {
    /// Centred over the overlay (dialog, alert dialog), or placed by the
    /// content itself (sheet, drawer).
    pub(crate) centred: bool,
    /// How long the overlay fades.
    pub(crate) overlay_ms: Option<f32>,
    /// A press on the overlay closes it (an alert dialog's does not).
    pub(crate) dismiss_outside: bool,
    /// The X in its top-right corner.
    pub(crate) close_button: bool,
}

/// What a modal's content part is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModalContentProps {
    pub(crate) look: ModalLook,
}

/// A modal's content on `root`: nothing while the dialog is closed; open, on
/// its own window-sized layer over the overlay, where Escape closes it.
/// While it is mounted it takes focus off the page when the dialog opens and
/// gives it back when it closes (Reka's `FocusScope`), whoever opened or
/// closed it.
pub(crate) fn modal_content(root: Div, look: ModalLook) -> Part<ModalContentProps> {
    Part::new(ModalContentProps { look }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let dialog = DialogContext::find();
            if let Some(d) = dialog {
                return_focus(move || d.is_open(), true, Some(d.focus));
            }
            move || {
                let Some(d) = dialog else {
                    return Element::Empty;
                };
                if !d.is_open() {
                    return Element::Empty;
                }
                props.with(|p| {
                    let content = p
                        .slot
                        .0
                        .clone()
                        .when(p.props.look.close_button, |c| c.child(corner_close(d)));
                    modal_layer(d, p.props.look, content)
                })
            }
        })
    })
}

/// Where a layer gives focus back when it closes: its trigger, as it is
/// mounted and as its press recorded it (Reka's `triggerElement` and
/// `recordTriggerPress`). Made in the root's setup.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct FocusReturn {
    /// The mounted trigger's focus.
    trigger: Signal<Option<FocusId>>,
    /// The trigger whose press is opening the layer, until it opens.
    pressed: Signal<Option<FocusId>>,
}

impl FocusReturn {
    pub(crate) fn new() -> Self {
        Self {
            trigger: Signal::new(None),
            pressed: Signal::new(None),
        }
    }

    /// Trigger `id` was pressed to open the layer: focus returns to it, even
    /// when the press did not focus it.
    pub(crate) fn pressed(self, id: FocusId) {
        self.pressed.set(Some(id));
    }
}

/// Called in a part's setup: when `open` turns true, remember where focus is
/// to return and (when `take` holds) take it off the page, so keys and Tab
/// go to the opened layer; when it turns false, give it back. Focus returns
/// as Reka's `getReturnFocusElement` has it: to the trigger whose press
/// opened the layer, else to what held focus as it opened, else to the
/// trigger still mounted; with none, nothing keeps focus (the page's body),
/// so focus is not left on a node the closed layer took away.
pub(crate) fn return_focus(open: impl Fn() -> bool + 'static, take: bool, trigger: Option<FocusReturn>) {
    /// Where focus goes back to, as the layer opened.
    struct Held {
        to: Option<FocusId>,
        /// `to` is the pressed trigger: skipped if it has gone since.
        trigger: bool,
        by_keyboard: bool,
    }
    let mut held: Option<Held> = None;
    watch(open, move |open, _| {
        if *open {
            let pressed = trigger.and_then(|t| {
                let id = t.pressed.get_untracked();
                if id.is_some() {
                    t.pressed.set(None);
                }
                id
            });
            if take {
                held = Some(Held {
                    to: pressed.or_else(focus::focused),
                    trigger: pressed.is_some(),
                    by_keyboard: focus::focus_visible(),
                });
                focus::set_focus(None, false);
            }
        } else if let Some(h) = held.take() {
            let mounted = trigger.and_then(|t| t.trigger.get_untracked());
            let to = h.to.filter(|id| !h.trigger || mounted == Some(*id)).or(mounted);
            focus::set_focus(to, h.by_keyboard && to.is_some());
        }
    });
}

/// A modal on its own window-sized layer: the overlay, then `content`,
/// centred or placed by itself. Escape closes it; so does a press on the
/// overlay when the look allows it.
pub(crate) fn modal_layer(dialog: DialogContext, look: ModalLook, content: Div) -> Element {
    let t = theme();
    let frame = if look.centred {
        div()
            .display(Display::Flex)
            .align_items(Align::Center)
            .justify_content(Justify::Center)
    } else {
        div()
    };
    let overlay = dialog_overlay().when_some(look.overlay_ms, Div::duration_ms);
    let root = frame
        .named("dialog-portal")
        .size(pct(100.0))
        .text_color(t.foreground)
        .on_key_down(move |e| {
            let escape = e.is(NamedKey::Escape);
            if escape {
                dialog.set_open(false);
            }
            escape
        })
        .child(overlay.when(look.dismiss_outside, |o| {
            o.on_mouse_down(move |_| dialog.set_open(false))
        }))
        .child(content);
    deferred(Positioning::Fill, root).into_element()
}

/// The close button in a modal's top-right corner: an X at 70% opacity.
fn corner_close(dialog: DialogContext) -> Div {
    let t = theme();
    let i = Interaction::new();
    let ring = Ring::new(2.0, t.ring);
    div()
        .named("dialog-close")
        .interaction(i)
        .position(Position::Absolute)
        .top(16.0)
        .right(16.0)
        .radius(2.0)
        .opacity(move || if i.hovered() || i.focused() { 1.0 } else { 0.7 })
        .transition(Properties::OPACITY)
        .cursor(Cursor::Pointer)
        .focusable()
        .ring(move || i.focused().then_some(ring))
        .aria_label("Close")
        .on_click(move |_| dialog.set_open(false))
        .child(icon(Icon::X))
}

/// The dialog itself, centred on a dimmed page with a close button in its
/// corner (`DialogContent`); style and fill it as a `Div`.
#[must_use]
pub fn dialog_content() -> Part<ModalContentProps> {
    modal_content(
        centred_surface().named("dialog-content").role(gui::Role::Dialog),
        ModalLook {
            centred: true,
            overlay_ms: None,
            dismiss_outside: true,
            close_button: true,
        },
    )
}

impl Part<ModalContentProps> {
    /// Without the X in its corner (`:show-close-button="false"`).
    #[must_use]
    pub fn close_button(self, shown: bool) -> Self {
        self.with_props(|p| p.look.close_button = shown)
    }
}

/// The box of a centred modal (dialog, alert dialog), fading and zooming in
/// and out over 200ms (`fade-in-0 zoom-in-95 duration-200` and their `-out`).
pub(crate) fn centred_surface() -> Div {
    let t = theme();
    div()
        .animation(Animation::enter().fade(0.0).zoom(0.95))
        .exit_animation(Animation::exit().fade(0.0).zoom(0.95))
        .duration_ms(200.0)
        .background(t.background)
        .display(Display::Grid)
        // `w-full max-w-[calc(100%-2rem)] sm:max-w-lg`: the window less
        // 1rem a side, at most 32rem (where the window is narrower than
        // `sm`, the calc is the smaller of the two, as in CSS).
        .width(Length::Calc {
            fraction: 1.0,
            px: -32.0,
        })
        .gap(16.0)
        .radius(10.0)
        .border(1.0)
        .border_color(t.border)
        .padding(24.0)
        .shadow(crate::popover::LIFTED_SHADOW)
        .max_width(512.0)
}

#[must_use]
pub fn dialog_header() -> Div {
    div()
        .named("dialog-header")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .text_align(TextAlign::Left)
}

#[must_use]
pub fn dialog_footer() -> Div {
    div()
        .named("dialog-footer")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Row)
        .gap(8.0)
        .justify_content(Justify::End)
}

#[must_use]
pub fn dialog_title(title: impl Into<String>) -> Text {
    text(title)
        .named("dialog-title")
        .text_size(18.0)
        .line_height(LineHeight::Relative(1.0))
        .font_weight(600)
}

#[must_use]
pub fn dialog_description(description: impl Into<String>) -> Text {
    let t = theme();
    text(description)
        .named("dialog-description")
        .text_color(t.muted_foreground)
        .text_size(14.0)
        .line_height(20.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui::{Modifiers, Scenario};

    /// A page with a focusable box, `before`, and a dialog opened by `open`,
    /// with a trigger when `trigger` holds; its content focuses `inner`.
    fn page(open: Signal<bool>, before: FocusHandle, inner: FocusHandle, trigger: Signal<bool>) -> Scenario {
        let mut s =
            Scenario::measuring(
                move || {
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .child(div().named("before").width(40.0).height(20.0).track_focus(before))
                        .child(
                            dialog()
                                .open(open)
                                .child(trigger.get().then(|| {
                                    dialog_trigger(div().named("trigger").width(80.0).height(20.0).child("Open"))
                                }))
                                .child(
                                    dialog_content()
                                        .child(div().named("inner").width(40.0).height(20.0).track_focus(inner)),
                                ),
                        )
                        .into_element()
                },
                400,
                300,
                1.0,
            )
            .expect("a scenario");
        s.settle().expect("settled");
        s
    }

    /// Open, focus `inner` inside the dialog, then close by writing the model.
    fn open_and_close(s: &mut Scenario, open: Signal<bool>, inner: FocusHandle) {
        s.act("open", || open.set(true));
        s.settle().expect("settled");
        s.act("focus inside", || inner.focus());
        s.settle().expect("settled");
        assert!(inner.is_focused(), "the content holds focus while open");
        s.act("close", || open.set(false));
        s.settle().expect("settled");
    }

    #[test]
    fn with_no_trigger_focus_returns_to_what_held_it_as_it_opened() {
        let (open, trigger) = (Signal::new(false), Signal::new(false));
        let (before, inner) = (FocusHandle::new(), FocusHandle::new());
        let mut s = page(open, before, inner, trigger);
        s.act("focus before", || before.focus());
        s.settle().expect("settled");
        open_and_close(&mut s, open, inner);
        assert!(before.is_focused(), "focus is back where it was");
    }

    #[test]
    fn with_nothing_to_return_to_no_node_keeps_focus() {
        // The docs' ⌘K palette: no trigger, nothing focused as it opens.
        let (open, trigger) = (Signal::new(false), Signal::new(false));
        let (before, inner) = (FocusHandle::new(), FocusHandle::new());
        let mut s = page(open, before, inner, trigger);
        open_and_close(&mut s, open, inner);
        assert_eq!(focus::focused(), None, "focus is not left on the closed content");
    }

    #[test]
    fn a_trigger_press_returns_focus_to_the_trigger() {
        let (open, trigger) = (Signal::new(false), Signal::new(true));
        let (before, inner) = (FocusHandle::new(), FocusHandle::new());
        let mut s = page(open, before, inner, trigger);
        s.act("focus before", || before.focus());
        s.settle().expect("settled");
        s.click_text("Open").expect("the trigger");
        s.settle().expect("settled");
        assert!(open.get_untracked(), "the press opened it");
        s.act("focus inside", || inner.focus());
        s.act("close", || open.set(false));
        s.settle().expect("settled");
        let back = focus::focused();
        assert!(
            back.is_some() && !before.is_focused() && !inner.is_focused(),
            "focus is on the trigger"
        );
        // The trigger holds it: Enter presses it and opens the dialog again.
        s.press(NamedKey::Enter, Modifiers::default());
        s.settle().expect("settled");
        assert!(open.get_untracked(), "Enter on the focused trigger opens it");
    }

    #[test]
    fn opened_by_the_caller_focus_returns_to_what_held_it_before_the_trigger() {
        let (open, trigger) = (Signal::new(false), Signal::new(true));
        let (before, inner) = (FocusHandle::new(), FocusHandle::new());
        let mut s = page(open, before, inner, trigger);
        s.act("focus before", || before.focus());
        s.settle().expect("settled");
        open_and_close(&mut s, open, inner);
        assert!(before.is_focused(), "what held focus wins over the trigger");
    }

    #[test]
    fn a_trigger_gone_since_its_press_is_skipped() {
        let (open, trigger) = (Signal::new(false), Signal::new(true));
        let (before, inner) = (FocusHandle::new(), FocusHandle::new());
        let mut s = page(open, before, inner, trigger);
        s.click_text("Open").expect("the trigger");
        s.settle().expect("settled");
        s.act("focus inside", || inner.focus());
        s.act("drop the trigger", || trigger.set(false));
        s.settle().expect("settled");
        s.act("close", || open.set(false));
        s.settle().expect("settled");
        assert_eq!(focus::focused(), None, "nothing keeps focus");
    }
}
