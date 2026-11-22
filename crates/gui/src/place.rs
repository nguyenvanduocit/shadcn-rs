//! A child's position among its siblings (ADR-0022): the parent knows it
//! at render and hands it to each child, so a child that looks different
//! first or last (no bottom border on the last item, joined corners in a
//! group) reads a typed value instead of a selector (`:first-child`,
//! `:last-child`), as Flutter's `ListTile.divideTiles` does. Named `Place`
//! because [`crate::Position`] is CSS `position`.

/// Child `index` of `count` siblings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub index: usize,
    pub count: usize,
}

impl Place {
    /// The only child.
    pub const ONLY: Self = Self { index: 0, count: 1 };

    /// Child `index` of `count`; `index` is kept below `count`.
    #[must_use]
    pub const fn new(index: usize, count: usize) -> Self {
        let count = if count == 0 { 1 } else { count };
        let index = if index < count { index } else { count.saturating_sub(1) };
        Self { index, count }
    }

    /// Each place of `count` siblings, in order.
    pub fn all(count: usize) -> impl ExactSizeIterator<Item = Self> + DoubleEndedIterator {
        (0..count).map(move |index| Self { index, count })
    }

    /// Whether no sibling comes before it.
    #[must_use]
    pub const fn first(self) -> bool {
        self.index == 0
    }

    /// Whether no sibling comes after it.
    #[must_use]
    pub const fn last(self) -> bool {
        self.index.saturating_add(1) >= self.count
    }

    /// Whether it has no siblings.
    #[must_use]
    pub const fn only(self) -> bool {
        self.count <= 1
    }
}

#[cfg(test)]
mod tests {
    use super::Place;

    #[test]
    fn a_place_knows_its_ends() {
        let all: Vec<_> = Place::all(3).map(|p| (p.first(), p.last(), p.only())).collect();
        assert_eq!(all, [(true, false, false), (false, false, false), (false, true, false)]);
        assert!(Place::ONLY.first() && Place::ONLY.last() && Place::ONLY.only());
        assert_eq!(Place::new(5, 3), Place { index: 2, count: 3 });
        assert_eq!(Place::new(0, 0), Place::ONLY);
    }
}
