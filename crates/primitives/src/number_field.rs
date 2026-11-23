//! A number typed, or stepped with keys and two buttons (reka's
//! `NumberFieldRoot`, `NumberFieldInput`, `NumberFieldIncrement`,
//! `NumberFieldDecrement`; research §10 "NumberField").
//!
//! - **Value.** An `f64`; NaN is "no value" (reka's `null`), the default.
//!   The input shows it formatted ([`NumberFormat`]), empty while there is
//!   none; while the user types, the text is a draft and the value stays.
//! - **Commit** ([`NumberField::commit`], on Enter and when the input loses
//!   focus; `NumberFieldRoot.vue:238-263`): blank text clears the value;
//!   text with no digit (`.`, `-`, `%`) or that does not parse (`1.2.3`)
//!   clears it too, or, with [`NumberConfig::keep_unparsable`], keeps the
//!   value and puts its text back without telling the caller; text that
//!   parses is clamped and snapped (or, with
//!   [`NumberConfig::allow_out_of_range`], kept, only normalised to the
//!   format's precision) and shown formatted (`7` → `EUR 7.00`). Digits are
//!   those of every script plus the Han numerals `〇一二三四五六七八九`.
//! - **Steps** ([`NumberField::step`]): read the text in the input, the
//!   draft included, not the stored value (`NumberFieldRoot.vue:157`). Blank
//!   or unparsable text, stepped either way, takes the starting value: the
//!   caller's, clamped but not snapped; else min (or 0), clamped and snapped.
//!   On the grid, ±step × count, in exact decimals; off it, first to the
//!   grid line in the step's direction (18.98 ↑ is 19, 18.11 ↓ is 18), and
//!   PageUp/PageDown add the other nine. The result is clamped and snapped.
//!   A value out of range (typed with `allow_out_of_range`) stepped further
//!   out does nothing, stepped back is clamped to the edge
//!   (`NumberFieldRoot.vue:170-173`).
//! - **Snap** ([`snap_to_step`], `shared/clamp.ts:68-90`): the grid counts
//!   from min, or 0, to the nearest line; above max, the last line at or
//!   under max, so a max off the grid is not reached while snapping (min 2,
//!   max 21, step 3 gives 20) and is without it. Precision is the larger of
//!   step's and min's decimals. Every value the user writes is normalised to
//!   the format's precision (format, then parse).
//! - **Keys** on the input: ArrowUp/ArrowDown ± one step, PageUp/PageDown ±
//!   ten, Home to min and End to max (each consumed with or without a
//!   bound: the caret does not move), Enter commits and goes on (a form's
//!   submit still hears it). While an input method composes, none of them
//!   act (`NumberFieldInput.vue:27-28`). Typed text that would not be a
//!   number in progress is refused (`NumberFieldInput.vue:130-144`); a minus
//!   only while min is below 0 or out-of-range values are allowed. Deleting
//!   is never refused. Readonly or disabled, no key writes and nothing can
//!   be typed, pasted or composed (the input is read-only itself, and told
//!   so to assistive technology); reka's Home and End still write min and
//!   max while readonly (`NumberFieldRoot.vue:184-189`; research §4 item 13);
//!   not here.
//! - **Buttons** ([`NumberField::button`]): out of the Tab order, named
//!   "Increase" and "Decrease" ([`NumberNames`]). A mouse press steps at
//!   once; held [`REPEAT_AFTER`], it repeats every [`REPEAT_EVERY`] until
//!   released or until the button is disabled (a bound reached;
//!   `NumberField/utils.ts:8-9,40-48,73-78`). A touch or pen steps nothing
//!   as it goes down: released before [`REPEAT_AFTER`] it steps once (a
//!   tap), held still that long it steps and repeats, and moved past
//!   [`TOUCH_TOLERANCE`] or taken by the platform it is cancelled
//!   (`NumberField/utils.ts:80-109`); held long it opens no context menu. Each step focuses the input
//!   unless told otherwise ([`NumberConfig::focus_on_change`]). A button is
//!   disabled when a step could not give a larger (smaller) value; with no
//!   value both are enabled (`NumberFieldRoot.vue:106-117`). Assistive
//!   technology's press steps once.
//! - **Wheel** ([`NumberConfig::wheel`], `NumberFieldInput.vue:69-89`): on
//!   the focused input only, one step per event whatever its size, none
//!   when it moves more across than up or down; down steps up unless
//!   inverted. Consumed then (read-only too, stepping nothing); otherwise
//!   it scrolls what is under the field.
//! - **ARIA.** The root is a `group`; the input a `spinbutton` with the raw
//!   value (none while empty) and the bounds (none told without one),
//!   described as a "Number
//!   field" (`aria-roledescription`); assistive technology's
//!   Increment and Decrement step it as ArrowUp and ArrowDown do.
//! - **Form.** The raw number (`5`, `0.05`), not the formatted text; `name=`
//!   while empty; nothing while disabled (`f64`'s [`crate::form::Submit`]).
//! - **Locale.** The format's ([`NumberFormat::locale`]): it writes and
//!   reads the locale's separators, digits and patterns
//!   ([`crate::number_format`]; `de-DE` reads `1.234` as 1234).

use std::time::Duration;

use gui::{Div, DragPhase, Key, KeyEvent, NamedKey, Role, Signal, TextField, TextInput, UseModel, ValueRange, watch};

use crate::availability::Availability;
use crate::number_format::NumberFormat;

/// How long a button is held before it repeats (`NumberField/utils.ts:8`).
pub const REPEAT_AFTER: Duration = Duration::from_millis(400);
/// How often a held button repeats (`NumberField/utils.ts:9`).
pub const REPEAT_EVERY: Duration = Duration::from_millis(60);

/// How many decimals `x` is written with (at most 15).
fn decimals(x: f64) -> i32 {
    let mut scaled = x.abs();
    let mut count: i32 = 0;
    while count < 15 && (scaled - scaled.round()).abs() > 1e-9 * scaled.max(1.0) {
        scaled *= 10.0;
        count = count.saturating_add(1);
    }
    count
}

/// `v` rounded to `places` decimals.
fn round_to(v: f64, places: i32) -> f64 {
    let scale = 10_f64.powi(places);
    (v * scale).round() / scale
}

/// `a + b` in exact decimals (`handleDecimalOperation`).
fn add(a: f64, b: f64) -> f64 {
    round_to(a + b, decimals(a).max(decimals(b)))
}

/// `v` clamped to whichever of `min` and `max` there are.
fn clamp(v: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let v = min.map_or(v, |m| v.max(m));
    max.map_or(v, |m| v.min(m))
}

/// `v` on the grid of `step` from `min` (or 0), nearest line; above `max`,
/// the last line at or under it (`shared/clamp.ts:68-90`).
#[must_use]
pub fn snap_to_step(v: f64, min: Option<f64>, max: Option<f64>, step: f64) -> f64 {
    let places = decimals(step).max(min.map_or(0, decimals));
    let base = min.unwrap_or(0.0);
    let lines = ((v - base) / step).round();
    let mut snapped = round_to(base + lines * step, places);
    if let Some(m) = min
        && snapped < m
    {
        snapped = m;
    } else if let Some(x) = max
        && snapped > x
    {
        snapped = round_to(base + (round_to((x - base) / step, places)).floor() * step, places);
    }
    snapped
}

/// Which way a step goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StepDirection {
    Up,
    Down,
}

/// What a screen reader is told of the buttons: reka's English names by
/// default (`NumberFieldIncrement.vue:35`, `NumberFieldDecrement.vue:35`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberNames {
    pub increase: String,
    pub decrease: String,
    /// What the input's role is called (`aria-roledescription`).
    pub field: String,
}

impl Default for NumberNames {
    fn default() -> Self {
        Self {
            increase: "Increase".to_owned(),
            decrease: "Decrease".to_owned(),
            field: "Number field".to_owned(),
        }
    }
}

/// What the wheel over the focused input does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum WheelSteps {
    /// It scrolls past the field.
    Off,
    /// A turn down (towards the user) steps up, as reka's.
    #[default]
    DownIncreases,
    /// A turn down steps down (`invertWheelChange`).
    DownDecreases,
}

impl WheelSteps {
    /// The step a wheel event of `(dx, dy)` makes: none when off, or when
    /// it moves more across than down or up (a trackpad's sideways scroll).
    #[must_use]
    pub fn of(self, dx: f32, dy: f32) -> Option<StepDirection> {
        if self == Self::Off || dy.abs() <= dx.abs() {
            return None;
        }
        let down = dy > 0.0;
        Some(if down == (self == Self::DownIncreases) {
            StepDirection::Up
        } else {
            StepDirection::Down
        })
    }
}

/// How far a touch or pen held on a button moves before its hold is
/// cancelled, logical pixels (`NumberField/utils.ts:10`).
pub const TOUCH_TOLERANCE: f32 = gui::gesture::SLOP;

/// A number field's settings (reka's props; defaults as reka's).
#[derive(Clone, Debug, PartialEq)]
pub struct NumberConfig {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: f64,
    /// Values snap to the step's grid (`stepSnapping`, on).
    pub snap: bool,
    /// A typed value out of range or off the grid is kept (`allowInvalid`, off).
    pub allow_out_of_range: bool,
    /// Unparsable text keeps the value (`keepValueOnUnparsableInput`, off).
    pub keep_unparsable: bool,
    /// A step focuses the input (`focusOnChange`, on).
    pub focus_on_change: bool,
    /// What a step from no value starts at (`startingValue`; none: min, or 0).
    pub start: Option<f64>,
    /// What the wheel does on the focused input (`disableWheelChange`,
    /// `invertWheelChange`; down steps up by default).
    pub wheel: WheelSteps,
    pub format: NumberFormat,
    pub availability: Availability,
    pub read_only: bool,
}

impl Default for NumberConfig {
    fn default() -> Self {
        Self {
            min: None,
            max: None,
            step: 1.0,
            snap: true,
            allow_out_of_range: false,
            keep_unparsable: false,
            focus_on_change: true,
            start: None,
            wheel: WheelSteps::DownIncreases,
            format: NumberFormat::default(),
            availability: Availability::Enabled,
            read_only: false,
        }
    }
}

impl NumberConfig {
    /// `v` clamped and, when snapping, on the grid, then at the format's
    /// precision (`clampInputValue`).
    #[must_use]
    pub fn fit(&self, v: f64) -> f64 {
        let fitted = if self.snap && self.step > 0.0 {
            snap_to_step(v, self.min, self.max, self.step)
        } else {
            clamp(v, self.min, self.max)
        };
        self.format.normalise(fitted)
    }

    /// Where a step `count` steps `dir` from `from` lands (`getNextValue`):
    /// off the grid, first to its line that way.
    #[must_use]
    pub fn next(&self, dir: StepDirection, from: f64, count: u32) -> f64 {
        let step = self.step;
        let signed = |n: f64| if dir == StepDirection::Up { n } else { -n };
        let count = f64::from(count);
        let to = if self.snap && step > 0.0 {
            let snapped = snap_to_step(from, self.min, self.max, step);
            if snapped == from {
                add(from, signed(step * count))
            } else {
                let aligned = match dir {
                    StepDirection::Up if snapped > from => snapped,
                    StepDirection::Down if snapped < from => snapped,
                    _ => add(snapped, signed(step)),
                };
                if count > 1.0 {
                    add(aligned, signed(step * (count - 1.0)))
                } else {
                    aligned
                }
            }
        } else {
            add(from, signed(step * count))
        };
        self.fit(to)
    }

    /// What blank or unparsable text, stepped, takes.
    #[must_use]
    pub fn starting(&self) -> f64 {
        match self.start {
            Some(s) => self.format.normalise(clamp(s, self.min, self.max)),
            None => self.fit(self.min.unwrap_or(0.0)),
        }
    }

    /// The value `text` (the input's, draft or not) stepped `count` steps
    /// `dir`; none when it would move away from the range it is out of.
    #[must_use]
    pub fn stepped(&self, text: &str, dir: StepDirection, count: u32) -> Option<f64> {
        let Some(now) = self.format.parse(text) else {
            return Some(self.starting());
        };
        let next = self.next(dir, now, count);
        let backwards = match dir {
            StepDirection::Up => next < now,
            StepDirection::Down => next > now,
        };
        (!backwards).then_some(next)
    }

    /// Whether a step `dir` from `value` could give a larger (smaller) one:
    /// a button's enabled state. No value: both can.
    #[must_use]
    pub fn can_step(&self, dir: StepDirection, value: f64) -> bool {
        if value.is_nan() {
            return true;
        }
        let next = self.next(dir, value, 1);
        match dir {
            StepDirection::Up => next > value,
            StepDirection::Down => next < value,
        }
    }

    /// What committing `text` does: the value it gives (NaN clears it), or
    /// none to keep the value as it is.
    #[must_use]
    pub fn committed(&self, text: &str) -> Option<f64> {
        if text.trim().is_empty() {
            return Some(f64::NAN);
        }
        match self.format.parse(text) {
            None if self.keep_unparsable => None,
            None => Some(f64::NAN),
            Some(v) if self.allow_out_of_range => Some(self.format.normalise(v)),
            Some(v) => Some(self.fit(v)),
        }
    }

    /// Whether a minus may be typed.
    fn negative(&self) -> bool {
        self.allow_out_of_range || self.min.is_none_or(|m| m < 0.0)
    }

    /// Whether the field takes the user's writes.
    fn writable(&self) -> bool {
        self.availability.actionable() && !self.read_only
    }
}

/// Values equal as a number field holds them: NaN (no value) equals NaN.
fn same(a: f64, b: f64) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}

/// A button being held: the timer of its next repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hold {
    dir: StepDirection,
    timer: gui::clock::TimerId,
}

/// A touch or pen contact on a button (reka's `usePressedHold` for a
/// pointer that is not a mouse): it steps on release unless its hold
/// stepped already, and a move past [`TOUCH_TOLERANCE`] cancels it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Contact {
    id: u64,
    start: (f32, f32),
    /// Its hold has stepped: the release steps no more.
    held: bool,
}

/// A number field: made once in its root's setup, `Copy`.
pub struct NumberField<P: 'static> {
    value: UseModel<P, f64>,
    /// The input's text: the value formatted, or a draft.
    field: TextField,
    config: Signal<NumberConfig>,
    hold: Signal<Option<Hold>>,
    /// Whether the last press of a button was the pointer's, so its click
    /// steps nothing more.
    pointer: Signal<bool>,
    /// The touch or pen contact on a button, while there is one.
    contact: Signal<Option<Contact>>,
    names: Signal<NumberNames>,
}

impl<P: 'static> Clone for NumberField<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for NumberField<P> {}

impl<P: 'static> std::fmt::Debug for NumberField<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumberField").finish_non_exhaustive()
    }
}

impl<P: 'static> NumberField<P> {
    /// A field showing `value` (its `v-model`) by `config`; called in setup.
    /// What was typed commits when the input loses focus; a value written
    /// from outside (a form's reset) shows formatted.
    #[must_use]
    #[track_caller]
    pub fn new(value: UseModel<P, f64>, config: NumberConfig) -> Self {
        let field = TextField::new(config.format.format(value.get_untracked()));
        let n = Self {
            value,
            field,
            config: Signal::labeled("number field config", config),
            hold: Signal::labeled("number field hold", None),
            pointer: Signal::labeled("number field pointer press", false),
            contact: Signal::labeled("number field contact", None),
            names: Signal::labeled("number field names", NumberNames::default()),
        };
        watch(
            move || field.focus.is_focused(),
            move |focused, _| {
                if !*focused {
                    n.commit();
                }
            },
        );
        watch(move || value.get(), move |v, _| n.show(*v));
        n
    }

    /// Its settings from now on.
    pub fn set_config(self, config: NumberConfig) {
        self.config.set(config);
    }

    /// Name the buttons by `names` from now on.
    pub fn set_names(self, names: NumberNames) {
        self.names.set(names);
    }

    /// Its settings (tracked).
    #[must_use]
    pub fn config(self) -> NumberConfig {
        self.config.get()
    }

    /// The input's text state, for the input element.
    #[must_use]
    pub fn field(self) -> TextField {
        self.field
    }

    /// The value; NaN while there is none (tracked).
    #[must_use]
    pub fn value(self) -> f64 {
        self.value.get()
    }

    fn set(self, v: f64) {
        if !same(v, self.value.get_untracked()) {
            self.value.set(v);
        }
    }

    /// Show `v` formatted in the input, unless it shows that already.
    fn show(self, v: f64) {
        let shown = self.config.get_untracked().format.format(v);
        if self.field.edit.with_untracked(|e| e.text() != shown) {
            self.field.set_text(shown);
        }
    }

    /// Take the input's text as the value, as Enter and leaving the input
    /// do, and show the value formatted.
    pub fn commit(self) {
        let c = self.config.get_untracked();
        if !c.writable() {
            self.show(self.value.get_untracked());
            return;
        }
        let text = self.field.edit.with_untracked(|e| e.text().to_owned());
        if let Some(v) = c.committed(&text) {
            self.set(v);
        }
        self.show(self.value.get_untracked());
    }

    /// Step `count` steps `dir` from the input's text, draft or not.
    pub fn step(self, dir: StepDirection, count: u32) {
        let c = self.config.get_untracked();
        if c.focus_on_change {
            self.field.focus.focus();
        }
        if !c.writable() {
            return;
        }
        let text = self.field.edit.with_untracked(|e| e.text().to_owned());
        if let Some(v) = c.stepped(&text, dir, count) {
            self.set(v);
            self.show(v);
        }
    }

    /// Min (or, `max`, max), when there is one, as Home and End do.
    fn to_end(self, max: bool) {
        let c = self.config.get_untracked();
        if !c.writable() {
            return;
        }
        let end = if max { c.max } else { c.min };
        if let Some(e) = end {
            let v = c.fit(e);
            self.set(v);
            self.show(v);
        }
    }

    /// Whether the button `dir` can step now (tracked): a look reads it,
    /// and the button is disabled while it cannot.
    #[must_use]
    pub fn can_step(self, dir: StepDirection) -> bool {
        let c = self.config.get();
        c.writable() && c.can_step(dir, self.value.get())
    }

    /// What a key on the input does; whether it was the field's. Enter
    /// commits and is left to others (a form's submit).
    fn key(self, ev: &KeyEvent) -> bool {
        if self.field.edit.with_untracked(|e| e.preedit().is_some()) {
            return false;
        }
        let m = ev.modifiers;
        let plain = !(m.control || m.alt || m.command);
        match &ev.key {
            Key::Named(NamedKey::ArrowUp) if plain => self.step(StepDirection::Up, 1),
            Key::Named(NamedKey::ArrowDown) if plain => self.step(StepDirection::Down, 1),
            Key::Named(NamedKey::PageUp) if plain => self.step(StepDirection::Up, 10),
            Key::Named(NamedKey::PageDown) if plain => self.step(StepDirection::Down, 10),
            Key::Named(NamedKey::Home) if plain => self.to_end(false),
            Key::Named(NamedKey::End) if plain => self.to_end(true),
            Key::Named(NamedKey::Enter) => {
                self.commit();
                return false;
            }
            _ => return self.refuses(ev),
        }
        true
    }

    /// Whether typing `ev` would leave text that is not a number in
    /// progress: then it is consumed, and nothing is typed.
    fn refuses(self, ev: &KeyEvent) -> bool {
        let m = ev.modifiers;
        if m.control || m.command {
            return false;
        }
        let Some(typed) = ev.text.as_deref().filter(|t| !t.chars().all(char::is_control)) else {
            return false;
        };
        let c = self.config.get_untracked();
        let next = self.field.edit.with_untracked(|e| {
            let (a, b) = e.selection();
            let mut t = e.text().to_owned();
            t.replace_range(a..b, typed);
            t
        });
        !c.format.partial(&next, c.negative())
    }

    /// `d` as the root: a `group`.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        d.role(Role::Group)
    }

    /// `input` (the text input showing [`Self::field`]) as the field's
    /// spin button: its value and bounds, availability, read-only state and
    /// keys. Read-only or unavailable, the input is read-only itself: it
    /// takes focus and selection but no typing, paste or composition, and
    /// assistive technology hears it is read-only (reka's `readonly`
    /// input). It sets the node's key handler.
    #[must_use]
    pub fn input(self, input: TextInput) -> TextInput {
        let c = self.config.get();
        let now = self.value.get();
        let described = self.names.with(|n| n.field.clone());
        let input = input
            .role(Role::SpinButton)
            .aria_role_description(described)
            .aria_value_range(ValueRange {
                now: (!now.is_nan()).then_some(now),
                min: c.min.unwrap_or(f64::NEG_INFINITY),
                max: c.max.unwrap_or(f64::INFINITY),
                text: None,
            });
        let input = match c.availability {
            Availability::Enabled => input,
            Availability::Unavailable => input.unavailable(true),
            Availability::Disabled => input.disabled(true),
        };
        // Focusable and not to be written: typing is refused by the input.
        let input = if c.availability.focusable() && !c.writable() {
            input.read_only()
        } else {
            input
        };
        input
            .on_key_down(move |ev| self.key(ev))
            .on_wheel(move |ev| self.wheel(ev))
    }

    /// What the wheel over the input does; whether it consumed it. Only
    /// while the input is focused: one step per event, whatever its size
    /// (`NumberFieldInput.vue:69-89`); otherwise it scrolls past.
    fn wheel(self, ev: &gui::WheelEvent) -> bool {
        let c = self.config.get_untracked();
        if !gui::untracked(|| self.field.focus.is_focused()) {
            return false;
        }
        let Some(dir) = c.wheel.of(ev.dx, ev.dy) else {
            return false;
        };
        // Read-only, it is consumed and steps nothing, as reka's.
        if c.writable() {
            self.step(dir, 1);
        }
        true
    }

    /// Repeat the held button's step every [`REPEAT_EVERY`] while it can.
    fn repeat(self, dir: StepDirection, after: Duration) {
        let timer = gui::clock::after(after, move || {
            if self.hold.get_untracked().is_none_or(|h| h.dir != dir) {
                return;
            }
            if !gui::untracked(|| self.can_step(dir)) {
                self.release();
                return;
            }
            self.step(dir, 1);
            if let Some(c) = self.contact.get_untracked() {
                self.contact.set(Some(Contact { held: true, ..c }));
            }
            self.repeat(dir, REPEAT_EVERY);
        });
        self.hold.set(Some(Hold { dir, timer }));
    }

    fn release(self) {
        if let Some(h) = self.hold.get_untracked() {
            gui::clock::cancel(h.timer);
            self.hold.set(None);
        }
    }

    /// A touch or pen contact on the button `dir`: nothing as it goes
    /// down; held [`REPEAT_AFTER`] it steps and repeats; released before,
    /// it steps once (a tap); moved past [`TOUCH_TOLERANCE`] or taken by
    /// the platform, it is cancelled (`NumberField/utils.ts:80-109`).
    fn touch(self, dir: StepDirection, ev: &gui::TouchEvent) {
        use gui::TouchPhase;
        match ev.phase {
            TouchPhase::Started => {
                // One contact holds the field's buttons at a time: another
                // that comes meanwhile (on the other button; the engine
                // gives none on the held one) restarts nothing, as in Chrome.
                if self.contact.get_untracked().is_some() || !gui::untracked(|| self.can_step(dir)) {
                    return;
                }
                self.contact.set(Some(Contact {
                    id: ev.id,
                    start: (ev.x, ev.y),
                    held: false,
                }));
                self.repeat(dir, REPEAT_AFTER);
            }
            TouchPhase::Moved => {
                if let Some(c) = self.contact.get_untracked().filter(|c| c.id == ev.id) {
                    let (dx, dy) = (ev.x - c.start.0, ev.y - c.start.1);
                    if dx * dx + dy * dy > TOUCH_TOLERANCE * TOUCH_TOLERANCE {
                        self.contact.set(None);
                        self.release();
                    }
                }
            }
            TouchPhase::Ended => {
                let Some(c) = self.contact.get_untracked().filter(|c| c.id == ev.id) else {
                    return;
                };
                self.contact.set(None);
                self.release();
                if !c.held && gui::untracked(|| self.can_step(dir)) {
                    self.step(dir, 1);
                }
            }
            TouchPhase::Cancelled => {
                if self.contact.get_untracked().is_some_and(|c| c.id == ev.id) {
                    self.contact.set(None);
                    self.release();
                }
            }
        }
    }

    /// Whether the button `dir` is held now (tracked).
    #[must_use]
    pub fn held(self, dir: StepDirection) -> bool {
        self.hold.get().is_some_and(|h| h.dir == dir)
    }

    /// `d` as the button that steps `dir`: named, out of the Tab order,
    /// disabled while it cannot step (tracked), stepping on a press and
    /// repeating while held, once for assistive technology's press.
    #[must_use]
    pub fn button(self, d: Div, dir: StepDirection) -> Div {
        let can = self.can_step(dir);
        let name = self.names.with(|n| match dir {
            StepDirection::Up => n.increase.clone(),
            StepDirection::Down => n.decrease.clone(),
        });
        d.role(Role::Button)
            .aria_label(name)
            .disabled(!can)
            .on_touch(move |ev| self.touch(dir, ev))
            .on_mouse_down(move |_| {
                self.pointer.set(true);
                // A touch or pen tap's press: its contact did the stepping.
                if gui::untracked(|| self.contact.get().is_some()) {
                    return;
                }
                if gui::untracked(|| self.can_step(dir)) {
                    self.step(dir, 1);
                    self.repeat(dir, REPEAT_AFTER);
                }
            })
            .on_drag(move |ev| {
                if ev.phase.is_last() && self.contact.get_untracked().is_none() {
                    self.release();
                }
                // A press that ends with no click after it (cancelled as the
                // button turned disabled at the end of the range, or released
                // off the button) lets go of its mark, so the next press of
                // assistive technology steps.
                let clicks = ev.phase == DragPhase::End && ev.bounds.contains(ev.x, ev.y);
                if ev.phase.is_last() && !clicks {
                    self.pointer.set(false);
                }
            })
            .on_click(move |_| {
                // The pointer's press stepped already; a screen reader's steps now.
                if self.pointer.get_untracked() {
                    self.pointer.set(false);
                } else {
                    self.step(dir, 1);
                }
            })
            // Held long, a contact opens no context menu (`NumberFieldIncrement.vue:37-39`).
            .on_long_press(|_| {})
    }
}

#[cfg(test)]
mod tests {
    use super::{NumberConfig, NumberFormat, StepDirection, snap_to_step};

    fn config(min: Option<f64>, max: Option<f64>, step: f64) -> NumberConfig {
        NumberConfig {
            min,
            max,
            step,
            ..NumberConfig::default()
        }
    }

    #[test]
    fn snapping_counts_from_min_and_stays_under_max() {
        assert_eq!(snap_to_step(20.5, Some(2.0), Some(21.0), 3.0), 20.0);
        assert_eq!(
            snap_to_step(22.0, Some(2.0), Some(21.0), 3.0),
            20.0,
            "max off the grid is not reached"
        );
        assert_eq!(snap_to_step(0.30000000000000004, None, None, 0.1), 0.3);
        assert_eq!(snap_to_step(1.0, Some(2.0), None, 3.0), 2.0);
        let unsnapped = NumberConfig {
            snap: false,
            ..config(Some(2.0), Some(21.0), 3.0)
        };
        assert_eq!(unsnapped.fit(22.0), 21.0, "without snapping, max is reached");
    }

    #[test]
    fn steps_align_to_the_grid_in_their_direction_first() {
        let c = config(None, None, 1.0);
        let c2 = NumberConfig {
            format: NumberFormat {
                max_fraction: 2,
                ..NumberFormat::default()
            },
            ..c
        };
        assert_eq!(c2.next(StepDirection::Up, 18.98, 1), 19.0);
        assert_eq!(c2.next(StepDirection::Down, 18.11, 1), 18.0);
        assert_eq!(
            c2.next(StepDirection::Up, 18.98, 10),
            28.0,
            "the page keys add the other nine"
        );
        assert_eq!(c.next(StepDirection::Up, 5.0, 1), 6.0);
        let tenth = config(None, None, 0.1);
        assert_eq!(tenth.next(StepDirection::Up, 0.2, 1), 0.3, "exact decimals");
        // Blank text takes the start: min (snapped), else the caller's (clamped only).
        let bounded = config(Some(2.0), Some(21.0), 3.0);
        assert_eq!(bounded.stepped("", StepDirection::Down, 1), Some(2.0));
        let started = NumberConfig {
            start: Some(40.0),
            ..config(None, Some(100.0), 3.0)
        };
        assert_eq!(started.stepped("", StepDirection::Up, 1), Some(40.0));
        // Out of range: further out does nothing; back is clamped.
        let out = NumberConfig {
            allow_out_of_range: true,
            ..config(Some(0.0), Some(10.0), 1.0)
        };
        assert_eq!(out.stepped("15", StepDirection::Up, 1), None);
        assert_eq!(out.stepped("15", StepDirection::Down, 1), Some(10.0));
        assert!(!bounded.can_step(StepDirection::Up, 20.0), "the last line under max");
        assert!(bounded.can_step(StepDirection::Up, f64::NAN), "no value: both buttons");
    }

    #[test]
    fn commits_clear_keep_or_fit() {
        let c = config(Some(0.0), Some(10.0), 1.0);
        let cleared = |v: Option<f64>| v.is_some_and(f64::is_nan);
        assert!(cleared(c.committed("  ")));
        assert!(cleared(c.committed("-")));
        assert!(cleared(c.committed("1.2.3")));
        assert!(cleared(c.committed("abc")));
        let keep = NumberConfig {
            keep_unparsable: true,
            ..c.clone()
        };
        assert_eq!(keep.committed("abc"), None, "kept, the text put back");
        assert!(cleared(keep.committed("")), "blank always clears");
        assert_eq!(c.committed("12"), Some(10.0));
        assert_eq!(c.committed("3.4"), Some(3.0));
        let eur = NumberConfig {
            format: NumberFormat::currency("EUR", false),
            ..config(None, None, 1.0)
        };
        assert_eq!(
            eur.format.format(eur.committed("7").unwrap_or_default()),
            "EUR\u{a0}7.00"
        );
    }
}
