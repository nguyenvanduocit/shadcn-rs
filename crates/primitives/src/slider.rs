//! One value, or several, picked along a track (reka's `SliderRoot`,
//! `SliderTrack`, `SliderRange`, `SliderThumb`; research §10 "Slider").
//!
//! - **Value.** A list of `f64`, one per thumb (so a decimal grid is held
//!   as snapped, not rounded again to `f32`), kept ascending after each
//!   of the user's writes (`Slider/utils.ts:19-23`). A write that changes
//!   nothing, or brings two neighbours closer than the minimum distance, is
//!   dropped whole, not clamped (`SliderRoot.vue:143-152`). The caller hears
//!   each change ([`gui::UseModel`]'s `on_change`) and, apart, each commit
//!   ([`Slider::set_on_commit`]): every key that changed a value, and one
//!   release of the pointer when the drag changed one (`SliderRoot.vue:40-51,128-147`).
//! - **Snap** ([`snap`]): `round((v − min) / step) · step + min`, rounded to
//!   the larger decimal count of min and step, then clamped, so the grid
//!   counts from min (min 0.5, step 1 gives 1.5) and a max off the grid is
//!   still reached by the arrows (min 0, max 10, step 3: End gives 9, the
//!   grid's last point under max, and an arrow up from 9 gives 10).
//! - **Minimum distance** is counted in steps ([`spaced`]). reka compares
//!   raw floats, so with a decimal step it can refuse a valid write
//!   (0.3 − 0.2 < 0.1; `utils.ts:96-101`, research §4 item 13); here two
//!   neighbours are far enough when they are the steps apart they should be.
//! - **Keys** (on a focused thumb; each consumed, so the page does not
//!   scroll; `SliderImpl.vue:39-55`):
//!
//!   | Key | Result |
//!   | --- | --- |
//!   | an arrow forward / back | ± one step |
//!   | Shift + any arrow | ± ten steps (`SliderRoot.vue:194-196`) |
//!   | PageUp / PageDown | ± ten steps, by the slide's back key |
//!   | Home | the **first** thumb to min, focused, whichever thumb had focus |
//!   | End | the **last** thumb to max, focused |
//!
//!   Which keys are "back" follows where the slider starts ([`Slide`],
//!   `utils.ts:127-132`): from the left, ArrowLeft, ArrowDown and PageDown;
//!   from the right (right to left, or inverted), ArrowRight, ArrowDown and
//!   PageDown; from the bottom, ArrowDown, ArrowLeft, PageDown; from the
//!   top, ArrowUp, ArrowLeft, PageUp. Home and End never turn.
//! - **Pointer** ([`Slider::root`]). A press on a thumb focuses it and
//!   changes nothing; dragging keeps the point grabbed under the pointer
//!   (thumbs inside the track) or the thumb's centre on it (thumbs over the
//!   ends). A press on the track moves the thumb nearest by value (the lower
//!   index on a tie) with its centre under the pointer, focuses it, and
//!   drags on. Dragging past another thumb re-sorts the list, and focus and
//!   the active thumb follow the value being dragged. The drag goes on
//!   outside the slider until release, and ends once (the engine's drag).
//! - **Mapping** ([`value_at`], [`offset`]): with the thumbs inside the
//!   track (shadcn's), the usable length is the track's less one thumb, the
//!   thumb's leading edge on the track's start at min and its trailing edge
//!   on the track's end at max (`SliderHorizontal.vue:31,39-41`).
//! - **ARIA.** Each thumb is a `slider` with its value, min, max and
//!   orientation, named "Minimum" and "Maximum" when there are two, "Value
//!   i of n" when more, nothing when one ([`SliderNames`], `utils.ts:35-44`),
//!   unless its label names it. A disabled slider's thumbs are disabled
//!   themselves: reka leaves them only out of the tab order and says
//!   `aria-disabled` on the root, which has no role (research §4 item 13).
//!   Assistive technology's Increment and Decrement step a thumb as
//!   ArrowUp and ArrowDown do.
//! - **Form** ([`Slider`] is a [`FormValue`], its value [`SliderValues`]):
//!   one number per value, as `name[i]` (`Slider.test.ts:496-499`); nothing
//!   while disabled (`FormField::availability(move || slider.availability())`).

use gui::{
    Axis, Bounds, Direction, Div, DragEvent, DragPhase, FocusHandle, Key, KeyEvent, NamedKey, Role, Signal, UseModel,
    ValueRange,
};

use crate::availability::Availability;
use crate::direction::ReadDirection;
use crate::form::{FormData, FormValue, Submit};

/// A slider's numbers and the way it runs (reka's `min`, `max`, `step`,
/// `minStepsBetweenThumbs`, `orientation`, `inverted`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderConfig {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    /// How many steps apart neighbouring thumbs stay at least.
    pub min_steps_between: u32,
    pub orientation: Axis,
    /// Runs from the other end.
    pub inverted: bool,
}

impl Default for SliderConfig {
    /// 0 to 100 in steps of 1, across, from the start (`SliderRoot.vue:81-92`).
    fn default() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            min_steps_between: 0,
            orientation: Axis::Horizontal,
            inverted: false,
        }
    }
}

/// Where a slider's minimum is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slide {
    FromLeft,
    FromRight,
    FromBottom,
    FromTop,
}

impl Slide {
    /// Where a slider of `orientation`, `inverted` or not, starts in
    /// `direction`: across, it starts on the reading side's start;
    /// vertically, from the bottom, direction aside (`SliderHorizontal.vue:22`,
    /// `SliderVertical.vue:19`).
    #[must_use]
    pub fn of(orientation: Axis, inverted: bool, direction: Direction) -> Self {
        match orientation {
            Axis::Horizontal if (direction == Direction::Rtl) != inverted => Self::FromRight,
            Axis::Horizontal => Self::FromLeft,
            Axis::Vertical if inverted => Self::FromTop,
            Axis::Vertical => Self::FromBottom,
        }
    }

    /// Whether `key` moves the value back (towards min).
    #[must_use]
    pub fn is_back(self, key: NamedKey) -> bool {
        use NamedKey::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, PageDown, PageUp};
        match self {
            Self::FromLeft => matches!(key, ArrowLeft | ArrowDown | PageDown),
            Self::FromRight => matches!(key, ArrowRight | ArrowDown | PageDown),
            Self::FromBottom => matches!(key, ArrowDown | ArrowLeft | PageDown),
            Self::FromTop => matches!(key, ArrowUp | ArrowLeft | PageUp),
        }
    }

    /// The distance of `(x, y)` from the slider's start in `b`, along its
    /// axis, and the axis' length.
    #[must_use]
    pub fn along(self, b: Bounds, x: f32, y: f32) -> (f64, f64) {
        let (x, y) = (f64::from(x - b.x), f64::from(y - b.y));
        let (w, h) = (f64::from(b.width), f64::from(b.height));
        match self {
            Self::FromLeft => (x, w),
            Self::FromRight => (w - x, w),
            Self::FromTop => (y, h),
            Self::FromBottom => (h - y, h),
        }
    }
}

/// What a screen reader names the thumbs by when their label does not:
/// reka's English strings by default (`utils.ts:35-44`).
#[derive(Clone, Debug)]
pub struct SliderNames {
    pub minimum: String,
    pub maximum: String,
    /// Thumb `i` (from 1) of `n`, when there are more than two.
    pub value_of: fn(usize, usize) -> String,
}

impl PartialEq for SliderNames {
    fn eq(&self, other: &Self) -> bool {
        self.minimum == other.minimum
            && self.maximum == other.maximum
            && std::ptr::fn_addr_eq(self.value_of, other.value_of)
    }
}

impl Default for SliderNames {
    fn default() -> Self {
        Self {
            minimum: "Minimum".to_owned(),
            maximum: "Maximum".to_owned(),
            value_of: |i, n| format!("Value {i} of {n}"),
        }
    }
}

impl SliderNames {
    /// The name of thumb `i` (from 0) of `n`; none for a lone thumb.
    #[must_use]
    pub fn of(&self, i: usize, n: usize) -> Option<String> {
        match n {
            0 | 1 => None,
            2 if i == 0 => Some(self.minimum.clone()),
            2 => Some(self.maximum.clone()),
            _ => Some((self.value_of)(i.saturating_add(1), n)),
        }
    }
}

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

/// `v` on the slider's grid: counted in steps from min, rounded to the
/// decimals of min and step, clamped (`SliderRoot.vue:137-139`).
#[must_use]
pub fn snap(v: f64, c: &SliderConfig) -> f64 {
    let steps = if c.step > 0.0 {
        ((v - c.min) / c.step).round()
    } else {
        0.0
    };
    let scale = 10_f64.powi(decimals(c.min).max(decimals(c.step)));
    let snapped = ((steps * c.step + c.min) * scale).round() / scale;
    snapped.clamp(c.min, c.max.max(c.min))
}

/// Whether every pair of neighbours in the ascending `values` is at least
/// `c.min_steps_between` steps apart, counted in steps: the distance may
/// fall short of `need · step` by the rounding error of the subtraction
/// (a few units in the last place of the larger value), never by more.
#[must_use]
pub fn spaced(values: &[f64], c: &SliderConfig) -> bool {
    if c.min_steps_between == 0 || c.step <= 0.0 {
        return true;
    }
    let need = f64::from(c.min_steps_between) * c.step;
    values.windows(2).all(|w| {
        matches!(w, [a, b] if {
            let slack = 8.0 * f64::EPSILON * a.abs().max(b.abs()).max(need);
            b - a + slack >= need
        })
    })
}

/// The index of the value nearest `v`; the lower index on a tie
/// (`utils.ts:54-60`).
#[must_use]
pub fn closest(values: &[f64], v: f64) -> usize {
    let mut best = (0, f64::INFINITY);
    for (i, x) in values.iter().enumerate() {
        let d = (x - v).abs();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

/// The values after thumb `at` is moved to `v` (snapped), sorted, with the
/// index the moved value then has; none when nothing changes or the
/// minimum distance would break.
#[must_use]
pub fn moved(values: &[f64], at: usize, v: f64, c: &SliderConfig) -> Option<(Vec<f64>, usize)> {
    let v = snap(v, c);
    let mut next = values.to_vec();
    let slot = next.get_mut(at)?;
    if *slot == v {
        return None;
    }
    *slot = v;
    next.sort_by(f64::total_cmp);
    if !spaced(&next, c) {
        return None;
    }
    let to = next.iter().position(|x| *x == v)?;
    Some((next, to))
}

/// Where a key on thumb `at` sends a value, by the table above: the thumb
/// it moves and its target (before snapping); none for a key that is not
/// the slider's.
#[must_use]
pub fn key_target(values: &[f64], at: usize, ev: &KeyEvent, slide: Slide, c: &SliderConfig) -> Option<(usize, f64)> {
    let Key::Named(key) = &ev.key else { return None };
    let key = *key;
    let last = values.len().checked_sub(1)?;
    match key {
        NamedKey::Home => return Some((0, c.min)),
        NamedKey::End => return Some((last, c.max)),
        NamedKey::ArrowLeft
        | NamedKey::ArrowRight
        | NamedKey::ArrowUp
        | NamedKey::ArrowDown
        | NamedKey::PageUp
        | NamedKey::PageDown => {}
        _ => return None,
    }
    let page = matches!(key, NamedKey::PageUp | NamedKey::PageDown);
    let steps = if page || ev.modifiers.shift { 10.0 } else { 1.0 };
    let sign = if slide.is_back(key) { -1.0 } else { 1.0 };
    let now = *values.get(at)?;
    Some((at, now + sign * steps * c.step))
}

/// Where `v` sits between min and max, 0 to 1.
#[must_use]
pub fn fraction(v: f64, c: &SliderConfig) -> f64 {
    let span = c.max - c.min;
    if span > 0.0 {
        ((v - c.min) / span).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The value at distance `along` from the slider's start of `length`,
/// thumbs `thumb` long and kept inside the track when `inset` (unsnapped).
#[must_use]
pub fn value_at(along: f64, length: f64, thumb: f64, inset: bool, c: &SliderConfig) -> f64 {
    let (start, usable) = if inset {
        (thumb / 2.0, length - thumb)
    } else {
        (0.0, length)
    };
    let f = if usable > 0.0 {
        ((along - start) / usable).clamp(0.0, 1.0)
    } else {
        0.0
    };
    c.min + f * (c.max - c.min)
}

/// The distance of a thumb's centre at `fraction` from the slider's start.
#[must_use]
pub fn offset(fraction: f64, length: f64, thumb: f64, inset: bool) -> f64 {
    if inset {
        thumb / 2.0 + fraction * (length - thumb)
    } else {
        fraction * length
    }
}

/// A drag under way: the thumb it moves, how far from that thumb's centre
/// it was grabbed, and the values it started from.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    thumb: usize,
    grab: f64,
    start: Vec<f64>,
}

type Commit = Option<gui::Callback<Vec<f64>>>;

/// A slider: made once in its root's setup, `Copy`.
pub struct Slider<P: 'static> {
    value: UseModel<P, Vec<f64>>,
    config: Signal<SliderConfig>,
    availability: Signal<Availability>,
    /// Whether the thumbs stay inside the track, and their length along it.
    thumb: Signal<(bool, f64)>,
    direction: ReadDirection,
    /// Each thumb's focus, by its index: issued as the thumb is first asked
    /// for, let go once there is no such thumb.
    focus: gui::Keyed<usize, FocusHandle>,
    /// The thumb a drag or key moves: the one focused last.
    active: Signal<usize>,
    drag: Signal<Option<Drag>>,
    on_commit: Signal<Commit>,
    names: Signal<SliderNames>,
}

impl<P: 'static> Clone for Slider<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Slider<P> {}

impl<P: 'static> std::fmt::Debug for Slider<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slider").finish_non_exhaustive()
    }
}

impl<P: 'static> Slider<P> {
    /// A slider writing `value` (its `v-model`) in `direction`; called in
    /// setup. Its thumbs are inside the track and 16 px long until told
    /// otherwise ([`Self::set_thumb`]).
    #[must_use]
    #[track_caller]
    pub fn new(value: UseModel<P, Vec<f64>>, direction: impl Into<ReadDirection>) -> Self {
        let slider = Self {
            value,
            config: Signal::labeled("slider config", SliderConfig::default()),
            availability: Signal::labeled("slider availability", Availability::Enabled),
            thumb: Signal::labeled("slider thumb", (true, 16.0)),
            direction: direction.into(),
            focus: gui::Keyed::new(),
            active: Signal::labeled("slider active thumb", 0),
            drag: Signal::labeled("slider drag", None),
            on_commit: Signal::new(None),
            names: Signal::labeled("slider names", SliderNames::default()),
        };
        // A thumb there is no more lets go of its focus.
        gui::watch(
            move || slider.value.with(Vec::len),
            move |n, _| {
                let n = *n;
                slider.focus.retain(|&i, _| i < n);
            },
        );
        // The thumb focused last is the one a drag or key moves.
        gui::watch(gui::focus::focused, move |now, _| {
            if let Some(i) = now.and_then(|f| slider.thumb_of(f)) {
                slider.active.set(i);
            }
        });
        slider
    }

    /// Its numbers and the way it runs from now on.
    pub fn set_config(self, config: SliderConfig) {
        self.config.set(config);
    }

    pub fn set_availability(self, availability: Availability) {
        self.availability.set(availability);
    }

    /// Thumbs kept inside the track (`inset`) or reaching half over its
    /// ends, `length` px along it.
    pub fn set_thumb(self, inset: bool, length: f64) {
        self.thumb.set((inset, length));
    }

    /// Call `f` with the values at each commit: a key that changed one, a
    /// release that ended a drag that changed one (`@value-commit`).
    pub fn set_on_commit(self, f: impl Into<gui::Callback<Vec<f64>>>) {
        let f: gui::Callback<Vec<f64>> = f.into();
        self.on_commit.update(|c| *c = Some(f));
    }

    /// Name the thumbs by `names` from now on.
    pub fn set_names(self, names: SliderNames) {
        self.names.set(names);
    }

    /// The values (tracked).
    #[must_use]
    pub fn values(self) -> Vec<f64> {
        self.value.get()
    }

    /// Its numbers (tracked).
    #[must_use]
    pub fn config(self) -> SliderConfig {
        self.config.get()
    }

    /// Whether it can be used now (tracked): what its form field reads.
    #[must_use]
    pub fn availability(self) -> Availability {
        self.availability.get()
    }

    /// Where it starts now (untracked: read by keys and drags; a look
    /// placing thumbs reads its own direction).
    #[must_use]
    pub fn slide(self) -> Slide {
        let c = self.config.get_untracked();
        Slide::of(c.orientation, c.inverted, self.direction.get())
    }

    /// The span the range part fills, as fractions from the start: from the
    /// start to the value with one thumb, from the lowest to the highest
    /// value with more (`SliderRange.vue:19-24`) (tracked).
    #[must_use]
    pub fn range(self) -> (f64, f64) {
        let c = self.config.get();
        self.value.with(|v| match v.as_slice() {
            [] => (0.0, 0.0),
            [one] => (0.0, fraction(*one, &c)),
            [first, .., last] => (fraction(*first, &c), fraction(*last, &c)),
        })
    }

    /// The focus of thumb `i`.
    #[must_use]
    pub fn focus_of(self, i: usize) -> FocusHandle {
        self.focus.get(&i, FocusHandle::new)
    }

    /// How many thumbs the slider keeps a focus for: those it has (each
    /// let go as its thumb goes).
    #[must_use]
    pub fn kept_thumbs(self) -> usize {
        self.focus.len()
    }

    /// The thumb focused through `focus`, among the thumbs there are.
    fn thumb_of(self, focus: gui::focus::FocusId) -> Option<usize> {
        let thumbs = gui::untracked(|| self.value.with(Vec::len));
        (0..thumbs).find(|&i| self.focus_of(i).id() == focus)
    }

    /// Move thumb `at` to `v`; whether anything changed. Focus follows the
    /// moved value to its index.
    fn write(self, at: usize, v: f64, by_keyboard: bool) -> bool {
        let c = self.config.get_untracked();
        let values = self.value.get_untracked();
        let Some((next, to)) = moved(&values, at, v, &c) else {
            return false;
        };
        self.active.set(to);
        self.value.set(next);
        gui::focus::set_focus(Some(self.focus_of(to).id()), by_keyboard);
        true
    }

    fn commit(self) {
        if let Some(f) = self.on_commit.get_untracked() {
            f.call(self.value.get_untracked());
        }
    }

    /// What a key on thumb `at` does; whether it was the slider's.
    fn key(self, at: usize, ev: &KeyEvent) -> bool {
        if !self.availability.get_untracked().actionable() {
            return false;
        }
        let c = self.config.get_untracked();
        let values = self.value.get_untracked();
        let Some((thumb, target)) = key_target(&values, at, ev, self.slide(), &c) else {
            return false;
        };
        if self.write(thumb, target, true) {
            self.commit();
        } else if thumb != at {
            // Home or End on another thumb, already at its end: focus goes there.
            gui::focus::set_focus(Some(self.focus_of(thumb).id()), true);
        }
        true
    }

    /// What the pointer does on the slider laid out in `b`.
    fn drag_event(self, ev: &DragEvent) {
        let c = self.config.get_untracked();
        let (inset, thumb) = self.thumb.get_untracked();
        let (along, length) = self.slide().along(ev.bounds, ev.x, ev.y);
        match ev.phase {
            DragPhase::Start => {
                let values = self.value.get_untracked();
                let centre = |v: f64| offset(fraction(v, &c), length, thumb, inset);
                // A press on a thumb grabs it; elsewhere, the nearest by value jumps.
                let on = values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i, along - centre(*v)))
                    .filter(|(_, d)| d.abs() <= thumb / 2.0)
                    .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
                let start = self.value.get_untracked();
                match on {
                    Some((i, grabbed)) => {
                        self.active.set(i);
                        self.drag.set(Some(Drag {
                            thumb: i,
                            grab: if inset { grabbed } else { 0.0 },
                            start,
                        }));
                        gui::focus::set_focus(Some(self.focus_of(i).id()), false);
                    }
                    None => {
                        let v = value_at(along, length, thumb, inset, &c);
                        let i = closest(&values, v);
                        self.active.set(i);
                        self.drag.set(Some(Drag {
                            thumb: i,
                            grab: 0.0,
                            start,
                        }));
                        if !self.write(i, v, false) {
                            gui::focus::set_focus(Some(self.focus_of(i).id()), false);
                        }
                    }
                }
            }
            DragPhase::Move | DragPhase::End | DragPhase::Cancel => {
                let Some(drag) = self.drag.get_untracked() else { return };
                let v = value_at(along - drag.grab, length, thumb, inset, &c);
                if ev.phase == DragPhase::Move {
                    self.write(self.active.get_untracked(), v, false);
                    return;
                }
                // Released or cancelled (the slider was disabled mid-drag),
                // nothing more is written; what the moves wrote is committed.
                self.drag.set(None);
                if self.value.get_untracked() != drag.start {
                    self.commit();
                }
            }
        }
    }

    /// `d` as the slider's root: it takes the pointer, as the availability
    /// set says. It has no role of its own (its thumbs are the controls).
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        let availability = self.availability.get();
        availability.apply(d).on_drag(move |ev| {
            // A slider disabled mid-drag gets the drag's cancel, to let go.
            if availability.actionable() || ev.phase == DragPhase::Cancel {
                self.drag_event(ev);
            }
        })
    }

    /// `d` as thumb `i` of `n`: a focusable `slider` with its value, range,
    /// orientation and default name, disabled with the slider, and its
    /// keys. It sets the node's key handler.
    #[must_use]
    pub fn thumb(self, d: Div, i: usize, n: usize) -> Div {
        let c = self.config.get();
        let now = self.value.with(|v| v.get(i).copied());
        let name = self.names.with(|names| names.of(i, n));
        let d = d
            .role(Role::Slider)
            .aria_value_range(ValueRange {
                now,
                min: c.min,
                max: c.max,
                text: None,
            })
            .aria_orientation(c.orientation)
            .track_focus(self.focus_of(i));
        let d = match name {
            Some(name) => d.aria_label(name),
            None => d,
        };
        self.availability.get().apply(d).on_key_down(move |ev| self.key(i, ev))
    }
}

/// A slider's values as a form sends them: one number per value, under
/// `name[i]` (reka's twin per thumb, `Slider.test.ts:496-499`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SliderValues(pub Vec<f64>);

impl Submit for SliderValues {
    fn form_data(&self) -> FormData {
        FormData::List(self.0.iter().map(f64::to_string).collect())
    }
}

/// A form reads, checks and resets the values; it focuses no one thumb.
impl<P: 'static> FormValue for Slider<P> {
    type Value = SliderValues;

    fn value(&self) -> SliderValues {
        SliderValues(self.value.get())
    }

    fn set_value(&self, value: SliderValues) {
        self.value.set(value.0);
    }
}

#[cfg(test)]
mod tests {
    use gui::events::{Key, KeyEvent, Modifiers, NamedKey};
    use gui::{Axis, Direction};

    use super::{Slide, SliderConfig, SliderNames, closest, key_target, moved, snap, spaced, value_at};

    fn config(min: f64, max: f64, step: f64) -> SliderConfig {
        SliderConfig {
            min,
            max,
            step,
            ..SliderConfig::default()
        }
    }

    #[test]
    fn the_grid_counts_from_min_and_a_max_off_it_is_reached() {
        assert_eq!(snap(1.2, &config(0.5, 10.0, 1.0)), 1.5, "the grid counts from min");
        assert_eq!(snap(0.30000000000000004, &config(0.0, 1.0, 0.1)), 0.3);
        assert_eq!(
            snap(11.0, &config(0.0, 10.0, 3.0)),
            10.0,
            "clamped after snapping: max is reached"
        );
        assert_eq!(snap(9.4, &config(0.0, 10.0, 3.0)), 9.0);
        assert_eq!(
            snap(10.0, &config(0.0, 10.0, 3.0)),
            9.0,
            "End's target, max, rounds to the grid's 9; an arrow from 9 (12) clamps to 10"
        );
        assert_eq!(snap(-4.0, &config(0.0, 10.0, 1.0)), 0.0);
    }

    #[test]
    fn neighbours_are_spaced_by_steps_not_raw_floats() {
        let c = SliderConfig {
            min_steps_between: 1,
            ..config(0.0, 1.0, 0.1)
        };
        // 0.3 − 0.2 is 0.09999999999999998 in floats: reka refuses it.
        assert!(spaced(&[0.2, 0.3], &c));
        assert!(!spaced(&[0.3, 0.3], &c));
        // Far from zero, the subtraction's error grows with the values.
        let far = SliderConfig {
            min_steps_between: 1,
            ..config(1000.0, 2000.0, 0.1)
        };
        assert!(spaced(&[snap(1000.2, &far), snap(1000.3, &far)], &far));
        assert!(!spaced(&[1000.2, 1000.25], &far));
        assert_eq!(moved(&[0.2, 0.5], 1, 0.3, &c), Some((vec![0.2, 0.3], 1)));
        assert_eq!(
            moved(&[0.2, 0.5], 1, 0.2, &c),
            None,
            "too close: the write is dropped whole"
        );
        // A jump over the other thumb is taken when the new spacing holds.
        assert_eq!(moved(&[0.2, 0.5], 0, 0.9, &c), Some((vec![0.5, 0.9], 1)));
        assert_eq!(moved(&[0.2, 0.5], 0, 0.2, &c), None, "no change, no write");
    }

    #[test]
    fn the_back_keys_follow_where_the_slider_starts() {
        use NamedKey::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, End, Home, PageDown, PageUp};
        let c = config(0.0, 100.0, 1.0);
        let key = |k, shift| KeyEvent {
            key: Key::Named(k),
            text: None,
            modifiers: Modifiers {
                shift,
                ..Modifiers::default()
            },
        };
        let ltr = Slide::of(Axis::Horizontal, false, Direction::Ltr);
        let rtl = Slide::of(Axis::Horizontal, false, Direction::Rtl);
        let top = Slide::of(Axis::Vertical, true, Direction::Ltr);
        assert_eq!(ltr, Slide::FromLeft);
        assert_eq!(rtl, Slide::FromRight);
        assert_eq!(Slide::of(Axis::Horizontal, true, Direction::Rtl), Slide::FromLeft);
        assert_eq!(Slide::of(Axis::Vertical, false, Direction::Rtl), Slide::FromBottom);
        let v = [50.0, 60.0];
        // (key, shift, slide, thumb, target)
        let table = [
            (ArrowRight, false, ltr, 1, 61.0),
            (ArrowLeft, false, ltr, 1, 59.0),
            (ArrowUp, false, ltr, 1, 61.0),
            (ArrowLeft, true, ltr, 1, 50.0),
            (PageUp, false, ltr, 1, 70.0),
            (PageDown, false, ltr, 1, 50.0),
            (ArrowLeft, false, rtl, 1, 61.0),
            (ArrowRight, false, rtl, 1, 59.0),
            (ArrowDown, false, top, 1, 61.0),
            (PageDown, false, top, 1, 70.0),
            (ArrowUp, false, top, 1, 59.0),
            (Home, false, rtl, 0, 0.0),
            (End, false, top, 1, 100.0),
        ];
        for (k, shift, slide, thumb, target) in table {
            assert_eq!(
                key_target(&v, 1, &key(k, shift), slide, &c),
                Some((thumb, target)),
                "{k:?} shift={shift} {slide:?}"
            );
        }
    }

    #[test]
    fn the_pointer_maps_with_thumbs_inside_the_track() {
        let c = config(0.0, 100.0, 1.0);
        // A 216 px slider, 16 px thumbs: 200 px of travel from 8 px in.
        assert_eq!(value_at(8.0, 216.0, 16.0, true, &c), 0.0);
        assert_eq!(value_at(108.0, 216.0, 16.0, true, &c), 50.0);
        assert_eq!(value_at(216.0, 216.0, 16.0, true, &c), 100.0);
        assert_eq!(value_at(108.0, 216.0, 16.0, false, &c), 50.0);
        assert_eq!(closest(&[20.0, 40.0], 30.0), 0, "a tie goes to the lower index");
        assert_eq!(closest(&[20.0, 40.0], 31.0), 1);
    }

    #[test]
    fn thumbs_are_named_by_count() {
        let n = SliderNames::default();
        assert_eq!(n.of(0, 1), None);
        assert_eq!(n.of(0, 2).as_deref(), Some("Minimum"));
        assert_eq!(n.of(1, 2).as_deref(), Some("Maximum"));
        assert_eq!(n.of(2, 3).as_deref(), Some("Value 3 of 3"));
    }
}
