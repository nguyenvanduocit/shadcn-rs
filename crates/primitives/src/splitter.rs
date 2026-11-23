//! Panels along one axis whose sizes the handles between them change
//! (reka's `Splitter`: `SplitterGroup`, `SplitterPanel`,
//! `SplitterResizeHandle`; the ARIA window splitter pattern).
//!
//! **Model.** The layout: each panel's size in percent of the group, the
//! sizes summing to 100 (`Splitter/utils/layout.ts:266-271`), the part's
//! `v-model` ([`gui::use_model`]). Each panel has [`Constraints`] (min 0,
//! max 100, collapsible or not, its collapsed size 0, a default size), in
//! percent or in pixels ([`Unit`]); pixels are turned into percent of the
//! room the panels share as last measured (`Splitter/utils/units.ts:68-75`):
//! the group's content box less its handles, where reka takes the whole
//! box and so comes out short by the handles' share. Panels are in the
//! order of their index.
//!
//! **Layout at first, and as panels come and go** (`Splitter/SplitterGroup.vue:340-400`),
//! worked out after the first layout pass that follows (so panels mounting
//! together join first, and a pixel panel has the room; until then the
//! caller's layout or the default shows): the layout saved for these panels if the group saves one ([`Storage`]);
//! else each panel with a default size keeps it, the rest share what is
//! left evenly (`Splitter/utils/calculate.ts:138-183`); either is then made
//! valid ([`validate`]: scaled to 100, each panel clamped, what is left
//! over given to the first panel that takes it,
//! `Splitter/utils/validation.ts:33-94`) and committed, so the caller is
//! told the first layout too. A panel's default size changed after that is
//! ignored (`Splitter/SplitterPanel.vue:120-132`).
//!
//! **The group resized** (`SplitterGroup.vue:402-458`, `units.ts:90-173`):
//! a pixel panel keeps its pixels and the percent panels share the rest in
//! their proportions. A group first measured under 50 px that then grows
//! past ten times that was measured before its parent laid it out (a
//! nested group): its layout is worked out afresh instead (`:420-424`).
//!
//! **Moving a handle** by d percent (d > 0 towards the end;
//! [`adjust`], `Splitter/utils/layout.ts:20-271`):
//!
//! - |d| is at most what the panels on the growing side can still take;
//! - the panel next to the handle on the shrinking side shrinks first, then
//!   the ones further on; the one next to it on the growing side grows,
//!   what it cannot take going further on;
//! - a collapsible panel pushed under its min snaps to collapsed below the
//!   midpoint of its collapsed size and its min, else to its min; others
//!   stop at their min; all stop at their max; sizes are rounded to 10
//!   digits (`Splitter/utils/resizePanel.ts:21-36`);
//! - nothing moved, or a total other than 100, keeps the layout.
//!
//! A drag measures d from the layout as it began, not added up move by
//! move, so a handle dragged past a limit and back stays under the pointer
//! (`Splitter/SplitterGroup.vue:470`, `:500`). A key collapses or expands a
//! collapsible panel in one press (`layout.ts:63-118`). The key step is a
//! percent of the group, for pixel panels too (reka's quirk, kept).
//!
//! **Told to the caller** after every change: the layout in percent (the
//! model) and in each panel's own unit ([`Splitter::on_layout`], reka's
//! `@layout`); each panel its new size and the one it was last told
//! (`@resize`), and its collapsing or expanding (`@collapse`, `@expand`),
//! the first telling included (`Splitter/utils/callPanelCallbacks.ts:25-50`).
//! A group that saves its layout writes it [`SAVE_DELAY`] (100 ms) after the
//! last change (`SplitterGroup.vue:31`, `:267-302`), keyed by the caller's
//! id and its panels' ids, or their order and constraints
//! (`Splitter/utils/storage.ts:53-68`).
//!
//! **Keys** (focus on a handle):
//!
//! | Key | Result |
//! | --- | --- |
//! | Tab | each handle is a Tab stop, an unavailable one too |
//! | ArrowRight, ArrowLeft (horizontal group); ArrowDown, ArrowUp (vertical) | d = ± the step (10 % of the group unless set) |
//! | Shift with those | d = ± 100: as far as it goes |
//! | End, Home | d = + 100, − 100: the panel before the handle as large, as small as it can be |
//! | Enter | the panel before it, if collapsible: collapsed → its size before it collapsed (its min when that was smaller); else collapsed |
//! | F6, Shift+F6 | the next, previous usable handle of the group, wrapping |
//! | an arrow across the axis | nothing |
//!
//! Right to left, ArrowLeft and ArrowRight swap, and a drag's d turns.
//! Home and End do not: they are about the panel before the handle, as the
//! window splitter pattern has them; reka turns them too, so under RTL its
//! Home makes that panel as large as it can be (research §4 item 13). A
//! handle that cannot be used is unavailable, not disabled: it keeps focus
//! and its place in Tab order, is read as disabled (reka tells assistive
//! technology nothing), and moves nothing, by key or pointer; F6 passes it.
//! Enter expanding to the size before collapsing is the caller's `expand`,
//! where reka's Enter expands to the min instead (its docs say the
//! former): one meaning for both (ADR-0025, group 8).
//!
//! **Pointer** (`Splitter/utils/registry.ts:103-334`, `style.ts`): a handle
//! takes the pointer within [`MARGIN_FINE`] (5 px) of its box, or
//! [`MARGIN_COARSE`] (15 px) once a finger or a pen has pressed in the
//! group (reka asks the device once, at load: `registry.ts:34-39`), over
//! the panels beside it too, though not through a node laid over it (one
//! that takes the press or the drag itself). There it is hovered, shows
//! its resize cursor, and a press of any button drags it, the nearest one
//! when two are in reach. While a drag runs the panels and all they hold
//! take no pointer, so nothing in them is hovered
//! ([`gui::Div::pass_through_all`], reka's `pointer-events: none`), the
//! cursor stays the drag's wherever the pointer goes, and a secondary
//! press (the context menu) ends it. A drag pushed past a limit shows the
//! one-way cursor of the way it can still go (VS Code's, as reka has it);
//! reka reads that way after a right to left flip, so it points the wrong
//! way there; here it is the way the pointer can move.
//!
//! **Groups in groups** (`registry.ts:103-334`): a group laid out in
//! another's panel is one family with it (by context). The pointer within
//! reach of handles of several of them reaches them all: they are hovered
//! together and a press drags them together, each along its own axis, so
//! two handles crossing move in two dimensions; the cursor there is the
//! move cursor, or the one-way cursor of both ways still open. A handle is
//! reached through the group the pointer is over, or, just outside a
//! group, through the group around it, not through a node laid over it
//! (reka compares stacking order for that).
//!
//! **To assistive technology.** A handle is a separator, its orientation
//! across the group's (a horizontal group's handles are vertical), its
//! value the size of the panel before it, rounded, between
//! max(min, 100 − Σ other max) and min(max, 100 − Σ other min)
//! (`Splitter/utils/calculate.ts:94-136`), controlling that panel, and
//! named for it: "Resize panel N" for the handle after panel N (from 1)
//! unless the caller names it ([`Splitter::set_name`], a locale's words).
//! reka sets no orientation, no disabled state and no name (research §4
//! item 13), so its handles are read as an unnamed separator.

#[expect(
    clippy::disallowed_types,
    reason = "the group's own, written only by its own layout callbacks, actions and drag handlers, never by a setup or render: the handles' boxes (pruned to those laid out), the sizes before a collapse, those told, the drag partners; the memory storage is the caller's, written only by its `save` from the group's commit"
)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gui::prelude::*;
use gui::{
    Axis, Bounds, CaptureGroup, Direction, Div, DragEvent, DragPhase, FocusHandle, KeyEvent, NamedKey, NodeRef,
    PointerKind, Signal, UseModel,
};

use crate::availability::Availability;
use crate::collection::{Collection, Item};
use crate::direction::ReadDirection;
use crate::hover::Delay;
use crate::keymap::{Intent, Wrap, step};

/// How far a key moves a handle, in percent of the group, unless the
/// caller says otherwise (reka's `keyboardResizeBy`,
/// `Splitter/SplitterGroup.vue:101-105`).
pub const KEY_STEP: f64 = 10.0;

/// The digits sizes are rounded and compared to (`Splitter/utils/constants.ts`).
pub const PRECISION: i32 = 10;

/// How far past its box a handle takes a mouse (`Splitter/SplitterResizeHandle.vue:136-140`).
pub const MARGIN_FINE: f32 = 5.0;

/// How far past its box a handle takes a finger or a pen.
pub const MARGIN_COARSE: f32 = 15.0;

/// How long after the last change a saved layout is written (`Splitter/SplitterGroup.vue:31`).
pub const SAVE_DELAY: Duration = Duration::from_millis(100);

/// A group measured under this at its first layout, then grown past ten
/// times that, works its layout out afresh (`Splitter/SplitterGroup.vue:420-424`).
pub const UNRELIABLE_SIZE: f32 = 50.0;

/// What a panel's sizes are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Unit {
    /// Percent of the group.
    #[default]
    Percent,
    /// Pixels: the panel keeps them as the group resizes.
    Pixels,
}

/// What a panel allows of its size, in its [`Unit`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Constraints {
    pub min: f64,
    /// None: as large as the group (100 %).
    pub max: Option<f64>,
    pub collapsible: bool,
    /// Its size while collapsed.
    pub collapsed: f64,
    /// Its size in the default layout; none shares what is left.
    pub default: Option<f64>,
    pub unit: Unit,
}

impl Default for Constraints {
    fn default() -> Self {
        Self {
            min: 0.0,
            max: None,
            collapsible: false,
            collapsed: 0.0,
            default: None,
            unit: Unit::Percent,
        }
    }
}

impl Constraints {
    /// Its largest size: its max, or the whole group.
    #[must_use]
    pub fn largest(self) -> f64 {
        self.max.unwrap_or(100.0)
    }

    /// These constraints in percent of a group `group` pixels long; none for
    /// pixels before the group has a size (reka's `convertPanelConstraintsToPercent`).
    #[must_use]
    pub fn in_percent(self, group: Option<f32>) -> Option<Self> {
        match self.unit {
            Unit::Percent => Some(self),
            Unit::Pixels => {
                let g = f64::from(group.filter(|g| *g > 0.0)?);
                let p = |v: f64| v / g * 100.0;
                Some(Self {
                    min: p(self.min),
                    max: self.max.map(p),
                    collapsible: self.collapsible,
                    collapsed: p(self.collapsed),
                    default: self.default.map(p),
                    unit: Unit::Percent,
                })
            }
        }
    }

    /// A size in percent of a group `group` pixels long, in this panel's unit.
    #[must_use]
    pub fn native(self, percent: f64, group: Option<f32>) -> f64 {
        match (self.unit, group) {
            (Unit::Pixels, Some(g)) => percent / 100.0 * f64::from(g),
            _ => percent,
        }
    }
}

/// Every panel's constraints in percent; none while a pixel panel waits for
/// the group's size.
#[must_use]
pub fn percent_constraints(constraints: &[Constraints], group: Option<f32>) -> Option<Vec<Constraints>> {
    constraints.iter().map(|c| c.in_percent(group)).collect()
}

/// The layout after a group `prev` pixels long becomes `next` long: each
/// pixel panel keeps its pixels, the percent panels share what is left in
/// their proportions, summing to 100 (reka's
/// `recalculateLayoutForPixelPanels`); none without a pixel panel.
#[must_use]
pub fn relayout_for_pixels(layout: &[f64], constraints: &[Constraints], prev: f32, next: f32) -> Option<Vec<f64>> {
    let pixels: Vec<bool> = constraints.iter().map(|c| c.unit == Unit::Pixels).collect();
    if !pixels.contains(&true) || prev <= 0.0 || next <= 0.0 || layout.len() != constraints.len() {
        return None;
    }
    let (prev, next) = (f64::from(prev), f64::from(next));
    let mut out: Vec<f64> = layout.to_vec();
    let mut pixel_total = 0.0;
    for (size, px) in out.iter_mut().zip(&pixels) {
        if *px {
            *size = *size / 100.0 * prev / next * 100.0;
            pixel_total += *size;
        }
    }
    let percent_before: f64 = layout
        .iter()
        .zip(&pixels)
        .filter(|(_, px)| !**px)
        .map(|(s, _)| *s)
        .sum();
    let available = (100.0 - pixel_total).max(0.0);
    let k = if percent_before > 0.0 {
        available / percent_before
    } else {
        0.0
    };
    for (size, px) in out.iter_mut().zip(&pixels) {
        if !*px {
            *size *= k;
        }
    }
    let total: f64 = out.iter().sum();
    if total > 0.0 && (total - 100.0).abs() > 1e-9 {
        for size in &mut out {
            *size *= 100.0 / total;
        }
    }
    Some(out)
}

/// A percentage as a flex share.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "a percentage in 0..=100, within f32; `as` rounds to nearest"
)]
fn share(percent: f64) -> f32 {
    percent as f32
}

fn round(v: f64) -> f64 {
    let k = 10_f64.powi(PRECISION);
    (v * k).round() / k
}

/// reka's `fuzzyCompareNumbers`: the order of `a` and `b` to [`PRECISION`] digits.
fn compare(a: f64, b: f64) -> std::cmp::Ordering {
    round(a).total_cmp(&round(b))
}

fn same(a: f64, b: f64) -> bool {
    compare(a, b).is_eq()
}

/// What a move is from: a key collapses and expands in one press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Keyboard,
    Pointer,
    Code,
}

/// The size panel `index` may take for `size` (reka's `resizePanel`).
#[must_use]
pub fn resize(constraints: &[Constraints], index: usize, size: f64) -> f64 {
    let c = constraints.get(index).copied().unwrap_or_default();
    let mut size = size;
    if compare(size, c.min).is_lt() {
        size = if c.collapsible {
            // Collapsible panels snap closed or open past the midpoint.
            let half = (c.collapsed + c.min) / 2.0;
            if compare(size, half).is_lt() {
                c.collapsed
            } else {
                c.min
            }
        } else {
            c.min
        };
    }
    round(size.min(c.largest()))
}

/// The layout after moving the handle between panels `pivot` and
/// `pivot + 1` by `delta` percent (reka's `adjustLayoutByDelta`).
#[must_use]
#[expect(
    clippy::indexing_slicing,
    reason = "every index is checked against the layout's length first"
)]
pub fn adjust(delta: f64, layout: &[f64], constraints: &[Constraints], pivot: usize, trigger: Trigger) -> Vec<f64> {
    let n = layout.len();
    let (first, second) = (pivot, pivot.saturating_add(1));
    if same(delta, 0.0) || second >= n || constraints.len() != n {
        return layout.to_vec();
    }
    let mut delta = delta;
    let mut next = layout.to_vec();
    if trigger == Trigger::Keyboard {
        // Expand a collapsed panel to its min in one press.
        let i = if delta < 0.0 { second } else { first };
        let c = constraints[i];
        if c.collapsible && same(layout[i], c.collapsed) {
            let local = c.min - layout[i];
            if compare(local, delta.abs()).is_gt() {
                delta = if delta < 0.0 { -local } else { local };
            }
        }
        // Collapse a panel at its min in one press.
        let i = if delta < 0.0 { first } else { second };
        let c = constraints[i];
        if c.collapsible && same(layout[i], c.min) {
            let local = layout[i] - c.collapsed;
            if compare(local, delta.abs()).is_gt() {
                delta = if delta < 0.0 { -local } else { local };
            }
        }
    }
    // At most what the growing side can take.
    let mut available = 0.0;
    let mut i = if delta < 0.0 { second } else { first };
    loop {
        available += resize(constraints, i, 100.0) - layout[i];
        let Some(at) = (if delta < 0.0 {
            i.checked_add(1)
        } else {
            i.checked_sub(1)
        })
        .filter(|&at| at < n) else {
            break;
        };
        i = at;
    }
    let limited = delta.abs().min(available.abs());
    delta = if delta < 0.0 { -limited } else { limited };
    // Take it from the shrinking side, nearest first.
    let mut applied = 0.0;
    let mut i = Some(if delta < 0.0 { first } else { second });
    while let Some(at) = i.filter(|&at| at < n) {
        let remaining = delta.abs() - applied;
        let safe = resize(constraints, at, layout[at] - remaining);
        if !same(layout[at], safe) {
            applied += layout[at] - safe;
            next[at] = safe;
            if compare(applied, delta.abs()).is_ge() {
                break;
            }
        }
        i = if delta < 0.0 {
            at.checked_sub(1)
        } else {
            at.checked_add(1)
        };
    }
    if same(applied, 0.0) {
        return layout.to_vec();
    }
    // Give it to the growing side, spilling on past a max.
    let grow = if delta < 0.0 { second } else { first };
    let unsafe_size = layout[grow] + applied;
    let safe = resize(constraints, grow, unsafe_size);
    next[grow] = safe;
    if !same(safe, unsafe_size) {
        let mut remaining = unsafe_size - safe;
        let mut i = Some(grow);
        while let Some(at) = i.filter(|&at| at < n) {
            let unsafe_size = next[at] + remaining;
            let safe = resize(constraints, at, unsafe_size);
            if !same(next[at], safe) {
                remaining -= safe - next[at];
                next[at] = safe;
            }
            if same(remaining, 0.0) {
                break;
            }
            i = if delta > 0.0 {
                at.checked_sub(1)
            } else {
                at.checked_add(1)
            };
        }
    }
    if same(next.iter().sum(), 100.0) {
        next
    } else {
        layout.to_vec()
    }
}

/// `layout` made valid for `constraints`: scaled to 100, each panel
/// clamped, the rest given to the first panel that takes it (reka's
/// `validatePanelGroupLayout`).
#[must_use]
pub fn validate(layout: &[f64], constraints: &[Constraints]) -> Vec<f64> {
    let mut next = layout.to_vec();
    let total: f64 = next.iter().sum();
    if !same(total, 100.0) && total > 0.0 {
        for size in &mut next {
            *size *= 100.0 / total;
        }
    }
    let mut remaining = 0.0;
    for (i, size) in next.iter_mut().enumerate() {
        let safe = resize(constraints, i, *size);
        remaining += *size - safe;
        *size = safe;
    }
    if !same(remaining, 0.0) {
        for (i, size) in next.iter_mut().enumerate() {
            let safe = resize(constraints, i, *size + remaining);
            if !same(*size, safe) {
                remaining -= safe - *size;
                *size = safe;
                if same(remaining, 0.0) {
                    break;
                }
            }
        }
    }
    next
}

/// The default layout of `constraints`: default sizes kept, the rest shared
/// evenly (reka's `calculateUnsafeDefaultLayout`), then made valid.
#[must_use]
pub fn default_layout(constraints: &[Constraints]) -> Vec<f64> {
    let given: f64 = constraints.iter().filter_map(|c| c.default).sum();
    let rest = constraints.iter().filter(|c| c.default.is_none()).count();
    let share = if rest == 0 {
        0.0
    } else {
        (100.0 - given) / f64::from(u32::try_from(rest).unwrap_or(u32::MAX))
    };
    let layout: Vec<f64> = constraints.iter().map(|c| c.default.unwrap_or(share)).collect();
    validate(&layout, constraints)
}

/// What a screen reader is told of the handle after panel `pivot`: the
/// panel's size, and the least and most it can be (reka's
/// `calculateAriaValues`).
#[must_use]
pub fn aria_values(layout: &[f64], constraints: &[Constraints], pivot: usize) -> (f64, f64, f64) {
    let own = constraints.get(pivot).copied().unwrap_or_default();
    let (others_min, others_max) = constraints
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != pivot)
        .fold((0.0, 0.0), |(lo, hi), (_, c)| (lo + c.min, hi + c.largest()));
    let max = own.largest().min(100.0 - others_min);
    let min = own.min.max(100.0 - others_max);
    (layout.get(pivot).copied().unwrap_or(0.0), min, max)
}

/// What the caller decides of a group, read when it is used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterOptions {
    /// The axis the panels run along.
    pub orientation: Axis,
    /// How far a key moves a handle, in percent.
    pub step: f64,
}

impl Default for SplitterOptions {
    fn default() -> Self {
        Self {
            orientation: Axis::Horizontal,
            step: KEY_STEP,
        }
    }
}

/// A layout a group saved: its sizes in percent, and each collapsed
/// panel's size before it collapsed, by index.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SavedLayout {
    pub layout: Vec<f64>,
    pub expand_to: Vec<(usize, f64)>,
}

/// Where a group saves its layout between runs (reka's `storage`,
/// `localStorage` by default): the app's file, settings or memory.
pub trait Storage {
    /// The layout saved under `key`, if any.
    fn load(&self, key: &str) -> Option<SavedLayout>;
    /// Save `layout` under `key`.
    fn save(&self, key: &str, layout: SavedLayout);
}

/// A [`Storage`] that lasts as long as the app: layouts survive a group
/// unmounting and mounting again.
#[derive(Clone, Debug, Default)]
#[expect(
    clippy::disallowed_types,
    reason = "the group's own, written only by its own layout callbacks, actions and drag handlers, never by a setup or render: the handles' boxes (pruned to those laid out), the sizes before a collapse, those told, the drag partners; the memory storage is the caller's, written only by its `save` from the group's commit"
)]
pub struct MemoryStorage(Rc<RefCell<HashMap<String, SavedLayout>>>);

impl Storage for MemoryStorage {
    fn load(&self, key: &str) -> Option<SavedLayout> {
        self.0.borrow().get(key).cloned()
    }

    fn save(&self, key: &str, layout: SavedLayout) {
        self.0.borrow_mut().insert(key.to_owned(), layout);
    }
}

type SizeCallback = gui::Callback<(f64, Option<f64>)>;
type LayoutCallback = gui::Callback<Vec<f64>>;
type Notify = gui::Callback<()>;

/// A panel as it joins its group: its constraints, its id for a saved
/// layout's key, and what it is told (reka's `SplitterPanel` props and
/// events).
#[derive(Clone, Default)]
pub struct Panel {
    pub constraints: Constraints,
    /// The caller's id: a saved layout is keyed by it, else by the panel's
    /// order and constraints.
    pub id: Option<Rc<str>>,
    /// Its new size and the one it was last told, in its unit (`@resize`).
    pub on_resize: Option<SizeCallback>,
    /// It collapsed (`@collapse`).
    pub on_collapse: Option<Notify>,
    /// It expanded (`@expand`).
    pub on_expand: Option<Notify>,
}

impl std::fmt::Debug for Panel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Panel")
            .field("constraints", &self.constraints)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl From<Constraints> for Panel {
    fn from(constraints: Constraints) -> Self {
        Self {
            constraints,
            ..Self::default()
        }
    }
}

/// A handle's state, what its look reads (reka's `data-resize-handle-state`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleState {
    Inactive,
    /// The pointer is within its reach.
    Hover,
    Drag,
}

/// Where a drag pushed past a limit and the layout stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Limit {
    /// The pointer went towards the start (left, up) and no further.
    Start,
    End,
}

/// A drag: written as it starts and ends, not as it moves, so what reads
/// it is not run again by each move.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    handle: usize,
    from: f32,
    start: Vec<f64>,
}

/// What a group of a family is asked by the one the pointer acts on, in
/// the window's coordinates.
trait Member {
    fn alive(&self) -> bool;
    fn orientation(&self) -> Axis;
    /// The pointer is over the group (its hover).
    fn inside(&self) -> bool;
    /// The point is in the group's box.
    fn contains(&self, x: f32, y: f32) -> bool;
    /// The usable handle the point reaches, if any.
    fn reach(&self, x: f32, y: f32) -> Option<usize>;
    /// The handle the pointer reaches now.
    fn set_reached(&self, at: Option<usize>);
    /// Hovered or dragged (tracked), and the limit its drag is pushed
    /// past.
    fn engaged(&self) -> bool;
    fn limit(&self) -> Option<Limit>;
    /// Whether a drag of it runs (untracked).
    fn dragging(&self) -> bool;
    /// A drag of `handle` starts with `e`; `e` goes on; it stops.
    fn start(&self, e: &DragEvent, handle: usize);
    fn drag(&self, e: &DragEvent);
    fn stop(&self);
}

#[derive(Clone)]
struct Entry {
    id: gui::Registration,
    parent: Option<gui::Registration>,
    member: Rc<dyn Member>,
}

/// The groups laid out in one another's panels.
struct Family {
    /// Its groups, each while it is mounted.
    members: gui::Registry<Entry>,
    /// Handles of both axes are reached or dragged together.
    crossing: Signal<bool>,
    /// Its groups' nodes, captured together: the pointer that drags one
    /// (a crossing drags its partners too) holds them all, and another
    /// pointer pressing any of them meanwhile drags nothing there.
    capture: CaptureGroup,
}

/// What a group gives the groups in its panels: its family and itself.
#[derive(Clone)]
struct InFamily {
    family: Rc<Family>,
    parent: gui::Registration,
}

impl Family {
    /// The groups whose handles the point reaches, with that handle: through
    /// the group it is over (`me`, acting on it, is), or, just outside a
    /// group, through the group around it.
    fn reached(&self, me: gui::Registration, x: f32, y: f32) -> Vec<(Rc<dyn Member>, usize)> {
        let members = self.members.to_vec_untracked();
        let inside = |id: Option<gui::Registration>| {
            id.and_then(|id| members.iter().find(|e| e.id == id))
                .is_some_and(|e| e.id == me || e.member.inside())
        };
        members
            .iter()
            .filter(|e| e.member.alive())
            .filter_map(|e| {
                let h = e.member.reach(x, y)?;
                let through = inside(Some(e.id)) || (!e.member.contains(x, y) && inside(e.parent));
                through.then(|| (Rc::clone(&e.member), h))
            })
            .collect()
    }

    fn members(&self) -> Vec<Rc<dyn Member>> {
        self.members
            .with_untracked(|l| l.map(|e| Rc::clone(&e.member)).collect())
    }

    /// The pointer at `(x, y)` over group `me`: every group's reach, and
    /// whether two axes cross there.
    fn hover(&self, me: gui::Registration, x: f32, y: f32) {
        let reached = self.reached(me, x, y);
        for m in self.members() {
            let at = reached.iter().find(|(r, _)| Rc::ptr_eq(r, &m)).map(|(_, h)| *h);
            m.set_reached(at);
        }
        self.cross(&reached);
    }

    fn cross(&self, engaged: &[(Rc<dyn Member>, usize)]) {
        let crossing = engaged.iter().any(|(m, _)| m.orientation() == Axis::Horizontal)
            && engaged.iter().any(|(m, _)| m.orientation() == Axis::Vertical);
        if self.crossing.get_untracked() != crossing {
            self.crossing.set(crossing);
        }
    }

    /// The cursor where handles of both axes cross (tracked): the move
    /// cursor, or the one-way cursor of both ways still open
    /// (`Splitter/utils/style.ts:15-58`).
    fn crossing_cursor(&self) -> gui::Cursor {
        use gui::Cursor;
        let (mut h, mut v) = (None, None);
        for m in self.members() {
            if m.alive() && m.engaged() {
                match m.orientation() {
                    Axis::Horizontal => h = h.or(m.limit()),
                    Axis::Vertical => v = v.or(m.limit()),
                }
            }
        }
        match (h, v) {
            (Some(Limit::Start), Some(Limit::Start)) => Cursor::ResizeSouthEast,
            (Some(Limit::Start), Some(Limit::End)) => Cursor::ResizeNorthEast,
            (Some(Limit::End), Some(Limit::Start)) => Cursor::ResizeSouthWest,
            (Some(Limit::End), Some(Limit::End)) => Cursor::ResizeNorthWest,
            (Some(Limit::Start), None) => Cursor::ResizeEast,
            (Some(Limit::End), None) => Cursor::ResizeWest,
            (None, Some(Limit::Start)) => Cursor::ResizeSouth,
            (None, Some(Limit::End)) => Cursor::ResizeNorth,
            (None, None) => Cursor::Move,
        }
    }
}

/// The persisting of a group's layout.
struct Persist {
    id: String,
    storage: Rc<dyn Storage>,
    save: Delay,
}

#[expect(
    clippy::disallowed_types,
    reason = "the group's own, written only by its own layout callbacks, actions and drag handlers, never by a setup or render: the handles' boxes (pruned to those laid out), the sizes before a collapse, those told, the drag partners; the memory storage is the caller's, written only by its `save` from the group's commit"
)]
struct State {
    options: Rc<dyn Fn() -> SplitterOptions>,
    direction: ReadDirection,
    /// Each handle's box as last laid out, by the panel it follows: what
    /// the panels' sizes do not share, and the reach of the pointer. Read
    /// only for handles mounted now.
    handle_boxes: RefCell<HashMap<usize, Bounds>>,
    /// The handles mounted, for F6 and the pointer.
    handles: Collection<usize>,
    /// Each collapsed panel's size before it collapsed.
    before_collapse: RefCell<HashMap<usize, f64>>,
    /// What each panel was last told of its size, in its unit.
    told: RefCell<HashMap<usize, f64>>,
    /// The group's length when its layout was last worked out afresh.
    init_size: std::cell::Cell<Option<f32>>,
    /// The panels changed since the layout was last worked out: the next
    /// layout pass works it out, once every panel of that pass has joined.
    stale: std::cell::Cell<bool>,
    /// The group's content box as this pass measured it, until the pass
    /// has measured the handles too.
    content: std::cell::Cell<Option<f32>>,
    on_layout: gui::Holder<LayoutCallback>,
    persist: gui::Holder<Persist>,
    /// The last delta of the drag, so tiny moves that change nothing do
    /// not flicker the cursor.
    last: std::cell::Cell<f64>,
    /// The pointer is over the group, and its box as last laid out.
    inside: std::cell::Cell<bool>,
    bounds: std::cell::Cell<Option<Bounds>>,
    /// Its family, its id in it, and the groups of it a drag that started
    /// here drags too.
    family: Rc<Family>,
    id: gui::Registration,
    partners: RefCell<Vec<Rc<dyn Member>>>,
    /// A handle's name, from the panel before it (from 1).
    name: std::cell::Cell<fn(usize) -> String>,
}

/// What a screen reader names the handle after panel `n` (from 1) by, in
/// English unless the caller says otherwise ([`Splitter::set_name`]).
#[must_use]
pub fn handle_name(n: usize) -> String {
    format!("Resize panel {n}")
}

/// A splitter group's state: made once in its root's setup, `Copy`.
pub struct Splitter<P: 'static> {
    layout: UseModel<P, Vec<f64>>,
    /// Each panel by index, as the panels registered.
    panels: Signal<Vec<Option<PanelEntry>>>,
    /// Bumped as a panel joins or leaves: the layout is worked out afresh.
    roster: Signal<u64>,
    /// The group's length along its axis, as last laid out.
    size: Signal<Option<f32>>,
    drag: Signal<Option<Drag>>,
    /// The handle the pointer is within reach of.
    hovered: Signal<Option<usize>>,
    /// The drag is pushed past a limit.
    limit: Signal<Option<Limit>>,
    /// A finger or a pen pressed in the group: handles reach further.
    coarse: Signal<bool>,
    state: Signal<State>,
}

/// A registered panel: what compares (its constraints and id) and what it is told.
#[derive(Clone)]
struct PanelEntry {
    panel: Panel,
    /// Its node, made by the panel itself: what its handle's
    /// `aria-controls` names.
    node: NodeRef,
    /// Which registration holds the slot: a panel that replaced one at its
    /// index registers before the one it replaces leaves.
    owner: gui::Registration,
}

impl PartialEq for PanelEntry {
    fn eq(&self, other: &Self) -> bool {
        self.panel.constraints == other.panel.constraints && self.panel.id == other.panel.id && self.node == other.node
    }
}

impl<P: 'static> Clone for Splitter<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Splitter<P> {}

impl<P: 'static> PartialEq for Splitter<P> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl<P: 'static> std::fmt::Debug for Splitter<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Splitter").finish_non_exhaustive()
    }
}

fn cursor_for(orientation: Axis, limit: Option<Limit>) -> gui::Cursor {
    use gui::Cursor;
    match (orientation, limit) {
        (Axis::Horizontal, None) => Cursor::ResizeEastWest,
        (Axis::Vertical, None) => Cursor::ResizeNorthSouth,
        // Stopped going left: it can only go right.
        (Axis::Horizontal, Some(Limit::Start)) => Cursor::ResizeEast,
        (Axis::Horizontal, Some(Limit::End)) => Cursor::ResizeWest,
        (Axis::Vertical, Some(Limit::Start)) => Cursor::ResizeSouth,
        (Axis::Vertical, Some(Limit::End)) => Cursor::ResizeNorth,
    }
}

impl<P: 'static> Splitter<P> {
    /// A group whose layout `layout` holds (the part's `v-model`), with the
    /// caller's `options` read when used, its keys and drags turning in
    /// `direction`. Called in setup.
    #[must_use]
    #[track_caller]
    #[expect(
        clippy::disallowed_types,
        reason = "the group's own, written only by its own layout callbacks, actions and drag handlers, never by a setup or render: the handles' boxes (pruned to those laid out), the sizes before a collapse, those told, the drag partners; the memory storage is the caller's, written only by its `save` from the group's commit"
    )]
    pub fn new(
        layout: UseModel<P, Vec<f64>>,
        options: impl Fn() -> SplitterOptions + 'static,
        direction: impl Into<ReadDirection>,
    ) -> Self {
        // A group in another's panel joins its family.
        let around = gui::use_context::<InFamily>();
        let family = around.as_ref().map_or_else(
            || {
                Rc::new(Family {
                    members: gui::Registry::new(),
                    crossing: Signal::labeled("splitter handles crossing", false),
                    capture: CaptureGroup::new(),
                })
            },
            |f| Rc::clone(&f.family),
        );
        let id = gui::Registration::new();
        let me = Self {
            layout,
            panels: Signal::new(Vec::new()),
            roster: Signal::labeled("splitter roster", 0),
            size: Signal::labeled("splitter size", None),
            drag: Signal::labeled("splitter drag", None),
            hovered: Signal::labeled("splitter hovered", None),
            limit: Signal::labeled("splitter limit", None),
            coarse: Signal::labeled("splitter coarse", false),
            state: Signal::new(State {
                options: Rc::new(options),
                direction: direction.into(),
                handle_boxes: RefCell::new(HashMap::new()),
                handles: Collection::new(),
                before_collapse: RefCell::new(HashMap::new()),
                told: RefCell::new(HashMap::new()),
                init_size: std::cell::Cell::new(None),
                stale: std::cell::Cell::new(false),
                content: std::cell::Cell::new(None),
                on_layout: gui::Holder::new(),
                persist: gui::Holder::new(),
                last: std::cell::Cell::new(0.0),
                inside: std::cell::Cell::new(false),
                bounds: std::cell::Cell::new(None),
                family: Rc::clone(&family),
                id,
                partners: RefCell::new(Vec::new()),
                name: std::cell::Cell::new(handle_name),
            }),
        };
        family.members.register(Entry {
            id,
            parent: around.map(|f| f.parent),
            member: Rc::new(me),
        });
        gui::provide(InFamily { family, parent: id });
        // A panel joined or left: the layout is worked out afresh at the
        // next layout pass, once all the panels mounting with it have joined
        // (one at a time, each would throw the caller's layout away as not
        // theirs). Until then [`Self::layout`] shows the caller's, or the
        // default.
        gui::watch(
            move || me.roster.get(),
            move |_, _| {
                if me.state.is_alive() {
                    me.state.with_untracked(|s| s.stale.set(true));
                }
            },
        );
        // The group resized: pixel panels keep their pixels.
        gui::watch(
            move || me.size.get(),
            move |next, prev| {
                if let (Some(next), Some(prev)) = (*next, *prev)
                    && me.state.is_alive()
                {
                    me.resized(prev, next);
                }
            },
        );
        me
    }

    /// Save the layout under `id` in `storage`, and start from what it saved
    /// (reka's `auto-save-id` and `storage`), while the calling setup's
    /// part is mounted. Called in setup, before the panels join.
    #[must_use]
    #[track_caller]
    pub fn persist(self, id: impl Into<String>, storage: Rc<dyn Storage>) -> Self {
        let persist = Persist {
            id: id.into(),
            storage,
            save: Delay::new(),
        };
        self.state.with_untracked(|s| s.persist).hold(persist);
        self
    }

    /// Tell `f` each layout, in each panel's own unit (reka's `@layout`),
    /// while the calling setup's part is mounted.
    #[must_use]
    #[track_caller]
    pub fn on_layout(self, f: impl Into<gui::Callback<Vec<f64>>>) -> Self {
        let f: LayoutCallback = f.into();
        self.state.with_untracked(|s| s.on_layout).hold(f);
        self
    }

    /// The caller's options now (tracked through what its closure reads).
    #[must_use]
    pub fn options(self) -> SplitterOptions {
        let f = self.state.with_untracked(|s| Rc::clone(&s.options));
        f()
    }

    /// Panel `index` joins as `panel` says (called in the panel's setup); it
    /// leaves the group as its scope ends, with what was kept for it.
    pub fn register(self, index: usize, panel: impl Into<Panel>) {
        let panel = panel.into();
        // The panel's own node, made with it.
        let node = NodeRef::new();
        // From its mount to its unmount (a setup, as a render, writes only
        // what it made); its unmounting takes back the slot only if this
        // registration still holds it.
        gui::while_mounted(move || {
            let owner = gui::Registration::new();
            self.panels.update(|panels| {
                if panels.len() <= index {
                    panels.resize(index.saturating_add(1), None);
                }
                if let Some(slot) = panels.get_mut(index) {
                    *slot = Some(PanelEntry { panel, node, owner });
                }
            });
            self.roster.update(|r| *r = r.wrapping_add(1));
            move || self.unregister(index, owner)
        });
    }

    /// The panel registered at `index` as `owner` leaves; nothing when a
    /// panel that replaced it holds the index now.
    fn unregister(self, index: usize, owner: gui::Registration) {
        if !self.panels.is_alive() {
            return;
        }
        let own = self.panels.with_untracked(|panels| {
            panels
                .get(index)
                .and_then(Option::as_ref)
                .is_none_or(|e| e.owner == owner)
        });
        if !own {
            // Replaced at its index: the slot and what was kept for it are
            // the new panel's.
            return;
        }
        self.panels.update(|panels| {
            if let Some(slot) = panels.get_mut(index) {
                *slot = None;
            }
            while panels.last().is_some_and(Option::is_none) {
                panels.pop();
            }
        });
        self.state.with_untracked(|s| {
            s.before_collapse.borrow_mut().remove(&index);
            s.told.borrow_mut().remove(&index);
        });
        self.roster.update(|r| *r = r.wrapping_add(1));
    }

    /// Panel `index`, registered, now has `constraints` (its props
    /// changed). Its default size counts only when the layout is next
    /// worked out afresh.
    pub fn set_constraints(self, index: usize, constraints: Constraints) {
        let same = self
            .panels
            .with_untracked(|p| p.get(index).and_then(Option::as_ref).map(|e| e.panel.constraints))
            == Some(constraints);
        if same {
            return;
        }
        self.panels.update(|panels| {
            if let Some(Some(entry)) = panels.get_mut(index) {
                entry.panel.constraints = constraints;
            }
        });
    }

    /// Every panel's constraints in its own unit, by index (tracked).
    #[must_use]
    pub fn native_constraints(self) -> Vec<Constraints> {
        self.panels.with(|p| {
            p.iter()
                .map(|e| e.as_ref().map(|e| e.panel.constraints).unwrap_or_default())
                .collect()
        })
    }

    /// Every panel's constraints in percent of the group, by index (tracked);
    /// a pixel panel's are its percent of the group once it is measured, the
    /// defaults until then.
    #[must_use]
    pub fn constraints(self) -> Vec<Constraints> {
        let size = self.size.get();
        self.native_constraints()
            .into_iter()
            .map(|c| c.in_percent(size).unwrap_or_default())
            .collect()
    }

    /// The layout as shown (tracked): the model's when it has one size per
    /// panel, made valid; else the default one.
    #[must_use]
    pub fn layout(self) -> Vec<f64> {
        let constraints = self.constraints();
        let held = self.layout.get();
        if held.len() == constraints.len() {
            validate(&held, &constraints)
        } else {
            default_layout(&constraints)
        }
    }

    /// Panel `index`'s size, in percent (tracked).
    #[must_use]
    pub fn size(self, index: usize) -> f64 {
        self.layout().get(index).copied().unwrap_or(0.0)
    }

    /// Whether panel `index` is collapsed (tracked): its look reads it.
    #[must_use]
    pub fn is_collapsed(self, index: usize) -> bool {
        let c = self.constraints().get(index).copied().unwrap_or_default();
        c.collapsible && compare(self.size(index), c.collapsed).is_le()
    }

    /// The handle being dragged, if any (tracked): its look reads it.
    #[must_use]
    pub fn dragging(self) -> Option<usize> {
        self.drag.with(|d| d.as_ref().map(|d| d.handle))
    }

    /// The state of the handle after panel `index` (tracked).
    #[must_use]
    pub fn handle_state(self, index: usize) -> HandleState {
        if self.dragging() == Some(index) {
            HandleState::Drag
        } else if self.hovered.get() == Some(index) {
            HandleState::Hover
        } else {
            HandleState::Inactive
        }
    }

    /// What made the handle after panel `index` active, if it is (tracked):
    /// a drag, or focus (reka's `data-resize-handle-active`).
    #[must_use]
    pub fn active(self, index: usize, focus: FocusHandle) -> Option<Trigger> {
        if self.dragging() == Some(index) {
            Some(Trigger::Pointer)
        } else if focus.is_focused() {
            Some(Trigger::Keyboard)
        } else {
            None
        }
    }

    /// The cursor the handle after panel `index` shows (tracked): a resize
    /// cursor along the group's axis, one way only while its drag is pushed
    /// past a limit.
    #[must_use]
    pub fn cursor(self, index: usize) -> gui::Cursor {
        let dragged = self.dragging() == Some(index);
        if (dragged || self.hovered.get() == Some(index)) && self.family().crossing.get() {
            return self.family().crossing_cursor();
        }
        let limit = if dragged { self.limit.get() } else { None };
        cursor_for(self.options().orientation, limit)
    }

    fn family(self) -> Rc<Family> {
        self.state.with_untracked(|s| Rc::clone(&s.family))
    }

    fn id(self) -> gui::Registration {
        self.state.with_untracked(|s| s.id)
    }

    /// The cursor over the group (tracked): the dragged or reached handle's.
    fn group_cursor(self) -> Option<gui::Cursor> {
        let at = self.dragging().or_else(|| self.hovered.get())?;
        Some(self.cursor(at))
    }

    /// Commit `next`: the model, the caller's `on_layout`, the panels'
    /// callbacks, the saved layout.
    fn commit(self, next: Vec<f64>) {
        let now = gui::untracked(|| self.layout());
        let constraints = gui::untracked(|| self.constraints());
        self.state.with_untracked(|s| {
            // Remember a panel's size as it collapses: expanding restores it.
            let mut memory = s.before_collapse.borrow_mut();
            for (i, c) in constraints.iter().enumerate() {
                let (Some(&was), Some(&is)) = (now.get(i), next.get(i)) else {
                    continue;
                };
                if c.collapsible && same(is, c.collapsed) && !same(was, c.collapsed) {
                    memory.insert(i, was);
                }
            }
        });
        let held = gui::untracked(|| self.layout.get());
        if held != next {
            self.layout.set(next.clone());
        }
        self.told(&next);
    }

    /// Tell the caller and the panels `layout`, and save it.
    fn told(self, layout: &[f64]) {
        let (native, size) = gui::untracked(|| (self.native_constraints(), self.size.get()));
        let sizes: Vec<f64> = layout
            .iter()
            .zip(native.iter().copied().chain(std::iter::repeat(Constraints::default())))
            .map(|(p, c)| c.native(*p, size))
            .collect();
        let (on_layout, panels) = self.state.with_untracked(|s| {
            (
                s.on_layout.get_untracked(),
                self.panels.with_untracked(|p| {
                    p.iter()
                        .map(|e| e.as_ref().map(|e| e.panel.clone()))
                        .collect::<Vec<_>>()
                }),
            )
        });
        if let Some(f) = on_layout {
            f.call(sizes.clone());
        }
        // reka's `callPanelCallbacks`: each panel whose size differs from
        // what it was last told, the first telling included.
        for (i, (&shown, panel)) in sizes.iter().zip(&panels).enumerate() {
            let Some(panel) = panel else { continue };
            let last = self.state.with_untracked(|s| s.told.borrow().get(&i).copied());
            if last.is_some_and(|l| same(l, shown)) {
                continue;
            }
            self.state.with_untracked(|s| s.told.borrow_mut().insert(i, shown));
            if let Some(f) = &panel.on_resize {
                f.call((shown, last));
            }
            let c = panel.constraints;
            if c.collapsible {
                let was_collapsed = last.map(|l| same(l, c.collapsed));
                let is_collapsed = same(shown, c.collapsed);
                if let Some(f) = &panel.on_expand
                    && was_collapsed != Some(false)
                    && !is_collapsed
                {
                    f.call(());
                }
                if let Some(f) = &panel.on_collapse
                    && was_collapsed != Some(true)
                    && is_collapsed
                {
                    f.call(());
                }
            }
        }
        self.save(layout);
    }

    /// The key a saved layout of these panels has: their ids, or their
    /// order and constraints, sorted (reka's `getPanelKey`).
    fn storage_key(self, id: &str) -> String {
        let mut keys: Vec<String> = self.panels.with_untracked(|p| {
            p.iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let e = e.as_ref()?;
                    Some(
                        e.panel
                            .id
                            .as_deref()
                            .map_or_else(|| format!("{i}:{:?}", e.panel.constraints), str::to_owned),
                    )
                })
                .collect()
        });
        keys.sort();
        format!("{id}/{}", keys.join(","))
    }

    fn save(self, layout: &[f64]) {
        let Some((id, storage, save)) = self.state.with_untracked(|s| {
            s.persist
                .with_untracked(|p| p.map(|p| (p.id.clone(), Rc::clone(&p.storage), p.save)))
        }) else {
            return;
        };
        let count = self.panels.with_untracked(|p| p.iter().filter(|e| e.is_some()).count());
        if layout.is_empty() || layout.len() != count {
            return;
        }
        let key = self.storage_key(&id);
        let saved = SavedLayout {
            layout: layout.to_vec(),
            expand_to: self.state.with_untracked(|s| {
                let mut e: Vec<(usize, f64)> = s.before_collapse.borrow().iter().map(|(i, v)| (*i, *v)).collect();
                e.sort_by_key(|(i, _)| *i);
                e
            }),
        };
        save.after(SAVE_DELAY, move || storage.save(&key, saved.clone()));
    }

    /// Work the layout out afresh: the saved one, else the default, made
    /// valid, committed (reka's `panelDataArrayChanged` watch).
    fn init(self) {
        let (native, size) = gui::untracked(|| (self.native_constraints(), self.size.get()));
        if native.is_empty() {
            return;
        }
        let Some(constraints) = percent_constraints(&native, size) else {
            return;
        };
        let saved = self.state.with_untracked(|s| {
            s.persist
                .with_untracked(|p| p.map(|p| (p.id.clone(), Rc::clone(&p.storage))))
        });
        let saved = saved.and_then(|(id, storage)| storage.load(&self.storage_key(&id)));
        let held = gui::untracked(|| self.layout.get());
        let unsafe_layout = match saved {
            Some(saved) if saved.layout.len() == constraints.len() => {
                self.state.with_untracked(|s| {
                    *s.before_collapse.borrow_mut() = saved.expand_to.iter().copied().collect();
                });
                saved.layout
            }
            // The caller's own layout, given for these panels.
            _ if held.len() == constraints.len() => held,
            _ => default_layout(&constraints),
        };
        let next = validate(&unsafe_layout, &constraints);
        self.state.with_untracked(|s| s.init_size.set(size));
        self.commit(next);
    }

    fn resized(self, prev: f32, next: f32) {
        let native = gui::untracked(|| self.native_constraints());
        if !native.iter().any(|c| c.unit == Unit::Pixels) {
            return;
        }
        let layout = gui::untracked(|| self.layout.get());
        if layout.is_empty() {
            return;
        }
        // Measured before a parent laid it out: start again.
        if let Some(first) = self.state.with_untracked(|s| s.init_size.get())
            && first > 0.0
            && first < UNRELIABLE_SIZE
            && next > first * 10.0
        {
            self.init();
            return;
        }
        let Some(re) = relayout_for_pixels(&layout, &native, prev, next) else {
            return;
        };
        let Some(constraints) = percent_constraints(&native, Some(next)) else {
            return;
        };
        self.commit(validate(&re, &constraints));
    }

    /// Move the handle after panel `pivot` by `delta` percent.
    pub fn move_handle(self, pivot: usize, delta: f64, trigger: Trigger) {
        let (layout, constraints) = gui::untracked(|| (self.layout(), self.constraints()));
        self.commit(adjust(delta, &layout, &constraints, pivot, trigger));
    }

    /// Set panel `index` to `size` in its own unit (reka's `resize`).
    pub fn resize_panel(self, index: usize, size: f64) {
        let (layout, constraints, native, group) = gui::untracked(|| {
            (
                self.layout(),
                self.constraints(),
                self.native_constraints(),
                self.size.get(),
            )
        });
        let (Some(&now), Some(c)) = (layout.get(index), native.get(index)) else {
            return;
        };
        let to = match c.unit {
            Unit::Percent => size,
            Unit::Pixels => match group {
                Some(g) if g > 0.0 => size / f64::from(g) * 100.0,
                _ => return,
            },
        };
        let last = index.saturating_add(1) == layout.len();
        let (pivot, delta) = if last {
            (index.saturating_sub(1), now - to)
        } else {
            (index, to - now)
        };
        self.commit(adjust(delta, &layout, &constraints, pivot, Trigger::Code));
    }

    /// Collapse panel `index`, if it is collapsible.
    pub fn collapse(self, index: usize) {
        let (layout, constraints) = gui::untracked(|| (self.layout(), self.constraints()));
        let (Some(&size), Some(c)) = (layout.get(index), constraints.get(index)) else {
            return;
        };
        if !c.collapsible || same(size, c.collapsed) {
            return;
        }
        let last = index.saturating_add(1) == layout.len();
        let (pivot, delta) = if last {
            (index.saturating_sub(1), size - c.collapsed)
        } else {
            (index, c.collapsed - size)
        };
        self.commit(adjust(delta, &layout, &constraints, pivot, Trigger::Code));
    }

    /// Expand panel `index`, if it is collapsed: to its size before it
    /// collapsed, or its min when that was smaller.
    pub fn expand(self, index: usize) {
        let (layout, constraints) = gui::untracked(|| (self.layout(), self.constraints()));
        let (Some(&size), Some(c)) = (layout.get(index), constraints.get(index)) else {
            return;
        };
        if !c.collapsible || compare(size, c.collapsed).is_gt() {
            return;
        }
        let before = self
            .state
            .with_untracked(|s| s.before_collapse.borrow().get(&index).copied());
        let to = before.filter(|b| *b >= c.min).unwrap_or(c.min);
        let last = index.saturating_add(1) == layout.len();
        let (pivot, delta) = if last {
            (index.saturating_sub(1), size - to)
        } else {
            (index, to - size)
        };
        self.commit(adjust(delta, &layout, &constraints, pivot, Trigger::Code));
    }

    fn along(self, x: f32, y: f32) -> f32 {
        match gui::untracked(|| self.options()).orientation {
            Axis::Horizontal => x,
            Axis::Vertical => y,
        }
    }

    fn rtl(self) -> bool {
        gui::untracked(|| self.options()).orientation == Axis::Horizontal
            && self.state.with_untracked(|s| s.direction.get()) == Direction::Rtl
    }

    /// The boxes of the handles mounted now, by the panel each follows.
    fn handle_boxes(self) -> Vec<(usize, Bounds)> {
        self.state.with_untracked(|s| {
            let mut boxes = s.handle_boxes.borrow_mut();
            let mounted: Vec<(usize, bool)> = s
                .handles
                .with(|l| l.iter().map(|h| (h.value, h.availability.actionable())).collect());
            // A handle no longer mounted is forgotten, its thickness too.
            boxes.retain(|i, _| mounted.iter().any(|(m, _)| m == i));
            let mut out: Vec<(usize, Bounds)> = boxes.iter().map(|(i, b)| (*i, *b)).collect();
            out.sort_by_key(|(i, _)| *i);
            out
        })
    }

    /// The usable handle the pointer at `(x, y)` reaches: within the
    /// margin of its box, the nearest.
    fn reached(self, x: f32, y: f32) -> Option<usize> {
        let margin = if self.coarse.get_untracked() {
            MARGIN_COARSE
        } else {
            MARGIN_FINE
        };
        let usable: Vec<usize> = self.state.with_untracked(|s| {
            s.handles.with(|l| {
                l.iter()
                    .filter(|h| h.availability.actionable())
                    .map(|h| h.value)
                    .collect()
            })
        });
        self.handle_boxes()
            .into_iter()
            .filter(|(i, b)| usable.contains(i) && b.outset(margin).contains(x, y))
            .min_by(|(_, a), (_, b)| {
                let d = |r: &Bounds| (r.x + r.width / 2.0 - x).abs() + (r.y + r.height / 2.0 - y).abs();
                d(a).total_cmp(&d(b))
            })
            .map(|(i, _)| i)
    }

    /// `d`, the look's group box, as the group: its panels run along its
    /// orientation (the look lays them out); a press within reach of a
    /// handle drags it, a secondary press ends a drag, and the group's
    /// cursor is the reached handle's. Sets `on_drag`, `on_pointer_move`,
    /// `on_hover`, `on_secondary_click`, `on_touch`, `on_measured` and
    /// `cursor`.
    #[must_use]
    pub fn group(self, mut d: Div) -> Div {
        let style = d.style_mut();
        let (pad, border) = (style.padding, style.border_widths);
        d.capture_group(self.family().capture)
            .on_drag(move |e| self.on_drag(e))
            .on_pointer_move(move |p| {
                self.state.with_untracked(|s| s.inside.set(true));
                // A drag holds what it reached until it ends.
                let family = self.family();
                if !family.members().iter().any(|m| m.alive() && m.dragging()) {
                    family.hover(self.id(), p.x, p.y);
                }
            })
            .on_hover(move |inside| {
                self.state.with_untracked(|s| s.inside.set(inside));
                if !inside {
                    // What only this group's hover reached goes with it.
                    for m in self.family().members() {
                        if m.alive() && !m.inside() {
                            m.set_reached(None);
                        }
                    }
                    if !self.family().members().iter().any(|m| m.alive() && m.engaged()) {
                        self.family().cross(&[]);
                    }
                }
            })
            // The context menu ends a drag (`registry.ts:334`).
            .on_secondary_click(move |_| self.end_drag())
            .on_touch(move |t| {
                let coarse = matches!(t.kind, PointerKind::Touch | PointerKind::Pen);
                if self.coarse.get_untracked() != coarse {
                    self.coarse.set(coarse);
                }
            })
            .on_measured(move |b| {
                if self.state.is_alive() {
                    self.state.with_untracked(|s| s.bounds.set(Some(*b)));
                }
                // The content box along the axis; the room the panels share
                // is worked out once the pass has measured the handles too.
                let horizontal = gui::untracked(|| self.options()).orientation == Axis::Horizontal;
                let content = if horizontal {
                    b.width - pad.left - pad.right - border.left - border.right
                } else {
                    b.height - pad.top - pad.bottom - border.top - border.bottom
                };
                if !self.state.is_alive() {
                    return;
                }
                let waiting = self
                    .state
                    .with_untracked(|s| s.content.replace(Some(content)).is_some());
                if !waiting {
                    gui::after_layout(move || self.measured());
                }
            })
            .cursor(move || self.group_cursor())
    }

    /// The pass laid the group and its handles out: the room the panels
    /// share is the content box less the handles. reka takes the whole
    /// content box (`contentRect`), so its pixel panels come out short by
    /// their share of the handles. The first room, or panels that joined
    /// since, work the layout out afresh (a pixel panel needed the room).
    fn measured(self) {
        if !self.state.is_alive() {
            return;
        }
        let Some(content) = self.state.with_untracked(|s| s.content.take()) else {
            return;
        };
        let horizontal = gui::untracked(|| self.options()).orientation == Axis::Horizontal;
        let handles: f32 = self
            .handle_boxes()
            .iter()
            .map(|(_, h)| if horizontal { h.width } else { h.height })
            .sum();
        let room = Some((content - handles).max(0.0));
        let first = self.size.get_untracked().is_none();
        if self.size.get_untracked() != room {
            self.size.set(room);
        }
        if self.state.with_untracked(|s| s.stale.take()) || first {
            self.init();
        }
    }

    fn end_drag(self) {
        if self.drag.get_untracked().is_some() {
            self.drag.set(None);
        }
        if self.limit.get_untracked().is_some() {
            self.limit.set(None);
        }
        let partners = self
            .state
            .with_untracked(|s| std::mem::take(&mut *s.partners.borrow_mut()));
        for m in partners {
            if m.alive() {
                m.stop();
            }
        }
        let family = self.family();
        if !family.members().iter().any(|m| m.alive() && m.engaged()) {
            family.cross(&[]);
        }
    }

    /// The pointer pressed, moved or let go over the group: its own drag,
    /// and the drags of the family's groups it reached as it pressed.
    fn on_drag(self, e: &DragEvent) {
        if e.phase == DragPhase::Start {
            if e.kind == PointerKind::Mouse && self.coarse.get_untracked() {
                self.coarse.set(false);
            }
            // The set of handles dragged is fixed from the press to the
            // release (`registry.ts:132-136`).
            let family = self.family();
            let reached = family.reached(self.id(), e.x, e.y);
            let mut partners = Vec::new();
            for (m, handle) in &reached {
                if self.is(m) {
                    self.start_drag(e, *handle);
                } else {
                    m.start(e, *handle);
                    partners.push(Rc::clone(m));
                }
            }
            family.cross(&reached);
            self.state.with_untracked(|s| *s.partners.borrow_mut() = partners);
            return;
        }
        let partners = self.state.with_untracked(|s| s.partners.borrow().clone());
        for m in &partners {
            if m.alive() {
                m.drag(e);
            }
        }
        self.drag_by(e);
        if matches!(e.phase, DragPhase::End | DragPhase::Cancel) {
            self.end_drag();
        }
    }

    /// Whether `m` is this group.
    fn is(self, m: &Rc<dyn Member>) -> bool {
        let family = self.family();
        let id = self.id();
        family
            .members
            .with_untracked(|mut l| l.any(|e| e.id == id && Rc::ptr_eq(&e.member, m)))
    }

    fn start_drag(self, e: &DragEvent, handle: usize) {
        self.state.with_untracked(|s| s.last.set(0.0));
        self.hovered.set(None);
        self.drag.set(Some(Drag {
            handle,
            from: self.along(e.x, e.y),
            start: gui::untracked(|| self.layout()),
        }));
    }

    /// A move of this group's drag: the delta is the pointer's travel in
    /// percent of the room the panels share, the unit the sizes are in.
    fn drag_by(self, e: &DragEvent) {
        match e.phase {
            DragPhase::Start => {}
            DragPhase::Move => {
                let Some(drag) = self.drag.get_untracked() else {
                    return;
                };
                let pointer = self.along(e.x, e.y);
                let room = self.size.get_untracked().unwrap_or(1.0).max(1.0);
                let moved = f64::from((pointer - drag.from) / room * 100.0);
                let delta = if self.rtl() { -moved } else { moved };
                if same(delta, 0.0) {
                    return;
                }
                let (constraints, prev) = gui::untracked(|| (self.constraints(), self.layout()));
                let next = adjust(delta, &drag.start, &constraints, drag.handle, Trigger::Pointer);
                // Pushed past a limit: the cursor shows the way still open,
                // the way the pointer went (not the flipped delta).
                let last = self.state.with_untracked(|s| s.last.replace(delta));
                if !same(last, delta) {
                    let limit = if next == prev {
                        Some(if moved < 0.0 { Limit::Start } else { Limit::End })
                    } else {
                        None
                    };
                    if self.limit.get_untracked() != limit {
                        self.limit.set(limit);
                    }
                }
                if next != prev {
                    self.commit(next);
                }
            }
            // Cancelled, the panels stay where the moves put them.
            DragPhase::End | DragPhase::Cancel => {
                if self.drag.get_untracked().is_some() {
                    self.drag.set(None);
                }
                if self.limit.get_untracked().is_some() {
                    self.limit.set(None);
                }
            }
        }
    }

    /// `d`, the look's panel box, as panel `index`: its share of the group
    /// is its size (bound to the node: a drag's moves lay it out and render
    /// nothing), the whole group when it is alone; it and all it holds take
    /// no pointer while a drag runs (a drag's start and end render it); it
    /// is named for its handle's `aria-controls`.
    #[must_use]
    pub fn panel(self, d: Div, index: usize) -> Div {
        let dragging = self.drag.with(Option::is_some);
        let d = match self.panel_node(index) {
            Some(node) => d.node_ref(node),
            None => d,
        };
        // reka's panel style: no basis, so the shares alone size it.
        d.flex_basis(Length::Px(0.0))
            .flex_shrink(1.0)
            .flex_grow(move || {
                if self.panels.with(|p| p.iter().filter(|e| e.is_some()).count()) == 1 {
                    1.0
                } else {
                    share(self.size(index))
                }
            })
            .pass_through_all(dragging)
    }

    /// The node of the panel registered at `index` (tracked): none while
    /// no panel is mounted there.
    fn panel_node(self, index: usize) -> Option<NodeRef> {
        self.panels
            .with(|p| p.get(index).and_then(Option::as_ref).map(|e| e.node))
    }

    /// How many panel nodes the group knows: the registered panels' own,
    /// none for an index no panel is mounted at.
    #[must_use]
    pub fn kept_panel_nodes(self) -> usize {
        self.panels.with_untracked(|p| p.iter().flatten().count())
    }

    /// Name the handles by `name` from now on: the handle after panel `n`
    /// (from 1) is `name(n)`. Read when a handle renders.
    pub fn set_name(self, name: fn(usize) -> String) {
        self.state.with_untracked(|s| s.name.set(name));
    }

    /// `d`, the look's handle, as the handle after panel `index`, usable as
    /// `availability` says (an unavailable one keeps focus and moves
    /// nothing), focused through `focus` (its own, made in its setup), and
    /// named for that panel. Sets `on_key_down`, `on_measured` and `cursor`.
    #[must_use]
    pub fn handle(self, d: Div, index: usize, availability: Availability, focus: FocusHandle) -> Div {
        let availability = match availability {
            Availability::Enabled => Availability::Enabled,
            Availability::Unavailable | Availability::Disabled => Availability::Unavailable,
        };
        let orientation = self.options().orientation;
        let (now, min, max) = aria_values(&self.layout(), &self.constraints(), index);
        let across = match orientation {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        };
        let name = self.state.with_untracked(|s| (s.name.get())(index.saturating_add(1)));
        let d = d
            .role(gui::Role::Splitter)
            .aria_label(name)
            .aria_orientation(across)
            .aria_value_range(gui::ValueRange {
                now: Some(now.round()),
                min: min.round(),
                max: max.round(),
                text: None,
            });
        let d = match self.panel_node(index) {
            Some(node) => d.controls(node),
            None => d,
        };
        let handles = self.state.with_untracked(|s| s.handles.clone());
        handles
            .item(d, Item::new(index, focus).availability(availability))
            .on_measured(move |b| {
                self.state.with_untracked(|s| {
                    s.handle_boxes.borrow_mut().insert(index, *b);
                });
            })
            .cursor(move || availability.actionable().then(|| self.cursor(index)))
            .on_key_down(move |ev| self.key(index, focus, availability, ev))
    }

    fn key(self, index: usize, focus: FocusHandle, availability: Availability, ev: &KeyEvent) -> bool {
        if gui::untracked(gui::focus::focused) != Some(focus.id()) {
            return false;
        }
        let m = ev.modifiers;
        if ev.is(NamedKey::F6) && !m.control && !m.alt && !m.command {
            let handles = self.state.with_untracked(|s| s.handles.clone());
            let intent = if m.shift { Intent::Prev } else { Intent::Next };
            let to = handles.with(|list| {
                let usable = crate::collection::from_fn(
                    list.len(),
                    |i| {
                        list.get(i).map_or(Availability::Disabled, |h| {
                            if h.availability.actionable() {
                                Availability::Enabled
                            } else {
                                Availability::Disabled
                            }
                        })
                    },
                    |_| std::borrow::Cow::Borrowed(""),
                );
                let at = list.iter().position(|h| h.focus == focus);
                step(&usable, at, intent, Wrap::Around)
                    .and_then(|i| list.get(i))
                    .map(|h| h.focus)
            });
            if let Some(to) = to {
                gui::focus::set_focus(Some(to.id()), true);
            }
            return true;
        }
        if m.control || m.alt || m.command {
            return false;
        }
        let options = gui::untracked(|| self.options());
        let step = if m.shift { 100.0 } else { options.step };
        let rtl = self.rtl();
        let (back, forward) = match options.orientation {
            Axis::Horizontal if rtl => (NamedKey::ArrowRight, NamedKey::ArrowLeft),
            Axis::Horizontal => (NamedKey::ArrowLeft, NamedKey::ArrowRight),
            Axis::Vertical => (NamedKey::ArrowUp, NamedKey::ArrowDown),
        };
        let delta = if ev.is(back) {
            -step
        } else if ev.is(forward) {
            step
        } else if ev.is(NamedKey::Home) {
            -100.0
        } else if ev.is(NamedKey::End) {
            100.0
        } else if ev.is(NamedKey::Enter) {
            if availability.actionable() {
                if gui::untracked(|| self.is_collapsed(index)) {
                    self.expand(index);
                } else {
                    self.collapse(index);
                }
            }
            return true;
        } else {
            return false;
        };
        if availability.actionable() {
            self.move_handle(index, delta, Trigger::Keyboard);
        }
        true
    }
}

impl<P: 'static> Member for Splitter<P> {
    fn alive(&self) -> bool {
        self.state.is_alive()
    }

    fn orientation(&self) -> Axis {
        gui::untracked(|| self.options()).orientation
    }

    fn inside(&self) -> bool {
        self.state.with_untracked(|s| s.inside.get())
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        self.state
            .with_untracked(|s| s.bounds.get())
            .is_some_and(|b| b.contains(x, y))
    }

    fn reach(&self, x: f32, y: f32) -> Option<usize> {
        self.reached(x, y)
    }

    fn set_reached(&self, at: Option<usize>) {
        if self.drag.get_untracked().is_none() && self.hovered.get_untracked() != at {
            self.hovered.set(at);
        }
    }

    fn engaged(&self) -> bool {
        self.drag.with(Option::is_some) || self.hovered.get().is_some()
    }

    fn limit(&self) -> Option<Limit> {
        self.limit.get()
    }

    fn dragging(&self) -> bool {
        self.drag.get_untracked().is_some()
    }

    fn start(&self, e: &DragEvent, handle: usize) {
        self.start_drag(e, handle);
    }

    fn drag(&self, e: &DragEvent) {
        self.drag_by(e);
    }

    fn stop(&self) {
        if self.drag.get_untracked().is_some() {
            self.drag.set(None);
        }
        if self.limit.get_untracked().is_some() {
            self.limit.set(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Constraints, Trigger, adjust, aria_values, default_layout, resize, validate};

    fn c(min: f64, max: f64) -> Constraints {
        Constraints {
            min,
            max: Some(max),
            ..Constraints::default()
        }
    }

    fn collapsible(min: f64, collapsed: f64) -> Constraints {
        Constraints {
            min,
            collapsible: true,
            collapsed,
            ..Constraints::default()
        }
    }

    #[test]
    fn a_collapsible_panel_snaps_at_the_midpoint() {
        let cs = [collapsible(20.0, 4.0)];
        assert_eq!(resize(&cs, 0, 11.9), 4.0, "below (4 + 20) / 2: collapsed");
        assert_eq!(resize(&cs, 0, 12.0), 20.0, "from the midpoint: its min");
        assert_eq!(resize(&[c(10.0, 60.0)], 0, 80.0), 60.0);
    }

    #[test]
    fn the_shrinking_side_spills_and_the_growing_side_is_limited() {
        let cs = [c(0.0, 100.0), c(10.0, 100.0), c(10.0, 100.0)];
        // The panel next to the handle shrinks to its min, the next one gives the rest.
        assert_eq!(
            adjust(30.0, &[40.0, 30.0, 30.0], &cs, 0, Trigger::Pointer),
            [70.0, 10.0, 20.0]
        );
        let capped = [c(0.0, 50.0), c(0.0, 100.0)];
        assert_eq!(
            adjust(40.0, &[40.0, 60.0], &capped, 0, Trigger::Pointer),
            [50.0, 50.0],
            "no more than the growing panel takes"
        );
        assert_eq!(adjust(0.0, &[40.0, 60.0], &capped, 0, Trigger::Pointer), [40.0, 60.0]);
    }

    #[test]
    fn a_key_collapses_and_expands_in_one_press() {
        let cs = [collapsible(20.0, 0.0), c(0.0, 100.0)];
        assert_eq!(
            adjust(-10.0, &[20.0, 80.0], &cs, 0, Trigger::Keyboard),
            [0.0, 100.0],
            "at its min, one press collapses it"
        );
        assert_eq!(
            adjust(10.0, &[0.0, 100.0], &cs, 0, Trigger::Keyboard),
            [20.0, 80.0],
            "collapsed, one press opens it to its min"
        );
        assert_eq!(
            adjust(-10.0, &[20.0, 80.0], &cs, 0, Trigger::Pointer),
            [20.0, 80.0],
            "a drag of 10 stays above the midpoint: its min"
        );
    }

    #[test]
    fn default_and_valid_layouts() {
        let cs = [
            Constraints {
                default: Some(30.0),
                ..Constraints::default()
            },
            Constraints::default(),
            Constraints::default(),
        ];
        assert_eq!(default_layout(&cs), [30.0, 35.0, 35.0]);
        assert_eq!(validate(&[1.0, 1.0], &[c(0.0, 100.0), c(0.0, 100.0)]), [50.0, 50.0]);
        assert_eq!(validate(&[5.0, 95.0], &[c(20.0, 100.0), c(0.0, 100.0)]), [20.0, 80.0]);
        assert_eq!(
            aria_values(&[30.0, 70.0], &[c(10.0, 90.0), c(20.0, 80.0)], 0),
            (30.0, 20.0, 80.0)
        );
    }
}
