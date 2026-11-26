//! A table's columns over a virtual list (ADR-0014): every row is a grid
//! on the same tracks, so cells line up without the rows being laid out
//! together. A [`Width::Auto`] column is as wide as the widest cell seen so
//! far (the rows laid out, not all of them, which a list of a million rows
//! never lays out) and only grows, so scrolling never makes a column jump
//! narrower under the pointer.

use std::rc::Rc;

use crate::element::{Div, Element, IntoElement, div};
use crate::engine::{ForkSlab, SlabId};
use crate::prelude::*;
use crate::reactive::{Signal, detached};
use crate::style::Track;

/// How a column's width is found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    Px(f32),
    /// A share of what the fixed and `Auto` columns leave.
    Fr(f32),
    /// The widest cell seen, within `min..=max`.
    Auto {
        min: f32,
        max: f32,
    },
}

impl Width {
    /// The widest cell seen, from 0 up.
    pub const AUTO: Self = Self::Auto {
        min: 0.0,
        max: f32::INFINITY,
    };
}

/// A column model's state, in [`COLUMNS`].
struct State {
    /// Never written after it is made: a check's copy shares it.
    widths: Rc<[Width]>,
    /// The widest cell seen in each column (0 where none was).
    seen: Seen,
    /// Bumped when an `Auto` column grew: the rows take the new tracks.
    version: Signal<u64>,
}

/// The widest cell seen in each column. Live, `base` is written in place
/// (alone, so with no allocation); a check's copy shares `base` and keeps
/// what it saw in `over`, its own, so its first write costs O(1) in the
/// columns it did not write, however many there are (ADR-0034, S7).
struct Seen {
    base: Rc<Vec<f32>>,
    /// A check's copy's widths, by column (`None`: live, which has none).
    over: Option<Vec<(usize, f32)>>,
}

impl Seen {
    /// Column `i`'s widest cell seen; `None` past the last column.
    fn get(&self, i: usize) -> Option<f32> {
        let over = self.over.as_deref().unwrap_or_default();
        over.iter()
            .find(|(c, _)| *c == i)
            .map(|(_, w)| *w)
            .or_else(|| self.base.get(i).copied())
    }

    /// Set column `i`'s (one that exists) to `width`.
    fn set(&mut self, i: usize, width: f32) {
        match &mut self.over {
            Some(over) => match over.iter_mut().find(|(c, _)| *c == i) {
                Some(entry) => entry.1 = width,
                None => over.push((i, width)),
            },
            // Live: alone, so in place (a copy only if a check's copy
            // outlived its check, which would be a bug in gui).
            None => {
                if let Some(seen) = Rc::make_mut(&mut self.base).get_mut(i) {
                    *seen = width;
                }
            }
        }
    }
}

impl std::fmt::Debug for Seen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries((0..self.base.len()).filter_map(|i| self.get(i)))
            .finish()
    }
}

// A logged value (ADR-0034 S7, audited): `&State` changes nothing.
crate::engine::fork_value_audited!(State);
impl crate::engine::parts_seal::Sealed for State {}
impl crate::engine::Parts for State {
    /// Each field that drops anything on its own box (S9, review r17).
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        // Every field named (the drain lint).
        let Self { widths, seen, version } = self;
        let Seen { base, over } = seen;
        crate::engine::Parts::parts(widths, out);
        crate::engine::Parts::parts(base, out);
        crate::engine::whole(over, out);
        // A signal handle: nothing to drop.
        let _data = version;
    }
}
impl crate::engine::sealed::Sealed for State {}
impl crate::engine::ForkClone for State {
    /// O(1): `widths` and the live widths seen are shared (two counts
    /// bumped), the copy's own widths seen start empty (no allocation), and
    /// its writes cost O(the columns it wrote). No callback is held.
    fn fork_clone(&self) -> Self {
        Self {
            widths: Rc::clone(&self.widths),
            seen: Seen {
                base: Rc::clone(&self.seen.base),
                over: Some(self.seen.over.clone().unwrap_or_default()),
            },
            version: self.version,
        }
    }
}

crate::engine::engine_local! {
    /// Every column model's state, by its id (ADR-0028, rule 4: a handle
    /// is an id): a cold layout that measures a cell writes the fork's.
    [Fork(log)] static COLUMNS: std::cell::RefCell<ForkSlab<State>> = std::cell::RefCell::default();
}

/// A table's column model: a `Copy` handle, owned by the scope it is made
/// in (made outside any, it lives until [`Self::dispose`]). Once its owner
/// ended it is inert: it has no columns and notes nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Columns(SlabId);

impl std::fmt::Debug for Columns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.peek(|s| {
            f.debug_struct("Columns")
                .field("widths", &s.widths)
                .field("seen", &s.seen)
                .finish()
        })
        .unwrap_or_else(|| f.write_str("Columns(disposed)"))
    }
}

impl Columns {
    #[must_use]
    pub fn new(widths: impl IntoIterator<Item = Width>) -> Self {
        let widths: Rc<[Width]> = widths.into_iter().collect();
        let seen = Seen {
            base: Rc::new(vec![0.0; widths.len()]),
            over: None,
        };
        let version = detached(|| Signal::labeled("table columns", 0));
        let id = crate::engine::allocate(&COLUMNS, State { widths, seen, version });
        let me = Self(id);
        crate::engine::owned_by_scope(move || me.dispose());
        me
    }

    /// Let its state go now: it is inert from here on.
    pub fn dispose(self) {
        // `try_with`: dropped as the thread's state is torn down, it is gone.
        if let Ok(Some(s)) = COLUMNS.try_with(|c| c.borrow_mut().remove(self.0)) {
            s.version.dispose_quietly();
        }
    }

    /// Whether its owner has not ended.
    #[must_use]
    pub fn is_alive(self) -> bool {
        COLUMNS.with(|c| c.borrow().contains(self.0))
    }

    /// Read its state; `None` once it is gone, which its owner's code
    /// reaching it is a bug, loud in a debug build
    /// ([`crate::engine::check_alive`]).
    #[track_caller]
    fn read<R>(self, f: impl FnOnce(&State) -> R) -> Option<R> {
        let read = self.peek(f);
        #[cfg(debug_assertions)]
        crate::engine::check_alive(read.is_some(), "column set");
        read
    }

    /// [`Self::read`], quiet once it is gone: for the engine's own paths
    /// (a cell measured once more as its tree is torn down).
    fn peek<R>(self, f: impl FnOnce(&State) -> R) -> Option<R> {
        COLUMNS.with(|c| c.borrow().read(self.0, f))
    }

    /// Its owner's code uses it: gone, a loud bug in a debug build.
    #[track_caller]
    fn used(self) {
        #[cfg(debug_assertions)]
        crate::engine::check_alive(self.is_alive(), "column set");
    }

    /// The tracks every row lays its cells on (read, so a column that grew
    /// renders the rows again).
    #[must_use]
    pub fn tracks(&self) -> Vec<Track> {
        let Some(version) = self.read(|s| s.version) else {
            return Vec::new();
        };
        version.with(|_| ());
        self.read(|s| {
            s.widths
                .iter()
                .enumerate()
                .map(|(i, w)| (w, s.seen.get(i).unwrap_or(0.0)))
                .map(|(w, seen)| match *w {
                    Width::Px(v) => Track::Px(v),
                    Width::Fr(f) => Track::Fr(f),
                    Width::Auto { min, max } => Track::Px(seen.clamp(min, max.max(min))),
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// The widest cell seen in `column`.
    #[must_use]
    pub fn seen(&self, column: usize) -> f32 {
        self.read(|s| s.seen.get(column).unwrap_or(0.0)).unwrap_or(0.0)
    }

    /// Whether `column` is as wide as its widest cell.
    fn is_auto(self, column: usize) -> bool {
        self.peek(|s| matches!(s.widths.get(column), Some(Width::Auto { .. })))
            .unwrap_or(false)
    }

    /// Note a cell of `column` `width` pixels wide; whether the column grew.
    #[track_caller]
    pub fn observe(&self, column: usize, width: f32) -> bool {
        self.used();
        self.note(column, width)
    }

    /// [`Self::observe`] from a cell's measure: quiet once it is gone.
    fn note(self, column: usize, width: f32) -> bool {
        if !self.is_auto(column) {
            return false;
        }
        let grew = COLUMNS.with(|c| {
            c.borrow().write(self.0, |s| match s.seen.get(column) {
                Some(seen) if width > seen + 0.01 => {
                    s.seen.set(column, width);
                    Some(s.version)
                }
                _ => None,
            })
        });
        match grew.flatten() {
            Some(version) => {
                version.update(|v| *v = v.wrapping_add(1));
                true
            }
            None => false,
        }
    }

    /// `cells` as a row lays them on [`Self::tracks`]: an `Auto` column's
    /// cell has its content at its own width (a flex item that does not
    /// shrink), measured as it is laid out and clipped to the track until
    /// the column grows to it.
    #[track_caller]
    pub fn cells<E: IntoElement>(&self, cells: impl IntoIterator<Item = E>) -> Vec<Element> {
        self.used();
        cells
            .into_iter()
            .enumerate()
            .map(|(i, cell)| {
                if self.is_auto(i) {
                    let c = *self;
                    div()
                        .display(Display::Flex)
                        .min_width(0.0)
                        .overflow(Overflow::Hidden)
                        .child(
                            div()
                                .flex(Flex::NONE)
                                .on_measured(move |b| {
                                    c.note(i, b.width);
                                })
                                .child(cell),
                        )
                        .into_element()
                } else {
                    cell.into_element()
                }
            })
            .collect()
    }

    /// A bare row of `cells` on the columns' tracks.
    pub fn row<E: IntoElement>(&self, cells: impl IntoIterator<Item = E>) -> Div {
        div()
            .display(Display::Grid)
            .grid_columns(self.tracks())
            .children(self.cells(cells))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_columns_only_grow() {
        let c = Columns::new([
            Width::Px(40.0),
            Width::AUTO,
            Width::Fr(1.0),
            Width::Auto { min: 50.0, max: 80.0 },
        ]);
        assert_eq!(
            c.tracks(),
            vec![Track::Px(40.0), Track::Px(0.0), Track::Fr(1.0), Track::Px(50.0)]
        );
        assert!(c.observe(1, 120.0));
        assert!(!c.observe(1, 90.0));
        assert!(!c.observe(0, 500.0));
        assert!(c.observe(3, 200.0));
        assert_eq!(
            c.tracks(),
            vec![Track::Px(40.0), Track::Px(120.0), Track::Fr(1.0), Track::Px(80.0)]
        );
    }

    crate::engine::engine_local! {
        #[cfg(debug_assertions)]
        [Shared("a test's live columns, made by whatever first reads them")] static SHARED_COLUMNS: Columns = Columns::new([Width::AUTO]);
    }

    /// Codex 1 (F1 review 5), F1 review 9: a Shared piece first read in a
    /// check gets the fork's stand-in, gone with it; its columns are made
    /// live at the first live read (ADR-0028), and stay alive, with what
    /// live code wrote.
    #[cfg(debug_assertions)]
    #[test]
    fn columns_a_shared_init_makes_in_a_check_stay_alive_after_it() {
        let stand_in = {
            let _cold = crate::engine::ColdFrame::begin();
            SHARED_COLUMNS.with(|c| *c)
        };
        assert!(!stand_in.is_alive(), "the fork's, gone with it");
        let made = SHARED_COLUMNS.with(|c| *c);
        assert!(made.is_alive(), "made live after the check");
        assert!(made.observe(0, 30.0), "and written live");
        assert_eq!(SHARED_COLUMNS.with(|c| c.tracks()), vec![Track::Px(30.0)]);
    }

    /// Widths a fork's rows report (ADR-0028, the alias test) are its own:
    /// after it the columns, and a copy made before it, are as they were.
    #[cfg(debug_assertions)]
    #[test]
    fn widths_a_fork_observed_are_put_back() {
        let c = Columns::new([Width::AUTO, Width::Px(10.0)]);
        assert!(c.observe(0, 30.0));
        let alias = c;
        let before = (c.tracks(), c.seen(0));
        {
            let _cold = crate::engine::ColdFrame::begin();
            assert!(c.observe(0, 300.0));
            assert_eq!(alias.seen(0), 300.0, "the fork sees its write through any copy");
        }
        assert_eq!((c.tracks(), c.seen(0)), before, "put back");
        assert_eq!(
            (alias.tracks(), alias.seen(0)),
            before,
            "the copy made before reads the same"
        );
        assert!(c.observe(0, 300.0), "the live columns grow anew");
    }

    /// ADR-0034, S7: a check's first write to a table of many columns
    /// copies none of them: the widths and the widths seen are the live
    /// ones, shared, and what it saw is its own; live is as it was after.
    #[cfg(debug_assertions)]
    #[test]
    fn a_checks_first_write_to_many_columns_copies_none_of_them() {
        let n = 20_000;
        let c = Columns::new(std::iter::repeat_n(Width::AUTO, n));
        for i in 0..n {
            let _grew = c.observe(i, 10.0);
        }
        let roots = |c: Columns| {
            c.peek(|s| (Rc::clone(&s.widths), Rc::clone(&s.seen.base)))
                .expect("alive")
        };
        let (widths, seen) = roots(c);
        let (shared, bytes) = crate::engine::in_cold_frame(|| {
            crate::heap::allocated_bytes(|| {
                assert!(c.observe(7, 300.0), "grew in the check");
                assert_eq!(c.seen(7), 300.0, "the check reads its write");
                let (w, s) = roots(c);
                Rc::ptr_eq(&w, &widths) && Rc::ptr_eq(&s, &seen)
            })
        });
        assert!(shared, "the check's copy shares the live columns");
        // The 20,000 widths seen are 80 KB: the copy is the fork's own
        // bookkeeping and one width.
        assert!(bytes < 8 * 1024, "{bytes} bytes for one write to one column");
        assert_eq!((c.seen(7), c.seen(8)), (10.0, 10.0), "live is as it was");
        assert!(c.observe(7, 300.0), "and grows anew");
    }

    /// A row built while its columns' owner ran, by a longer-lived scope,
    /// and laid out after the columns' owner ended: its `Auto` cell's
    /// measure is the engine's path, quiet on the gone columns (Codex, F1
    /// review 1).
    #[test]
    fn a_row_laid_out_after_its_columns_ended_is_inert() {
        let owner = crate::Scope::new();
        let rows = crate::Scope::new();
        let columns = owner.run(|| Columns::new([Width::AUTO, Width::Px(10.0)]));
        let row = rows.run(|| {
            columns
                .row([crate::element::text("cell"), crate::element::text("x")])
                .into_element()
        });
        owner.dispose();
        assert!(!columns.is_alive());
        let row = Rc::new(std::cell::RefCell::new(Some(row)));
        let mut s = crate::app::Scenario::measuring(
            move || {
                div()
                    .width(300.0)
                    .child(row.borrow_mut().take().unwrap_or_else(|| div().into_element()))
                    .into_element()
            },
            300,
            100,
            1.0,
        )
        .expect("a scenario");
        // The view hands over one element built before.
        s.check_cold(false);
        s.settle().expect("settles");
        assert!(format!("{columns:?}").contains("disposed"), "its Debug is quiet too");
        rows.dispose();
    }
}
