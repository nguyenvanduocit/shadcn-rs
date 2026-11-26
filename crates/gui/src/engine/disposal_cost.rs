//! What a check's disposal costs (review r16, decision 4), against a plain
//! std drop of the same value: a million-entry map of inert values goes
//! whole (one box), and one of caller values one box per value.
//!
//! Checks are compiled with debug assertions, so run it optimised with them
//! on:
//!
//! ```text
//! cargo test --config 'profile.measure.inherits="release"' \
//!   --config 'profile.measure.debug-assertions=true' --config 'profile.measure.debug=false' \
//!   --profile measure -p gui --lib disposal_cost -- --ignored --nocapture
//! ```

use super::FreshValue;
use super::frame::in_cold_frame;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Duration;

const ENTRIES: u64 = 1_000_000;

/// This thread's CPU time (the machine is shared: wall time measures the
/// other loads too).
fn cpu() -> Duration {
    cpu_time::ThreadTime::try_now().map_or(Duration::ZERO, |t| t.as_duration())
}

/// A caller's value: its drop may run app code, so one box each.
struct Fresh(#[expect(dead_code, reason = "a payload, never read")] u64);

impl FreshValue for Fresh {}

impl Drop for Fresh {
    fn drop(&mut self) {
        std::hint::black_box(std::ptr::from_mut(self));
    }
}

crate::engine_local! {
    [Fresh] static STRINGS: RefCell<HashMap<u64, String>> = RefCell::default();
    [Fresh] static VALUES: RefCell<HashMap<u64, Box<Fresh>>> = RefCell::default();
}

fn strings() -> HashMap<u64, String> {
    (0..ENTRIES).map(|i| (i, i.to_string())).collect()
}

fn values() -> HashMap<u64, Box<Fresh>> {
    (0..ENTRIES).map(|i| (i, Box::new(Fresh(i)))).collect()
}

/// The teardown of one check that fills a piece by `fill` and leaves it to
/// the disposal: its thread CPU time, from the body's end to the check's
/// return, and
/// its allocations (the check's, less the fill's, less an empty check's).
fn teardown(fill: &dyn Fn()) -> (Duration, u64) {
    let (_, empty) = crate::heap::allocations(|| in_cold_frame(|| ()));
    let ended = Cell::new(None);
    let filled = Cell::new(0);
    let (_, all) = crate::heap::allocations(|| {
        in_cold_frame(|| {
            let ((), a) = crate::heap::allocations(fill);
            filled.set(a);
            ended.set(Some(cpu()));
        });
    });
    let time = ended.get().map_or(Duration::MAX, |e| cpu().saturating_sub(e));
    (time, all.saturating_sub(filled.get()).saturating_sub(empty))
}

/// The best of `runs` teardowns.
fn disposal(runs: usize, fill: impl Fn()) -> (Duration, u64) {
    let mut best = (Duration::MAX, u64::MAX);
    for _ in 0..runs {
        let (t, a) = teardown(&fill);
        best = (best.0.min(t), best.1.min(a));
    }
    best
}

/// The best of `runs` plain drops of what `make` makes.
fn std_drop<T>(runs: usize, make: impl Fn() -> T) -> (Duration, u64) {
    let mut best = (Duration::MAX, u64::MAX);
    for _ in 0..runs {
        let made = make();
        let start = cpu();
        let ((), a) = crate::heap::allocations(|| drop(made));
        best = (best.0.min(cpu().saturating_sub(start)), best.1.min(a));
    }
    best
}

/// How many boxes `value` comes apart into.
fn boxes<T: super::Parts>(value: T) -> usize {
    let mut out = Vec::new();
    value.parts(&mut out);
    out.len()
}

/// Decision 4: a value whose drop runs no app code goes whole, one box,
/// however many elements; one that may run app code, one box per value.
#[test]
fn inert_values_go_whole_and_caller_values_one_box_each() {
    use std::collections::{BTreeMap, HashSet, VecDeque};
    let strings: HashMap<u64, String> = (0..1000).map(|i| (i, i.to_string())).collect();
    assert_eq!(boxes(strings), 1, "a map of strings is inert");
    let names: Vec<Box<str>> = (0..100).map(|i| i.to_string().into_boxed_str()).collect();
    assert_eq!(boxes(names), 1, "boxed strs are inert");
    type Row = (Option<String>, VecDeque<u8>, BTreeMap<u8, String>);
    let nested: Vec<Row> = (0..100)
        .map(|i| {
            (
                Some(i.to_string()),
                VecDeque::from([1]),
                BTreeMap::from([(1, String::new())]),
            )
        })
        .collect();
    assert_eq!(boxes(nested), 1, "containers of inert values are inert");
    let set: HashSet<String> = (0..100).map(|i| i.to_string()).collect();
    assert_eq!(boxes(set), 1);
    let values: HashMap<u64, Box<Fresh>> = (0..100).map(|i| (i, Box::new(Fresh(i)))).collect();
    assert_eq!(boxes(values), 101, "each caller value, then the emptied map");
    let fresh: Vec<Fresh> = (0..100).map(Fresh).collect();
    assert_eq!(boxes(fresh), 100, "each caller value on its own");
    let mixed: Vec<(String, Fresh)> = (0..10).map(|i| (String::new(), Fresh(i))).collect();
    assert_eq!(boxes(mixed), 20, "an inert part beside a caller value is split from it");
}

fn per_entry(d: Duration) -> f64 {
    d.as_secs_f64() * 1e9 / f64::from(u32::try_from(ENTRIES).unwrap_or(u32::MAX))
}

#[test]
#[ignore = "a measurement: run optimised with --ignored (the module's doc)"]
fn a_million_entry_map_is_disposed_at_std_cost() {
    const RUNS: usize = 7;
    STRINGS.with(|_| ());
    VALUES.with(|_| ());
    let (inert, inert_a) = disposal(RUNS, || {
        let m = strings();
        STRINGS.with_borrow_mut(|s| *s = m);
    });
    let (std_s, std_s_a) = std_drop(RUNS, strings);
    let (fresh, fresh_a) = disposal(RUNS, || {
        let m = values();
        VALUES.with_borrow_mut(|s| *s = m);
    });
    let (std_v, std_v_a) = std_drop(RUNS, values);
    let ratio = inert.as_secs_f64() / std_s.as_secs_f64().max(f64::MIN_POSITIVE);
    eprintln!(
        "HashMap<u64, String>, {ENTRIES} entries: check teardown {:.2} ns/entry, {inert_a} allocations; std drop \
         {:.2} ns/entry, {std_s_a} allocations; ratio {ratio:.3}",
        per_entry(inert),
        per_entry(std_s),
    );
    eprintln!(
        "HashMap<u64, Box<caller value>>, {ENTRIES} entries: check teardown {:.2} ns/entry, {fresh_a} allocations; \
         std drop {:.2} ns/entry, {std_v_a} allocations; ratio {:.3}, overhead {:.2} ns per box",
        per_entry(fresh),
        per_entry(std_v),
        fresh.as_secs_f64() / std_v.as_secs_f64().max(f64::MIN_POSITIVE),
        per_entry(fresh.saturating_sub(std_v)),
    );
    assert!(inert_a <= 16, "an inert map goes whole: {inert_a} allocations");
}
