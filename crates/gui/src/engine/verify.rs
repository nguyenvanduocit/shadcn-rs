//! ADR-0034's verification: V1, the collection model (seeded operations
//! on a `ForkMap` and a `ForkSlab` inside a check, against plain std
//! models, every observation compared; the live collection untouched
//! after); V3, the transition generator (short generated sequences of what
//! a check can do, each run's end asserted against the decisions).

#![expect(
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::expect_used,
    reason = "test helpers: small known counts, and a failed precondition fails the test"
)]

use super::{ForkMap, ForkSlab, SlabId, in_cold_frame};
use crate::testing::{Rng, run_seeded};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Sender, channel};

crate::engine_local! {
    [Fork(log)] static MAP: RefCell<ForkMap<u64, u32>> = RefCell::default();
    [Fork(log)] static SLAB: RefCell<ForkSlab<u32>> = RefCell::default();
}

/// The map's observable state, as a sorted set of pairs.
fn map_seen(m: &ForkMap<u64, u32>) -> Vec<(u64, u32)> {
    let mut v: Vec<(u64, u32)> = m.iter().map(|(k, v)| (*k, *v)).collect();
    v.sort_unstable();
    v
}

/// V1, the map: insert, overwrite, remove, get, entry (write in place),
/// contains, len, is_empty and iteration as a set, keys drawn from a small
/// range so writes meet live keys, the overlay's own and removed ones.
#[test]
fn a_fork_map_in_a_check_is_the_std_map_and_live_is_untouched() {
    run_seeded(64, &[], |seed| {
        let mut rng = Rng::new(seed);
        let mut model: HashMap<u64, u32> = HashMap::new();
        MAP.with_borrow_mut(|m| {
            for k in m.iter().map(|(k, _)| *k).collect::<Vec<_>>() {
                m.remove(&k);
            }
            for _ in 0..rng.below(12) {
                let (k, v) = (rng.below(16), u32::try_from(rng.below(1000)).unwrap_or(0));
                m.insert(k, v);
                model.insert(k, v);
            }
        });
        let live = MAP.with_borrow(map_seen);
        let steps = in_cold_frame(|| {
            let mut model = model.clone();
            let mut steps = 0;
            for _ in 0..rng.below(200) {
                let k = rng.below(16);
                let v = u32::try_from(rng.below(1000)).unwrap_or(0);
                MAP.with_borrow_mut(|m| match rng.below(7) {
                    0 | 1 => assert_eq!(m.insert(k, v), model.insert(k, v), "insert {k}"),
                    2 => assert_eq!(m.remove(&k), model.remove(&k), "remove {k}"),
                    3 => assert_eq!(m.get(&k), model.get(&k), "get {k}"),
                    4 => {
                        *m.entry_mut(k) += 1;
                        *model.entry(k).or_default() += 1;
                    }
                    5 => assert_eq!(m.contains_key(&k), model.contains_key(&k)),
                    _ => assert_eq!((m.len(), m.is_empty()), (model.len(), model.is_empty())),
                });
                let mut want: Vec<(u64, u32)> = model.iter().map(|(k, v)| (*k, *v)).collect();
                want.sort_unstable();
                assert_eq!(MAP.with_borrow(map_seen), want, "seed {seed}: as a set");
                steps += 1;
            }
            steps
        });
        assert_eq!(
            MAP.with_borrow(map_seen),
            live,
            "seed {seed} ({steps} steps): live untouched"
        );
    });
}

/// V1, the slab: insert (free-slot reuse in the fork's own region),
/// remove, read, write, contains, len, live ids and the fork's own, and
/// handles escaped from the check (which name nothing after it).
#[test]
fn a_fork_slab_in_a_check_is_the_std_model_and_live_is_untouched() {
    run_seeded(64, &[], |seed| {
        let mut rng = Rng::new(seed);
        // The live entries, and the ids of some removed (stale handles).
        let mut live: BTreeMap<u64, (SlabId, u32)> = BTreeMap::new();
        let mut stale = Vec::new();
        SLAB.with_borrow_mut(|s| {
            for n in 0..rng.below(16) {
                let v = u32::try_from(rng.below(1000)).unwrap_or(0);
                let id = s.insert(v);
                if rng.chance(0.3) {
                    s.remove(id);
                    stale.push(id);
                } else {
                    live.insert(n, (id, v));
                }
            }
        });
        let before: Vec<Option<u32>> = live
            .values()
            .map(|(id, _)| SLAB.with_borrow(|s| s.read(*id, |v| *v)))
            .collect();
        let escaped = in_cold_frame(|| {
            // The model: each handle the check knows, and what it holds.
            let mut model: Vec<(SlabId, Option<u32>)> = live.values().map(|(id, v)| (*id, Some(*v))).collect();
            model.extend(stale.iter().map(|id| (*id, None)));
            for _ in 0..rng.below(200) {
                let v = u32::try_from(rng.below(1000)).unwrap_or(0);
                match rng.below(6) {
                    0 | 1 => {
                        let id = SLAB.with_borrow_mut(|s| s.insert(v));
                        assert!(model.iter().all(|(m, held)| *m != id || held.is_none()), "a fresh id");
                        model.push((id, Some(v)));
                    }
                    2 if !model.is_empty() => {
                        let at = rng.index(model.len());
                        let (id, held) = model[at];
                        assert_eq!(SLAB.with_borrow_mut(|s| s.remove(id)), held, "remove");
                        model[at].1 = None;
                    }
                    3 if !model.is_empty() => {
                        let (id, held) = model[rng.index(model.len())];
                        assert_eq!(SLAB.with_borrow(|s| s.read(id, |x| *x)), held, "read");
                        assert_eq!(SLAB.with_borrow(|s| s.contains(id)), held.is_some(), "contains");
                    }
                    4 if !model.is_empty() => {
                        let at = rng.index(model.len());
                        let (id, held) = model[at];
                        let wrote = SLAB.with_borrow(|s| s.write(id, |x| *x = v));
                        assert_eq!(wrote.is_some(), held.is_some(), "write");
                        if held.is_some() {
                            model[at].1 = Some(v);
                        }
                    }
                    _ => {
                        let held = model.iter().filter(|(_, h)| h.is_some()).count();
                        assert_eq!(SLAB.with_borrow(|s| s.len()), held, "len");
                    }
                }
            }
            model.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        });
        let after: Vec<Option<u32>> = live
            .values()
            .map(|(id, _)| SLAB.with_borrow(|s| s.read(*id, |v| *v)))
            .collect();
        assert_eq!(after, before, "seed {seed}: live untouched");
        assert_eq!(SLAB.with_borrow(|s| s.len()), live.len(), "seed {seed}: live length");
        let live_ids: Vec<SlabId> = live.values().map(|(id, _)| *id).collect();
        for id in escaped.into_iter().filter(|id| !live_ids.contains(id)) {
            assert_eq!(
                SLAB.with_borrow(|s| s.read(id, |v| *v)),
                None,
                "seed {seed}: an escaped handle names nothing"
            );
        }
        SLAB.with_borrow_mut(|s| {
            for (id, _) in live.values() {
                s.remove(*id);
            }
        });
    });
}

// ── V3: the transition generator ─────────────────────────────────────────────

/// A value a generated check makes, by what its drop does.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// Says it dropped.
    Quiet(u8),
    /// Says it dropped, then panics.
    Panics(u8),
    /// Makes another of itself with one less, while any are left: a
    /// destructor that schedules work, and succeeds.
    Schedules(u8),
    /// Makes another of itself, always: runaway disposal.
    Runaway,
    /// Asks for a live release, numbered.
    Releases(u8),
}

/// A generated value: what its drop does, and where it says so.
#[derive(Clone)]
struct Made(Kind, Sender<String>);

crate::engine::fork_clone_by_clone!(Made);

crate::engine_local! {
    [Fork(log)] static V3: RefCell<ForkSlab<Made>> = RefCell::default();
    [Fork] static V3_LIVE: Cell<bool> = Cell::new(false);
}

impl Drop for Made {
    #[expect(clippy::panic, reason = "a generated destructor that panics")]
    fn drop(&mut self) {
        let tx = self.1.clone();
        match self.0 {
            Kind::Quiet(i) => {
                let _sent = tx.send(format!("drop {i}"));
            }
            Kind::Panics(i) => {
                let _sent = tx.send(format!("drop {i}"));
                panic!("drop {i} fails");
            }
            Kind::Schedules(n) => {
                let _sent = tx.send(format!("schedule {n}"));
                if n > 0 {
                    V3.with_borrow_mut(|s| s.insert(Made(Kind::Schedules(n - 1), tx)));
                }
            }
            Kind::Runaway => {
                V3.with_borrow_mut(|s| s.insert(Made(Kind::Runaway, tx)));
            }
            Kind::Releases(i) => {
                super::when_live(move || {
                    let _sent = tx.send(format!("release {i}"));
                });
            }
        }
    }
}

/// One step of a generated check's body.
#[derive(Clone, Copy, Debug)]
enum Step {
    Make(Kind),
    /// A write to live state, which the switch puts back.
    Write,
    /// A live release asked for by the body, numbered.
    Enqueue(u8),
    /// A site that cannot answer inside a check.
    Doom,
    /// The check cancelled by its owner (S10): wins over a doom.
    Cancel,
}

/// What a run is expected to end as, by the decisions (S1, S2, S5, S9):
/// the model of the disposal loop, the order of the live releases, and
/// what was dropped.
#[derive(Debug, PartialEq, Eq)]
struct Expected {
    outcome: String,
    said: Vec<String>,
    poisoned: bool,
    /// How many faults the report holds (the first and the later ones).
    faults: usize,
}

fn expect(steps: &[Step], body_panics: bool) -> Expected {
    let mut said = Vec::new();
    let mut released = Vec::new();
    let mut doomed = false;
    let mut cancelled = false;
    let mut pending: Vec<Kind> = Vec::new();
    for s in steps {
        match *s {
            Step::Make(k) => pending.push(k),
            Step::Enqueue(i) => released.push(format!("release {i}")),
            Step::Doom => doomed = true,
            Step::Cancel => cancelled = true,
            Step::Write => {}
        }
    }
    // The disposal loop: a round drops what the last made, in order; a
    // round whose drops touched engine state is counted, and the 16th such
    // in a row is a runaway; a panicking drop is a fault, recorded, and the
    // disposal goes on (S2, review r16): the first fault is the report, and
    // only a runaway sets the rest aside (never dropped).
    let mut fault = None;
    let mut faults = 0;
    let mut rounds = 0;
    loop {
        let batch = std::mem::take(&mut pending);
        let mut touched = false;
        for k in batch {
            match k {
                Kind::Quiet(i) => said.push(format!("drop {i}")),
                Kind::Panics(i) => {
                    said.push(format!("drop {i}"));
                    fault.get_or_insert("Disposal");
                    faults += 1;
                }
                Kind::Schedules(n) => {
                    said.push(format!("schedule {n}"));
                    if n > 0 {
                        pending.push(Kind::Schedules(n - 1));
                        touched = true;
                    }
                }
                Kind::Runaway => {
                    pending.push(Kind::Runaway);
                    touched = true;
                }
                Kind::Releases(i) => {
                    released.push(format!("release {i}"));
                    touched = true;
                }
            }
        }
        if !touched {
            break;
        }
        rounds += 1;
        if rounds >= super::frame::DRAIN_ROUNDS {
            fault.get_or_insert("Runaway");
            faults += 1;
            break;
        }
    }
    said.extend(released);
    let outcome = match (fault, body_panics, cancelled, doomed) {
        (Some(phase), ..) => format!("faulted {phase}"),
        (None, true, ..) => String::from("panicked"),
        (None, false, true, _) => String::from("cancelled"),
        (None, false, false, true) => String::from("dropped"),
        (None, false, false, false) => String::from("kept"),
    };
    Expected {
        outcome,
        said,
        poisoned: fault.is_some(),
        faults,
    }
}

/// Run one generated check on a thread of its own, and say how it ended.
fn run(steps: Vec<Step>, body_panics: bool) -> Expected {
    std::thread::spawn(move || {
        let (tx, rx) = channel();
        let body = tx.clone();
        let ended = super::fork(move || {
            for s in steps {
                match s {
                    Step::Make(k) => {
                        V3.with_borrow_mut(|v| v.insert(Made(k, body.clone())));
                    }
                    Step::Write => V3_LIVE.set(true),
                    Step::Enqueue(i) => {
                        let tx = body.clone();
                        super::when_live(move || {
                            let _sent = tx.send(format!("release {i}"));
                        });
                    }
                    Step::Doom => super::drop_fork("v3"),
                    Step::Cancel => super::cancel_check(),
                }
            }
            assert!(!body_panics, "the body fails");
        });
        let faults = match &ended {
            super::Ended::Faulted(f) => 1 + f.later.len(),
            _ => 0,
        };
        let outcome = match &ended {
            super::Ended::Kept(()) => String::from("kept"),
            super::Ended::Dropped(_) => String::from("dropped"),
            super::Ended::Panicked(_) => String::from("panicked"),
            super::Ended::Faulted(f) => format!("faulted {:?}", f.phase),
            super::Ended::Poisoned => String::from("poisoned"),
            super::Ended::Cancelled => String::from("cancelled"),
        };
        // The live releases, after the switch, as the pipeline runs them.
        let _failed = std::panic::catch_unwind(super::run_releases);
        assert!(!V3_LIVE.get(), "the live state is whole");
        assert_eq!(V3.with_borrow(|v| v.len()), 0, "the live slab is whole");
        drop(tx);
        Expected {
            outcome,
            said: rx.try_iter().collect(),
            poisoned: super::frame::poisoned().is_some(),
            faults,
        }
    })
    .join()
    .unwrap_or_else(|_| Expected {
        outcome: String::from("the thread panicked"),
        said: Vec::new(),
        poisoned: false,
        faults: 0,
    })
}

fn kind(rng: &mut Rng, i: u8) -> Kind {
    match rng.below(10) {
        0..=3 => Kind::Quiet(i),
        4 | 5 => Kind::Panics(i),
        6 => Kind::Schedules(u8::try_from(rng.below(20)).unwrap_or(0)),
        7 => Kind::Runaway,
        _ => Kind::Releases(i),
    }
}

/// V3: generated sequences of what a check can do (values made whose drops
/// succeed, panic, schedule more, run away or ask for a release; live
/// writes; releases asked for by the body; a doom; a body that panics),
/// several faults in one run included. Each ends exactly as the decisions
/// say: the winning (first) fault and its phase, how many faults the report
/// holds, what was dropped (everything, past each fault, unless a runaway
/// set the rest aside: S2 revised in review r16), the live releases in
/// order, poisoning, and the live state whole.
#[test]
fn generated_checks_end_as_the_decisions_say() {
    run_seeded(300, &[], |seed| {
        let mut rng = Rng::new(seed);
        let mut steps = Vec::new();
        for i in 0..u8::try_from(rng.below(7)).unwrap_or(0) {
            steps.push(match rng.below(6) {
                0..=2 => Step::Make(kind(&mut rng, i)),
                3 => Step::Write,
                4 => Step::Enqueue(i.wrapping_add(100)),
                5 if rng.chance(0.5) => Step::Cancel,
                _ => Step::Doom,
            });
        }
        let body_panics = rng.chance(0.2);
        let want = expect(&steps, body_panics);
        let got = run(steps.clone(), body_panics);
        assert_eq!(got, want, "seed {seed}: {steps:?}, body panics: {body_panics}");
    });
}

// ── V2: the whole-engine differential ────────────────────────────────────────

/// One generated input of the differential's page.
#[derive(Clone, Copy, Debug)]
enum Input {
    Wheel(f32),
    /// A click on the counter button.
    Bump,
    /// A write from outside a turn (an app's own act).
    Act(u32),
    /// The views' worker asked for a new order.
    Sort(bool),
    /// The views' live owner ends (once): the scope they were made in is
    /// disposed of and the app lets go of its `Rc` of them. The worker
    /// closes, their loan ends, their claim on the heights goes and the
    /// waiting views wake, whatever a faulted check still pins (S4).
    DropViews,
    /// The page shows text in a face and a variation it had not used
    /// (Geist Mono at 500): the ids live text gets next.
    Mono,
    /// A root the app holds (made in a scope of its own) resolves with its
    /// render failing: poisoned, live (review r17, decision 1).
    Poison,
    /// That root resolves again: mounted anew if poisoned, as live code
    /// alone decides.
    Remount,
    /// That root is closed live (`Root::close`), after any check closed it
    /// in its own world (review r17, decision 3).
    CloseRoot,
    /// The second window is closed live, whole (`Pipeline::close`, then
    /// dropped): its effects, timer and mailbox end (review r18, decision 6).
    CloseWindow,
}

/// Where a check goes between two inputs, in the checking engine.
#[derive(Clone, Copy, Debug)]
enum Probe {
    None,
    Check,
    /// A check a row of which panics once the fork has written.
    Interrupted,
    /// A check cancelled at its `n`th cancellation point, inside a phase
    /// (as input arriving cancels one, ADR-0028).
    CancelAt(u64),
    /// A check whose render draws text only a check draws (Geist Mono at
    /// 800): the first use of a face and a variation is the check's. It
    /// reports a layout mismatch.
    Stray,
    /// A check whose disposal faults twice (S2): its render writes a
    /// logged map's live handle (the engine's copy of it) and parks two
    /// values whose drops panic and the app's `Rc` of the live views, held
    /// opaquely (as a closure an app clones into a handler holds them).
    /// With `pins` the views sit between the two faults (fail-stop, the
    /// old S2, forgot them there and pinned the live views for the rest of
    /// the run), else before both. Both faults are reported, in order, and
    /// everything is dropped. Checks on the thread are off after it.
    Teardown {
        pins: bool,
    },
    /// A check whose render reaches the app's root (review r17): resolves
    /// it (a poisoned one mounted anew in the check's world). It is the
    /// check's: the live root stays as live code left it.
    Speculative,
    /// [`Probe::Speculative`], then the check closes the live root and
    /// disposes of the live scope that owns it: a value's end outside the
    /// world that made it, refused, the check faulted (ADR-0035, D3, as
    /// revised by D008), and checks off from there. The live root and its
    /// scope stand.
    CloseRootChecked,
    /// A check whose render closes the second, live window explicitly
    /// (`Pipeline::close`): its disposal of the live window's signals is
    /// refused and the check faults (ADR-0035, D3); the live window stays
    /// open (review r17, decision 3; r18, decision 6).
    CloseWindowChecked,
    /// A check whose render lets go of the second, live window (the app's
    /// last owner of it): its drop queues the window's close, which runs
    /// live once the check has switched back (review r18, decision 4). The
    /// engine that never checks drops it live at the same step
    /// ([`Probe::LiveDropWindow`]).
    DropWindow,
    /// The never-checking engine's side of [`Probe::DropWindow`]: the app
    /// lets go of the second window live, at the same step. No check.
    LiveDropWindow,
}

/// A value a check parks whose drop panics.
struct Boom;

impl Drop for Boom {
    #[expect(clippy::panic, reason = "a generated destructor that panics")]
    fn drop(&mut self) {
        panic!("a parked value's drop fails");
    }
}

crate::engine_local! {
    /// What a check parks for its disposal ([`Probe::Teardown`]).
    [Fresh] static PARKED: RefCell<Vec<Box<dyn std::any::Any>>> = RefCell::new(Vec::new());
    /// A live handle in a logged map, which a teardown check's first write
    /// copies into its overlay (review r16, Opus I1).
    [Fork(log)] static HELD: RefCell<ForkMap<u64, crate::engine::AppHandle<dyn std::any::Any>>> = RefCell::default();
}

/// Sets its flag as it drops: a live value's end, seen.
struct Flagged(std::rc::Rc<Cell<bool>>);

impl Drop for Flagged {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

/// The live ids this thread handed out so far, of every kind (turns,
/// interactions, focus handles, chunks pushed, recordings, mailboxes,
/// loans, …): each kind by where it is declared, and its count (the id's
/// place in its block, which is the count modulo the block, as blocks are
/// used up in order). Two engines on two threads lease different blocks;
/// the counts compare.
fn live_ids() -> Vec<String> {
    let leases = std::ptr::from_ref(&super::ID_BLOCKS_LEASED).addr();
    let block = 1_u64 << 16;
    let mut ids: Vec<String> = super::ID_CURSORS.with(|c| {
        c.borrow()
            .iter()
            .filter(|((_, _, _, l), c)| *l == leases && c.live != 0)
            .map(|((file, line, column, _), c)| format!("{file}:{line}:{column} {}", c.live % block))
            .collect()
    });
    ids.sort_unstable();
    ids
}

/// The chunk ids of `scene` (those pushed, numbered, by their place in
/// their block), and the faces and variations its glyphs name, by id.
fn scene_ids(scene: &crate::scene::Scene) -> (Vec<u64>, Vec<(u32, u16)>) {
    let pushed = 1_u64 << 63;
    let chunks = scene
        .chunks
        .iter()
        .map(|c| {
            if c.id.0 & pushed == 0 {
                c.id.0
            } else {
                pushed | ((c.id.0 & !pushed) % (1 << 16))
            }
        })
        .collect();
    let mut faces: Vec<(u32, u16)> = scene
        .chunks
        .iter()
        .flat_map(|c| c.display.items.iter())
        .filter_map(|i| match i {
            crate::scene::Item::Glyph { key, .. } => Some((key.font, key.axes)),
            _ => None,
        })
        .collect();
    faces.sort_unstable();
    faces.dedup();
    (chunks, faces)
}

/// Run `inputs` on a page (a virtual list over a worker's view, made in a
/// scope that owns them and shared by the app behind an `Rc`, views
/// waiting on the list's claim, a counter button and an effect tracing
/// it) in an engine of its own, checking where `probes` say, and give
/// back what each step showed and the checks' reports.
#[expect(clippy::too_many_lines, reason = "one page, its inputs and what each step shows")]
fn differential(inputs: &[Input], probes: &[Probe]) -> (Vec<String>, Vec<String>) {
    use crate::app::Scenario;
    use crate::element::{IntoElement, div, text};
    use crate::list::{ListState, View, Views, virtual_list};
    use crate::reactive::{Collection, Scope};
    use crate::style::FontFamily;
    use crate::styled::Styled;
    use std::rc::Rc;
    const ROWS: u32 = 500;
    static KIND: super::IdKind = super::IdKind::new("v2 differential");
    let first_id = super::unique_id(&KIND);
    let c = Collection::new((0..ROWS).map(|v| v.wrapping_mul(13) % ROWS));
    let list = ListState::new(24.0);
    let view = crate::Signal::new(View::identity(ROWS));
    let count = crate::Signal::new(0_u32);
    let mono = crate::Signal::new(false);
    let traced = Rc::new(RefCell::new(Vec::new()));
    let t = Rc::clone(&traced);
    let effect = crate::reactive::Effect::new(move || t.borrow_mut().push(count.get()));
    // The views' live owner, and the app's `Rc` of them (a rebuild
    // closure an app clones into a handler holds them so).
    let mut owner = Some(Scope::new());
    let old: Rc<RefCell<Option<Rc<Views>>>> =
        Rc::new(RefCell::new(owner.as_ref().map(|o| {
            Rc::new(o.run(|| Views::new(&list, View::identity(ROWS), move |v: View| view.set(v))))
        })));
    let woken = Rc::new(RefCell::new(Vec::new()));
    let w = Rc::clone(&woken);
    let waiting =
        crate::engine::owned(|| Views::new(&list, View::identity(ROWS), move |v: View| w.borrow_mut().push(v.len())));
    let id = |v: &u32| u64::from(*v);
    // Whether a view arrived within `ms` milliseconds.
    let arrive = |ms: u32| {
        for _ in 0..ms {
            if crate::mailbox::deliver() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        false
    };
    let shown = || old.borrow().clone();
    if let Some(o) = shown() {
        o.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| b.cmp(a)));
    }
    assert!(arrive(10_000), "no view arrived");
    waiting.spawn(c.snapshot(), id, |_: &u32| true, Some(|a: &u32, b: &u32| a.cmp(b)));
    // A live value the engine itself copies into a check's world (review
    // r16): a logged map's entry. It ends with its live owner, at
    // `DropViews`.
    let held_dropped = Rc::new(Cell::new(false));
    let held: Rc<dyn std::any::Any> = Rc::new(Flagged(Rc::clone(&held_dropped)));
    let held_weak = Rc::downgrade(&held);
    HELD.with_borrow_mut(|h| h.replace(1, crate::engine::AppHandle::new(held)));
    // Ownerless constructors are refused, in both engines alike (review
    // r17, decision 5): a debug build panics, and nothing is left made.
    let refused = [
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(Views::new(&list, View::identity(ROWS), |_: View| {}));
        }))
        .is_err(),
        std::panic::catch_unwind(|| drop(crate::mailbox::mailbox(|_: u32| {}))).is_err(),
        std::panic::catch_unwind(|| drop(crate::component::Root::new(|| crate::element::Element::Empty))).is_err(),
    ];
    assert_eq!(refused, [true; 3], "an ownerless constructor was not refused");
    // A root the app holds, in a scope of its own (review r17, decisions
    // 1 and 3): its state is the registry's, so what a check does to it
    // (resolves it poisoned, closes it, disposes of its scope) is the
    // check's.
    let fail = crate::Signal::new(false);
    let mut root_scope = Some(Scope::new());
    let app_root = Rc::new(root_scope.as_ref().map(|o| {
        o.run(|| {
            crate::component::Root::new(move || {
                assert!(!fail.get(), "the app's root fails");
                text("app root").into_element()
            })
        })
    }));
    // What a check does to the app's root: `Some(false)` resolves it,
    // `Some(true)` resolves it, closes it and disposes of its scope.
    let speculative: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    // A second window the app holds, whose work (an effect owning an
    // hour's timer) and mailbox are its own (review r18, decision 6).
    let second: Rc<RefCell<Option<crate::app::Pipeline>>> = Rc::new(RefCell::new(None));
    let mut second_mail = None;
    {
        let mut w = crate::app::Pipeline::new(|| text("second window").into_element(), false);
        w.act("second window's setup", || {
            second_mail = Some(crate::mailbox::mailbox(|_: u8| {}));
            // The window's own effect (no scope owns it), owning the timer.
            let _effect = crate::reactive::Effect::new(|| {
                let _hour = crate::clock::after(std::time::Duration::from_secs(3600), || {});
            });
        });
        second.replace(Some(w));
    }
    let second_mail = second_mail.expect("made in the second window");
    // What a check does to the second window: `Some(true)` closes it,
    // `Some(false)` lets go of it.
    let window_probe: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    let trip: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(None));
    let stray = Rc::new(Cell::new(false));
    let teardown: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    let (st, armed) = (list, Rc::clone(&trip));
    let (stray_ui, teardown_ui, old_ui) = (Rc::clone(&stray), Rc::clone(&teardown), Rc::clone(&old));
    let (spec_ui, root_ui) = (Rc::clone(&speculative), Rc::clone(&app_root));
    let (second_ui, window_probe_ui) = (Rc::clone(&second), Rc::clone(&window_probe));
    let root_scope_ui = root_scope;
    let ui = move || {
        let armed = Rc::clone(&armed);
        if super::forking() {
            match window_probe_ui.take() {
                Some(true) => {
                    if let Some(w) = second_ui.borrow_mut().as_mut() {
                        // The check's copy of the window closes; what
                        // its close says is the check's too.
                        drop(w.close());
                    }
                }
                Some(false) => drop(second_ui.borrow_mut().take()),
                None => {}
            }
        }
        // In a check only: text no live frame shows, and the parked values
        // whose disposal faults.
        let mut only_checked = None;
        if super::forking()
            && let Some(ends) = spec_ui.take()
        {
            if let Some(r) = root_ui.as_ref() {
                let _ = r.resolve();
                if ends {
                    r.close();
                }
            }
            if ends && let Some(o) = root_scope_ui {
                o.dispose();
            }
        }
        if super::forking() {
            if stray_ui.get() {
                only_checked = Some(
                    div()
                        .font_family(FontFamily::GEIST_MONO)
                        .font_weight(800)
                        .child(text("only the check"))
                        .into_element(),
                );
            }
            if let Some(pins) = teardown_ui.take() {
                // The engine's own writes first: its copies of live
                // handles, which the disposal drops past every fault.
                // (The list's own scroll handle is written by every
                // check's layout: its shadow shares the live callbacks.)
                HELD.with_borrow_mut(|h| {
                    if let Some(copied) = h.get(&1).cloned() {
                        h.insert(1, copied);
                    }
                });
                let boom: Box<dyn std::any::Any> = Box::new(Boom);
                let again: Box<dyn std::any::Any> = Box::new(Boom);
                let views: Box<dyn std::any::Any> = Box::new(old_ui.borrow().clone());
                PARKED.with_borrow_mut(|v| {
                    // Two faults, the views between or after them.
                    if pins {
                        v.extend([boom, views, again]);
                    } else {
                        v.extend([views, boom, again]);
                    }
                });
            }
        }
        let mono_text = mono.get().then(|| {
            div()
                .font_family(FontFamily::GEIST_MONO)
                .font_weight(500)
                .child(text("mono"))
                .into_element()
        });
        let row = move |i: u64| {
            if let Some(at) = armed.get()
                && super::forking()
                && super::fork_work_done() > at
            {
                armed.set(None);
                panic!("a row of the check fails");
            }
            let shown = view.with(|v| v.row(usize::try_from(i).unwrap_or(usize::MAX)));
            div().height(24.0).child(text(format!("row {shown:?}"))).into_element()
        };
        // The render makes and ends a handle (a slab entry), as a render
        // may: a check renders again, so it allocates in the check's world.
        SLAB.with_borrow_mut(|s| {
            let handle = s.insert(7);
            s.remove(handle);
        });
        div()
            .width(300.0)
            .child(
                div()
                    .height(40.0)
                    .child(text(format!("count {}", count.get())))
                    .on_click(move |_| count.update(|n| *n += 1)),
            )
            .child(mono_text)
            .child(only_checked)
            .child(virtual_list(&st, u64::from(ROWS), |i| i, row).height(360.0))
            .into_element()
    };
    let mut s = Scenario::measuring(ui, 300, 400, 1.0).expect("a scenario");
    s.check_cold(false);
    s.settle().expect("settled");
    let mut seen = Vec::new();
    let mut reports = Vec::new();
    for (input, probe) in inputs.iter().zip(probes) {
        match probe {
            Probe::None => {}
            Probe::Check => reports.push(format!("{:?}", s.check_report())),
            Probe::CancelAt(n) => {
                super::log_slices(Some(*n));
                reports.push(format!("{:?}", s.check_report()));
                super::log_slices(None);
            }
            Probe::Interrupted => {
                trip.set(Some(super::fork_work_done()));
                reports.push(format!("{:?}", s.check_report()));
                trip.set(None);
            }
            Probe::Stray => {
                stray.set(true);
                reports.push(format!("{:?}", s.check_report()));
                stray.set(false);
            }
            Probe::Teardown { pins } => {
                teardown.set(Some(*pins));
                reports.push(format!("{:?}", s.check_report()));
                teardown.set(None);
            }
            Probe::Speculative | Probe::CloseRootChecked => {
                speculative.set(Some(matches!(probe, Probe::CloseRootChecked)));
                reports.push(format!("{:?}", s.check_report()));
                speculative.set(None);
            }
            Probe::CloseWindowChecked => {
                window_probe.set(Some(true));
                reports.push(format!("{:?}", s.check_report()));
                window_probe.set(None);
            }
            Probe::DropWindow => {
                window_probe.set(Some(false));
                reports.push(format!("{:?}", s.check_report()));
                window_probe.set(None);
            }
            Probe::LiveDropWindow => drop(second.borrow_mut().take()),
        }
        match *input {
            Input::Wheel(dy) => {
                s.hover(150.0, 200.0);
                seen.push(format!("wheel {}", s.wheel(dy)));
            }
            Input::Bump => {
                let _ = s.click(150.0, 20.0);
            }
            Input::Act(n) => s.act("act", || count.set(n)),
            Input::Mono => s.act("mono", || mono.set(true)),
            Input::Poison => {
                if let Some(r) = app_root.as_ref() {
                    fail.set(true);
                    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r.resolve())).is_err();
                    fail.set(false);
                    seen.push(format!("root failed {failed} poisoned {}", r.is_poisoned()));
                }
            }
            Input::Remount => {
                if let Some(r) = app_root.as_ref() {
                    let was = r.is_poisoned();
                    let shown = !matches!(r.resolve(), crate::element::Element::Empty);
                    seen.push(format!(
                        "root was poisoned {was} shows {shown} poisoned {}",
                        r.is_poisoned()
                    ));
                }
            }
            Input::CloseRoot => {
                if let Some(r) = app_root.as_ref() {
                    let unmounts = crate::component::stats().unmounts;
                    r.close();
                    let shown = !matches!(r.resolve(), crate::element::Element::Empty);
                    seen.push(format!(
                        "root closed: unmounted {} shows {shown}",
                        crate::component::stats().unmounts > unmounts
                    ));
                }
            }
            Input::CloseWindow => {
                let closed = second.borrow_mut().take().map(|mut w| {
                    let closed = w.close();
                    drop(w);
                    closed
                });
                seen.push(format!("window closed {closed:?}"));
            }
            Input::Sort(desc) => {
                if let Some(o) = shown() {
                    o.spawn(
                        c.snapshot(),
                        id,
                        |_: &u32| true,
                        Some(move |a: &u32, b: &u32| if desc { b.cmp(a) } else { a.cmp(b) }),
                    );
                    assert!(arrive(10_000), "no view arrived");
                }
            }
            Input::DropViews => {
                if let Some(o) = owner.take() {
                    o.dispose();
                    drop(old.borrow_mut().take());
                    // The map's live owner lets go of its entry too.
                    HELD.with_borrow_mut(|h| h.forget(&1));
                    super::run_releases();
                    // Bounded: views a check pinned that never woke show
                    // as a difference, not a hang.
                    seen.push(format!("woke {}", arrive(3_000)));
                    seen.push(format!(
                        "held strong {} dropped {}",
                        held_weak.strong_count(),
                        held_dropped.get()
                    ));
                }
            }
        }
        s.settle().expect("settled");
        let (chunks, faces) = scene_ids(s.parts().0);
        seen.push(format!(
            "lent {} ids {:?} chunks {chunks:?} faces {faces:?}",
            list.is_lent(),
            live_ids()
        ));
        // The windows' work and focus, the mailboxes, the timers and the
        // hooks due, as the live registry holds them, and whether the
        // second window still takes mail (review r18, decision 6).
        seen.push(format!(
            "{} | {} | {} | post {} | second open {} takes mail {}",
            crate::focus::registry_summary(),
            crate::mailbox::registry_summary(),
            crate::clock::registry_summary(),
            crate::component::post_summary(),
            second.borrow().is_some(),
            second_mail.send(0),
        ));
        // The handles' state, as the registry holds it (review r17,
        // decision 1): the app's root and the views'.
        seen.push(format!(
            "root poisoned {} views failure {:?} waiting failure {:?}",
            app_root
                .as_ref()
                .as_ref()
                .is_some_and(crate::component::Root::is_poisoned),
            shown().map(|o| o.failure()),
            waiting.failure(),
        ));
        let g = crate::reactive::graph();
        let slab = SLAB.with_borrow_mut(|s| {
            let id = s.insert(0);
            s.remove(id);
            id
        });
        seen.push(format!(
            "id +{} slab {slab:?} node {:?} pending {} graph {} {} {} {} traced {:?} woken {:?} top {:?}\n{}",
            super::unique_id(&KIND).wrapping_sub(first_id),
            {
                let probe = crate::Signal::new(0_u8);
                let at = probe.node_id();
                probe.dispose();
                at
            },
            super::releases_pending(),
            g.signals,
            g.computeds,
            g.effects,
            g.subscriptions,
            traced.borrow(),
            woken.borrow(),
            list.window(),
            crate::inspect::dump(&s.pipeline().frame().nodes),
        ));
    }
    effect.dispose();
    if let Some(o) = owner {
        o.dispose();
    }
    if let Some(o) = root_scope.take() {
        o.dispose();
    }
    drop(second.borrow_mut().take());
    drop((old, waiting, s, app_root, second_mail));
    c.dispose();
    (seen, reports)
}

/// V2: the same generated inputs in two engines on threads of their own,
/// one never checking, the other checking (or checking and being
/// interrupted after its first write, cancelled inside a phase, drawing
/// what only a check draws, or faulting as it disposes of what it made) at
/// generated boundaries. Every live step after shows the same: input
/// outcomes, the laid-out frame, every kind of live id handed out (turns,
/// interactions, focus handles, chunks pushed, recordings, mailboxes,
/// loans, …), the scene's chunk ids and the face and variation ids its
/// glyphs name, graph sizes (no growth the checks left), the effect's
/// trace, the worker's views, whether the list's heights are lent, the
/// waiting views' wake and releases pending.
///
/// Most runs have a teardown fault followed, later, by the live owner's
/// end: the views' worker must close (the release closes it, ends the
/// loan and lets go of the claim in one), and the waiting views wake, in
/// the engine that faulted as in the one that never checked (ADR-0034,
/// S4). The teardown check first makes the engine copy live handles into
/// its world (a logged map's entry holding an `Rc`; every check's layout
/// writes the list's scroll handle and list state, whose shadows share the
/// live callbacks), then faults twice: as the live owners let go, the
/// entry ends (its strong count and drop flag compared), since
/// the disposal goes on past each fault (S2, review r16). Checks are off
/// on the thread after a fault: each later one reports skipped.
///
/// The reports are counted, up to a fault: an interrupted check reports
/// its panic, a cancelled one its cancellation, a stray one a mismatch,
/// a teardown its fault, and at least half of the plain ones stand (a
/// coverage floor: skipping every hard sample fails).
#[test]
fn checks_at_generated_boundaries_change_no_live_step() {
    run_seeded(6, &[], |seed| {
        let mut rng = Rng::new(seed);
        let mut inputs = Vec::new();
        let mut probes = Vec::new();
        for _ in 0..12 {
            inputs.push(match rng.below(10) {
                0 | 1 => Input::Wheel(if rng.chance(0.5) { 240.0 } else { -120.0 }),
                2 => Input::Bump,
                3 => Input::Act(u32::try_from(rng.below(9)).unwrap_or(0)),
                4 => Input::Sort(rng.chance(0.5)),
                5 => Input::Mono,
                6 => Input::Poison,
                7 => Input::Remount,
                8 => Input::CloseWindow,
                _ => Input::DropViews,
            });
            probes.push(match rng.below(7) {
                0 => Probe::None,
                1 => Probe::Interrupted,
                2 => Probe::CancelAt(rng.below(40).saturating_add(1)),
                3 => Probe::Stray,
                4 => Probe::Speculative,
                _ => Probe::Check,
            });
        }
        // The r17 sequence in every run, before any teardown fault turns
        // checks off: the app's root poisoned live, a check that resolves
        // it (mounting it anew in its world), then the live remount; and
        // at the end a live close (review r17, decision 1). A check that
        // closes it and disposes of its scope faults (ADR-0035, D3), so
        // that one runs first in the runs below that have no other fault.
        inputs[0] = Input::Poison;
        probes[1] = Probe::Speculative;
        inputs[1] = Input::Remount;
        let last = inputs.len() - 1;
        probes[last] = Probe::Speculative;
        inputs[last] = Input::CloseRoot;
        // The r18 sequence, before any teardown fault turns checks off: a
        // check that closes the second window explicitly (the live one
        // stays open), then one that lets go of it (the live close after
        // the switch back), or a live whole-window close (review r18,
        // decision 6).
        // A check that closes the live window explicitly disposes of its
        // live signals: refused, the check faulted (ADR-0035, D3), and
        // checks off from there; in runs with no teardown fault, its live
        // close follows.
        if seed % 5 == 4 {
            probes[0] = Probe::CloseWindowChecked;
            inputs[2] = Input::CloseWindow;
        } else if seed % 5 == 3 {
            probes[1] = Probe::CloseRootChecked;
            inputs[2] = Input::CloseWindow;
        } else if seed % 2 == 0 {
            probes[2] = Probe::DropWindow;
        } else {
            inputs[2] = Input::CloseWindow;
        }
        // A teardown fault in most runs (the panicking value first, so
        // the views are pinned, in most of those), then the owner's end
        // at or after it; the views live until then.
        if seed % 5 < 3 {
            let at = 3 + rng.index(5);
            probes[at] = Probe::Teardown { pins: seed % 3 != 2 };
            for i in &mut inputs[..at] {
                if matches!(i, Input::DropViews) {
                    *i = Input::Bump;
                }
            }
            // Before the last step, the live close of the app's root.
            let end = at + rng.index(inputs.len() - 1 - at);
            inputs[end] = Input::DropViews;
        }
        let (i, p) = (inputs.clone(), probes.clone());
        // The engine that never checks does live what a check's drop
        // queues live: the app lets go of the second window there.
        let plain: Vec<Probe> = p
            .iter()
            .map(|p| match p {
                Probe::DropWindow => Probe::LiveDropWindow,
                _ => Probe::None,
            })
            .collect();
        let never = std::thread::spawn(move || differential(&i, &plain))
            .join()
            .expect("the plain engine ran");
        let (i, p) = (inputs.clone(), probes.clone());
        let checked = std::thread::spawn(move || differential(&i, &p))
            .join()
            .expect("the checking engine ran");
        for (step, (got, want)) in checked.0.iter().zip(&never.0).enumerate() {
            assert_eq!(got, want, "seed {seed}, step {step}: {inputs:?} {probes:?}");
        }
        assert_eq!(checked.0.len(), never.0.len(), "seed {seed}");
        let probed: Vec<&Probe> = probes.iter().filter(|p| !matches!(p, Probe::None)).collect();
        assert_eq!(probed.len(), checked.1.len(), "seed {seed}: one report a check");
        let (mut off, mut plain, mut stood) = (false, 0, 0);
        for (probe, report) in probed.iter().zip(&checked.1) {
            let r = report.as_str();
            let fits = if off {
                r.starts_with("Skipped(")
            } else {
                match probe {
                    Probe::None => false,
                    Probe::Check => {
                        plain += 1;
                        stood += usize::from(r == "Ok");
                        true
                    }
                    Probe::Interrupted => r.contains("Panicked(\"a row of the check fails\")"),
                    Probe::CancelAt(_) => r == "Cancelled",
                    Probe::Stray => r.starts_with("Mismatch(Layout("),
                    // Its closes and dispose are its own world's: it
                    // stands, as a check whose render did nothing else.
                    Probe::Speculative | Probe::DropWindow => r == "Ok",
                    // Its close disposes of the live window's signals, or
                    // of the live root's nodes and scope: refused, a fault
                    // (ADR-0035, D3); checks off after.
                    Probe::CloseWindowChecked | Probe::CloseRootChecked => {
                        off = true;
                        r.starts_with("Faulted(") && r.contains("(Foreign of")
                    }
                    Probe::LiveDropWindow => false,
                    Probe::Teardown { .. } => {
                        off = true;
                        // Both parked faults, the second after the first.
                        r.starts_with("Faulted(")
                            && r.contains("(Disposal of PARKED): a parked value's drop fails")
                            && r.contains("then (Disposal of PARKED): a parked value's drop fails")
                    }
                }
            };
            assert!(fits, "seed {seed}: {probe:?} reported {r}: {:?}", checked.1);
        }
        assert!(stood * 2 >= plain, "seed {seed}: the coverage floor: {:?}", checked.1);
    });
}

/// S10: a check cancelled midway (as the release sampler cancels one when
/// input arrives, F2) stops at its next cancellation point, reports
/// `Cancelled`, and leaves live as a check that never ran: the next live
/// step and the next check are as without it.
#[test]
fn a_check_cancelled_midway_reports_cancelled_and_changes_nothing_live() {
    use crate::app::Scenario;
    use crate::element::{IntoElement, div, text};
    use crate::styled::Styled;
    use std::rc::Rc;
    let armed = Rc::new(Cell::new(false));
    let a = Rc::clone(&armed);
    // Whether the cancelled check built past its next point: the child's
    // render is one, so it must not run.
    let reached = Rc::new(Cell::new(false));
    let r = Rc::clone(&reached);
    let count = crate::Signal::new(0_u32);
    let ui = move || {
        if a.get() && super::forking() {
            a.set(false);
            super::cancel_check();
        }
        let r = Rc::clone(&r);
        div()
            .width(200.0)
            .child(text(format!("count {}", count.get())))
            .child(crate::component::component((), move |_| {
                move || {
                    if super::forking() {
                        r.set(true);
                    }
                    crate::element::Element::Empty
                }
            }))
            .on_click(move |_| count.update(|n| *n += 1))
            .into_element()
    };
    let mut s = Scenario::measuring(ui, 200, 100, 1.0).expect("a scenario");
    s.check_cold(false);
    s.settle().expect("settled");
    let before = crate::inspect::dump(&s.pipeline().frame().nodes);
    armed.set(true);
    reached.set(false);
    assert_eq!(s.check_report(), super::CheckReport::Cancelled);
    assert!(!armed.get(), "the check ran up to its cancellation");
    assert!(!reached.get(), "and stopped at its next point: the child's render");
    assert_eq!(
        crate::inspect::dump(&s.pipeline().frame().nodes),
        before,
        "live is untouched"
    );
    assert_eq!(super::releases_pending(), 0, "nothing waits");
    let _ = s.click(100.0, 10.0);
    s.settle().expect("settled");
    assert_eq!(count.get(), 1, "the next input runs live");
    assert_eq!(s.check_report(), super::CheckReport::Ok, "the next check stands");
    assert!(reached.get(), "a check that stands builds the child");
}

// ── V6: gates ────────────────────────────────────────────────────────────────

/// The `p`th percentile of `samples` (sorted here), in microseconds.
/// A live pipeline a check lets go of ends whole, live, as one the app lets
/// go of live: its trackers' scope and their two render nodes too (V2
/// found the scope disposed in the check's world, so never live), not
/// only its window's close.
#[test]
fn a_live_pipeline_dropped_in_a_check_ends_as_a_live_drop() {
    use crate::element::{IntoElement, text};
    // The whole graph (its scopes and renders too: the trackers' scope is
    // one) and the cleanups its owners hold.
    let size = || (crate::reactive::graph(), crate::reactive::cleanups());
    let make = || crate::app::Pipeline::new(|| text("window").into_element(), false);
    // A live drop first: the arena as a pipeline made and ended leaves it.
    drop(make());
    super::run_releases();
    let live = size();
    let w = make();
    in_cold_frame(move || drop(w));
    super::run_releases();
    assert_eq!(size(), live, "a live pipeline a check let go of left nodes behind");
}

/// What a rich window left once it ended ([`rich_window_ends`]).
#[derive(Debug, PartialEq)]
struct Left {
    /// The reactive graph and its cleanups left.
    graph: (crate::reactive::Graph, usize),
    /// `BOUND` holds its two handles, and their flags.
    bound: (bool, bool),
    hovered: bool,
    focused: bool,
    /// The workers' closures still held (their tokens' extra counts).
    workers: usize,
    /// A send to its mailbox after, and the handler's count of deliveries.
    mail: (bool, u32),
    /// Every live id cursor its end moved.
    ids: Vec<String>,
}

/// A window with interactive nodes hovered and focused, a list with a
/// worker and a waiting view, a mailbox and a chart series, ended live or
/// inside a check (ADR-0035, Verification, V2), on a thread of its own: an
/// engine of its own.
fn rich_window_ends(in_check: bool) -> Left {
    std::thread::spawn(move || {
        use crate::element::{IntoElement, div};
        use crate::list::{ListState, View, Views};
        use crate::styled::Styled;
        use std::rc::Rc;
        use std::sync::Arc;
        let size = || (crate::reactive::graph(), crate::reactive::cleanups());
        let (hover, focus) = (crate::Interaction::new(), crate::Interaction::new());
        let mut s = crate::app::Scenario::measuring(
            move || {
                div()
                    .display(crate::Display::Flex)
                    .flex_direction(crate::FlexDirection::Column)
                    .child(div().width(40.0).height(40.0).interaction(hover))
                    .child(div().width(40.0).height(40.0).focusable().interaction(focus))
                    .into_element()
            },
            100,
            100,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        let token = Arc::new(());
        let delivered = Rc::new(Cell::new(0_u32));
        let mut sender = None;
        let mut series = None;
        let mut views = None;
        let arrived = Rc::new(Cell::new(false));
        let (t, d, a) = (Arc::clone(&token), Rc::clone(&delivered), Rc::clone(&arrived));
        s.act("the window's work", || {
            let list = ListState::new(24.0);
            let rows = crate::reactive::Collection::new(0..2_000_u32);
            let shown = Views::new(&list, View::identity(2_000), move |_: View| a.set(true));
            let t2 = Arc::clone(&t);
            shown.spawn(
                rows.snapshot(),
                |v: &u32| u64::from(*v),
                move |_: &u32| {
                    let _ = &t2;
                    true
                },
                Some(|a: &u32, b: &u32| b.cmp(a)),
            );
            // A second view, its result left waiting in its mailbox.
            let waiting = Views::new(&list, View::identity(2_000), |_: View| {});
            waiting.spawn(
                rows.snapshot(),
                |v: &u32| u64::from(*v),
                move |_: &u32| {
                    let _ = &t;
                    true
                },
                Some(|a: &u32, b: &u32| a.cmp(b)),
            );
            views = Some((shown, waiting));
            sender = Some(crate::mailbox::mailbox(move |n: u32| d.set(d.get() + n)));
            let plot = crate::chunk::plot::series();
            plot.push(1, 2.0);
            series = Some(plot);
        });
        s.click(10.0, 60.0);
        s.settle().expect("settled");
        s.hover(10.0, 10.0);
        s.settle().expect("settled");
        assert_eq!(
            (hover.hovered(), focus.focused()),
            (true, true),
            "hovered and focused before"
        );
        let sender = sender.expect("made");
        assert!(sender.send(1));
        // The shown view arrives (its loan of heights made), so both
        // engines drop the same window.
        for _ in 0..10_000 {
            let _ = crate::mailbox::deliver();
            if arrived.get() && delivered.get() == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(arrived.get(), "no view arrived");
        assert_eq!(delivered.get(), 1, "the mailbox delivers while the window lives");
        // The window, its list's views and its chart, let go of together.
        // (How many turns the arrival took varies: ids compare from here.)
        let made = live_ids();
        let all = (s, views, series);
        if in_check {
            in_cold_frame(move || drop(all));
        } else {
            drop(all);
        }
        super::run_releases();
        // A worker cancelled ends its closure once it sees so.
        for _ in 0..2_000 {
            if Arc::strong_count(&token) == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let sent = sender.send(2);
        let _ = crate::mailbox::deliver();
        Left {
            graph: size(),
            bound: (hover.is_bound(), focus.is_bound()),
            hovered: hover.hovered(),
            focused: focus.focused(),
            workers: Arc::strong_count(&token) - 1,
            mail: (sent, delivered.get()),
            // The turns' own count aside: a turn runs as each worker's
            // outcome arrives, which is the outside world's timing.
            ids: live_ids()
                .into_iter()
                .filter(|id| !made.contains(id) && !id.starts_with("crates/gui/src/engine/turn.rs:"))
                .collect(),
        }
    })
    .join()
    .expect("ran")
}

/// A rich window a check lets go of ends as one the app lets go of live
/// (ADR-0035, V2): no signal, binding, flag, worker closure, delivery or
/// id is left or made that the live drop does not leave or make.
#[test]
fn a_rich_window_dropped_in_a_check_ends_as_a_live_drop() {
    let live = rich_window_ends(false);
    assert!(
        !live.hovered && !live.focused && live.workers == 0,
        "the live drop itself leaves something: {live:?}"
    );
    assert_eq!(rich_window_ends(true), live);
}

/// The median time of `runs` checks each running `f(k)` for the next `k`.
fn median_check(runs: usize, f: impl Fn(usize)) -> std::time::Duration {
    let mut samples: Vec<std::time::Duration> = (0..runs)
        .map(|k| {
            let at = std::time::Instant::now();
            in_cold_frame(|| f(k));
            at.elapsed()
        })
        .collect();
    samples.sort_unstable();
    samples.get(samples.len() / 2).copied().unwrap_or_default()
}

/// What a check's first write to one entry of a logged slab and of
/// `VIEWS` costs, with 10 and with 10,000 live entries (review r18,
/// decision 6): the entry, not the table. A check that writes one of
/// 10,000 live entries (a root's state, as `ROOTS` holds it), or asks one
/// of 10,000 live views for a build, copies that entry alone; its cost
/// over an empty check stays within a small factor of the same check over
/// 10 entries. The numbers are printed (`--nocapture`). (A check that
/// closes a live root faults since ADR-0035, D3: the slab's write is the
/// one the close made first.)
#[test]
fn a_checks_first_write_costs_the_entry_not_the_table() {
    use crate::list::{ListState, View, Views};
    use crate::reactive::{Collection, Scope};
    const RUNS: usize = 41;
    let measure = |n: usize| {
        let scope = Scope::new();
        let list = ListState::new(24.0);
        let (roots, views): (Vec<SlabId>, Vec<Views>) = scope.run(|| {
            (0..n)
                .map(|_| {
                    let entry = super::allocate(&SLAB, 0_u32);
                    (entry, Views::new(&list, View::identity(4), |_: View| {}))
                })
                .unzip()
        });
        let c = Collection::new(0..4_u32);
        let empty = median_check(RUNS, |_| {});
        let root = median_check(RUNS, |k| {
            SLAB.with_borrow(|s| s.write(roots[(k * 7919) % n], |v| *v = v.wrapping_add(1)));
        });
        let view = median_check(RUNS, |k| {
            views[(k * 7919) % n].spawn(
                c.snapshot(),
                |v: &u32| u64::from(*v),
                |_: &u32| true,
                None::<fn(&u32, &u32) -> std::cmp::Ordering>,
            );
        });
        SLAB.with_borrow_mut(|s| {
            roots.into_iter().for_each(|id| {
                let _ = s.remove(id);
            })
        });
        drop(views);
        scope.dispose();
        c.dispose();
        (empty, root.saturating_sub(empty), view.saturating_sub(empty))
    };
    let (empty_small, root_small, view_small) = measure(10);
    let (empty_large, root_large, view_large) = measure(10_000);
    println!(
        "first write, 10 entries: empty check {empty_small:?}, a root {root_small:?}, a view {view_small:?}; \
         10,000 entries: empty check {empty_large:?}, a root {root_large:?}, a view {view_large:?}"
    );
    // The entry, not the table: a copy of 10,000 entries would cost 1,000
    // times one; noise on a busy machine stays well under 8.
    let floor = std::time::Duration::from_micros(50);
    assert!(
        root_large <= root_small.max(floor) * 8,
        "a root's first write grew with the table: {root_small:?} at 10, {root_large:?} at 10,000"
    );
    assert!(
        view_large <= view_small.max(floor) * 8,
        "a view's first write grew with the table: {view_small:?} at 10, {view_large:?} at 10,000"
    );
}

fn percentile(samples: &mut [std::time::Duration], p: usize) -> u128 {
    samples.sort_unstable();
    let at = (samples.len().saturating_mul(p) / 100).min(samples.len().saturating_sub(1));
    samples.get(at).map_or(0, std::time::Duration::as_micros)
}

/// V6: the check's costs at scale, in an optimized build with the check
/// compiled in (`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo test -p
/// gui --lib --release -- --ignored v6_gates --nocapture
/// --test-threads=1`). Asserted: what a check touches is the records written,
/// not the data's size; one key written 1,000 times copies once; repeated
/// checks retain nothing; and an input arriving during a page check waits
/// at most the ADR-0028 budget (the longest slice between two
/// cancellation points, resolve, layout and paint included, plus the
/// teardown after one), in thread CPU time.
#[test]
#[ignore = "a measurement: run in an optimized build, alone"]
fn v6_gates() {
    use std::time::Instant;
    const N: u64 = 1_000_000;
    crate::engine_local! {
        [Fork(log)] static BIG: RefCell<ForkMap<u64, u64>> = RefCell::default();
        [Fork(log)] static ONE: RefCell<ForkMap<u64, Vec<u64>>> = RefCell::default();
        [Fork(log)] static ROWS: RefCell<ForkSlab<u64>> = RefCell::default();
    }
    // A million-key map, a million-entry slab: one write each in a check.
    BIG.with_borrow_mut(|m| {
        for k in 0..N {
            let _was = m.insert(k, k);
        }
    });
    let ids: Vec<SlabId> = ROWS.with_borrow_mut(|s| (0..N).map(|v| s.insert(v)).collect());
    let mut times = Vec::new();
    let mut touched = Vec::new();
    for i in 0..50_u64 {
        let before = super::fork_work_done();
        let at = Instant::now();
        in_cold_frame(|| {
            let _was = BIG.with_borrow_mut(|m| m.insert(i, 0));
            let id = ids[usize::try_from(i).unwrap_or(0)];
            ROWS.with_borrow(|s| s.write(id, |v| *v = 0));
        });
        times.push(at.elapsed());
        touched.push(super::fork_work_done() - before);
    }
    let max_touched = touched.iter().copied().max().unwrap_or(0);
    println!(
        "V6 million-key map + million-entry slab, one write each: p50 {} us, p99 {} us, records touched (entry included) max {max_touched} of {}",
        percentile(&mut times.clone(), 50),
        percentile(&mut times, 99),
        2 * N
    );
    assert!(
        max_touched < 1_000,
        "a check touches what it writes, not the data: {max_touched}"
    );
    // A large value written 1,000 times under one key: one copy.
    ONE.with_borrow_mut(|m| drop(m.insert(1, (0..100_000).collect())));
    let before = super::fork_work_done();
    let at = Instant::now();
    in_cold_frame(|| {
        for i in 0..1_000 {
            ONE.with_borrow_mut(|m| m.entry_mut(1).push(i));
        }
    });
    let (spent, copies) = (at.elapsed(), super::fork_work_done() - before);
    println!(
        "V6 one key of 100,000 written 1,000 times: {} us, records touched {copies}",
        spent.as_micros()
    );
    assert!(
        copies < 100,
        "one copy of the key, however often it is written: {copies}"
    );
    // A million-key memo (`Keyed`): one key asked for in a check.
    let memo = crate::holder::Keyed::<u64, u64>::new();
    for k in 0..N {
        let _v = memo.get(&k, || k);
    }
    let mut times = Vec::new();
    let mut touched = Vec::new();
    for i in 0..50_u64 {
        let before = super::fork_work_done();
        let at = Instant::now();
        in_cold_frame(|| {
            let _new = memo.get(&N.wrapping_add(i), || i);
            let _old = memo.get(&i, || 0);
        });
        times.push(at.elapsed());
        touched.push(super::fork_work_done() - before);
    }
    let max_touched = touched.iter().copied().max().unwrap_or(0);
    println!(
        "V6 a million-key memo, one new key and one kept key per check: p50 {} us, p99 {} us, records touched max {max_touched} of {N}",
        percentile(&mut times.clone(), 50),
        percentile(&mut times, 99),
    );
    assert!(
        max_touched < 1_000,
        "a memo check touches what it writes: {max_touched}"
    );
    assert_eq!(memo.peek(&N), None, "the check's key is the check's");
    // Repeated checks of a page with a million-row list, plain and with a
    // worker whose view the list shows and a claimant waiting on the
    // list's heights: the gate, and nothing retained.
    for worker in [false, true] {
        let page = std::thread::spawn(move || page_checks(N, worker))
            .join()
            .expect("the page ran");
        let Page {
            mut times,
            reports,
            held,
            same_graph,
            touched,
            mut waited,
            cancelled,
            mut slices,
            mut torn,
            points,
            mut whole,
        } = page;
        let waited_p99 = percentile(&mut waited, 99);
        println!(
            "V6 input arriving during a check of that page (100, at 0 to 1.5 ms in, wall time on this machine as loaded): {cancelled} cancelled; input waited p50 {} us, p99 {waited_p99} us, max {} us",
            percentile(&mut waited.clone(), 50),
            percentile(&mut waited, 100),
        );
        let (slice_p99, torn_p99) = (percentile(&mut slices, 99), percentile(&mut torn, 99));
        println!(
            "V6 the same page in thread CPU time: a whole check p50 {} us, p99 {} us (what input waited with no cancellation point); {points} cancellation points a check; longest uninterruptible slice p50 {} us, p99 {slice_p99} us; cancel to live back p50 {} us, p99 {torn_p99} us; most input waits p99 {} us (ADR-0028 gate: +2,000 us)",
            percentile(&mut whole.clone(), 50),
            percentile(&mut whole, 99),
            percentile(&mut slices.clone(), 50),
            percentile(&mut torn.clone(), 50),
            slice_p99.saturating_add(torn_p99),
        );
        let stood = reports.iter().filter(|r| **r == super::CheckReport::Ok).count();
        let mut warm: Vec<_> = times.get(10..).unwrap_or_default().to_vec();
        println!(
            "V6 a page with a million-row list{}, 100 checks: {stood} stood; p50 {} us, p99 {} us, p99 after 10 warm-up checks {} us (ADR-0028 budget for the first input after idle: +2,000 us); allocations per check: first {}, last {}; records touched max {}; graph unchanged: {same_graph}",
            if worker {
                ", a worker and a waiting claimant"
            } else {
                ""
            },
            percentile(&mut times.clone(), 50),
            percentile(&mut times, 99),
            percentile(&mut warm, 99),
            held.first().copied().unwrap_or(0),
            held.last().copied().unwrap_or(0),
            touched.iter().max().copied().unwrap_or(0),
        );
        assert_eq!(stood, 100, "every check stood: {reports:?}");
        assert!(same_graph, "the checks left no reactive node or link");
        assert!(
            slice_p99.saturating_add(torn_p99) <= 2_000,
            "input waits at most 2 ms of the check's work at p99 (ADR-0028): slice {slice_p99} us + teardown {torn_p99} us"
        );
    }
}

/// What [`page_checks`] measured.
struct Page {
    times: Vec<std::time::Duration>,
    reports: Vec<super::CheckReport>,
    held: Vec<u64>,
    same_graph: bool,
    touched: Vec<u64>,
    /// What input arriving during a check waited for it: from its arrival
    /// to the live world back (zero if the check had ended).
    waited: Vec<std::time::Duration>,
    /// Checks the arriving input cancelled.
    cancelled: usize,
    /// Per check, the longest thread CPU time between two cancellation
    /// points: the most input arriving in it waits before the check stops.
    slices: Vec<std::time::Duration>,
    /// Per cancelled check, the thread CPU time from the cancel to the
    /// live world back.
    torn: Vec<std::time::Duration>,
    /// Cancellation points a check passes.
    points: u64,
    /// Per check, its whole thread CPU time: what input would wait with
    /// no cancellation point.
    whole: Vec<std::time::Duration>,
}

/// 100 checks of a page with an `n`-row virtual list; with `worker`, the
/// list shows a worker's order of the rows and a second view waits on the
/// list's heights (a claimant).
fn page_checks(n: u64, worker: bool) -> Page {
    use crate::element::{IntoElement, div, text};
    use crate::list::{ListState, View, Views, virtual_list};
    use crate::reactive::Collection;
    use crate::styled::Styled;
    use std::time::Instant;
    let list = ListState::new(24.0);
    let rows = u32::try_from(n).unwrap_or(u32::MAX);
    let view = crate::Signal::new(View::identity(rows));
    let mut keep = Vec::new();
    let c = Collection::new(0..rows);
    if worker {
        let arrive = || {
            for _ in 0..60_000 {
                if crate::mailbox::deliver() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            panic!("no view arrived");
        };
        let shown = crate::engine::owned(|| Views::new(&list, View::identity(rows), move |v: View| view.set(v)));
        shown.spawn(
            c.snapshot(),
            |v: &u32| u64::from(*v),
            |_: &u32| true,
            Some(|a: &u32, b: &u32| b.cmp(a)),
        );
        arrive();
        let waiting = crate::engine::owned(|| Views::new(&list, View::identity(rows), |_: View| {}));
        waiting.spawn(
            c.snapshot(),
            |v: &u32| u64::from(*v),
            |_: &u32| true,
            Some(|a: &u32, b: &u32| a.cmp(b)),
        );
        keep.push(shown);
        keep.push(waiting);
    }
    let ui = move || {
        let row = move |i: u64| {
            let shown = view.with(|v| v.row(usize::try_from(i).unwrap_or(usize::MAX)));
            text(format!("row {shown:?}")).into_element()
        };
        div()
            .width(300.0)
            .child(virtual_list(&list, n, |i| i, row).height(400.0))
            .into_element()
    };
    let mut s = crate::app::Scenario::measuring(ui, 300, 400, 1.0).expect("a scenario");
    s.check_cold(false);
    s.settle().expect("settled");
    let graph = crate::reactive::graph();
    let mut page = Page {
        times: Vec::new(),
        reports: Vec::new(),
        held: Vec::new(),
        same_graph: false,
        touched: Vec::new(),
        waited: Vec::new(),
        cancelled: 0,
        slices: Vec::new(),
        torn: Vec::new(),
        points: 0,
        whole: Vec::new(),
    };
    let mut points = 0;
    for _ in 0..100 {
        let before = super::fork_work_done();
        super::log_slices(None);
        let at = Instant::now();
        let (_, started) = super::slices();
        let (report, allocations) = crate::heap::allocations(|| s.check_report());
        page.times.push(at.elapsed());
        let (log, ended) = super::slices();
        if let (Some(a), Some(b)) = (started, ended) {
            page.whole.push(b.saturating_sub(a));
        }
        page.slices.push(log.max);
        points = log.seen;
        page.touched.push(super::fork_work_done() - before);
        page.reports.push(report);
        page.held.push(allocations);
    }
    // Cancelled at each of its cancellation points in turn: the thread CPU
    // time from the cancel to the live world back (the teardown).
    for i in 0..100_u64 {
        super::log_slices(Some(i % points.max(1) + 1));
        let report = s.check_report();
        let (log, now) = super::slices();
        if let (Some(fired), Some(now)) = (log.fired, now) {
            page.torn.push(now.saturating_sub(fired));
        }
        assert_eq!(report, super::CheckReport::Cancelled, "cancelled at point {i}");
    }
    page.points = points;
    // Input arriving during a check (ADR-0028's gate): from another
    // thread, at a generated moment, the check is cancelled; what input
    // waits is from that moment to the live world back.
    let mut rng = Rng::new(0x5eed);
    for _ in 0..100 {
        let flag = super::CancelFlag::default();
        let delay = std::time::Duration::from_micros(rng.below(1_500));
        let (f, set) = (flag.clone(), std::sync::Arc::new(std::sync::Mutex::new(None)));
        let at = std::sync::Arc::clone(&set);
        let input = std::thread::spawn(move || {
            std::thread::sleep(delay);
            f.cancel();
            if let Ok(mut at) = at.lock() {
                *at = Some(Instant::now());
            }
        });
        let report = s.check_report_cancelled_by(&flag);
        let back = Instant::now();
        let _joined = input.join();
        let arrived = set.lock().ok().and_then(|a| *a);
        page.waited
            .push(arrived.map_or(std::time::Duration::ZERO, |a| back.saturating_duration_since(a)));
        if report == super::CheckReport::Cancelled {
            page.cancelled = page.cancelled.saturating_add(1);
        }
    }
    page.same_graph = graph == crate::reactive::graph();
    drop((keep, s));
    c.dispose();
    page
}
