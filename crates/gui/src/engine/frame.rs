//! A check's frame from open to switch back (ADR-0034): what it made is
//! disposed of inside its own world, one element at a time, and the switch
//! to live is moves alone, which run no app code (S1). A fault of a
//! disposal does not stop it: every box is still dropped, each under its
//! own catch, to the fixed point, as `core::ptr::drop_in_place` drops every
//! field of a value though one's drop panics (S2, review r16). The first
//! fault is the report, later ones follow it in order, the check reports
//! `Faulted`, and the thread's sampling is poisoned. Only a runaway (a
//! disposal that still makes more after [`DRAIN_ROUNDS`] rounds) sets what
//! is left aside (forgotten unread): there, dropping it would never end.
//! Only the body's own panic is recovered: reported, as a string (S3),
//! with the rollback done.

use super::{Doom, Entry, REGISTRY, Undo, doomed, release::plain};
use std::any::Any;
use std::fmt;

/// How many rounds in a row dropping what a check made may make more
/// before the disposal is a runaway (a fault).
pub(crate) const DRAIN_ROUNDS: usize = 16;

/// How a check ended (ADR-0034, S2, S10).
#[derive(Debug)]
pub(crate) enum Ended<R> {
    /// Its result stands.
    Kept(R),
    /// It reached a site that cannot answer inside a check, named: its
    /// result was dropped inside it (a sampled check skips the sample).
    Dropped(&'static str),
    /// It was cancelled ([`super::cancel_check`]): stopped at a
    /// cancellation point, its result dropped inside it.
    Cancelled,
    /// Its body panicked: rolled back, the panic said as a string.
    Panicked(String),
    /// Disposing of what it made failed: the rest was still dropped (set
    /// aside only on a runaway), the switch made, and sampling on this
    /// thread poisoned.
    Faulted(Fault),
    /// No check opened: an earlier one faulted on this thread.
    Poisoned,
}

/// A teardown fault (ADR-0034, S2): where, in what, and what it said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fault {
    pub(crate) phase: Phase,
    pub(crate) piece: String,
    pub(crate) message: String,
    /// The body's own panic, if it panicked first.
    pub(crate) body: Option<String>,
    /// The faults after this one, in the order they happened: the
    /// disposal went on past each (S2).
    pub(crate) later: Vec<Fault>,
}

impl Fault {
    /// A fault in `phase` of `piece`, saying `message`.
    fn new(phase: Phase, piece: impl Into<String>, message: String) -> Self {
        Self {
            phase,
            piece: piece.into(),
            message,
            body: None,
            later: Vec::new(),
        }
    }
}

/// The faults of one teardown (S2): the first is the report, the later
/// ones follow it, and whether one was a runaway (what sets the rest
/// aside).
#[derive(Default)]
struct Faults {
    first: Option<Fault>,
    runaway: bool,
}

impl Faults {
    fn add(&mut self, fault: Fault) {
        self.runaway |= fault.phase == Phase::Runaway;
        match &mut self.first {
            Some(first) => first.later.push(fault),
            None => self.first = Some(fault),
        }
    }

    fn any(&self) -> bool {
        self.first.is_some()
    }
}

/// Where a teardown fault happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Dropping the check's result, not kept.
    Result,
    /// Moving what the check made out of a piece (a bug in gui, or a fork
    /// closed inside a use of a piece).
    Drain,
    /// Dropping what the check made.
    Disposal,
    /// Disposal still made more after [`DRAIN_ROUNDS`] rounds.
    Runaway,
    /// The switch back found what the drain should have taken (a bug in
    /// gui): quarantined.
    Switch,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rest = if self.phase == Phase::Runaway || self.later.iter().any(|l| l.phase == Phase::Runaway) {
            "What it still made was set aside (never dropped: a runaway)"
        } else {
            "Everything else it made was still dropped, each on its own"
        };
        write!(
            f,
            "gui: a check faulted ({:?} of {}): {}. {rest}; the live state is whole, and checks on this thread are off",
            self.phase, self.piece, self.message
        )?;
        for later in &self.later {
            write!(f, "; then ({:?} of {}): {}", later.phase, later.piece, later.message)?;
        }
        if let Some(body) = &self.body {
            write!(f, "; its body had panicked first: {body}")?;
        }
        Ok(())
    }
}

/// A frame built cold, from [`ColdFrame::begin`] until it closes
/// ([`ColdFrame::close`]) or drops: every piece of engine state on the
/// thread entered by its policy. It alone owns the life of what it makes.
/// Outside the crate, a check is scoped by a closure, never held as a guard.
#[must_use]
pub(crate) struct ColdFrame {
    ended: bool,
}

/// What the check made, by the piece it came from.
type Made = Vec<(&'static str, Box<dyn Any>)>;

impl ColdFrame {
    /// Open the check (E1), all or nothing, by moves of data alone. First
    /// every registered piece is checked free (no use of it under way, as
    /// one would keep its exit from moving the live value back): one in use
    /// refuses the check, naming it, with nothing changed. Then each is
    /// entered, which runs no user code. The guard that rolls the check
    /// back exists before any piece is entered. A thread whose check
    /// faulted opens none ([`fork`] answers `Poisoned`).
    #[expect(
        clippy::panic,
        reason = "a check opened inside a use of engine state, or after one faulted, is a bug in its caller"
    )]
    pub(crate) fn begin() -> Self {
        if let Some(fault) = poisoned() {
            panic!("no check opens on this thread: an earlier one faulted ({fault})");
        }
        let pieces = REGISTRY.with(|r| r.borrow().entries.clone());
        if let Some(e) = pieces.iter().find(|e| !e.free()) {
            panic!(
                "a fork was opened inside a use of the engine state `{}`: open it with nothing borrowed",
                e.name
            );
        }
        let entries = REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            let r = &mut *r;
            assert!(r.cold.is_none(), "a cold frame inside a cold frame");
            r.cold = Some(Vec::new());
            r.serial = r.serial.saturating_add(1);
            r.doom = Doom::Kept;
            r.dropped_at = None;
            r.cancel = r.next_cancel.take();
            r.ending = false;
            r.stood_down.clear();
            r.left.clear();
            r.entries.clone()
        });
        let frame = Self { ended: false };
        // Tests (V6): the first slice starts as the check opens.
        #[cfg(test)]
        super::slice_point(true);
        // Entering one may register another: that one enters as it does.
        for e in entries {
            let saved = e.enter();
            REGISTRY.with(|r| {
                if let Some(c) = r.borrow_mut().cold.as_mut() {
                    c.push((e, std::rc::Rc::new(saved)));
                }
            });
        }
        frame
    }

    /// Close the check with what its body gave ([`Self::teardown`]).
    pub(crate) fn close<R>(mut self, body: std::thread::Result<R>) -> Ended<R> {
        self.teardown(Some(body))
    }

    /// End the check (ADR-0034):
    ///
    /// - The body's panic, if any, said as a string as it is caught (S3).
    /// - The disposal loop, inside the check's world, until nothing it owns
    ///   is outstanding: the result dropped if it is not kept; every value
    ///   it made or wrote moved out of its piece, one box per element (S9),
    ///   each dropped under its own catch; once a round's drops reached no
    ///   engine state, one stand-in. A drop that panics is a fault and the
    ///   disposal goes on, through every box and to the fixed point, the
    ///   result dropped too (S2, review r16). Only a runaway past
    ///   [`DRAIN_ROUNDS`] rounds stops user code: what is left, the result
    ///   included, is moved out and set aside (never dropped).
    /// - The switch: every record put back by moves, under a tripwire that
    ///   asserts no engine state is reached meanwhile (S1).
    fn teardown<R>(&mut self, body: Option<std::thread::Result<R>>) -> Ended<R> {
        if std::mem::replace(&mut self.ended, true) {
            return Ended::Dropped("a check ended twice");
        }
        let (mut out, body_panic) = match body {
            Some(Ok(out)) => (Some(out), None),
            // The check's own unwind from a cancellation point: no panic,
            // the check cancelled ([`super::cancel_point`]).
            Some(Err(p)) if p.is::<super::CancelUnwind>() => {
                REGISTRY.try_with(|r| r.borrow_mut().doom = Doom::Cancelled).ok();
                (None, None)
            }
            Some(Err(p)) => (None, Some(plain(p))),
            None => (None, None),
        };
        // From here the check ends: no cancellation point unwinds (a drop
        // that reaches one runs to its end).
        #[cfg(test)]
        super::slice_point(true);
        REGISTRY
            .try_with(|r| {
                let mut r = r.borrow_mut();
                r.cancel = None;
                r.ending = true;
            })
            .ok();
        let mut faults = Self::dispose(&mut out, body_panic.is_some());
        if faults.runaway {
            // A runaway: dropping more would never end. What is left is
            // moved out (moves run no code) and set aside, never dropped.
            let (rest, failed) = Self::drain();
            for fault in failed {
                faults.add(fault);
            }
            quarantine(rest);
            while let Some(stand_in) = stand_down() {
                quarantine(vec![stand_in]);
            }
        }
        if faults.any()
            && let Some(out) = out.take()
        {
            // Dropped by the disposal unless a runaway stopped it first.
            forget_result(out);
        }
        let doom = doomed();
        let site = REGISTRY.try_with(|r| r.borrow().dropped_at).ok().flatten();
        // S1: the switch, by moves alone, under the tripwire.
        gui_tls::watch();
        restore();
        let reached = gui_tls::watched();
        assert!(
            reached.is_empty(),
            "the switch back to live reached engine state (slots {reached:?}): user code ran in it, a bug in gui"
        );
        let left = REGISTRY
            .try_with(|r| std::mem::take(&mut r.borrow_mut().left))
            .unwrap_or_default();
        if !left.is_empty() {
            faults.add(Fault::new(
                Phase::Switch,
                left.join(", "),
                String::from("a value the drain should have taken was left, and quarantined"),
            ));
        }
        if let Some(mut fault) = faults.first {
            fault.body = body_panic;
            eprintln!("{fault}");
            REGISTRY
                .try_with(|r| r.borrow_mut().poisoned = Some(fault.to_string()))
                .ok();
            return Ended::Faulted(fault);
        }
        if let Some(said) = body_panic {
            return Ended::Panicked(said);
        }
        match out {
            Some(out) if doom == Doom::Kept => Ended::Kept(out),
            _ if doom == Doom::Cancelled => Ended::Cancelled,
            _ => Ended::Dropped(site.unwrap_or("a site that cannot answer in a check")),
        }
    }

    /// The disposal loop (S2): every box dropped under its own catch, past
    /// any fault, to the fixed point; a runaway past [`DRAIN_ROUNDS`]
    /// rounds ends it with the rest left in place (the caller sets it
    /// aside). The result is dropped once the check will not keep it: its
    /// body failed, it was doomed, or its disposal faulted.
    fn dispose<R>(out: &mut Option<R>, body_failed: bool) -> Faults {
        let mut faults = Faults::default();
        let mut rounds = 0_usize;
        loop {
            if out.is_some()
                && (body_failed || doomed() != Doom::Kept || faults.any())
                && let Some(o) = out.take()
                && let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(o)))
            {
                faults.add(Fault::new(Phase::Result, "the check's result", plain(p)));
            }
            let (made, failed, reached) = Self::drain_watched();
            for fault in failed {
                faults.add(fault);
            }
            gui_tls::watch();
            drop_each(made, &mut faults);
            let mut wrote = gui_tls::watched();
            // A piece a drain hook reached is drained again (the fixed point).
            wrote.extend(reached);
            if !wrote.is_empty() || (out.is_some() && (doomed() != Doom::Kept || faults.any())) {
                rounds = rounds.saturating_add(1);
                if rounds >= DRAIN_ROUNDS {
                    faults.add(Fault::new(
                        Phase::Runaway,
                        named(&wrote),
                        format!(
                            "what it made still made more as it dropped after {DRAIN_ROUNDS} rounds: a destructor \
                             that schedules work again"
                        ),
                    ));
                    return faults;
                }
                continue;
            }
            rounds = 0;
            let Some(stand_in) = stand_down() else {
                return faults;
            };
            drop_each(vec![stand_in], &mut faults);
        }
    }

    /// What the check made, moved out of its pieces while it still runs
    /// (no user code runs in a move): what it wrote in every entered piece
    /// (last entered first), each record read in place. A drain hook that
    /// fails is a fault; the rest still move. Each hook runs under the
    /// switch's watch: one that reached another piece (a destructor it ran,
    /// which it must not) leaves that piece for the next round, so the
    /// drain reaches a fixed point before the switch (Codex r15-1).
    fn drain() -> (Made, Vec<Fault>) {
        let (made, failed, _) = Self::drain_watched();
        (made, failed)
    }

    /// [`Self::drain`], and the pieces other than its own a hook reached.
    /// Every hook that fails is a fault, in order (S2: the first is the
    /// report, each later one follows it).
    fn drain_watched() -> (Made, Vec<Fault>, Vec<usize>) {
        let mut failed = Vec::new();
        let records: Vec<(Entry, Undo)> = REGISTRY
            .try_with(|r| {
                r.borrow()
                    .cold
                    .iter()
                    .flatten()
                    .map(|(e, s)| (*e, std::rc::Rc::clone(s)))
                    .collect()
            })
            .unwrap_or_default();
        let mut made = Vec::with_capacity(records.len());
        let mut moved = |name: &'static str, take: &mut dyn FnMut() -> Vec<Box<dyn Any>>| match std::panic::catch_unwind(
            std::panic::AssertUnwindSafe(take),
        ) {
            Ok(m) => made.extend(m.into_iter().map(|m| (name, m))),
            Err(p) => {
                failed.push(Fault::new(Phase::Drain, name, plain(p)));
            }
        };
        let mut reached = Vec::new();
        let slots: Vec<usize> = records.iter().map(|(e, _)| e.slot()).collect();
        for (e, saved) in records.into_iter().rev() {
            gui_tls::watch();
            moved(e.name, &mut || e.drain(&**saved));
            let own = e.slot();
            // Another piece the check entered (its own bookkeeping aside).
            reached.extend(
                gui_tls::watched()
                    .into_iter()
                    .filter(|s| *s != own && slots.contains(s)),
            );
        }
        (made, failed, reached)
    }
}

/// Dropped unclosed (on unwind, or a guard in a test): ended, a fault raised
/// unless a panic already unwinds.
impl Drop for ColdFrame {
    #[expect(clippy::panic, reason = "a faulted check fails the test that held it as a guard")]
    fn drop(&mut self) {
        if let Ended::Faulted(fault) = self.teardown::<()>(None)
            && !std::thread::panicking()
        {
            panic!("{fault}");
        }
    }
}

/// Drop each of `made` on its own, under its own catch, every one though
/// some panic (S2: `drop_in_place`'s rule, without its abort, as no two
/// panics meet in one unwind); each that panicked, in order, a fault.
fn drop_each(made: Made, faults: &mut Faults) {
    for (name, m) in made {
        if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(m))) {
            faults.add(Fault::new(Phase::Disposal, name, plain(p)));
        }
    }
}

/// Never drop `made`: what a faulted check left (S2). Memory-safe and runs
/// no code; live obligations do not wait on it (S4).
#[expect(
    clippy::mem_forget,
    reason = "a faulted check's values are quarantined by design (ADR-0034, S2): dropping them runs user code"
)]
fn quarantine(made: Made) {
    for (_, m) in made {
        std::mem::forget(m);
    }
}

/// Never drop a faulted check's result (S2).
#[expect(
    clippy::mem_forget,
    reason = "a faulted check's result is quarantined by design (ADR-0034, S2)"
)]
fn forget_result<R>(out: R) {
    std::mem::forget(out);
}

/// A value a piece's exit found left at the switch (a bug in gui: the drain
/// takes everything): never dropped there, and the check faults (S1, S2).
/// Recorded unnoted by the switch's tripwire.
#[expect(
    clippy::mem_forget,
    reason = "a value left at the switch is quarantined by design (ADR-0034, S1): dropping it there runs user code"
)]
pub(crate) fn quarantined(piece: &'static str, value: Box<dyn Any>) {
    std::mem::forget(value);
    REGISTRY.switch_with(|r| {
        if let Ok(mut r) = r.try_borrow_mut() {
            r.left.push(piece);
        }
    });
}

/// The last stand-in the running check made, taken out to drop: its piece
/// is not stood in for again in this check.
fn stand_down() -> Option<(&'static str, Box<dyn Any>)> {
    let (slot, stand_in) = REGISTRY.try_with(|r| r.borrow_mut().stand_ins.pop()).ok().flatten()?;
    REGISTRY.with(|r| r.borrow_mut().stood_down.push(slot));
    Some(("a Shared piece's stand-in", Box::new(stand_in)))
}

/// The switch (S1): every entered piece put back by a move, last entered
/// first, and the check closed. Reaches the registry and the pieces
/// unnoted ([`super::EngineKey::switch_with`]); an exit that fails (a bug
/// in gui) or a record still shared is quarantined, never dropped.
fn restore() {
    let saved = REGISTRY
        .switch_with(|r| {
            let mut r = r.borrow_mut();
            let saved = r.cold.take();
            r.stood_down.clear();
            r.doom = Doom::Kept;
            saved
        })
        .flatten();
    for (e, s) in saved.into_iter().flatten().rev() {
        match std::rc::Rc::try_unwrap(s) {
            Ok(s) => {
                if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| e.exit(s))) {
                    let _said = plain(p);
                    quarantined(e.name, Box::new(()));
                }
            }
            Err(shared) => quarantined(e.name, Box::new(shared)),
        }
    }
}

/// The registered pieces among `slots`: each name and type.
fn named(slots: &[usize]) -> String {
    REGISTRY
        .try_with(|r| {
            let r = r.borrow();
            slots
                .iter()
                .filter_map(|s| r.named.iter().find(|(slot, ..)| slot == s))
                .map(|&(_, name, ty)| format!("{name} ({ty})"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// Why no check opens on this thread, if one faulted.
pub(crate) fn poisoned() -> Option<String> {
    REGISTRY.try_with(|r| r.borrow().poisoned.clone()).ok().flatten()
}

/// Run `f` in a check: every piece of engine state entered first, and put
/// back once `f` and all the check made have ended ([`ColdFrame`]). Scoped
/// by this closure, as `std::thread::scope` scopes its threads: no use of a
/// piece, and no stand-in, can outlast it.
pub(crate) fn fork<R>(f: impl FnOnce() -> R) -> Ended<R> {
    if poisoned().is_some() {
        return Ended::Poisoned;
    }
    let frame = ColdFrame::begin();
    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    frame.close(body)
}

/// Run `f` in a check ([`fork`]): the raw closure entry, engine-private
/// (ADR-0034, S10); a test of another crate reaches it through
/// [`crate::testing::in_check`]. Its result if kept.
///
/// # Panics
/// `f` panicked (its panic goes on, as a string); the check faulted, was
/// dropped at a site that cannot answer in one, or the thread is poisoned.
#[expect(clippy::panic, reason = "a test's check with no result: the test is wrong")]
pub(crate) fn in_cold_frame<R>(f: impl FnOnce() -> R) -> R {
    match fork(f) {
        Ended::Kept(out) => out,
        Ended::Panicked(said) => std::panic::resume_unwind(Box::new(said)),
        Ended::Faulted(fault) => panic!("{fault}"),
        Ended::Dropped(site) => panic!("the fork reached `{site}`, which cannot answer inside a fork, and was dropped"),
        Ended::Cancelled => panic!("the check was cancelled"),
        Ended::Poisoned => panic!("no check opens on this thread: an earlier one faulted"),
    }
}
