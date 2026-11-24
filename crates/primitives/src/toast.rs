//! Brief notices that leave on their own (reka's `Toast`, the nearest of
//! reka's primitives to the vue-sonner that shadcn's `sonner` draws on).
//!
//! - **The store.** A [`Toaster`] holds the toasts, newest first, each with
//!   its content (any `T`), its status, its countdown and what is read of it
//!   ([`ShowOptions`]). A toast shown with an id already shown is updated in
//!   place, its countdown starts again and it is read again (reka's
//!   `createToastManager` and `updateKey`). Unmounting the toaster's owner
//!   cancels every countdown.
//! - **The countdown.** Each toast leaves after its duration (reka: 5000 ms;
//!   none, or a zero one, stays until dismissed, as a loading toast always
//!   does). Every countdown pauses and resumes together, from the time it
//!   had left: while the pointer is over the toasts, while the window is not
//!   focused (each unless turned off), while focus is in them (always,
//!   whatever else is turned off), while one is pressed. A toast added
//!   while paused waits for the resume.
//! - **A promise** ([`Toaster::promise`], reka's `toastManager.promise`): a
//!   loading toast that the work's result, sent from any thread, turns into
//!   a success or an error toast in place.
//! - **The limit** ([`ToasterOptions::limit`]): toasts past it, the oldest,
//!   stay but are inert (no pointer, no focus, not read) until newer ones
//!   leave.
//! - **Keys.** The hotkey ([`ToasterOptions::hotkey`], reka's F8) focuses
//!   the region from anywhere in the window. Tab visits the newest toast
//!   first, then its controls, then the older ones (the look declares them
//!   oldest first, so the newest paints on top); past either end it goes on
//!   in the page; from before the region it reaches the newest toast, from
//!   after it the oldest toast's last control (reka's `ToastViewport`).
//!   Escape on a toast holding focus closes that toast and goes no further
//!   (no dialog around it closes). reka then leaves focus where it was, on
//!   nothing, as its close carries no event (research §4 item 13); here
//!   focus goes to the toasts' region, as a close by Close or Action from
//!   the keyboard does in reka. Recorded in ADR-0025.
//! - **Swipe** ([`ToastSwipe`], on every toast): toward the swipe way only
//!   (clamped), starting past 2 px (a mouse) or 10 px (a touch); a start the
//!   wrong way gives the press up; let go past 50 px, it closes (unless the
//!   caller refuses), else it slides back. No speed counts. The click that
//!   ends a swipe presses nothing.
//! - **A branch.** The region is a branch of every layer (reka's
//!   `DismissableLayerBranch`): pressing or focusing a toast closes no open
//!   popover or dialog that is not modal.
//! - **Anchored** ([`Toaster::positioner`], reka's `ToastPositioner`): a
//!   toast shown with a [`ToastAnchor`] is not in the region but beside the
//!   element its positioner is declared in, on the top side by default
//!   ([`POSITIONER`]); it pauses, is a branch and is read as the region's
//!   toasts are.
//! - **What is read.** The region is a `region` named by its label (reka:
//!   "Notifications (F8)"). Each toast is announced by a live node of its
//!   own, off screen: the toaster's label ("Notification", never blank),
//!   then the toast's [`Announcement`], its chunks read as sentences so a
//!   reader pauses between them; a Close is not in it, and an action is
//!   read by its alternative text ([`Spoken`], which it cannot be made
//!   without). Politely, or (a foreground toast, reka's default) at once;
//!   again on each update. reka shows the live text two frames (or 1000 ms)
//!   after the live node, for NVDA to notice it; AccessKit raises the live
//!   event itself as the node comes with its name, so it is read in the
//!   frame the toast shows (recorded in ADR-0025).

use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::element::Live;
use gui::events::{Key, KeyEvent, Modifiers};
use gui::{
    Callback, Deferred, Dismiss, DismissPolicy, Div, DragEvent, DragPhase, FocusHandle, Interaction, Layer, NamedKey,
    PointerKind, Role, Scope, Sender, Side, Signal, deferred, div,
};

use crate::popper::Popper;
use crate::visually_hidden::{Hidden, visually_hidden};

/// reka's `ToastProvider` `duration`.
pub const DURATION: Duration = Duration::from_millis(5000);
/// reka's `swipeThreshold`.
pub const SWIPE_THRESHOLD: f32 = 50.0;
/// How far a mouse moves before a swipe starts.
pub const SWIPE_START_MOUSE: f32 = 2.0;
/// How far a touch moves before a swipe starts.
pub const SWIPE_START_TOUCH: f32 = 10.0;
/// Where an anchored toast goes (reka's `ToastPositioner` defaults): above
/// its anchor, its arrow kept 5 px from its corners. reka's collision
/// padding (5 px) has no counterpart: layout keeps the toast on screen.
pub const POSITIONER: Popper = Popper {
    side: Side::Top,
    arrow_padding: 5.0,
    ..Popper::DEFAULT
};

/// A toast's name, to update or dismiss it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastId(pub u64);

/// How a toast is announced (reka's `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Sensitivity {
    /// Read at once: the user's own action caused it.
    #[default]
    Foreground,
    /// Read when the reader is idle.
    Background,
}

/// What a toast is about (reka's `status`, `data-status` for the look).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ToastStatus {
    #[default]
    Plain,
    /// Waiting for a result: it stays until updated or dismissed.
    Loading,
    Success,
    Error,
}

/// How long a toast stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Lifetime {
    /// The toaster's duration ([`ToasterOptions::duration`]).
    #[default]
    Toaster,
    /// This long; a zero one stays.
    For(Duration),
    /// Until dismissed (reka's `Infinity`).
    Stays,
}

/// Text read to a screen reader: never blank (reka's provider `label` and
/// an action's `altText`, which reka requires).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Spoken(String);

impl Spoken {
    /// `text`, unless it has nothing but spaces.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Option<Self> {
        let text = text.into();
        (!text.trim().is_empty()).then_some(Self(text))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a toast's announcement reads after the toaster's label: its text
/// in chunks (a title, a description) and its action's alternative text,
/// not its Close (reka's `getAnnounceTextContent` over the toast).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Announcement {
    chunks: Vec<String>,
}

impl Announcement {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Then `text`; a blank one is left out.
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        if !text.trim().is_empty() {
            self.chunks.push(text);
        }
        self
    }

    /// Then an action, read as its alternative text.
    #[must_use]
    pub fn action(self, alt: &Spoken) -> Self {
        self.text(alt.as_str())
    }

    /// What is read: `label`, then each chunk, each one ending a sentence
    /// so that a reader pauses between them (reka renders each chunk as a
    /// text node of its own for that pause; a live node's name is one
    /// string).
    #[must_use]
    pub fn read(&self, label: &Spoken) -> String {
        let mut out = String::new();
        for chunk in std::iter::once(label.as_str()).chain(self.chunks.iter().map(String::as_str)) {
            let chunk = chunk.trim();
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(chunk);
            if !chunk.ends_with(['.', '!', '?', ':', ';', '\u{2026}']) {
                out.push('.');
            }
        }
        out
    }
}

/// A place an anchored toast shows beside (reka's `positionerProps.anchor`):
/// made once, in the setup of what declares its [`Toaster::positioner`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastAnchor(Interaction);

impl Default for ToastAnchor {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastAnchor {
    #[must_use]
    pub fn new() -> Self {
        Self(Interaction::new())
    }
}

/// How a toast is shown, beside its content.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ShowOptions {
    pub sensitivity: Sensitivity,
    pub lifetime: Lifetime,
    pub status: ToastStatus,
    /// Shown beside this anchor, not in the region.
    pub anchor: Option<ToastAnchor>,
    /// What is read of it.
    pub announcement: Announcement,
}

/// A key and the modifiers held with it (reka's `hotkey`: every one must
/// be down; others may be too).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl Default for Hotkey {
    /// F8 (reka's `VIEWPORT_DEFAULT_HOTKEY`).
    fn default() -> Self {
        Self {
            key: Key::Named(NamedKey::F8),
            modifiers: Modifiers::default(),
        }
    }
}

impl Hotkey {
    /// Whether `ev` presses it: its key (a letter in either case), and at
    /// least its modifiers.
    #[must_use]
    pub fn matches(&self, ev: &KeyEvent) -> bool {
        let m = ev.modifiers;
        let held = (!self.modifiers.shift || m.shift)
            && (!self.modifiers.control || m.control)
            && (!self.modifiers.alt || m.alt)
            && (!self.modifiers.command || m.command);
        let key = match (&self.key, &ev.key) {
            (Key::Character(a), Key::Character(b)) => a.eq_ignore_ascii_case(b),
            (a, b) => a == b,
        };
        held && key
    }
}

/// What a toaster's caller chooses (reka's `ToastProvider` and
/// `ToastViewport` props).
#[derive(Clone, Debug, PartialEq)]
pub struct ToasterOptions {
    /// How long a toast stays by default (`None`: until dismissed).
    pub duration: Option<Duration>,
    /// Countdowns pause while the pointer is over the region (reka: on).
    pub pause_on_hover: bool,
    /// Countdowns pause while the window is not focused (reka: on).
    pub pause_on_window_blur: bool,
    /// How many toasts take input at once (reka: no limit).
    pub limit: Option<usize>,
    /// What focuses the region (reka: F8).
    pub hotkey: Hotkey,
    /// The way a toast is swiped away (reka: right).
    pub swipe: SwipeWay,
    /// Asked before a swipe takes toast `id` away.
    pub veto_swipe: Option<Callback<ToastId, Dismiss>>,
    /// Read before each toast's announcement (reka: "Notification").
    pub label: Spoken,
}

impl Default for ToasterOptions {
    fn default() -> Self {
        Self {
            duration: Some(DURATION),
            pause_on_hover: true,
            pause_on_window_blur: true,
            limit: None,
            hotkey: Hotkey::default(),
            swipe: SwipeWay::Right,
            veto_swipe: None,
            label: Spoken("Notification".to_owned()),
        }
    }
}

/// One toast as the store holds it.
#[derive(Clone, Debug)]
pub struct Shown<T> {
    pub id: ToastId,
    pub content: T,
    pub sensitivity: Sensitivity,
    pub status: ToastStatus,
    pub anchor: Option<ToastAnchor>,
    pub announcement: Announcement,
    /// How many times it was updated in place (reka's `updateKey`).
    pub updates: u64,
    /// Time left; `None` while it stays.
    pub left: Option<Duration>,
    /// The running countdown and when it started (on the clock).
    running: Option<(TimerId, Duration)>,
}

/// Why the countdowns pause; they run while none holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Paused {
    hover: bool,
    focus: bool,
    press: bool,
    /// The window is not focused.
    blurred: bool,
    /// The caller's own reason (vue-sonner's expanded stack).
    held: bool,
}

impl Paused {
    const fn any(self) -> bool {
        self.hover || self.focus || self.press || self.blurred || self.held
    }
}

/// A toast being swiped: which, how, how far.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Swiping {
    id: ToastId,
    swipe: ToastSwipe,
    offset: Option<(f32, f32)>,
}

/// The toasts of one toaster: made once in its setup (`Copy`).
pub struct Toaster<T: 'static> {
    shown: Signal<Vec<Shown<T>>>,
    paused: Signal<Paused>,
    /// What every countdown's timer belongs to.
    owner: Scope,
    region: Interaction,
    region_focus: FocusHandle,
    options: Signal<ToasterOptions>,
    next: Signal<u64>,
    swiping: Signal<Option<Swiping>>,
    /// A swipe just ended: the click it makes presses nothing.
    swallow: Signal<bool>,
}

impl<T: 'static> Clone for Toaster<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Toaster<T> {}

impl<T: 'static> PartialEq for Toaster<T> {
    fn eq(&self, other: &Self) -> bool {
        self.shown == other.shown
    }
}

impl<T: 'static> std::fmt::Debug for Toaster<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toaster").finish_non_exhaustive()
    }
}

impl<T: Clone + 'static> Toaster<T> {
    /// A toaster as `options` choose; called in its setup, which owns the
    /// countdowns.
    #[must_use]
    #[track_caller]
    pub fn new(options: ToasterOptions) -> Self {
        let t = Self {
            shown: Signal::new(Vec::new()),
            paused: Signal::labeled("toasts paused", Paused::default()),
            owner: Scope::new(),
            region: Interaction::new(),
            region_focus: FocusHandle::new(),
            options: Signal::new(options),
            next: Signal::new(1),
            swiping: Signal::labeled("toast swiping", None),
            swallow: Signal::labeled("toast swallow", false),
        };
        let region = t.region;
        // Watched from the first frame: a node's flags read while setup
        // renders are not subscribed to. The toaster's scope owns it. The
        // positioners of the anchored toasts count as the region does.
        gui::on_mounted(move || {
            t.owner.run(|| {
                gui::watch(
                    move || {
                        let anchors: Vec<Interaction> = t
                            .shown
                            .with(|all| all.iter().filter_map(|s| s.anchor.map(|a| a.0)).collect());
                        let any = |f: fn(Interaction) -> bool| f(region) || anchors.iter().any(|&a| f(a));
                        (
                            any(Interaction::hovered),
                            any(Interaction::focus_within),
                            any(Interaction::pressed),
                        )
                    },
                    move |&(hover, focus, press), _| {
                        let on_hover = t.options.with_untracked(|o| o.pause_on_hover);
                        t.paused.update(|p| {
                            p.hover = hover && on_hover;
                            p.focus = focus;
                            p.press = press;
                        });
                    },
                );
            });
        });
        let blur = move |active: bool| {
            let on_blur = t.options.with_untracked(|o| o.pause_on_window_blur);
            t.paused.update(|p| p.blurred = !active && on_blur);
        };
        if !gui::untracked(gui::interact::window_active) {
            blur(false);
        }
        gui::watch(gui::interact::window_active, move |&active, _| blur(active));
        gui::watch(
            move || t.paused.get().any(),
            move |&paused, _| {
                if paused { t.pause_all() } else { t.resume_all() }
            },
        );
        t
    }

    /// The toasts, newest first (tracked).
    pub fn with<R>(self, f: impl FnOnce(&[Shown<T>]) -> R) -> R {
        self.shown.with(|s| f(s))
    }

    /// The toasts shown beside `anchor`, or in the region when `None`,
    /// newest first (tracked).
    #[must_use]
    pub fn at(self, anchor: Option<ToastAnchor>) -> Vec<Shown<T>> {
        self.shown
            .with(|all| all.iter().filter(|s| s.anchor == anchor).cloned().collect())
    }

    /// Whether the countdowns are paused (tracked).
    #[must_use]
    pub fn is_paused(self) -> bool {
        self.paused.get().any()
    }

    /// Pause (or resume) for the caller's own reason, beside hover, focus
    /// and press.
    pub fn hold(self, held: bool) {
        self.paused.update(|p| p.held = held);
    }

    /// A fresh id.
    #[must_use]
    pub fn next_id(self) -> ToastId {
        let id = self.next.get_untracked();
        self.next.set(id.wrapping_add(1));
        ToastId(id)
    }

    /// Show `content` as toast `id`, as `options` say; an id already shown
    /// is updated in place, its countdown started again, read again.
    pub fn show(self, id: ToastId, content: T, options: ShowOptions) {
        let left = match (options.status, options.lifetime) {
            (ToastStatus::Loading, _) | (_, Lifetime::Stays) => None,
            (_, Lifetime::Toaster) => self.duration(),
            (_, Lifetime::For(d)) => Some(d),
        }
        .filter(|d| !d.is_zero());
        let paused = self.paused.get_untracked().any();
        let updates = self
            .shown
            .with_untracked(|all| all.iter().find(|s| s.id == id).map_or(0, |s| s.updates.wrapping_add(1)));
        let mut fresh = Shown {
            id,
            content,
            sensitivity: options.sensitivity,
            status: options.status,
            anchor: options.anchor,
            announcement: options.announcement,
            updates,
            left,
            running: None,
        };
        if !paused {
            self.start(&mut fresh);
        }
        self.shown.update(|all| match all.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                stop(s.running.take());
                *s = fresh;
            }
            None => all.insert(0, fresh),
        });
    }

    /// Show `loading` as loading toast `id` (its status set so: it stays),
    /// which the result sent through the returned sender, from any thread,
    /// turns into what `settle` makes of it, in place, as a success or an
    /// error (reka's `promise`; a fresh id is [`Self::next_id`]). A result
    /// for a toast dismissed meanwhile shows nothing, and only the first
    /// result counts; the toaster unmounting closes the mailbox.
    pub fn promise<V: Send + 'static, E: Send + 'static>(
        self,
        id: ToastId,
        loading: (T, ShowOptions),
        settle: impl FnOnce(Result<V, E>) -> (T, ShowOptions) + 'static,
    ) -> Sender<Result<V, E>> {
        let (content, mut options) = loading;
        options.status = ToastStatus::Loading;
        self.show(id, content, options);
        let mut settle = Some(settle);
        self.owner.run(|| {
            gui::mailbox(move |result: Result<V, E>| {
                let Some(settle) = settle.take() else {
                    return;
                };
                let status = if result.is_ok() {
                    ToastStatus::Success
                } else {
                    ToastStatus::Error
                };
                let (content, mut options) = settle(result);
                options.status = status;
                if self.shown.is_alive() && self.shown.with_untracked(|all| all.iter().any(|s| s.id == id)) {
                    self.show(id, content, options);
                }
            })
        })
    }

    /// The toaster's default duration.
    #[must_use]
    pub fn duration(self) -> Option<Duration> {
        self.options.with_untracked(|o| o.duration)
    }

    /// Whether toast `id` is past the limit (tracked): inert, the look may
    /// hide it.
    #[must_use]
    pub fn limited(self, id: ToastId) -> bool {
        let Some(limit) = self.options.with(|o| o.limit) else {
            return false;
        };
        self.shown
            .with(|all| all.iter().position(|s| s.id == id).is_some_and(|at| at >= limit))
    }

    /// Take toast `id` away.
    pub fn dismiss(self, id: ToastId) {
        if !self.shown.is_alive() {
            return;
        }
        let mut gone = None;
        self.shown.update(|all| {
            all.retain_mut(|s| {
                let keep = s.id != id;
                if !keep {
                    gone = s.running.take();
                }
                keep
            });
        });
        stop(gone);
    }

    /// Take every toast away.
    pub fn dismiss_all(self) {
        let all = self
            .shown
            .with_untracked(|s| s.iter().map(|s| s.id).collect::<Vec<_>>());
        for id in all {
            self.dismiss(id);
        }
    }

    fn start(self, s: &mut Shown<T>) {
        let Some(left) = s.left else {
            return;
        };
        if !self.owner.is_alive() {
            return;
        }
        let id = s.id;
        let timer = self.owner.run(|| clock::after(left, move || self.dismiss(id)));
        s.running = Some((timer, clock::current_time()));
    }

    fn pause_all(self) {
        let now = clock::current_time();
        self.shown.update(|all| {
            for s in all.iter_mut() {
                if let Some((timer, since)) = s.running.take() {
                    clock::cancel(timer);
                    let spent = now.saturating_sub(since);
                    s.left = s.left.map(|l| l.saturating_sub(spent));
                }
            }
        });
    }

    fn resume_all(self) {
        let mut all = self.shown.get_untracked();
        for s in &mut all {
            if s.running.is_none() {
                self.start(s);
            }
        }
        self.shown.update(|now| *now = all);
    }

    /// `d` as the region holding the toasts not anchored elsewhere: a
    /// `region` named `label` (reka: "Notifications (F8)"), focusable for
    /// the focus a closed toast gives back and for the hotkey, which
    /// focuses it from anywhere; pausing every countdown while hovered,
    /// focused within or pressed; a branch of every layer; Tab visiting its
    /// children last first. Give it its toasts oldest first (the newest
    /// painted on top). It holds their announcements, off screen. Its
    /// node's [`Interaction`] is the toaster's.
    #[must_use]
    pub fn region(self, d: Div, label: &str) -> Div {
        d.interaction(self.region)
            .role(Role::Region)
            .aria_label(label)
            .track_focus(self.region_focus)
            .tab_stop(false)
            .tab_children_reversed(true)
            .layer(Layer::new().branch().dismiss(DismissPolicy::NONE))
            .on_window_key(move |ev| {
                if !self.options.with_untracked(|o| o.hotkey.matches(ev)) {
                    return false;
                }
                self.region_focus.focus();
                true
            })
            .child(self.announcer(None))
    }

    /// `d`, holding the toasts shown beside `anchor` (oldest first), as
    /// their positioner (reka's `ToastPositioner`): on a layer of its own
    /// beside the element it is declared in, as `popper` places it
    /// ([`POSITIONER`] by default), pausing the countdowns as the region
    /// does, a branch of every layer, Tab visiting the newest first, and
    /// holding their announcements.
    #[must_use]
    pub fn positioner(self, anchor: ToastAnchor, popper: &Popper, d: Div) -> Deferred {
        deferred(
            popper.positioning(),
            d.interaction(anchor.0)
                .tab_children_reversed(true)
                .layer(Layer::new().branch().dismiss(DismissPolicy::NONE))
                .child(self.announcer(Some(anchor))),
        )
    }

    /// The live nodes that read the toasts at `anchor`, off screen: one per
    /// toast within the limit (one past it is not read), made again as it
    /// is updated, so it is read again.
    fn announcer(self, anchor: Option<ToastAnchor>) -> Div {
        let (label, limit) = self.options.with(|o| (o.label.clone(), o.limit));
        let nodes: Vec<Div> = self.shown.with(|all| {
            all.iter()
                .take(limit.unwrap_or(usize::MAX))
                .filter(|s| s.anchor == anchor)
                .map(|s| {
                    let live = match s.sensitivity {
                        Sensitivity::Foreground => Live::Assertive,
                        Sensitivity::Background => Live::Polite,
                    };
                    div()
                        .key(s.id.0.rotate_left(32) ^ s.updates)
                        .role(Role::Status)
                        .aria_live(live)
                        .aria_label(s.announcement.read(&label))
                })
                .collect()
        });
        visually_hidden(div().named("toast-announcer"), Hidden::Visually).children(nodes)
    }

    /// The region's focus: what the hotkey, and a close from the keyboard,
    /// focus.
    #[must_use]
    pub fn region_focus(self) -> FocusHandle {
        self.region_focus
    }

    /// The region's interaction: what a look reads its hover from (a stack
    /// spread out while hovered).
    #[must_use]
    pub fn region_interaction(self) -> Interaction {
        self.region
    }

    /// How far toast `id` is swiped from where it rests (tracked; `None`
    /// while it is not).
    #[must_use]
    pub fn swipe_offset(self, id: ToastId) -> Option<(f32, f32)> {
        self.swiping.with(|s| s.filter(|s| s.id == id).and_then(|s| s.offset))
    }

    /// `d` as toast `id`: a Tab stop, inert past the limit; Escape on it
    /// closes it and gives focus to the region; swiped its way, it goes.
    /// What is read of it as it comes is its announcement, which the region
    /// or its positioner holds.
    #[must_use]
    pub fn toast(self, d: Div, id: ToastId) -> Div {
        d.role(Role::Group)
            .focusable()
            .inert(self.limited(id))
            .on_key_down(move |ev| {
                if !ev.is(NamedKey::Escape) {
                    return false;
                }
                self.close_from_keyboard(id);
                true
            })
            .on_drag(move |e| self.drag(id, e))
    }

    fn drag(self, id: ToastId, e: &DragEvent) {
        match e.phase {
            DragPhase::Start => {
                self.swallow.set(false);
                let way = self.options.with_untracked(|o| o.swipe);
                let swipe = ToastSwipe::new(way, (e.x, e.y), e.kind == PointerKind::Touch);
                self.swiping.set(Some(Swiping {
                    id,
                    swipe,
                    offset: None,
                }));
            }
            DragPhase::Move => {
                let Some(mut s) = self.swiping.get_untracked().filter(|s| s.id == id) else {
                    return;
                };
                let offset = match s.swipe.moved((e.x, e.y)) {
                    SwipeStep::Moved(o) => Some(o),
                    SwipeStep::Idle => None,
                };
                if offset != s.offset || s.offset.is_none() {
                    s.offset = offset;
                    self.swiping.set(Some(s));
                }
            }
            DragPhase::End => {
                let Some(s) = self.swiping.get_untracked().filter(|s| s.id == id) else {
                    return;
                };
                self.swiping.set(None);
                if s.offset.is_some() {
                    self.swallow.set(true);
                }
                if s.swipe.release() {
                    let veto = self.options.with_untracked(|o| o.veto_swipe.clone());
                    if veto.is_none_or(|v| v.call(id) == Dismiss::Close) {
                        self.dismiss(id);
                    }
                }
            }
            // Cancelled (the toast was disabled mid-swipe): it goes back,
            // not dismissed.
            DragPhase::Cancel => {
                if self.swiping.get_untracked().is_some_and(|s| s.id == id) {
                    self.swiping.set(None);
                }
            }
        }
    }

    /// `d` as a button that closes toast `id` (reka's `ToastClose`).
    #[must_use]
    pub fn closer(self, d: Div, id: ToastId) -> Div {
        self.closing(d, id, gui::Callback::new(|()| {}))
    }

    /// `d` as toast `id`'s action (reka's `ToastAction`): it runs `act`,
    /// then closes the toast; it is read as `alt` (give the same to the
    /// toast's [`Announcement::action`]).
    #[must_use]
    pub fn action(self, d: Div, id: ToastId, alt: &Spoken, act: impl Into<gui::Callback<()>>) -> Div {
        self.closing(d.aria_label(alt.as_str()), id, act.into())
    }

    fn closing(self, d: Div, id: ToastId, act: gui::Callback<()>) -> Div {
        d.focusable().on_click(move |_| {
            // The click a swipe ends with presses nothing.
            if self.swallow.get_untracked() {
                self.swallow.set(false);
                return;
            }
            act.call(());
            if gui::focus::modality() == gui::Modality::Keyboard {
                self.close_from_keyboard(id);
            } else {
                self.dismiss(id);
            }
        })
    }

    fn close_from_keyboard(self, id: ToastId) {
        self.dismiss(id);
        self.region_focus.focus();
    }
}

/// Cancel a countdown.
fn stop(running: Option<(TimerId, Duration)>) {
    if let Some((timer, _)) = running {
        clock::cancel(timer);
    }
}

/// The way a toast is swiped away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum SwipeWay {
    Up,
    Down,
    Left,
    #[default]
    Right,
}

/// What a toast's swipe did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwipeStep {
    /// Not started, or given up.
    Idle,
    /// Started or moving, the toast this far from where it rests.
    Moved((f32, f32)),
}

/// A toast's swipe (reka's `ToastRootImpl` pointer handlers): fed the
/// pointer's moves from where it went down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToastSwipe {
    way: SwipeWay,
    start: Option<(f32, f32)>,
    delta: Option<(f32, f32)>,
    buffer: f32,
}

impl ToastSwipe {
    /// A press at `start`, by touch or not.
    #[must_use]
    pub fn new(way: SwipeWay, start: (f32, f32), touch: bool) -> Self {
        Self {
            way,
            start: Some(start),
            delta: None,
            buffer: if touch { SWIPE_START_TOUCH } else { SWIPE_START_MOUSE },
        }
    }

    fn toward(way: SwipeWay, (x, y): (f32, f32), past: f32) -> bool {
        match way {
            SwipeWay::Left => x < -past,
            SwipeWay::Right => x > past,
            SwipeWay::Up => y < -past,
            SwipeWay::Down => y > past,
        }
    }

    /// The pointer moved to `at`.
    pub fn moved(&mut self, at: (f32, f32)) -> SwipeStep {
        let Some(start) = self.start else {
            return SwipeStep::Idle;
        };
        let (x, y) = (at.0 - start.0, at.1 - start.1);
        let horizontal = matches!(self.way, SwipeWay::Left | SwipeWay::Right);
        let toward_negative = matches!(self.way, SwipeWay::Left | SwipeWay::Up);
        let clamp = |v: f32| if toward_negative { v.min(0.0) } else { v.max(0.0) };
        let delta = if horizontal { (clamp(x), 0.0) } else { (0.0, clamp(y)) };
        if self.delta.is_some() || Self::toward(self.way, delta, self.buffer) {
            self.delta = Some(delta);
            SwipeStep::Moved(delta)
        } else {
            if x.abs() > self.buffer || y.abs() > self.buffer {
                // The wrong way: no swipe for this press.
                self.start = None;
            }
            SwipeStep::Idle
        }
    }

    /// Let go: whether the toast closes (past 50 px the swipe's way).
    #[must_use]
    pub fn release(self) -> bool {
        self.delta.is_some_and(|d| Self::toward(self.way, d, SWIPE_THRESHOLD))
    }
}

#[cfg(test)]
mod tests {
    use gui::events::{Key, KeyEvent, Modifiers, NamedKey};

    use super::{Announcement, Hotkey, Spoken, SwipeStep, SwipeWay, ToastSwipe};

    fn press(key: Key, modifiers: Modifiers) -> KeyEvent {
        KeyEvent {
            key,
            text: None,
            modifiers,
        }
    }

    #[test]
    fn a_hotkey_matches_its_key_with_at_least_its_modifiers() {
        let f8 = Hotkey::default();
        assert!(f8.matches(&press(Key::Named(NamedKey::F8), Modifiers::default())));
        assert!(!f8.matches(&press(Key::Named(NamedKey::F7), Modifiers::default())));
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let alt_t = Hotkey {
            key: Key::Character("t".into()),
            modifiers: alt,
        };
        assert!(alt_t.matches(&press(Key::Character("T".into()), alt)), "either case");
        assert!(
            alt_t.matches(&press(Key::Character("t".into()), Modifiers { shift: true, ..alt })),
            "more modifiers held than asked"
        );
        assert!(
            !alt_t.matches(&press(Key::Character("t".into()), Modifiers::default())),
            "Alt not held"
        );
    }

    #[test]
    fn a_swipe_starts_past_its_buffer_and_closes_past_fifty_pixels() {
        let mut s = ToastSwipe::new(SwipeWay::Right, (0.0, 0.0), false);
        assert_eq!(s.moved((2.0, 0.0)), SwipeStep::Idle, "2 px is not past a mouse's 2");
        assert_eq!(s.moved((3.0, 1.0)), SwipeStep::Moved((3.0, 0.0)));
        assert_eq!(
            s.moved((-20.0, 0.0)),
            SwipeStep::Moved((0.0, 0.0)),
            "clamped: no way back"
        );
        assert!(!s.release());
        assert_eq!(s.moved((51.0, 0.0)), SwipeStep::Moved((51.0, 0.0)));
        assert!(s.release());
        let mut touch = ToastSwipe::new(SwipeWay::Right, (0.0, 0.0), true);
        assert_eq!(touch.moved((8.0, 0.0)), SwipeStep::Idle, "a touch's buffer is 10 px");
    }

    #[test]
    fn an_announcement_reads_the_label_then_each_chunk_as_a_sentence() {
        let label = Spoken::new("Notification").expect("not blank");
        let alt = Spoken::new("Undo the move").expect("not blank");
        let a = Announcement::new()
            .text("Moved to Trash")
            .text("  ")
            .text("3 files left.")
            .action(&alt);
        assert_eq!(
            a.read(&label),
            "Notification. Moved to Trash. 3 files left. Undo the move."
        );
        assert_eq!(Announcement::new().read(&label), "Notification.", "the label alone");
        assert!(Spoken::new(" \t").is_none(), "never blank");
    }

    #[test]
    fn a_start_the_wrong_way_gives_the_press_up() {
        let mut s = ToastSwipe::new(SwipeWay::Right, (0.0, 0.0), false);
        assert_eq!(s.moved((0.0, 5.0)), SwipeStep::Idle);
        assert_eq!(s.moved((60.0, 0.0)), SwipeStep::Idle, "given up for this press");
        assert!(!s.release());
    }
}
