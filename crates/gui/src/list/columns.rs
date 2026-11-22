//! A table's columns over a virtual list (ADR-0014): every row is a grid
//! on the same tracks, so cells line up without the rows being laid out
//! together. A [`Width::Auto`] column is as wide as the widest cell seen so
//! far (the rows laid out, not all of them, which a list of a million rows
//! never lays out) and only grows, so scrolling never makes a column jump
//! narrower under the pointer.

use std::cell::RefCell;
use std::rc::Rc;

use crate::element::{Div, Element, IntoElement, div};
use crate::reactive::{Signal, detached};
use crate::style::Track;
use crate::styled::Styled;

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

struct Inner {
    widths: Vec<Width>,
    /// The widest cell seen in each column (0 where none was).
    seen: RefCell<Vec<f32>>,
    /// Bumped when an `Auto` column grew: the rows take the new tracks.
    version: Signal<u64>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.version.dispose_quietly();
    }
}

/// A table's column model. Clones share it.
#[derive(Clone)]
pub struct Columns(Rc<Inner>);

impl std::fmt::Debug for Columns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Columns")
            .field("widths", &self.0.widths)
            .field("seen", &self.0.seen.borrow())
            .finish()
    }
}

impl PartialEq for Columns {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Columns {
    #[must_use]
    pub fn new(widths: impl IntoIterator<Item = Width>) -> Self {
        let widths: Vec<Width> = widths.into_iter().collect();
        let seen = RefCell::new(vec![0.0; widths.len()]);
        Self(Rc::new(Inner {
            widths,
            seen,
            version: detached(|| Signal::labeled("table columns", 0)),
        }))
    }

    /// The tracks every row lays its cells on (read, so a column that grew
    /// renders the rows again).
    #[must_use]
    pub fn tracks(&self) -> Vec<Track> {
        self.0.version.with(|_| ());
        let seen = self.0.seen.borrow();
        self.0
            .widths
            .iter()
            .zip(seen.iter())
            .map(|(w, &s)| match *w {
                Width::Px(v) => Track::Px(v),
                Width::Fr(f) => Track::Fr(f),
                Width::Auto { min, max } => Track::Px(s.clamp(min, max.max(min))),
            })
            .collect()
    }

    /// The widest cell seen in `column`.
    #[must_use]
    pub fn seen(&self, column: usize) -> f32 {
        self.0.seen.borrow().get(column).copied().unwrap_or(0.0)
    }

    /// Note a cell of `column` `width` pixels wide; whether the column grew.
    pub fn observe(&self, column: usize, width: f32) -> bool {
        if !matches!(self.0.widths.get(column), Some(Width::Auto { .. })) {
            return false;
        }
        let grew = {
            let mut seen = self.0.seen.borrow_mut();
            match seen.get_mut(column) {
                Some(s) if width > *s + 0.01 => {
                    *s = width;
                    true
                }
                _ => false,
            }
        };
        if grew {
            self.0.version.update(|v| *v = v.wrapping_add(1));
        }
        grew
    }

    /// `cells` as a row lays them on [`Self::tracks`]: an `Auto` column's
    /// cell has its content at its own width (a flex item that does not
    /// shrink), measured as it is laid out and clipped to the track until
    /// the column grows to it.
    pub fn cells<E: IntoElement>(&self, cells: impl IntoIterator<Item = E>) -> Vec<Element> {
        cells
            .into_iter()
            .enumerate()
            .map(|(i, cell)| {
                if matches!(self.0.widths.get(i), Some(Width::Auto { .. })) {
                    let c = self.clone();
                    div()
                        .flex()
                        .min_w_px(0.0)
                        .overflow_hidden()
                        .child(
                            div()
                                .flex_none()
                                .on_measured(move |b| {
                                    c.observe(i, b.width);
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
        div().grid().grid_cols_tracks(self.tracks()).children(self.cells(cells))
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
}
