//! The windowed list (ADR-0026, K4c): a virtual list whose rows are a
//! keyed [`each`] and whose source is the window.
//!
//! - **Built once.** [`virtual_each`] builds the list's boxes once; what
//!   changes after is the [`each`] of its rows. Its source is the window:
//!   the rows in view and prefetched, the rows pinned outside it (the one
//!   holding focus and those kept around it, the active row), and the room
//!   below the last row, in data order. It reads the list's state (the
//!   anchor, the window, the heights) and the data (`rows`, `key`), so a
//!   window moved by the list's layout hook, a scroll asked for or data
//!   that changed marks the [`each`] alone, and its commit mounts the rows
//!   that came and disposes the ones that left. Nothing else is built
//!   again, and the owner's body does not run.
//! - **A row is its key.** A kept key is the same row: its scope, its state,
//!   its focus and its node, wherever data moves it, pinned or in the
//!   window. Its row of the data and whether it is pinned are its item, so a
//!   row moved by data is written its new index ([`ListRow::index`]) and a
//!   row that scrolls out while it holds focus is placed outside the
//!   viewport by its own bindings. A key that leaves the window is
//!   unmounted and its scope disposed; one that comes back is a new mount
//!   (no row is pooled across keys, ADR-0026).
//! - **The plan** the source works out (the window, the keys, the pins, the
//!   anchor followed onto its key's new row) is what the layout hook takes
//!   in after the rows it built are laid out, as a render's is
//!   ([`super::ListState::take`]). The source keeps it in the list's state
//!   ([`super::State::planned`]), the one it kept while an equal one comes
//!   again, so a plan is taken once; a cold frame keeps its own in its copy
//!   of the state.

use std::rc::Rc;

use super::{Index, LISTS, ListState, Plan, pixels, resize, row_wrapper, thumb, total};
use crate::component::{Row, each};
use crate::element::{Div, Element, ElementKey, IntoElement, Role, div};
use crate::prelude::*;

/// What a row of the window is: a row of the data, in the window or pinned
/// outside it, or the room below the last row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    /// Row `row` of the data, keyed `key`; `pinned`: kept built outside
    /// the window, above the viewport where nothing shows it.
    Row { row: u64, key: ElementKey, pinned: bool },
    /// The room below the last row: its height's bits, in pixels.
    End { height: u32 },
}

impl Slot {
    /// Its key in the [`each`]: a row's own, `None` for the room below.
    fn key(&self) -> Option<ElementKey> {
        match *self {
            Self::Row { key, .. } => Some(key),
            Self::End { .. } => None,
        }
    }
}

/// A row of a windowed list's [`each`].
pub(super) type RowOf = Row<Slot, Option<ElementKey>>;

/// Row `r`'s row of the data, tracked.
pub(super) fn row_of(r: RowOf) -> u64 {
    r.with_item(|s| match *s {
        Slot::Row { row, .. } => row,
        Slot::End { .. } => 0,
    })
}

/// What a windowed list's scroll container hands its layout hook: the
/// plan is the one the list's state keeps ([`super::State::planned`]).
pub(super) struct Windowed;

/// A row of a [`virtual_each`], handed to its view once, when its key comes
/// into the window: a `Copy` handle.
#[derive(Clone, Copy, Debug)]
pub struct ListRow {
    row: RowOf,
    key: ElementKey,
}

impl ListRow {
    /// Its row of the data, tracked: a binding that reads it runs again
    /// when data moves its key to another row.
    #[must_use]
    pub fn index(&self) -> u64 {
        row_of(self.row)
    }

    /// Its row of the data, untracked (what a view reads as it builds).
    #[must_use]
    pub fn index_untracked(&self) -> u64 {
        crate::reactive::untracked(|| row_of(self.row))
    }

    /// Its key, fixed for the row's life.
    #[must_use]
    pub fn key(&self) -> ElementKey {
        self.key
    }

    /// Whether it is kept built outside the window (it holds focus, is
    /// next to the row that does, or is the active row), tracked.
    #[must_use]
    pub fn is_pinned(&self) -> bool {
        self.row.with_item(|s| matches!(s, Slot::Row { pinned: true, .. }))
    }
}

impl ListState {
    /// The window as the source of a windowed list's [`each`], over `rows`
    /// rows keyed `key`: the rows pinned before it, its rows, those pinned
    /// after it, and the room below the last row once the window reaches
    /// it. Tracks what moves the window ([`super::State::version`]) and
    /// what `rows` and `key` read. The plan it is worked out from is kept
    /// for the layout hook.
    fn slots(self, rows: u64, key: &dyn Fn(u64) -> ElementKey) -> Vec<Slot> {
        // Its owner ended: it holds no rows.
        let Some((version, end_space)) = self.peek(|s| (s.version, s.end_space)) else {
            return Vec::new();
        };
        version.with(|_| ());
        let plan = self.plan(rows, key);
        let w = plan.window;
        let (before, after) = plan.pins.split_at(plan.pins_before.min(plan.pins.len()));
        let pin = |&(row, key): &(u64, ElementKey)| Slot::Row { row, key, pinned: true };
        let mut out = Vec::with_capacity(plan.keys.len().saturating_add(plan.pins.len()).saturating_add(1));
        out.extend(before.iter().map(pin));
        out.extend((w.first..w.last).zip(&plan.keys).map(|(row, &key)| Slot::Row {
            row,
            key,
            pinned: false,
        }));
        out.extend(after.iter().map(pin));
        if w.last >= rows && end_space > 0.0 {
            out.push(Slot::End {
                height: end_space.to_bits(),
            });
        }
        self.keep_plan(plan);
        out
    }

    /// Keep `plan` for the layout hook unless an equal one is kept: that one
    /// stays, so it is taken once. Moves nothing (no reader's scroll is
    /// taken in, as [`Self::poke`] does): a cache of what the source
    /// worked out.
    fn keep_plan(self, plan: Plan) {
        let kept = self.peek(|s| s.planned.as_deref() == Some(&plan)).unwrap_or(true);
        if !kept {
            LISTS.with_borrow(|l| l.write(self.0, |s| s.planned = Some(Rc::new(plan))));
        }
    }
}

/// A list of `rows()` rows, each built by `view` once, when its key comes
/// into view or the prefetch margin around it, and disposed when it leaves
/// (ADR-0026, K4c): a windowed [`each`] keyed `key(i)`. Size it as any box
/// (it fills what it is given: give it a height, or
/// [`ListState::fit_rows`] before building it); it scrolls by itself.
///
/// `rows` and `key` are the source of the rows, tracked by the list's
/// [`each`] alone: data that changes what they give moves, mounts and
/// disposes rows, and runs nothing else. A row's view runs once per mount:
/// what it shows of changing data it reads in bindings, its row of the
/// data through [`ListRow::index`]. The active row is the one
/// [`ListState::active_from`] gives (read by the source, tracked).
#[track_caller]
pub fn virtual_each<E: IntoElement>(
    state: &ListState,
    rows: impl Fn() -> u64 + 'static,
    key: impl Fn(u64) -> ElementKey + 'static,
    view: impl Fn(ListRow) -> E + 'static,
) -> Div {
    virtual_each_in(state, rows, key, view, |viewport| viewport)
}

/// [`virtual_each`] with its scroll container (`virtual-list-viewport`)
/// styled by `viewport`: a fade over its edge, a gap's worth of padding.
#[track_caller]
pub fn virtual_each_in<E: IntoElement>(
    state: &ListState,
    rows: impl Fn() -> u64 + 'static,
    key: impl Fn(u64) -> ElementKey + 'static,
    view: impl Fn(ListRow) -> E + 'static,
    viewport: impl FnOnce(Div) -> Div,
) -> Div {
    let s = *state;
    let Some((scroll, fit)) = s.peek(|st| (st.scroll, st.fit)) else {
        // Its owner ended: it holds no rows.
        return div().named("virtual-list");
    };
    let rows: Rc<dyn Fn() -> u64> = Rc::new(rows);
    let source = {
        let rows = Rc::clone(&rows);
        move || s.slots(rows(), &key)
    };
    let built = each(source, Slot::key, move |r: RowOf| match r.item_untracked() {
        Slot::Row { key, .. } => placed(r, key, &view),
        Slot::End { .. } => end(r),
    });
    let scroller = viewport(
        div()
            .named("virtual-list-viewport")
            .position(Position::Absolute)
            .inset(0.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .overflow_y(Overflow::Scroll)
            .overflow_x(Overflow::Hidden)
            .scrollbar_hidden(true),
    )
    .child(built)
    .track_scroll_built(&scroll, Rc::new(Windowed));
    let count = Rc::clone(&rows);
    let hidden = move || s.peek(|st| st.no_scrollbar).is_none_or(|n| n.get());
    let list = div()
        .named("virtual-list")
        .position(Position::Relative)
        .overflow(Overflow::Hidden)
        .role(Role::List)
        .aria_row_count(move || usize::try_from(count()).unwrap_or(usize::MAX))
        .child(scroller)
        .child(thumb(s, hidden));
    match fit {
        Some(max) => {
            // Its rows' height so far; measured after layout, bound there.
            // Before its first layout, from the heights it starts with.
            let n = crate::reactive::untracked(|| rows());
            let rows_h = s
                .peek(|st| {
                    let mut ext = st.extents.clone();
                    if ext.len() != n {
                        resize(&mut ext, n);
                    }
                    crate::convert::f64_to_f32(pixels(total(st, &ext)))
                })
                .unwrap_or(0.0)
                .min(max);
            list.flex(Flex::NONE)
                .height(move || s.peek(|st| st.height).and_then(|h| h.get()).unwrap_or(rows_h))
        }
        None => list,
    }
}

/// Row `r`, keyed `k`, built by `view`: laid in the window's column, or,
/// pinned, kept built with its bottom on the viewport's top, out of the
/// scroll range. Where it is placed is bound to its item, so the same row
/// goes from one to the other.
fn placed<E: IntoElement>(r: RowOf, k: ElementKey, view: &impl Fn(ListRow) -> E) -> Element {
    let pinned = move || r.with_item(|s| matches!(s, Slot::Row { pinned: true, .. }));
    let built = view(ListRow { row: r, key: k }).into_element();
    row_wrapper(Index::Bound(r), k, built)
        .flex(Flex::NONE)
        .width(pct(100.0))
        .left(0.0)
        .position(move || {
            if pinned() {
                Position::Absolute
            } else {
                Position::Relative
            }
        })
        .bottom(move || if pinned() { Length::Fraction(1.0) } else { Length::Auto })
        .into_element()
}

/// The room below the last row, as tall as its item says.
fn end(r: RowOf) -> Element {
    div()
        .named("virtual-list-end")
        .flex(Flex::NONE)
        .height(move || {
            r.with_item(|s| match *s {
                Slot::End { height } => f32::from_bits(height),
                Slot::Row { .. } => 0.0,
            })
        })
        .into_element()
}

#[cfg(test)]
mod tests;
