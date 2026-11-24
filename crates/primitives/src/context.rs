//! A part reads the state its root provides (reka's `createContext`).
//!
//! reka's `inject` of a missing context throws, naming the root the part
//! needs (research §3.2). A part here reads its root with [`use_root`]: a
//! part outside its root is a reported error that names the part, the root
//! it needs and the place in the caller's code that made the part, once per
//! (part, root, place), and the part then renders its look without
//! behaviour. The place is the caller's: a public part constructor is
//! `#[track_caller]`, keeps `Location::caller()` and hands it to the setup
//! that calls [`use_root_at`]. (A part that keeps no place yet reads its
//! root with [`use_root`]: its place is a library line, so two orphans of
//! that part are one report.) It does not panic: the workspace denies panics
//! in library code, and a page that shows a misplaced trigger still beats a
//! page that is gone; the report makes the wiring bug as visible as reka's
//! throw does in a console.

#[expect(
    clippy::disallowed_types,
    reason = "diagnostics reported once per call site, per thread: no part's state"
)]
use std::cell::RefCell;
use std::collections::HashSet;
use std::panic::Location;

/// A part found outside the root it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingRoot {
    /// The part, as its public constructor is named.
    pub part: &'static str,
    /// The root it needs, as its public constructor is named.
    pub root: &'static str,
    /// Where the caller's code made the part.
    pub at: &'static Location<'static>,
}

gui::engine_local! {
    /// What was reported already: (part, root, place).
    #[expect(clippy::disallowed_types, reason = "diagnostics reported once per call site, per thread: no part's state")]
    [Shared("diagnostics, reported once per (part, root, place): no frame's result reads them")]
    static REPORTED: RefCell<HashSet<(&'static str, &'static str, &'static Location<'static>)>> = RefCell::new(HashSet::new());

    /// What was reported since the last [`take_missing_roots`], in order.
    #[expect(clippy::disallowed_types, reason = "diagnostics reported once per call site, per thread: no part's state")]
    [Shared("diagnostics, reported once per (part, root, place): no frame's result reads them")]
    static MISSING: RefCell<Vec<MissingRoot>> = RefCell::new(Vec::new());
}

/// What was reported on this thread since the last call (a test drains
/// before it mounts, and reads what that mount reported).
pub fn take_missing_roots() -> Vec<MissingRoot> {
    MISSING.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// The state of the root `root` that the part `part`, made at `at` in the
/// caller's code, sits in, provided with `gui::provide`; called in the
/// part's setup. Outside such a root it reports the error, once per (part,
/// root, `at`), and gives `None`.
///
/// The names and the place are `'static`: the set of what was reported (and
/// the list a test drains) has one entry per (part, root, place in the
/// source), so it is bounded by the program's own call sites, whatever runs.
#[must_use]
pub fn use_root_at<T: Clone + 'static>(
    part: &'static str,
    root: &'static str,
    at: &'static Location<'static>,
) -> Option<T> {
    let found = gui::use_context::<T>();
    if found.is_none() && REPORTED.with(|r| r.borrow_mut().insert((part, root, at))) {
        MISSING.with(|m| {
            m.borrow_mut().push(MissingRoot { part, root, at });
        });
        eprintln!("primitives: `{part}` made at {at} is outside a `{root}`; it renders without behaviour");
    }
    found
}

/// [`use_root_at`] for a part that keeps no place of the caller's code: the
/// place is this call's caller (the library line of the setup), so two
/// orphans of the same part are one report. Each part that keeps its place
/// reads its root with [`use_root_at`]; those that do not yet are listed in
/// `docs/k6a-use-context-2026-11-24.md`, and K6's typed parts replace both.
#[must_use]
#[track_caller]
pub fn use_root<T: Clone + 'static>(part: &'static str, root: &'static str) -> Option<T> {
    use_root_at(part, root, Location::caller())
}

#[cfg(test)]
mod tests {
    use std::panic::Location;

    use super::{take_missing_roots, use_root_at};

    #[derive(Clone)]
    struct Root;

    #[test]
    fn a_part_outside_its_root_is_reported_once_per_part_root_and_place() {
        take_missing_roots();
        let here = Location::caller();
        let there = Location::caller();
        for _ in 0..2 {
            assert!(
                use_root_at::<Root>("tabs_trigger", "tabs", here).is_none(),
                "no root: none"
            );
        }
        assert!(use_root_at::<Root>("tabs_list", "tabs", here).is_none());
        assert!(use_root_at::<Root>("tabs_trigger", "tabs", there).is_none());
        let got: Vec<(&str, &Location<'_>)> = take_missing_roots().into_iter().map(|m| (m.part, m.at)).collect();
        assert_eq!(
            got,
            vec![("tabs_trigger", here), ("tabs_list", here), ("tabs_trigger", there)],
            "twice from one place is once; another part or place is another report"
        );
        assert!(take_missing_roots().is_empty(), "drained");
    }
}
