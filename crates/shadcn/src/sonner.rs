//! Sonner (`ui/sonner`): toasts, as vue-sonner draws them with shadcn's
//! colours. As there, [`toaster`] is placed once (`<Toaster />`) and holds
//! the toasts, rendering them in the window's bottom-right corner, the newest
//! in front and up to two peeking behind it, spread out while the pointer is
//! over them; [`toast()`] and the functions of [`toast`](mod@toast) raise them from
//! anywhere (`toast('…')`, `toast.success('…')`), going to the toaster
//! mounted last. A toast rises in as it comes, sinks out as it goes, and
//! slides between front and behind.
//!
//! Each toast leaves after 4 seconds (a loading toast stays until it is
//! updated); the countdowns pause together while the pointer is over the
//! stack (which spreads it out), while focus is in it, or while a toast is
//! pressed, or while the window is not focused, and resume from the time
//! they had left. Escape on a toast, or a swipe right, takes it away;
//! Alt+T focuses the stack (vue-sonner's hotkey). The store and its countdowns are `primitives::toast`'s
//! (`Toaster`): unmounted, the toaster cancels them. This module draws them.

#[expect(
    clippy::disallowed_types,
    reason = "the toasters mounted, added and taken out only as each mounts and unmounts (`gui::while_mounted`)"
)]
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gui::events::{Key, Modifiers};
use gui::prelude::*;
use gui::style::Calc;
use gui::{
    Animation, Color, Div, Easing, Element, Icon, Positioning, Properties, Shadow, component, deferred, div, icon, text,
};
use primitives::toast::{
    Announcement, Hotkey, Lifetime, Sensitivity, ShowOptions, Shown, Spoken, ToastStatus, Toaster, ToasterOptions,
};

pub use primitives::toast::ToastId;

use crate::spinner::spinner;
use crate::theme::theme;

/// vue-sonner's `TOAST_LIFETIME`.
const LIFETIME: Duration = Duration::from_millis(4000);
/// vue-sonner's `TOAST_WIDTH`, in pixels.
const WIDTH: f32 = 356.0;
/// vue-sonner's `GAP`: between spread-out toasts, and how far each one
/// behind the front peeks out above it.
const GAP: f32 = 14.0;
/// vue-sonner's `VISIBLE_TOASTS_AMOUNT`.
const VISIBLE: usize = 3;
/// vue-sonner's `VIEWPORT_OFFSET`, in pixels.
const OFFSET: f32 = 24.0;
/// How much narrower each toast behind the front is (`scale(1 - n * 0.05)`).
const SHRINK: f32 = 0.05;

/// Which icon a toast carries (`toast()`, `toast.success()`, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToastKind {
    #[default]
    Default,
    Success,
    Info,
    Warning,
    Error,
    /// A spinner; the toast stays until it is updated.
    Loading,
}

/// A button on a toast that runs `on_click` and then dismisses the toast;
/// its label, never blank, is what it shows and what it is read as.
#[derive(Clone)]
pub struct ToastAction {
    pub label: Spoken,
    pub on_click: gui::Callback<()>,
}

/// What a toast says under its title: vue-sonner's `description`, a
/// string or a component.
#[derive(Clone)]
pub enum ToastDescription {
    Text(String),
    /// Drawn each frame the toast shows.
    View(Rc<dyn Fn() -> Element>),
}

/// A toast's own styles over the toaster's: vue-sonner's per-toast `class`
/// and `style`.
pub type ToastClass = Rc<dyn Fn(Div) -> Div>;

/// What a toast says.
#[derive(Clone)]
pub struct Toast {
    pub kind: ToastKind,
    pub title: String,
    pub description: Option<ToastDescription>,
    pub action: Option<ToastAction>,
    pub class: Option<ToastClass>,
}

impl Toast {
    /// A plain toast saying `title`.
    #[must_use]
    pub fn new(kind: ToastKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            description: None,
            action: None,
            class: None,
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(ToastDescription::Text(description.into()));
        self
    }

    /// A description drawn by `view` (vue-sonner's `description: h(...)`).
    #[must_use]
    pub fn description_view(mut self, view: impl Fn() -> Element + 'static) -> Self {
        self.description = Some(ToastDescription::View(Rc::new(view)));
        self
    }

    /// Style this toast after the toaster's (`class: 'flex flex-col gap-2'`).
    #[must_use]
    pub fn class(mut self, class: impl Fn(Div) -> Div + 'static) -> Self {
        self.class = Some(Rc::new(class));
        self
    }

    /// A button labelled `label` that runs `on_click`, then dismisses the
    /// toast. A blank label is refused: the toast gets no action, and a
    /// debug build reports where it was asked for (reka throws on an
    /// action with no `altText`; a button that shows nothing and is read as
    /// nothing is no action).
    #[must_use]
    #[track_caller]
    pub fn action(mut self, label: impl Into<String>, on_click: impl Into<gui::Callback<()>>) -> Self {
        let label = label.into();
        let on_click = on_click.into();
        match Spoken::new(label) {
            Some(label) => {
                self.action = Some(ToastAction { label, on_click });
            }
            None => {
                if cfg!(debug_assertions) {
                    eprintln!(
                        "shadcn: a toast action asked for at {} has a blank label: it is refused, its handler not attached.",
                        std::panic::Location::caller()
                    );
                }
            }
        }
        self
    }
}

impl Toast {
    /// How the toaster shows it: read politely (vue-sonner's `<section
    /// aria-live="polite">`), its title, its description's text and its
    /// action; a loading one stays.
    fn show_options(&self) -> ShowOptions {
        let mut announcement = Announcement::new().text(self.title.clone());
        if let Some(ToastDescription::Text(d)) = &self.description {
            announcement = announcement.text(d.clone());
        }
        if let Some(a) = &self.action {
            announcement = announcement.action(&a.label);
        }
        ShowOptions {
            sensitivity: Sensitivity::Background,
            lifetime: Lifetime::Toaster,
            status: match self.kind {
                ToastKind::Loading => ToastStatus::Loading,
                ToastKind::Success => ToastStatus::Success,
                ToastKind::Error => ToastStatus::Error,
                ToastKind::Default | ToastKind::Info | ToastKind::Warning => ToastStatus::Plain,
            },
            anchor: None,
            announcement,
        }
    }
}

impl From<&str> for Toast {
    fn from(title: &str) -> Self {
        Self::new(ToastKind::Default, title)
    }
}

impl From<String> for Toast {
    fn from(title: String) -> Self {
        Self::new(ToastKind::Default, title)
    }
}

gui::engine_local! {
    /// The toasters mounted, by the order they mounted in: [`toast()`] goes
    /// to the last.
    #[expect(clippy::disallowed_types, reason = "the toasters mounted, added and taken out only as each mounts and unmounts (`gui::while_mounted`)")]
    /// Each by handle (ADR-0034 S7): a store is engine handles alone, so
    /// nothing changes through the handle; a check's insert or forget is
    /// its own.
    [Fork(log)] static TOASTERS: RefCell<gui::engine::ForkMap<u64, gui::engine::AppHandle<Store>>> = RefCell::default();
}

/// The toasters' places in that order, increasing on a thread.
static MOUNTED: gui::engine::IdKind = gui::engine::IdKind::new("sonner toaster");
/// The toasts' ids.
static NEXT: gui::engine::IdKind = gui::engine::IdKind::new("sonner toast");

/// The toasts a [`toaster`] holds, newest first. Made in its setup.
#[derive(Clone, Copy, PartialEq)]
struct Store {
    toasts: Toaster<Toast>,
}

impl Store {
    fn last() -> Option<Self> {
        TOASTERS.with(|t| t.borrow().iter().max_by_key(|(n, _)| **n).map(|(_, s)| **s))
    }

    /// Show `toast` as `id`, in front of the others, or in place of the
    /// toast `id` it updates, its countdown started again.
    fn show(self, id: ToastId, toast: Toast) {
        let options = toast.show_options();
        self.toasts.show(id, toast, options);
    }

    /// Replace what toast `id` says, if it is shown.
    fn update(self, id: ToastId, toast: &Toast) {
        if self.toasts.with(|all| all.iter().any(|s| s.id == id)) {
            self.show(id, toast.clone());
        }
    }

    fn dismiss(self, id: ToastId) {
        self.toasts.dismiss(id);
    }
}

/// Show `toast` (a title, or a [`Toast`]) on the toaster mounted last
/// (vue-sonner's `toast('…')`); with none mounted it shows nowhere.
pub fn toast(toast: impl Into<Toast>) -> ToastId {
    let id = fresh_id();
    if let Some(store) = Store::last() {
        store.show(id, toast.into());
    }
    id
}

/// vue-sonner's `toast.success(…)` and its siblings, `toast.dismiss(id)`,
/// and updating a toast in place.
pub mod toast {
    use super::{Store, Toast, ToastId, ToastKind};

    /// `toast.success(title)`.
    pub fn success(title: impl Into<String>) -> ToastId {
        super::toast(Toast::new(ToastKind::Success, title))
    }

    /// `toast.info(title)`.
    pub fn info(title: impl Into<String>) -> ToastId {
        super::toast(Toast::new(ToastKind::Info, title))
    }

    /// `toast.warning(title)`.
    pub fn warning(title: impl Into<String>) -> ToastId {
        super::toast(Toast::new(ToastKind::Warning, title))
    }

    /// `toast.error(title)`.
    pub fn error(title: impl Into<String>) -> ToastId {
        super::toast(Toast::new(ToastKind::Error, title))
    }

    /// `toast.loading(title)`: stays until [`update`] or [`dismiss`].
    pub fn loading(title: impl Into<String>) -> ToastId {
        super::toast(Toast::new(ToastKind::Loading, title))
    }

    /// Replace what toast `id` says, in place, and restart its countdown
    /// (`toast.success(…, { id })`, how `toast.promise` turns a loading
    /// toast into its result).
    pub fn update(id: ToastId, toast: impl Into<Toast>) {
        let toast = toast.into();
        if let Some(store) = Store::last() {
            store.update(id, &toast);
        }
    }

    /// Take toast `id` off the screen (`toast.dismiss(id)`).
    pub fn dismiss(id: ToastId) {
        if let Some(store) = Store::last() {
            store.dismiss(id);
        }
    }

    /// `toast.promise(promise, { loading, success, error })`: a loading
    /// toast saying `loading` that the result sent through the returned
    /// sender, from any thread, turns into what `success` or `error` make
    /// of it, in place. With no toaster mounted it shows nowhere and there
    /// is no sender.
    pub fn promise<V: Send + 'static, E: Send + 'static>(
        loading: impl Into<String>,
        success: impl FnOnce(V) -> Toast + 'static,
        error: impl FnOnce(E) -> Toast + 'static,
    ) -> (ToastId, Option<gui::Sender<Result<V, E>>>) {
        let id = super::fresh_id();
        let sender = Store::last().map(|store| {
            let loading = Toast::new(ToastKind::Loading, loading);
            let options = loading.show_options();
            store.toasts.promise(id, (loading, options), move |result| {
                let toast = match result {
                    Ok(v) => success(v),
                    Err(e) => error(e),
                };
                let options = toast.show_options();
                (toast, options)
            })
        });
        (id, sender)
    }
}

/// The next toast's id.
fn fresh_id() -> ToastId {
    ToastId(gui::engine::unique_id(&NEXT))
}

/// The toaster (`<Toaster />`): the toasts over the window's bottom-right
/// corner, nothing while there are none. Place it once, last in the window's
/// tree; [`toast()`] goes to the one mounted last.
#[must_use]
pub fn toaster() -> Element {
    component((), |_| {
        let store = Store {
            toasts: Toaster::new(ToasterOptions {
                duration: Some(LIFETIME),
                hotkey: Hotkey {
                    key: Key::Character("t".into()),
                    modifiers: Modifiers {
                        alt: true,
                        ..Modifiers::default()
                    },
                },
                ..ToasterOptions::default()
            }),
        };
        // In the toasters while mounted: a setup never pushes into the
        // shared list (one that never mounts, a cold re-render's, leaves it).
        gui::while_mounted(move || {
            let n = gui::engine::unique_id(&MOUNTED);
            TOASTERS.with(|t| {
                t.borrow_mut()
                    .replace(n, gui::engine::AppHandle::new(std::rc::Rc::new(store)))
            });
            move || {
                TOASTERS.try_with(|t| t.borrow_mut().forget(&n)).ok();
            }
        });
        move || stack(store)
    })
    .named("toaster")
    .into_element()
}

/// What `store` shows.
fn stack(store: Store) -> Element {
    store.toasts.with(|shown| {
        if shown.is_empty() {
            return Element::Empty;
        }
        // Spread out while the pointer is over the stack.
        let expanded = store.toasts.region_interaction().hovered();
        let visible: Vec<&Shown<Toast>> = shown.iter().take(VISIBLE).collect();
        let stack = store.toasts.region(
            div().named("toaster").position(Position::Relative).width(WIDTH),
            // vue-sonner's `aria-label`: its label, then its hotkey (Alt+T).
            "Notifications alt+T",
        );
        let stack = if expanded {
            // Oldest on top, the newest at the bottom edge.
            stack
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(GAP)
                .children(visible.iter().rev().map(|s| toast_box(store, s, None)))
        } else {
            // All in one cell, the ones behind first so the front one paints
            // over them; a toast leaving shares the cell with the one taking its place.
            let mut behind = 0.0_f32;
            let toasts: Vec<Div> = visible
                .iter()
                .map(|s| {
                    let n = behind;
                    behind += 1.0;
                    toast_box(store, s, (n > 0.0).then_some(n))
                })
                .collect();
            stack
                .display(Display::Grid)
                .align_items(Align::End)
                .children(toasts.into_iter().rev())
        };
        let corner = div()
            .named("toaster-viewport")
            .size(pct(100.0))
            .pass_through()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .justify_content(Justify::End)
            .align_items(Align::End)
            .padding(OFFSET)
            .child(stack);
        deferred(Positioning::Fill, corner).into_element()
    })
}

/// The surface every toast has.
fn surface() -> Div {
    let t = theme();
    div()
        .background(t.popover)
        .text_color(t.popover_foreground)
        .border(1.0)
        .border_color(t.border)
        .radius(10.0)
        .shadow(Shadow {
            x: 0.0,
            y: 4.0,
            blur: 12.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.1),
        })
}

/// vue-sonner's description colour, which shadcn does not override.
fn description_color(dark: bool) -> Color {
    if dark {
        Color::hex(0x00e8_e8e8)
    } else {
        Color::hex(0x003f_3f3f)
    }
}

/// The toast `s`; `behind` is how many toasts are in front of it while the
/// stack is gathered: then it takes the front one's height, lifted by that
/// many gaps and narrowed, its content faded out.
fn toast_box(store: Store, s: &Shown<Toast>, behind: Option<f32>) -> Div {
    let t = theme();
    let kind_icon = match s.content.kind {
        ToastKind::Default => None,
        ToastKind::Success => Some(icon(Icon::CircleCheck)),
        ToastKind::Info => Some(icon(Icon::Info)),
        ToastKind::Warning => Some(icon(Icon::TriangleAlert)),
        ToastKind::Error => Some(icon(Icon::OctagonX)),
        ToastKind::Loading => Some(spinner()),
    };
    let content = div()
        .named("toast-content")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .row_gap(2.0)
        .child(
            text(s.content.title.clone())
                .named("toast-title")
                .font_weight(500)
                .line_height(LineHeight::Relative(1.5)),
        )
        .child(s.content.description.clone().map(|d| {
            let d = match d {
                ToastDescription::Text(d) => text(d).into_element(),
                ToastDescription::View(view) => view(),
            };
            div()
                .named("toast-description")
                .font_weight(400)
                .line_height(13.0 * 1.4)
                .text_color(description_color(t.dark))
                .child(d)
        }));
    let action = s.content.action.clone().map(|a| {
        let id = s.id;
        let button = div()
            .named("toast-action")
            .display(Display::Flex)
            .align_items(Align::Center)
            .flex_shrink(0.0)
            .margin_left(Length::Auto)
            .height(24.0)
            .padding_x(8.0)
            .radius(4.0)
            .text_size(12.0)
            .line_height(16.0)
            .font_weight(500)
            .background(t.popover_foreground)
            .text_color(t.popover)
            .cursor(Cursor::Pointer)
            .with_interaction(|d, i| {
                d.ring(move || i.focus_visible().then(|| Ring::new(2.0, Color::BLACK.opacity(0.4))))
            });
        // vue-sonner reads the action by its label.
        let label = a.label.as_str().to_owned();
        store
            .toasts
            .action(button, id, &a.label, a.on_click.clone())
            .child(label)
    });
    // vue-sonner's `[data-sonner-toast] > * { transition: opacity 400ms }`.
    let part = |d: Div| {
        d.transition(Properties::OPACITY)
            .duration_ms(400.0)
            .ease(Easing::EASE)
            .when(behind.is_some(), |d| d.opacity(0.0))
    };
    let (kind_icon, content, action) = (
        kind_icon.map(|i| {
            part(
                div()
                    .named("toast-icon")
                    .display(Display::Flex)
                    .align_items(Align::Center)
                    .size(16.0)
                    .flex_shrink(0.0)
                    .margin_left(-3.0)
                    .margin_right(4.0)
                    .child(i),
            )
        }),
        part(content),
        action.map(part),
    );
    let surface = store.toasts.toast(surface(), s.id);
    let (toasts, id) = (store.toasts, s.id);
    surface
        .named("toast")
        .key(s.id.0)
        // Swiped, it follows the pointer, with no transition while it does
        // (vue-sonner's `--swipe-amount`, `[data-swiping=true]`).
        .translate_x(move || Calc::px(toasts.swipe_offset(id).map_or(0.0, |o| o.0)))
        // Mounting rises from below and fades in; leaving sinks and fades
        // for the 200ms before vue-sonner unmounts it (`TIME_BEFORE_UNMOUNT`),
        // half its 400ms transition.
        .transition(Properties::TRANSFORM | Properties::OPACITY)
        .duration_ms(move || if toasts.swipe_offset(id).is_some() { 0.0 } else { 400.0 })
        .ease(Easing::EASE)
        .animation(Animation::enter().slide(Slide::Y(Calc::fraction(1.0))).fade(0.0))
        .exit_animation(
            Animation::exit()
                .slide(Slide::Y(Calc::fraction(1.0)))
                .fade(0.0)
                .iterations(0.5),
        )
        .grid_row_start(1)
        .grid_column_start(1)
        .when_some(behind, |d, n| {
            d.position(Position::Absolute)
                .top(0.0)
                .bottom(0.0)
                .left(0.0)
                .right(0.0)
                .overflow(Overflow::Hidden)
                .translate_y(Calc::px(-GAP * n))
                .scale(1.0 - SHRINK * n)
        })
        .width(WIDTH)
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(6.0)
        .padding(16.0)
        .text_size(13.0)
        .line_height(13.0 * 1.5)
        .child(kind_icon)
        .child(content)
        .child(action)
        .when_some(s.content.class.clone(), |d, class| class(d))
}
