//! Handles used after their owner ended, in a release build (ADR-0028,
//! rule 4): loud in a debug build (`engine::check_alive`, tested beside each
//! handle), inert in release. Run with `cargo test --release -p gui --test
//! handles_in_release`; a debug build runs nothing here. An integration test,
//! as the crate's unit tests need the debug-only cold frame.

/// A write through a gone handle is dropped: the handle is inert, and no
/// live handle made after it, in the slot it left, is written.
#[test]
#[cfg(not(debug_assertions))]
fn a_write_through_a_gone_handle_is_dropped_in_release() {
    use gui::list::{Columns, Width};
    use gui::{ListState, Scope, ScrollHandle};

    let scope = Scope::new();
    let (scroll, list, columns) =
        scope.run(|| (ScrollHandle::new(), ListState::new(24.0), Columns::new([Width::AUTO])));
    scope.dispose();
    let later = (ScrollHandle::new(), ListState::new(24.0), Columns::new([Width::AUTO]));
    scroll.scroll_to((0.0, 20.0));
    list.scroll_to_row(40);
    assert!(!columns.observe(0, 90.0), "a gone column set notes nothing");
    assert_eq!(scroll.offset(), (0.0, 0.0), "inert");
    assert_eq!(later.0.offset(), (0.0, 0.0), "the later scroll is not written");
    assert!(later.1.is_alive() && !list.is_alive());
    assert_eq!(later.2.seen(0), 0.0, "the later columns are not written");
    later.0.dispose();
    later.1.dispose();
    later.2.dispose();
}
