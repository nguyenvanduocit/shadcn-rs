//! Each node's layout cache, keyed by the inputs the node reads (ADR-0014).
//!
//! Taffy's own `Cache` compares a final layout's whole `parent_size`, so a
//! node that reads no percentage misses whenever its parent's size moves:
//! benchmark 5.2 saw one row growing 20 px rerun 5 000 sibling rows. An
//! entry here records which axes of the parent size the node read (its own
//! percentages, from its style), and matches on those only, as Ladybird's
//! `fc_run_cache` records what a run did not read. Known dimensions and
//! available space are keyed as Taffy keys them (a known dimension stands
//! for the available space on its axis, which it then makes irrelevant).
//! Taffy's repeated min- and max-content probes are memoized in up to
//! [`MEASURE_SLOTS`] size entries, replaced second-chance.
//!
//! Entries keep the whole input they answered, so a node whose content
//! changed can be laid out again for each of them and its new answers
//! compared with the old ([`NodeCache::take_check`]): when they agree, the
//! parent's results, built from them, stand.

use super::Measured;
use taffy::{
    AvailableSpace, CollapsibleMarginSet, LayoutInput, LayoutOutput, RequestedAxis, RunMode, Size, SizingMode,
};

/// How many size probes a node keeps. Taffy keeps 9; a flex item inside a
/// grid inside a flex row is probed at more widths than that in one pass.
pub const MEASURE_SLOTS: usize = 16;

/// What one axis of a key holds: a known dimension, else which available
/// space. Kept beside the size's bits, not folded into them, so no two
/// inputs share a key whatever their bits (a known `1.5` and an available
/// `-1.0` once did, folded into one `u32`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Known,
    Definite,
    MinContent,
    MaxContent,
}

/// One axis of a key: its kind, and the size's exact bits (0 for min- and
/// max-content).
fn axis_key(known: Option<f32>, available: AvailableSpace) -> (Kind, u32) {
    match (known, available) {
        (Some(v), _) => (Kind::Known, v.to_bits()),
        (None, AvailableSpace::Definite(v)) => (Kind::Definite, v.to_bits()),
        (None, AvailableSpace::MinContent) => (Kind::MinContent, 0),
        (None, AvailableSpace::MaxContent) => (Kind::MaxContent, 0),
    }
}

/// Which of a node's inputs its answers read, beyond what the key always
/// compares (ADR-0014: an entry matches on what was read only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reads {
    /// The axes of its parent's size its own percentages read.
    pub parent: Size<bool>,
    /// An answer given with no known height reads the available height:
    /// whether it is definite, min- or max-content, and how much.
    pub available_height: bool,
    /// A width probe (`RequestedAxis::Horizontal`) reads the height it is
    /// given, known or available.
    pub width_reads_height: bool,
}

impl Reads {
    /// Everything: what Taffy's own cache assumes, but the parent size,
    /// read on the axes `parent` says.
    #[must_use]
    pub const fn all(parent: Size<bool>) -> Self {
        Self {
            parent,
            available_height: true,
            width_reads_height: true,
        }
    }
}

/// What an entry was computed from. Kinds and presence sit apart from the
/// bits they qualify, so the key stays 24 bytes (it is in every entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    space: [u32; 2],
    kind: [Kind; 2],
    /// The parent size's bits, 0 where it has none ([`Key::has_parent`]).
    parent: [u32; 2],
    has_parent: [bool; 2],
    /// Whether each known dimension is definite (normalized to `true` where none is known).
    definite: [bool; 2],
    /// 1 horizontal, 2 vertical, 3 both.
    axis: u8,
    /// A size probe (else a final layout).
    probe: bool,
}

const _: () = assert!(size_of::<Key>() == 24, "a key is in every cache entry");

impl Key {
    fn of(input: &LayoutInput) -> Self {
        let k = input.known_dimensions;
        let a = input.available_space;
        let (w, h) = (axis_key(k.width, a.width), axis_key(k.height, a.height));
        let p = input.parent_size;
        Self {
            space: [w.1, h.1],
            kind: [w.0, h.0],
            parent: [p.width.map_or(0, f32::to_bits), p.height.map_or(0, f32::to_bits)],
            has_parent: [p.width.is_some(), p.height.is_some()],
            definite: [
                input.known_dimensions_are_definite.width || k.width.is_none(),
                input.known_dimensions_are_definite.height || k.height.is_none(),
            ],
            axis: match input.axis {
                RequestedAxis::Horizontal => 1,
                RequestedAxis::Vertical => 2,
                RequestedAxis::Both => 3,
            },
            probe: input.run_mode == RunMode::ComputeSize,
        }
    }

    /// Whether an entry under `self` answers a request under `want`, for a
    /// node whose answers read what `reads` says; `free`: this entry's
    /// answer was found not to read the available height.
    fn answers(&self, want: &Self, reads: Reads, free: bool) -> bool {
        let space = |i: usize| self.space.get(i) == want.space.get(i) && self.kind.get(i) == want.kind.get(i);
        let parent =
            |i: usize| self.parent.get(i) == want.parent.get(i) && self.has_parent.get(i) == want.has_parent.get(i);
        let width = space(0)
            && self.definite[0] == want.definite[0]
            && (!reads.parent.width || parent(0))
            && (self.axis == 3 || self.axis == want.axis);
        if !width {
            return false;
        }
        // A width probe of a node whose width reads no height: any height
        // it is given gives the same width.
        if want.probe && want.axis == 1 && !reads.width_reads_height {
            return true;
        }
        let height = if (free || !reads.available_height) && self.kind[1] != Kind::Known && want.kind[1] != Kind::Known
        {
            true
        } else {
            space(1)
        };
        height && self.definite[1] == want.definite[1] && (!reads.parent.height || parent(1))
    }
}

/// The inputs an entry's key leaves out, the same for every entry (as
/// Taffy's `CacheMode`): the sizing mode and which vertical margins collapse.
fn mode_of(input: &LayoutInput) -> u8 {
    let mut m = 0;
    if input.sizing_mode == SizingMode::InherentSize {
        m |= 1;
    }
    if input.vertical_margins_are_collapsible.start {
        m |= 2;
    }
    if input.vertical_margins_are_collapsible.end {
        m |= 4;
    }
    m
}

/// An answer the node gave, and what it was asked.
#[derive(Clone, Debug)]
pub struct Answer<T = LayoutOutput> {
    pub input: LayoutInput,
    pub output: T,
    /// It was laid out in its parent's block formatting context.
    pub in_flow: bool,
    /// The algorithm that gave it found it read no available height (a
    /// grid whose rows came out the same under any).
    pub free_height: bool,
}

/// A size probe's answer: the size alone, as Taffy rebuilds a hit.
type Probe = Answer<Size<f32>>;

/// A node's cached results.
#[derive(Clone, Debug, Default)]
pub struct NodeCache {
    mode: u8,
    layout: Option<(Key, Answer)>,
    sizes: Vec<(Key, Probe)>,
    /// Which of `sizes` were used since the replacing hand last passed them.
    used: u32,
    hand: usize,
    /// An answer since the last clear is not here (replaced, or never
    /// kept): the entries no longer list everything the parent was told.
    lost: bool,
    /// Bumped by each clear: answers kept elsewhere under the same
    /// generation are still the node's answers.
    generation: u32,
    /// Bumped by each final layout kept, and when its subtree is placed
    /// anew for no answer it keeps ([`NodeCache::unplace`]): the node's
    /// children are placed for the final layout kept under the same serial.
    serial: u32,
    /// Its subtree is placed for the final layout `layout` holds.
    placed: bool,
    /// A leaf whose measure reads the width alone
    /// ([`super::MeasureReads::width_alone`]): what it measured at the last
    /// two widths given, the newer second. Kept in place, not on the heap:
    /// a text is measured at its max- and min-content widths, then at the
    /// width it is given, which its final layout asks again.
    measures: [Option<(u64, Measured)>; 2],
}

/// A width given, as [`NodeCache::measured`] keys it.
fn width_key(width: AvailableSpace) -> u64 {
    match width {
        AvailableSpace::Definite(w) => u64::from(w.to_bits()),
        AvailableSpace::MinContent => 1 << 32,
        AvailableSpace::MaxContent => 2 << 32,
    }
}

impl NodeCache {
    /// Whether it holds a final layout its subtree is placed for.
    pub fn has_layout(&self) -> bool {
        self.placed && self.layout.is_some()
    }

    /// A run of its own algorithm that was not a final layout (a size
    /// probe) laid a descendant out for a final layout of its own: Taffy's
    /// grid and flexbox find baselines so, and a block places its floats
    /// so. That descendant's subtree is placed for the probe now, not for
    /// the final layout this node keeps, so that answer no longer stands
    /// for a final layout (which places the subtree, as a hit would not);
    /// it is still the node's answer for [`NodeCache::take_check`].
    pub fn unplace(&mut self) {
        if self.placed {
            self.placed = false;
            self.serial = self.serial.wrapping_add(1);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.layout.is_none() && self.sizes.is_empty()
    }

    /// Which clear the answers it gives date from.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Which final layout its node's children are placed for.
    pub fn serial(&self) -> u32 {
        self.serial
    }

    /// What its leaf measured at `width` since the last clear.
    pub fn measured(&self, width: AvailableSpace) -> Option<Measured> {
        let key = width_key(width);
        self.measures.iter().flatten().find(|(k, _)| *k == key).map(|&(_, m)| m)
    }

    /// Keep what its leaf measured at `width` (the older of the two goes).
    pub fn keep_measured(&mut self, width: AvailableSpace, m: Measured) {
        let [_, newer] = self.measures;
        self.measures = [newer, Some((width_key(width), m))];
    }

    pub fn clear(&mut self) {
        self.measures = [None, None];
        self.generation = self.generation.wrapping_add(1);
        self.layout = None;
        self.sizes.clear();
        self.used = 0;
        self.hand = 0;
        self.lost = false;
        self.placed = false;
    }

    /// Note an answer given without being kept.
    pub fn lose(&mut self) {
        self.lost = true;
    }

    /// Every answer given since the last clear, when the cache still holds
    /// them all; the cache is left empty either way.
    pub fn take_check(&mut self) -> Option<Vec<Answer>> {
        let complete = !self.lost && !self.is_empty();
        let mut out: Vec<Answer> = self
            .sizes
            .drain(..)
            .map(|(_, a)| Answer {
                input: a.input,
                output: LayoutOutput::from_outer_size(a.output),
                in_flow: a.in_flow,
                free_height: a.free_height,
            })
            .collect();
        out.extend(self.layout.take().map(|(_, a)| a));
        self.clear();
        complete.then_some(out)
    }

    /// Whether it holds a result for `input` (nothing marked used).
    pub fn has(&self, input: &LayoutInput, reads: Reads) -> bool {
        if mode_of(input) != self.mode {
            return false;
        }
        let want = Key::of(input);
        match input.run_mode {
            RunMode::PerformLayout => self
                .layout
                .as_ref()
                .is_some_and(|(k, a)| self.placed && k.answers(&want, reads, a.free_height) && k.axis == 3),
            RunMode::ComputeSize => self.sizes.iter().any(|(k, a)| k.answers(&want, reads, a.free_height)),
            RunMode::PerformHiddenLayout => false,
        }
    }

    /// Whether the result cached for `input` was found to read no available
    /// height ([`Answer::free_height`]).
    pub fn is_free(&self, input: &LayoutInput, reads: Reads) -> bool {
        let want = Key::of(input);
        match input.run_mode {
            RunMode::PerformLayout => self.layout.as_ref().is_some_and(|(k, a)| {
                self.placed && k.answers(&want, reads, a.free_height) && k.axis == 3 && a.free_height
            }),
            RunMode::ComputeSize => self
                .sizes
                .iter()
                .find(|(k, a)| k.answers(&want, reads, a.free_height))
                .is_some_and(|(_, a)| a.free_height),
            RunMode::PerformHiddenLayout => false,
        }
    }

    /// The result cached for `input`, for a node whose answers read what `reads` says.
    pub fn get(&mut self, input: &LayoutInput, reads: Reads) -> Option<LayoutOutput> {
        if mode_of(input) != self.mode {
            return None;
        }
        let want = Key::of(input);
        match input.run_mode {
            RunMode::PerformLayout => self
                .layout
                .as_ref()
                .filter(|(k, a)| self.placed && k.answers(&want, reads, a.free_height) && k.axis == 3)
                .map(|(_, a)| a.output.clone()),
            RunMode::ComputeSize => {
                let found = self
                    .sizes
                    .iter()
                    .position(|(k, a)| k.answers(&want, reads, a.free_height))?;
                self.used |= 1_u32.checked_shl(u32::try_from(found).ok()?).unwrap_or(0);
                self.sizes
                    .get(found)
                    .map(|(_, a)| LayoutOutput::from_outer_size(a.output))
            }
            RunMode::PerformHiddenLayout => None,
        }
    }

    /// Keep `output` as the answer to `input`; `free_height`: the algorithm
    /// found it read no available height ([`Answer::free_height`]).
    pub fn store(&mut self, input: &LayoutInput, output: &LayoutOutput, in_flow: bool, free_height: bool) {
        if input.run_mode == RunMode::PerformHiddenLayout {
            return;
        }
        let mode = mode_of(input);
        if mode != self.mode {
            let had = !self.is_empty();
            // What its leaf measured does not depend on the sizing mode
            // (the same content, the same width): the answers a parent's
            // pair measure kept stand.
            let measures = self.measures;
            self.clear();
            self.measures = measures;
            self.lost = had;
            self.mode = mode;
        }
        let key = Key::of(input);
        match input.run_mode {
            RunMode::PerformLayout => {
                if self.layout.as_ref().is_some_and(|(k, _)| *k != key) {
                    self.lost = true;
                }
                self.serial = self.serial.wrapping_add(1);
                self.placed = true;
                self.layout = Some((
                    key,
                    Answer {
                        input: *input,
                        output: output.clone(),
                        in_flow,
                        free_height,
                    },
                ));
            }
            RunMode::ComputeSize => {
                // A size alone cannot carry margin-collapsing results (Taffy rebuilds
                // a hit with `from_outer_size`), so such results are not kept.
                if output.margins_can_collapse_through
                    || output.top_margin != CollapsibleMarginSet::ZERO
                    || output.bottom_margin != CollapsibleMarginSet::ZERO
                {
                    self.lost = true;
                    return;
                }
                let answer = Probe {
                    input: *input,
                    output: output.size,
                    in_flow,
                    free_height,
                };
                if let Some(at) = self.sizes.iter().position(|(k, _)| *k == key) {
                    if let Some(slot) = self.sizes.get_mut(at) {
                        slot.1 = answer;
                    }
                    self.mark(at);
                    return;
                }
                if self.sizes.len() < MEASURE_SLOTS {
                    if self.sizes.capacity() == 0 {
                        self.sizes.reserve_exact(4);
                    }
                    self.sizes.push((key, answer));
                    let at = self.sizes.len().saturating_sub(1);
                    self.mark(at);
                    return;
                }
                // Second chance: pass over the entries used since the hand last
                // came by, clearing their marks, and replace the first unused one.
                while self.is_marked(self.hand) {
                    self.unmark(self.hand);
                    self.hand = (self.hand.saturating_add(1)) % MEASURE_SLOTS;
                }
                let at = self.hand;
                if let Some(slot) = self.sizes.get_mut(at) {
                    *slot = (key, answer);
                }
                self.lost = true;
                self.mark(at);
                self.hand = (self.hand.saturating_add(1)) % MEASURE_SLOTS;
            }
            RunMode::PerformHiddenLayout => {}
        }
    }

    fn bit(at: usize) -> u32 {
        u32::try_from(at).ok().and_then(|s| 1_u32.checked_shl(s)).unwrap_or(0)
    }

    fn mark(&mut self, at: usize) {
        self.used |= Self::bit(at);
    }

    fn unmark(&mut self, at: usize) {
        self.used &= !Self::bit(at);
    }

    fn is_marked(&self, at: usize) -> bool {
        self.used & Self::bit(at) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taffy::Line;

    fn measure(width: f32, parent: Option<f32>) -> LayoutInput {
        LayoutInput {
            run_mode: RunMode::ComputeSize,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size {
                width: Some(width),
                height: None,
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: parent,
                height: None,
            },
            available_space: Size {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            vertical_margins_are_collapsible: Line::FALSE,
        }
    }

    fn perform(width: f32, height: f32, parent: (f32, f32)) -> LayoutInput {
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            known_dimensions: Size {
                width: Some(width),
                height: Some(height),
            },
            parent_size: Size {
                width: Some(parent.0),
                height: Some(parent.1),
            },
            ..measure(width, None)
        }
    }

    fn out(v: f32) -> LayoutOutput {
        LayoutOutput::from_outer_size(Size { width: v, height: v })
    }

    const NONE: Reads = Reads::all(Size {
        width: false,
        height: false,
    });
    const BOTH: Reads = Reads::all(Size {
        width: true,
        height: true,
    });

    #[test]
    fn a_node_that_reads_no_percentage_keeps_its_layout_when_its_parent_grows() {
        let mut c = NodeCache::default();
        c.store(&perform(100.0, 20.0, (100.0, 5000.0)), &out(1.0), false, false);
        assert!(c.get(&perform(100.0, 20.0, (100.0, 5020.0)), NONE).is_some());
        assert!(c.get(&perform(100.0, 20.0, (100.0, 5020.0)), BOTH).is_none());
        assert!(c.get(&perform(101.0, 20.0, (100.0, 5000.0)), NONE).is_none());
    }

    #[test]
    fn more_probes_than_slots_keep_the_ones_used_since() {
        let mut c = NodeCache::default();
        for i in 0..MEASURE_SLOTS {
            c.store(
                &measure(f32::from(u16::try_from(i).unwrap()), None),
                &out(1.0),
                false,
                false,
            );
        }
        assert!(c.get(&measure(3.0, None), NONE).is_some());
        c.store(&measure(100.0, None), &out(2.0), false, false);
        c.store(&measure(101.0, None), &out(2.0), false, false);
        assert!(c.get(&measure(100.0, None), NONE).is_some());
        assert_eq!(c.sizes.len(), MEASURE_SLOTS);
    }

    #[test]
    fn the_answers_are_checked_only_while_all_are_kept() {
        let mut c = NodeCache::default();
        c.store(&measure(1.0, None), &out(1.0), false, false);
        c.store(&perform(1.0, 1.0, (1.0, 1.0)), &out(1.0), false, false);
        assert_eq!(c.take_check().map(|a| a.len()), Some(2));
        assert!(c.is_empty());
        c.store(&perform(1.0, 1.0, (1.0, 1.0)), &out(1.0), false, false);
        c.store(&perform(2.0, 1.0, (1.0, 1.0)), &out(1.0), false, false);
        assert!(c.take_check().is_none(), "the first final layout was replaced");
        for i in 0..=MEASURE_SLOTS {
            c.store(
                &measure(f32::from(u16::try_from(i).unwrap()), None),
                &out(1.0),
                false,
                false,
            );
        }
        assert!(c.take_check().is_none(), "a probe was replaced");
    }

    #[test]
    fn an_answer_that_reads_no_available_height_serves_every_one() {
        let free = Reads {
            available_height: false,
            ..NONE
        };
        let probe = |height: AvailableSpace| LayoutInput {
            axis: RequestedAxis::Vertical,
            available_space: Size {
                width: AvailableSpace::Definite(100.0),
                height,
            },
            ..measure(100.0, None)
        };
        let mut c = NodeCache::default();
        c.store(&probe(AvailableSpace::MinContent), &out(1.0), false, false);
        assert!(c.get(&probe(AvailableSpace::MaxContent), free).is_some());
        assert!(c.get(&probe(AvailableSpace::Definite(24.0)), free).is_some());
        assert!(c.get(&probe(AvailableSpace::MaxContent), NONE).is_none());
        let known = LayoutInput {
            known_dimensions: Size {
                width: Some(100.0),
                height: Some(24.0),
            },
            ..probe(AvailableSpace::MinContent)
        };
        assert!(c.get(&known, free).is_none(), "a known height is read");
    }

    #[test]
    fn a_width_that_reads_no_height_serves_every_height() {
        let free = Reads {
            available_height: false,
            width_reads_height: false,
            ..NONE
        };
        let width = |known: Option<f32>, height: AvailableSpace| LayoutInput {
            axis: RequestedAxis::Horizontal,
            known_dimensions: Size {
                width: None,
                height: known,
            },
            available_space: Size {
                width: AvailableSpace::MinContent,
                height,
            },
            ..measure(0.0, None)
        };
        let mut c = NodeCache::default();
        c.store(&width(None, AvailableSpace::MaxContent), &out(1.0), false, false);
        assert!(
            c.get(&width(Some(20.0), AvailableSpace::Definite(20.0)), free)
                .is_some()
        );
        assert!(
            c.get(&width(Some(20.0), AvailableSpace::Definite(20.0)), NONE)
                .is_none()
        );
        let height = LayoutInput {
            axis: RequestedAxis::Vertical,
            ..width(None, AvailableSpace::MaxContent)
        };
        assert!(c.get(&height, free).is_none(), "a width probe answers no height probe");
    }

    #[test]
    fn an_unplaced_final_layout_answers_no_final_layout_but_is_still_checked() {
        let mut c = NodeCache::default();
        let input = perform(100.0, 20.0, (100.0, 20.0));
        c.store(&input, &out(1.0), false, false);
        let serial = c.serial();
        assert!(c.get(&input, NONE).is_some());
        c.unplace();
        assert_ne!(c.serial(), serial, "a memo of the final layout misses too");
        assert!(c.get(&input, NONE).is_none());
        assert!(!c.has(&input, NONE) && !c.has_layout());
        assert_eq!(c.take_check().map(|a| a.len()), Some(1));
        c.store(&input, &out(1.0), false, false);
        assert!(c.get(&input, NONE).is_some(), "laid out again, it is placed");
    }

    #[test]
    fn a_known_size_and_an_available_space_never_share_a_key() {
        // Frontiers 6.1: a known width `1.5` and an available `Definite(-1.0)`
        // once folded to the same `u32`; Taffy's block intrinsic sizing can
        // probe a child at a negative available width.
        let available = |width: AvailableSpace, parent: Option<f32>| LayoutInput {
            known_dimensions: Size {
                width: None,
                height: None,
            },
            available_space: Size {
                width,
                height: AvailableSpace::MaxContent,
            },
            ..measure(0.0, parent)
        };
        let mut c = NodeCache::default();
        c.store(&measure(1.5, None), &out(1.0), false, false);
        assert!(c.get(&measure(1.5, None), BOTH).is_some());
        assert!(c.get(&available(AvailableSpace::Definite(-1.0), None), BOTH).is_none());
        // And every kind against every other, at the bits that once met.
        let kinds = [-1.0_f32, 1.5, f32::INFINITY, f32::NEG_INFINITY]
            .into_iter()
            .flat_map(|v| [measure(v, None), available(AvailableSpace::Definite(v), None)])
            .chain([
                available(AvailableSpace::MinContent, None),
                available(AvailableSpace::MaxContent, None),
                available(AvailableSpace::MaxContent, Some(f32::INFINITY)),
            ])
            .collect::<Vec<_>>();
        for (i, a) in kinds.iter().enumerate() {
            for (j, b) in kinds.iter().enumerate() {
                assert_eq!(Key::of(a) == Key::of(b), i == j, "{a:?} against {b:?}");
            }
        }
    }

    #[test]
    fn another_mode_misses_and_storing_under_it_drops_the_rest() {
        let mut c = NodeCache::default();
        c.store(&measure(1.0, None), &out(1.0), false, false);
        let other = LayoutInput {
            sizing_mode: SizingMode::ContentSize,
            ..measure(1.0, None)
        };
        assert!(c.get(&other, NONE).is_none());
        c.store(&other, &out(2.0), false, false);
        assert!(c.get(&measure(1.0, None), NONE).is_none());
        assert!(c.get(&other, NONE).is_some());
    }
}
