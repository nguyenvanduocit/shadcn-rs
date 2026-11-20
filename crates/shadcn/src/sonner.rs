//! Sonner (`ui/sonner`): toasts, as vue-sonner draws them with shadcn's
//! colours. A [`Toaster`] holds the toasts; [`toaster`] renders them in the
//! window's bottom-right corner, the newest in front and up to two peeking
//! behind it, spread out while the pointer is over them. A toast rises in as
//! it comes, sinks out as it goes, and slides between front and behind.
//!
//! Each toast leaves after 4 seconds (a loading toast stays until it is
//! updated); the countdown pauses while the stack is spread out.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gui::clock::{self, TimerId};
use gui::prelude::*;
use gui::style::Calc;
use gui::{
    Animation, Color, Div, Easing, Element, Icon, Placement, Properties, Shadow, Signal, deferred, div, icon, text,
};

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
/// vue-sonner's `VIEWPORT_OFFSET` (24px), on Tailwind's scale.
const OFFSET: f32 = 6.0;
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

/// A button on a toast that runs `on_click` and then dismisses the toast.
#[derive(Clone)]
pub struct ToastAction {
    pub label: String,
    pub on_click: Rc<dyn Fn()>,
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

    #[must_use]
    pub fn action(mut self, label: impl Into<String>, on_click: impl Fn() + 'static) -> Self {
        self.action = Some(ToastAction {
            label: label.into(),
            on_click: Rc::new(on_click),
        });
        self
    }
}

/// Identifies a toast shown by a [`Toaster`], to update or dismiss it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastId(u64);

/// A shown toast and its countdown.
#[derive(Clone)]
struct Shown {
    id: ToastId,
    toast: Toast,
    /// Time left before it leaves; `None` while it stays (loading).
    left: Option<Duration>,
    /// The running countdown and when it started.
    running: Option<(TimerId, Instant)>,
}

/// The toasts on screen, newest first (`<Toaster />` with vue-sonner's `toast`).
#[derive(Clone)]
pub struct Toaster {
    shown: Signal<Vec<Shown>>,
    expanded: Signal<bool>,
    next: Rc<Cell<u64>>,
}

impl Default for Toaster {
    fn default() -> Self {
        Self::new()
    }
}

impl Toaster {
    #[must_use]
    pub fn new() -> Self {
        Self {
            shown: Signal::labeled("toasts", Vec::new()),
            expanded: Signal::labeled("toaster-expanded", false),
            next: Rc::new(Cell::new(1)),
        }
    }

    /// Show `toast` in front of the others.
    pub fn show(&self, toast: Toast) -> ToastId {
        let id = ToastId(self.next.get());
        self.next.set(self.next.get().wrapping_add(1));
        let shown = Shown {
            id,
            left: lifetime(toast.kind),
            toast,
            running: None,
        };
        // While the stack is spread out every countdown waits.
        let shown = if self.expanded.get() { shown } else { self.start(shown) };
        self.shown.update(|mut all| {
            all.insert(0, shown);
            all
        });
        id
    }

    /// `toast(title)`.
    pub fn toast(&self, title: impl Into<String>) -> ToastId {
        self.show(Toast::new(ToastKind::Default, title))
    }

    /// `toast.success(title)`.
    pub fn success(&self, title: impl Into<String>) -> ToastId {
        self.show(Toast::new(ToastKind::Success, title))
    }

    /// `toast.info(title)`.
    pub fn info(&self, title: impl Into<String>) -> ToastId {
        self.show(Toast::new(ToastKind::Info, title))
    }

    /// `toast.warning(title)`.
    pub fn warning(&self, title: impl Into<String>) -> ToastId {
        self.show(Toast::new(ToastKind::Warning, title))
    }

    /// `toast.error(title)`.
    pub fn error(&self, title: impl Into<String>) -> ToastId {
        self.show(Toast::new(ToastKind::Error, title))
    }

    /// `toast.loading(title)`: stays until [`Self::update`] or [`Self::dismiss`].
    pub fn loading(&self, title: impl Into<String>) -> ToastId {
        self.show(Toast::new(ToastKind::Loading, title))
    }

    /// Replace what toast `id` says, in place, and restart its countdown
    /// (how `toast.promise` turns a loading toast into its result).
    pub fn update(&self, id: ToastId, toast: Toast) {
        let paused = self.expanded.get();
        self.shown.update(|all| {
            all.into_iter()
                .map(|s| {
                    if s.id != id {
                        return s;
                    }
                    let fresh = Shown {
                        id,
                        left: lifetime(toast.kind),
                        toast: toast.clone(),
                        running: stop(s.running),
                    };
                    if paused { fresh } else { self.start(fresh) }
                })
                .collect()
        });
    }

    /// Take toast `id` off the screen.
    pub fn dismiss(&self, id: ToastId) {
        self.shown.update(|all| {
            all.into_iter()
                .filter(|s| {
                    let keep = s.id != id;
                    if !keep && let Some((timer, _)) = s.running {
                        clock::cancel(timer);
                    }
                    keep
                })
                .collect()
        });
    }

    /// Start `s`'s countdown, if it has one.
    fn start(&self, mut s: Shown) -> Shown {
        if let Some(left) = s.left {
            let me = self.clone();
            let id = s.id;
            s.running = Some((clock::after(left, move || me.dismiss(id)), Instant::now()));
        }
        s
    }

    /// Spread the stack out and pause every countdown, or gather it and resume.
    fn set_expanded(&self, expanded: bool) {
        if self.expanded.get() == expanded {
            return;
        }
        self.expanded.set(expanded);
        self.shown.update(|all| {
            all.into_iter()
                .map(|mut s| {
                    if expanded {
                        if let Some((timer, since)) = s.running.take() {
                            clock::cancel(timer);
                            s.left = s.left.map(|l| l.saturating_sub(since.elapsed()));
                        }
                        s
                    } else {
                        self.start(s)
                    }
                })
                .collect()
        });
    }
}

fn lifetime(kind: ToastKind) -> Option<Duration> {
    (kind != ToastKind::Loading).then_some(LIFETIME)
}

fn stop(running: Option<(TimerId, Instant)>) -> Option<(TimerId, Instant)> {
    if let Some((timer, _)) = running {
        clock::cancel(timer);
    }
    None
}

/// The toasts of `toaster` over the window's bottom-right corner; nothing
/// while there are none. Place it once, last in the window's tree.
#[must_use]
pub fn toaster(toaster: &Toaster) -> Element {
    let shown = toaster.shown.get();
    if shown.is_empty() {
        return Element::Empty;
    }
    let expanded = toaster.expanded.get();
    let visible: Vec<&Shown> = shown.iter().take(VISIBLE).collect();
    let on_hover = toaster.clone();
    let stack = div()
        .named("toaster")
        .relative()
        .w_px(WIDTH)
        .on_hover(move |inside| on_hover.set_expanded(inside));
    let stack = if expanded {
        // Oldest on top, the newest at the bottom edge.
        stack
            .flex_col()
            .gap(GAP / gui::styled::UNIT)
            .children(visible.iter().rev().map(|s| toast_box(toaster, s, None)))
    } else {
        // All in one cell, the ones behind first so the front one paints
        // over them; a toast leaving shares the cell with the one taking its place.
        let mut behind = 0.0_f32;
        let toasts: Vec<Div> = visible
            .iter()
            .map(|s| {
                let n = behind;
                behind += 1.0;
                toast_box(toaster, s, (n > 0.0).then_some(n))
            })
            .collect();
        stack.grid().items_end().children(toasts.into_iter().rev())
    };
    let corner = div()
        .named("toaster-viewport")
        .size_full()
        .pass_through()
        .flex_col()
        .justify_end()
        .items_end()
        .p(OFFSET)
        .child(stack);
    deferred(Placement::Fill, corner).into_element()
}

/// The surface every toast has.
fn surface() -> Div {
    let t = theme();
    div()
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .border()
        .border_color(t.border)
        .rounded_lg()
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
fn toast_box(toaster: &Toaster, s: &Shown, behind: Option<f32>) -> Div {
    let t = theme();
    let kind_icon = match s.toast.kind {
        ToastKind::Default => None,
        ToastKind::Success => Some(icon(Icon::CircleCheck)),
        ToastKind::Info => Some(icon(Icon::Info)),
        ToastKind::Warning => Some(icon(Icon::TriangleAlert)),
        ToastKind::Error => Some(icon(Icon::OctagonX)),
        ToastKind::Loading => Some(spinner()),
    };
    let content = div()
        .named("toast-content")
        .flex_col()
        .gap_y(0.5)
        .child(
            text(s.toast.title.clone())
                .named("toast-title")
                .font_medium()
                .leading_normal(),
        )
        .child(s.toast.description.clone().map(|d| {
            let d = match d {
                ToastDescription::Text(d) => text(d).into_element(),
                ToastDescription::View(view) => view(),
            };
            div()
                .named("toast-description")
                .font_normal()
                .leading_px(13.0 * 1.4)
                .text_color(description_color(t.dark))
                .child(d)
        }));
    let action = s.toast.action.clone().map(|a| {
        let me = toaster.clone();
        let id = s.id;
        div()
            .named("toast-action")
            .flex()
            .items_center()
            .shrink_0()
            .ml_auto()
            .h(6.0)
            .px(2.0)
            .rounded(4.0)
            .text_xs()
            .font_medium()
            .bg(t.popover_foreground)
            .text_color(t.popover)
            .cursor_pointer()
            .focusable()
            .focus_visible(|st| st.ring(2.0, Color::BLACK.opacity(0.4)))
            .on_click(move |_| {
                (a.on_click)();
                me.dismiss(id);
            })
            .child(a.label)
    });
    // vue-sonner's `[data-sonner-toast] > * { transition: opacity 400ms }`.
    let part = |d: Div| {
        d.transition_opacity()
            .duration_ms(400.0)
            .ease(Easing::EASE)
            .when(behind.is_some(), |d| d.opacity(0.0))
    };
    let (kind_icon, content, action) = (
        kind_icon.map(|i| {
            part(
                div()
                    .named("toast-icon")
                    .flex()
                    .items_center()
                    .size(4.0)
                    .shrink_0()
                    .ml(-0.75)
                    .mr(1.0)
                    .child(i),
            )
        }),
        part(content),
        action.map(part),
    );
    surface()
        .named("toast")
        .key(s.id.0)
        // Mounting rises from below and fades in; leaving sinks and fades
        // for the 200ms before vue-sonner unmounts it (`TIME_BEFORE_UNMOUNT`),
        // half its 400ms transition.
        .transition_props(Properties::TRANSFORM | Properties::OPACITY)
        .duration_ms(400.0)
        .ease(Easing::EASE)
        .animate(Animation::enter().slide_y(Calc::fraction(1.0)).fade(0.0))
        .animate_out(Animation::exit().slide_y(Calc::fraction(1.0)).fade(0.0).iterations(0.5))
        .row_start(1)
        .col_start(1)
        .when_some(behind, |d, n| {
            d.absolute()
                .top(0.0)
                .bottom(0.0)
                .left(0.0)
                .right(0.0)
                .overflow_hidden()
                .translate_y(Calc::px(-GAP * n))
                .scale(1.0 - SHRINK * n)
        })
        .w_px(WIDTH)
        .flex()
        .items_center()
        .gap(1.5)
        .p(4.0)
        .text_size(13.0, 13.0 * 1.5)
        .child(kind_icon)
        .child(content)
        .child(action)
        .when_some(s.toast.class.clone(), |d, class| class(d))
}
