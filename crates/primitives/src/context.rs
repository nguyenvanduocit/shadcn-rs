//! A part reads the state its root provides (reka's `createContext`).
//!
//! reka's `inject` of a missing context throws, naming the root the part
//! needs (research §3.2). A part here reads its root with [`use_root`]: a
//! part outside its root is a reported error that names the part, the root
//! it needs and where the part was made, once per place in the source, and
//! the part then renders its look without behaviour. It does not panic: the
//! workspace denies panics in library code, and a page that shows a
//! misplaced trigger still beats a page that is gone; the report makes the
//! wiring bug as visible as reka's throw does in a console.

#[expect(
    clippy::disallowed_types,
    reason = "diagnostics reported once per call site, per thread: no part's state"
)]
use std::cell::RefCell;
use std::collections::HashSet;
use std::panic::Location;

gui::engine_local! {
    /// The places a missing root was already reported from.
    #[expect(clippy::disallowed_types, reason = "diagnostics reported once per call site, per thread: no part's state")]
    [Shared("diagnostics, reported once per call site: no frame's result reads them")]
    static REPORTED: RefCell<HashSet<&'static Location<'static>>> = RefCell::new(HashSet::new());
}

/// The state of the root `root` that the part `part` sits in, provided with
/// `gui::provide`; called in the part's setup. Outside such a root it
/// reports the error, once per caller location, and gives `None`.
#[must_use]
#[track_caller]
pub fn use_root<T: Clone + 'static>(part: &str, root: &str) -> Option<T> {
    let found = gui::use_context::<T>();
    if found.is_none() {
        let at = Location::caller();
        if REPORTED.with(|r| r.borrow_mut().insert(at)) {
            eprintln!("primitives: `{part}` at {at} is outside a `{root}`; it renders without behaviour");
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::panic::Location;

    use super::{REPORTED, use_root};

    #[derive(Clone)]
    struct Root;

    /// A part's setup reading its root: the report names this call's caller.
    #[track_caller]
    fn trigger() -> (Option<Root>, &'static Location<'static>) {
        (use_root::<Root>("tabs_trigger", "tabs"), Location::caller())
    }

    #[test]
    fn a_part_outside_its_root_is_reported_where_it_was_made() {
        let (found, at) = trigger();
        assert!(found.is_none(), "no root: no behaviour");
        assert!(
            REPORTED.with(|r| r.borrow().contains(at)),
            "reported at the part's place"
        );
        let count = REPORTED.with(|r| r.borrow().len());
        for _ in 0..2 {
            assert!(use_root::<Root>("tabs_trigger", "tabs").is_none());
        }
        assert_eq!(
            REPORTED.with(|r| r.borrow().len()),
            count.saturating_add(1),
            "once per place in the source"
        );
    }
}
