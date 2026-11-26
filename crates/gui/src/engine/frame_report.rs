//! What a check of a frame says (ADR-0034, S10): the one production
//! operation is "check this frame", and its answer is closed. No caller
//! outside `gui` opens a check on a closure of its own.

/// How a check of a frame ended.
///
/// The boundary is closed (ADR-0034, S10, S7). Outside `gui`, no raw
/// closure entry opens a check:
///
/// ```compile_fail
/// gui::engine::in_cold_frame(|| ());
/// ```
///
/// ```compile_fail
/// use gui::prelude::*;
/// let root = gui::Root::new(|| gui::div().into_element());
/// let _ = root.cold(|_| ());
/// ```
///
/// The cold frame a check builds is scoped by its closure. Before S10 a
/// fixture showed it could not leave `Root::cold`'s closure; now no caller
/// outside `gui` gets one at all (the test hook, `gui::testing::in_check`,
/// hands its closure no frame), so there is nothing of the check's to keep
/// past it but values, and a handle among them names nothing in another
/// world (S6, `engine::r14`). The frame's type is not nameable:
///
/// ```compile_fail
/// fn keep(_: &gui::component::ColdFrame) {}
/// ```
///
/// Inside `gui`, `Root::check` takes `impl FnOnce(&ColdFrame) -> R`: `R`
/// cannot borrow the frame (a higher-ranked borrow), which the compiler
/// enforces at every caller.
///
/// No crate states a piece's hooks of its own: neither a policy arm of its
/// own, nor an entry, nor the raw constructor.
///
/// ```compile_fail
/// fn enter() -> Box<dyn std::any::Any> { Box::new(()) }
/// fn drain(_: &dyn std::any::Any) -> Vec<Box<dyn std::any::Any>> { Vec::new() }
/// fn exit(_: Box<dyn std::any::Any>) {}
/// gui::engine_local! {
///     [Fork(enter, drain, exit)] static MINE: std::cell::Cell<u8> = std::cell::Cell::new(0);
/// }
/// ```
///
/// ```compile_fail
/// let _ = gui::engine::Entry { name: "mine" };
/// ```
///
/// ```compile_fail
/// static MINE: gui::engine::EngineKey<u8> = gui::engine::EngineKey::new(|| 0, None);
/// ```
///
/// Nor a log or a copy of its own, which a check would trust to run no
/// user code:
///
/// ```compile_fail
/// struct Mine;
/// impl gui::engine::Logged for Mine {
///     fn begin(&mut self) {}
///     fn drain(&mut self) -> Vec<Box<dyn std::any::Any>> { Vec::new() }
///     fn rollback(&mut self) {}
/// }
/// ```
///
/// The stated policies, and the test hook, compile (the positive control):
///
/// ```
/// use std::cell::{Cell, RefCell};
/// gui::engine_local! {
///     [Fresh] static QUEUE: RefCell<Vec<u8>> = RefCell::new(Vec::new());
///     [Fork] static NEXT: Cell<u64> = Cell::new(1);
///     [Fork(log)] static BOUND: RefCell<gui::engine::ForkMap<u64, u8>> = RefCell::default();
///     [Shared("a constant")] static NAME: &'static str = "name";
/// }
/// let n = gui::testing::in_check(|| {
///     NEXT.set(2);
///     QUEUE.with_borrow_mut(|q| q.push(1));
///     BOUND.with_borrow_mut(|b| b.insert(1, 1));
///     NEXT.get()
/// });
/// assert_eq!((n, NEXT.get(), NAME.with(|n| *n)), (2, 1, "name"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckReport {
    /// The frame built cold equals the live one.
    Ok,
    /// It differs, or the cold build did what the live one did not.
    Mismatch(Mismatch),
    /// No sample was taken: the check reached a site that cannot answer
    /// inside one, or checks are off on this thread (one faulted), or this
    /// build has no check. Why, said.
    Skipped(String),
    /// The check was given up at a cancellation point: its owner cancelled
    /// it (input arrived, ADR-0028); the live state is as before it.
    Cancelled,
    /// Disposing of what the check made faulted (S2): the rest was still
    /// dropped (set aside only on a runaway), the live state is whole, and
    /// checks on this thread are off from now on. The faults, said.
    Faulted(String),
}

/// How a cold frame differed from the live one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// The cold build panicked (the payload, as a string, S3).
    Panicked(String),
    /// It wrote live state, which a render, a getter or a binding must not:
    /// the pieces written.
    Wrote(String),
    /// Its layout differs: the difference, said.
    Layout(String),
}
