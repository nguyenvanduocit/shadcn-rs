//! Dialog (`ui/dialog`): a modal window over a dimmed page, centred, with a
//! header, footer and a close button in its corner.
//!
//! The parts nest as in Vue: [`dialog`] keeps whether it is open, its own
//! (`default_open`) or the caller's (`open`, `v-model:open`); the
//! [`dialog_trigger`], [`dialog_content`], [`dialog_title`],
//! [`dialog_description`] and each [`dialog_close`] inside read it. Every
//! other modal on Reka's `DialogRoot` (sheet) is a [`dialog`] too; an alert
//! dialog and a drawer have roots of their own, and the trigger, close,
//! title and description parts work under each.
//!
//! The behaviour is `primitives::dialog`'s: modal by default (focus trapped
//! and cycling, the page below inert and hidden from screen readers),
//! Escape and a press on the overlay close it (a context menu's press does
//! not), opening focuses its first Tab stop, closing gives focus back to
//! what held it as it opened, else to the trigger. The content is read as a
//! dialog named by its title and described by its description. This module
//! draws it.
//!
//! ```ignore
//! dialog()
//!     .child(dialog_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child("Open")))
//!     .child(dialog_content().max_width(425.0).child(dialog_header().child(dialog_title("Edit profile"))))
//! ```

use gui::prelude::*;
use gui::{
    Animation, Bind, Color, Div, Element, Icon, Length, Model, Positioning, Slot, Text, component, deferred, div, icon,
    provide, text, use_model,
};
use primitives::alert_dialog::AlertDialog;
use primitives::context::use_root;
use primitives::dialog::{Dialog, DialogOptions};
use primitives::drawer::Drawer;
use primitives::presence::Presence;

pub use primitives::dialog::Veto;

use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`dialog`] (and a sheet) is given.
#[derive(Clone, PartialEq)]
pub struct DialogProps {
    open: Bind<bool>,
    modal: bool,
    veto: Option<Veto>,
}

impl Default for DialogProps {
    fn default() -> Self {
        Self {
            open: Bind::default(),
            modal: true,
            veto: None,
        }
    }
}

/// The dialog of a [`dialog`] root.
pub(crate) type DialogState = Dialog<PartProps<DialogProps>>;

/// The modal a trigger, close, title or description part sits in: the
/// state of a dialog (or sheet), an alert dialog, or a drawer root.
#[derive(Clone, Copy)]
#[expect(
    clippy::large_enum_variant,
    reason = "a Copy handle a part reads in its setup; boxing the drawer's would lose Copy for no gain"
)]
pub(crate) enum Modal {
    Dialog(DialogState),
    Alert(AlertDialog<PartProps<crate::alert_dialog::AlertDialogProps>>),
    Drawer(Drawer<PartProps<crate::drawer::DrawerProps>>),
}

impl Modal {
    /// The nearest modal root's, from a part's setup; outside one it is a
    /// reported error naming `part`.
    #[track_caller]
    pub(crate) fn find(part: &'static str) -> Option<Self> {
        use_root::<Self>(part, "dialog")
    }

    fn trigger(self, d: Div) -> Div {
        match self {
            Self::Dialog(m) => m.trigger(d),
            Self::Alert(m) => m.trigger(d),
            Self::Drawer(m) => m.trigger(d),
        }
    }

    fn close(self, d: Div) -> Div {
        match self {
            Self::Dialog(m) => m.close(d),
            Self::Alert(m) => m.dialog().close(d),
            Self::Drawer(m) => m.close(d),
        }
    }

    fn title(self, t: Text) -> Text {
        match self {
            Self::Dialog(m) => m.title(t),
            Self::Alert(m) => m.title(t),
            Self::Drawer(m) => m.title(t),
        }
    }

    fn description(self, t: Text) -> Text {
        match self {
            Self::Dialog(m) => m.description(t),
            Self::Alert(m) => m.description(t),
            Self::Drawer(m) => m.description(t),
        }
    }

    /// From a title's or description's setup: the content has one while
    /// it is mounted (what the primitive's missing-title check reads).
    fn register(self, names: Names) {
        match (self, names) {
            (Self::Dialog(m), Names::Title) => m.register_title(),
            (Self::Dialog(m), Names::Description) => m.register_description(),
            (Self::Alert(m), Names::Title) => m.register_title(),
            (Self::Alert(m), Names::Description) => m.register_description(),
            (Self::Drawer(m), Names::Title) => m.register_title(),
            (Self::Drawer(m), Names::Description) => m.register_description(),
        }
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

/// A dialog root on `root` (dialog, sheet).
pub(crate) fn modal_root(root: Div) -> Part<DialogProps> {
    Part::new(DialogProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let d = Dialog::new(
                use_model(props, |p: &PartProps<DialogProps>| &p.props.open),
                move || {
                    props.with(|p| DialogOptions {
                        modal: p.props.modal,
                        veto: p.props.veto.clone(),
                        ..DialogOptions::default()
                    })
                },
            );
            provide(d);
            provide(Modal::Dialog(d));
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
    pub fn on_open_change(self, f: impl Into<gui::Callback<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().on_change(f))
    }

    /// Modal or not (`:modal`, on by default): not modal, focus is not
    /// trapped and the page stays live, and focus going outside closes it.
    #[must_use]
    pub fn modal(self, modal: bool) -> Self {
        self.with_props(|p| p.modal = modal)
    }

    /// Answer each close a user asks for (Escape, a press or focus
    /// outside): reka's `escapeKeyDown`, `pointerDownOutside`,
    /// `focusOutside` and `interactOutside` with `preventDefault`.
    #[must_use]
    pub fn on_dismiss(self, f: impl Into<gui::Callback<gui::DismissReason, gui::Dismiss>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.veto = Some(f))
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
            let modal = Modal::find("dialog_trigger");
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match modal {
                    Some(m) => m.trigger(root),
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
            let modal = Modal::find("dialog_close");
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match modal {
                    Some(m) => m.close(root),
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
    /// The X in its top-right corner.
    pub(crate) close_button: bool,
}

/// What a modal's content part is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModalContentProps {
    pub(crate) look: ModalLook,
}

/// A dialog's content on `root`: nothing while the dialog is closed; open,
/// on its own window-sized layer over the overlay (see [`modal_layer`]).
pub(crate) fn modal_content(root: Div, look: ModalLook) -> Part<ModalContentProps> {
    Part::new(ModalContentProps { look }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let dialog = use_root::<DialogState>("dialog_content", "dialog");
            move || {
                let Some(d) = dialog else {
                    return Element::Empty;
                };
                let presence = d.presence();
                if presence == Presence::Gone {
                    return Element::Empty;
                }
                props.with(|p| {
                    let content = p
                        .slot
                        .0
                        .clone()
                        .when(p.props.look.close_button, |c| c.child(corner_close(d)));
                    modal_layer(presence, d.has_overlay(), p.props.look, d.content(content))
                })
            }
        })
    })
}

/// A modal on its own window-sized layer: the overlay when it has one, then
/// `content` (already its primitive's part), centred or placed by itself,
/// as `presence` makes it.
pub(crate) fn modal_layer(presence: Presence, overlay: bool, look: ModalLook, content: impl IntoElement) -> Element {
    modal_layer_with(presence, overlay, look, content, |f| f, |o| o)
}

/// [`modal_layer`], its window-sized `frame` and its `overlay` given to the
/// caller first (a drawer measures the frame and fades the overlay with
/// its swipe).
pub(crate) fn modal_layer_with(
    presence: Presence,
    overlay: bool,
    look: ModalLook,
    content: impl IntoElement,
    frame_part: impl FnOnce(Div) -> Div,
    overlay_part: impl FnOnce(Div) -> Div,
) -> Element {
    let t = theme();
    let frame = if look.centred {
        div()
            .display(Display::Flex)
            .align_items(Align::Center)
            .justify_content(Justify::Center)
    } else {
        div()
    };
    let root = frame_part(frame.named("dialog-portal"))
        .size(pct(100.0))
        .text_color(t.foreground)
        .when(overlay, |f| {
            f.child(overlay_part(
                dialog_overlay().when_some(look.overlay_ms, Div::duration_ms),
            ))
        })
        .child(content);
    deferred(Positioning::Fill, presence.render(root)).into_element()
}

/// The close button in a modal's top-right corner: an X at 70% opacity.
fn corner_close(dialog: DialogState) -> Div {
    let t = theme();
    let i = Interaction::new();
    let ring = Ring::new(2.0, t.ring);
    dialog.close(
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
            .child(icon(Icon::X)),
    )
}

/// The dialog itself, centred on a dimmed page with a close button in its
/// corner (`DialogContent`); style and fill it as a `Div`.
#[must_use]
pub fn dialog_content() -> Part<ModalContentProps> {
    modal_content(
        centred_surface().named("dialog-content"),
        ModalLook {
            centred: true,
            overlay_ms: None,
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

/// Which of a modal's texts a [`ModalText`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Names {
    Title,
    Description,
}

/// A modal's title or description being built: a [`Text`] its caller
/// styles, which becomes its part as it goes into a tree, naming or
/// describing the modal it sits in (`DialogTitle`, `DialogDescription`).
#[must_use]
pub struct ModalText {
    text: Text,
    names: Names,
}

impl ModalText {
    fn new(text: Text, names: Names) -> Self {
        Self { text, names }
    }

    /// The name the text shows in the debug tools and the verdict.
    pub fn named(mut self, name: &'static str) -> Self {
        self.text = self.text.named(name);
        self
    }
}

impl Fluent for ModalText {}

impl Styled for ModalText {
    fn style_and_bindings(&mut self) -> (&mut gui::Style, Option<&mut gui::value::Bindings>) {
        self.text.style_and_bindings()
    }

    fn node_interaction(&self) -> Option<gui::Interaction> {
        self.text.node_interaction()
    }
}

/// What a [`ModalText`]'s instance is given.
#[derive(Clone, PartialEq)]
struct ModalTextProps {
    names: Names,
    text: Slot<Text>,
}

impl IntoElement for ModalText {
    fn into_element(self) -> Element {
        let props = ModalTextProps {
            names: self.names,
            text: Slot(self.text),
        };
        component(props, |cx| {
            let props = cx.props();
            let part = match props.with_untracked(|p| p.names) {
                Names::Title => "dialog_title",
                Names::Description => "dialog_description",
            };
            let modal = Modal::find(part);
            if let Some(m) = modal {
                m.register(props.with_untracked(|p| p.names));
            }
            move || {
                let (names, t) = props.with(|p| (p.names, p.text.0.clone()));
                match (modal, names) {
                    (Some(m), Names::Title) => m.title(t),
                    (Some(m), Names::Description) => m.description(t),
                    (None, _) => t,
                }
                .into_element()
            }
        })
        .into_element()
    }
}

/// The modal's title, which names it to a screen reader (`DialogTitle`).
pub fn dialog_title(title: impl Into<String>) -> ModalText {
    ModalText::new(
        text(title)
            .named("dialog-title")
            .text_size(18.0)
            .line_height(LineHeight::Relative(1.0))
            .font_weight(600),
        Names::Title,
    )
}

/// The modal's description, which describes it to a screen reader
/// (`DialogDescription`).
pub fn dialog_description(description: impl Into<String>) -> ModalText {
    let t = theme();
    ModalText::new(
        text(description)
            .named("dialog-description")
            .text_color(t.muted_foreground)
            .text_size(14.0)
            .line_height(20.0),
        Names::Description,
    )
}

/// A title for a modal of another kind, from its `text`.
pub(crate) fn modal_title(text: Text) -> ModalText {
    ModalText::new(text, Names::Title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui::{FocusHandle, Modifiers, NamedKey, Scenario, Signal, focus};

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
