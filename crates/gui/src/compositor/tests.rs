//! ADR-0015: the compositor draws what the UI committed, scrolls and moves
//! by itself between commits without losing or doubling a pixel, and input
//! lands on what was on screen.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::scroll::{Scrolls, WheelTarget};
use super::*;

/// A trackpad's phase as the old tests told it: fingers down, lifted, or
/// moving in between.
fn phase_of(began: bool, ended: bool) -> WheelPhase {
    if began {
        WheelPhase::Began
    } else if ended {
        WheelPhase::Ended
    } else {
        WheelPhase::Changed
    }
}
use crate::app::Scenario;
use crate::clock::{self, Vsync};
use crate::color::Color;
use crate::element::{IntoElement, div, text};
use crate::prelude::*;
use crate::reactive::Signal;
use crate::scene::Properties;

/// The frame number `shown` stamps, 0 before the first.
fn stamp(shown: &Shown) -> u64 {
    numbered(shown.seen())
}

/// The number `seen` names, 0 before the first frame.
fn numbered(seen: Seen) -> u64 {
    match seen {
        Seen::Frame(n) => n,
        Seen::BeforeFirst => 0,
    }
}

/// `dy` pixels down, in units ([`motion`]).
fn y(dy: f32) -> motion::Exact {
    (0, motion::units(dy))
}

/// `n`'s base and range's end set to `offset` and `max` down, as paint
/// sets both its pixels and its units.
fn place(n: &mut ScrollNode, offset: f32, max: f32) {
    n.offset.1 = offset;
    n.applied.1 = -offset;
    n.max.1 = max;
    n.pos.1 = motion::units(offset);
    n.end.1 = motion::units(max);
}

fn node(key: u64, offset: f32, max: f32) -> ScrollNode {
    ScrollNode {
        key: NodeKey(key),
        owner: None,
        instance: key,
        content: 1,
        offset: (0.0, offset),
        applied: (0.0, -offset),
        max: (0.0, max),
        pos: y(offset),
        end: y(max),
        rtl: false,
        generation: 0,
        thumb: None,
        parent: None,
        wheel: false,
        wheel_above: false,
        below_modal: false,
    }
}

/// The compositor's tables of `nodes` alone, indexed as paint indexes them.
fn table(nodes: &[ScrollNode]) -> Compositing {
    Compositing {
        scrollers: Arc::from(nodes),
        index: Arc::new(TableIndex::of(nodes, &[])),
        ..Compositing::default()
    }
}

/// What the UI takes in now: what was moved since the last pull.
fn take(s: &mut Scrolls) -> Vec<ScrollDelta> {
    let mut into = Vec::new();
    s.pull(&mut into);
    into
}

/// A commit of `nodes` whose bases hold every pull the UI took so far.
fn commit(s: &mut Scrolls, nodes: &[ScrollNode]) {
    let taken = s.outbox().takes;
    s.committed(&table(nodes), taken, 0);
}

#[test]
fn a_scroll_is_owed_until_a_commit_takes_it_in_and_never_twice() {
    let mut s = Scrolls::default();
    let mut nodes = vec![node(1, 0.0, 500.0)];
    assert!(s.scroll(&nodes, WheelTarget(0), y(30.0), 0));
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 30.0), "shown at once");
    // The UI takes the 30 px in; 10 more come while it lays out.
    let took = take(&mut s);
    assert_eq!(took.len(), 1);
    assert_eq!(took[0].delta, y(30.0));
    assert!(s.scroll(&nodes, WheelTarget(0), y(10.0), 0));
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 40.0));
    // A commit built before that take holds none of it: nothing lost.
    s.committed(&table(&nodes), 0, 0);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 40.0), "nothing lost");
    // One built after has the 30 px in its base: only the 10 are owed.
    place(&mut nodes[0], 30.0, 500.0);
    commit(&mut s, &nodes);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 40.0), "nothing doubled");
    let took = take(&mut s);
    assert_eq!(took[0].delta, y(10.0));
    place(&mut nodes[0], 40.0, 500.0);
    commit(&mut s, &nodes);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 40.0));
    assert!(take(&mut s).is_empty(), "nothing owed");
    assert_eq!(s.outbox().entries(), 0, "nothing held");
}

/// List A scrolled while the commit in flight replaces it with B at the
/// same path (another instance): A's scroll, owed and pending, is A's, and
/// B is neither shown with it nor sent it.
#[test]
fn a_list_replaced_at_its_path_by_another_instance_gets_none_of_its_scroll() {
    let (a, b) = (crate::reactive::Scope::new(), crate::reactive::Scope::new());
    let (a, b) = (a.run(crate::reactive::owner), b.run(crate::reactive::owner));
    assert!(a.is_some() && a != b, "two instances");
    let mut s = Scrolls::default();
    let list_a = vec![ScrollNode {
        owner: a,
        ..node(1, 0.0, 500.0)
    }];
    assert!(s.scroll(&list_a, WheelTarget(0), y(30.0), 0));
    let took = take(&mut s);
    assert_eq!((took[0].owner, took[0].delta), (a, y(30.0)), "A's delta names A");
    // More while the UI lays out; its commit has B where A was.
    assert!(s.scroll(&list_a, WheelTarget(0), y(10.0), 0));
    let list_b = vec![ScrollNode {
        owner: b,
        instance: 2,
        ..node(1, 0.0, 500.0)
    }];
    s.committed(&table(&list_b), 0, 0);
    assert_eq!(s.offset(&list_b[0], 0), (0.0, 0.0), "B is shown at its own offset");
    assert!(take(&mut s).is_empty(), "and is sent nothing of A's");
    assert_eq!(s.outbox().entries(), 0, "nothing of A's is kept");
}

/// A list scrolled, hidden for one commit, and shown again at the same
/// path under the same owner is another instance (paint gives it a new
/// one): nothing of the first goes with it, neither a predicted reach nor
/// a scrollbar, until its own first wheel event. Hidden, the first's
/// scrollbar fades no more.
#[test]
fn a_list_hidden_and_shown_again_starts_with_no_reach_and_no_scrollbar() {
    let owner = crate::reactive::Scope::new().run(crate::reactive::owner);
    // Hidden for a commit between, or replaced by the new instance in one.
    for hidden in [true, false] {
        let mut s = Scrolls::default();
        let first = vec![ScrollNode {
            owner,
            ..node(1, 0.0, 500.0)
        }];
        for k in 0..4_u8 {
            assert!(s.scroll(&first, WheelTarget(0), y(10.0), clock::nanos_of(f32::from(k) * 0.01)));
        }
        let reach = s.reach(&table(&first), 0.04, 0.1);
        assert!(reach[0].reach.1 > 0.0, "a list scrolling has a reach");
        assert!(s.thumb_opacity(&first[0], 0.04) > 0.0, "and its scrollbar shows");
        if hidden {
            s.committed(&table(&[]), 0, 0);
            assert!(!s.animating(0.05), "hidden, its scrollbar is gone");
        }
        let again = vec![ScrollNode {
            owner,
            instance: 2,
            ..node(1, 0.0, 500.0)
        }];
        s.committed(&table(&again), 0, 0);
        assert!(!s.animating(0.05), "the first one's scrollbar is gone ({hidden})");
        assert_eq!(s.thumb_opacity(&again[0], 0.05), 0.0, "shown again, no scrollbar");
        assert!(
            s.reach(&table(&again), 0.05, 0.1).is_empty(),
            "nor any reach ({hidden})"
        );
        assert!(take(&mut s).is_empty(), "nor any of its scroll ({hidden})");
    }
}

/// The program jumps a list to 200 (`scroll_to`) while the compositor
/// scrolled it 30: what the UI took before the jump is in no base after it
/// (the jump is an assignment over it), and what it had not taken goes on
/// top of the place jumped to (ADR-0032, I4: real input is never dropped).
#[test]
fn a_jump_drops_what_the_ui_took_and_keeps_what_it_had_not_on_top() {
    for taken in [false, true] {
        let mut s = Scrolls::default();
        let mut nodes = vec![node(1, 0.0, 500.0)];
        assert!(s.scroll(&nodes, WheelTarget(0), y(30.0), 0));
        if taken {
            let _took = take(&mut s);
        }
        place(&mut nodes[0], 200.0, 500.0);
        nodes[0].generation = 1;
        commit(&mut s, &nodes);
        let want = if taken { 200.0 } else { 230.0 };
        assert_eq!(s.offset(&nodes[0], 0), (0.0, want), "taken: {taken}");
    }
}

/// I3: what is owed is what the range let the content move, so a push
/// out past an edge moves nothing then and nothing later; the step is
/// still told once.
#[test]
fn a_scroll_stops_at_the_edges() {
    let mut s = Scrolls::default();
    let mut nodes = vec![node(1, 490.0, 500.0)];
    assert!(s.scroll(&nodes, WheelTarget(0), y(30.0), 0));
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 500.0));
    // Against its end it moves no more; its wheel is already news for the
    // UI, so nothing more is asked for.
    assert!(!s.scroll(&nodes, WheelTarget(0), y(30.0), 0), "at the end");
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 500.0), "no further");
    // The range grows before the UI takes it: the push past the end that
    // moved nothing moves nothing now.
    place(&mut nodes[0], 490.0, 900.0);
    s.committed(&table(&nodes), 0, 0);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 500.0), "no blocked intent");
    let took = take(&mut s);
    assert_eq!(
        (took[0].delta, took[0].steps),
        (y(10.0), 2),
        "what moved, each step once"
    );
}

/// A range that changes under scroll the UI has not taken re-clamps it
/// once, in the order it moved (Codex r7 4): -10 at the top moves nothing,
/// +10 moves 10, and when the range grows the content is where those
/// moves put it, not where their merged sum would.
#[test]
fn a_range_change_re_clamps_what_is_owed_in_order() {
    let mut s = Scrolls::default();
    let mut nodes = vec![node(1, 0.0, 100.0)];
    assert!(!s.scroll(&nodes, WheelTarget(0), y(-10.0), 0) || s.offset(&nodes[0], 0) == (0.0, 0.0));
    assert!(s.scroll(&nodes, WheelTarget(0), y(10.0), 0));
    for k in 0..50 {
        let d = if k % 2 == 0 { 1.0 } else { -1.0 };
        s.scroll(&nodes, WheelTarget(0), y(d), 0);
    }
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 10.0));
    place(&mut nodes[0], 0.0, 200.0);
    s.committed(&table(&nodes), 0, 0);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 10.0), "where the moves put it");
    let took = take(&mut s);
    assert_eq!(took[0].delta, y(10.0));
}

#[test]
fn a_move_reaches_every_transform_under_it() {
    let t = |parent: u32| TransformNode {
        parent,
        offset: (0.0, 0.0),
        matrix: None,
    };
    let props = Properties {
        transforms: vec![t(0), t(0), t(1), t(2), t(0)],
        ..Properties::default()
    };
    let own: foldhash::HashMap<u32, (f32, f32)> = [(1, (0.0, -10.0)), (3, (5.0, 0.0))].into_iter().collect();
    assert_eq!(
        compose::totals(&props, &own),
        vec![(0.0, 0.0), (0.0, -10.0), (0.0, -10.0), (5.0, -10.0), (0.0, 0.0)]
    );
}

#[test]
fn a_scroll_prediction_interpolates_reaches_8_ms_ahead_and_stops_after_35() {
    let mut p = ScrollPredictor::default();
    p.push(0.000, y(10.0));
    p.push(0.010, y(10.0));
    p.push(0.020, y(10.0));
    let close = |a: motion::Exact, b: (f32, f32)| {
        let a = motion::px2(a);
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    };
    assert!(close(p.at(0.015), (0.0, 25.0)), "between events: {:?}", p.at(0.015));
    // 1 px a millisecond, at most min(8 ms, half the gap = 5 ms) ahead.
    assert!(close(p.at(0.022), (0.0, 32.0)), "{:?}", p.at(0.022));
    assert!(close(p.at(0.030), (0.0, 35.0)), "capped: {:?}", p.at(0.030));
    assert!(close(p.at(0.060), (0.0, 30.0)), "past 35 ms, the events' own total");
    let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
    assert!(close(p.velocity(), (0.0, 1000.0)), "{:?}", p.velocity());
}

#[test]
fn a_fling_slows_and_ends() {
    let f = Fling::new(1.0, (0.0, 2000.0)).expect("fast enough");
    let early = f.at(1.1).1;
    let later = f.at(1.2).1;
    assert!(early > 0 && later > early, "carries on");
    assert!(later - early < early, "slower and slower");
    assert!(f.moving(1.2));
    assert!(!f.moving(3.0), "ends");
    // Never further than velocity × τ.
    assert!(motion::px(f.at(10.0).1) <= 2000.0 * 0.325 + 1e-3);
    assert!(Fling::new(0.0, (0.0, 10.0)).is_none(), "too slow to fling");
}

#[test]
fn vsync_is_fitted_to_blocked_acquires() {
    let interval = Duration::from_secs_f64(1.0 / 120.0);
    let mut v = Vsync::new(interval);
    let base = Instant::now();
    let at = |k: u32, jitter_us: i64| {
        let t = base.checked_add(interval * k).unwrap_or(base);
        if jitter_us >= 0 {
            t.checked_add(Duration::from_micros(jitter_us.unsigned_abs()))
                .unwrap_or(t)
        } else {
            t.checked_sub(Duration::from_micros(jitter_us.unsigned_abs()))
                .unwrap_or(t)
        }
    };
    let jitter = [120, -80, 40, -150, 90, 0, -60, 110];
    for (k, j) in (0..).zip(jitter) {
        v.acquired(at(k, j), Duration::from_millis(3));
    }
    // An acquire far off the grid (a late wake) does not move the fit.
    v.acquired(at(8, 3000), Duration::from_millis(3));
    // One that did not block is no vblank.
    v.acquired(at(9, 4000), Duration::from_micros(100));
    let next = v.next_after(at(20, 1000));
    let grid = at(21, 0);
    let off = if next >= grid {
        next.duration_since(grid)
    } else {
        grid.duration_since(next)
    };
    let off = off.as_secs_f64();
    assert!(off < 0.000_5, "the next vblank is on the grid: {off}s off");
}

/// A list of 40 rows, 24 px each, in a 200 px scroll container; a click on
/// a row writes its number.
fn rows(clicked: Signal<i32>) -> impl IntoElement {
    div()
        .size(pct(100.0))
        .overflow_y(Overflow::Scroll)
        .overflow_x(Overflow::Hidden)
        .children((0..40).map(move |i| {
            div()
                .height(24.0)
                .background(if i % 2 == 0 { Color::hex(0xf4f4f5) } else { Color::WHITE })
                .on_click(move |_| clicked.set(i))
                .child(text(format!("row {i}")))
        }))
}

#[test]
fn a_click_while_the_compositor_scrolls_hits_what_was_on_screen() {
    let clicked = Signal::new(-1);
    let mut s = Scenario::measuring(move || rows(clicked).into_element(), 300, 200, 2.0).expect("a scenario");
    s.frame("first").expect("a frame");
    s.hover(100.0, 100.0);
    s.frame("rest").expect("a frame");
    // Scrolled two rows by the compositor; the UI has not laid that out.
    assert!(s.wheel(48.0));
    assert_eq!(
        s.last()
            .nodes
            .iter()
            .find_map(|n| n.scroll(crate::inspect::OUTSIDE))
            .map(|sc| sc.offset),
        Some((0.0, 0.0))
    );
    s.click(100.0, 12.0);
    assert_eq!(clicked.get_untracked(), 2, "row 2 was on screen there");
    // The UI takes the scroll in with its next frame, once.
    s.frame("scrolled").expect("a frame");
    assert_eq!(
        s.last()
            .nodes
            .iter()
            .find_map(|n| n.scroll(crate::inspect::OUTSIDE))
            .map(|sc| sc.offset),
        Some((0.0, 48.0))
    );
    s.click(100.0, 12.0);
    assert_eq!(clicked.get_untracked(), 2, "and only once");
    s.frame("again").expect("a frame");
    assert_eq!(
        s.last()
            .nodes
            .iter()
            .find_map(|n| n.scroll(crate::inspect::OUTSIDE))
            .map(|sc| sc.offset),
        Some((0.0, 48.0))
    );
}

/// What the compositor holds is the UI's scene, and what it composes at the
/// commit's own time is what the UI recorded, motion included.
fn same_as_committed(s: &mut Scenario, at: f32, what: &str) {
    let (ui, headless) = s.parts();
    let c = headless.compositor.committed();
    assert_eq!(c.chunks.len(), ui.chunks.len(), "{what}: chunks");
    for (a, b) in c.chunks.iter().zip(&ui.chunks) {
        assert_eq!(a.id, b.id, "{what}: chunk order");
        assert_eq!(a.state, b.state, "{what}: chunk state");
        assert!(Arc::ptr_eq(&a.display, &b.display), "{what}: the same recording");
    }
    assert_eq!(c.properties, ui.properties, "{what}: property trees");
    assert_eq!(c.compositing, ui.compositing, "{what}: what it moves");
    let ui_props = ui.properties.clone();
    let composed = headless.compositor.compose(at);
    assert_eq!(composed.properties, ui_props, "{what}: composed at its own time");
}

#[test]
fn a_commit_applied_is_the_ui_scene_and_composes_to_it() {
    let shown = Signal::new(true);
    let clicked = Signal::new(-1);
    let mut s = Scenario::measuring(
        move || {
            let on = shown.get();
            div()
                .size(pct(100.0))
                .child(
                    div()
                        .width(120.0)
                        .height(40.0)
                        .background(Color::hex(0x18181b))
                        .duration_ms(200.0)
                        .ease(Easing::Linear)
                        .transition(crate::motion::Properties::OPACITY)
                        .opacity(if on { 1.0 } else { 0.2 }),
                )
                .child(div().height(150.0).child(rows(clicked)))
                .into_element()
        },
        300,
        200,
        2.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    same_as_committed(&mut s, clock::current_seconds(), "first");
    s.play_motion();
    s.act("fade", || shown.set(false));
    s.frame("starts").expect("a frame");
    same_as_committed(&mut s, clock::current_seconds(), "starts");
    for step in 0..3 {
        s.wait(Duration::from_millis(40));
        s.frame(&format!("step {step}")).expect("a frame");
        same_as_committed(&mut s, clock::current_seconds(), &format!("step {step}"));
    }
    s.hover(100.0, 120.0);
    assert!(s.wheel(30.0));
    s.frame("scrolled").expect("a frame");
    // The user scrolled it: its overlay scrollbar shows, though committed
    // hidden, and composes back to the commit once it has faded.
    let now = clock::current_seconds();
    let (_, headless) = s.parts();
    let effect = headless
        .compositor
        .committed()
        .compositing
        .scrollers
        .iter()
        .find_map(|sc| sc.thumb)
        .map(|t| t.effect)
        .expect("a scrollbar");
    let at = usize::try_from(effect).unwrap_or(usize::MAX);
    let opacity = |props: &Properties| props.effects.get(at).map(|e| e.opacity);
    assert_eq!(
        opacity(&headless.compositor.committed().properties),
        Some(0.0),
        "hidden at rest"
    );
    let mut composed = headless.compositor.compose(now).properties.clone();
    assert_eq!(opacity(&composed), Some(1.0), "shown as it scrolls");
    if let Some(e) = composed.effects.get_mut(at) {
        e.opacity = 0.0;
    }
    assert_eq!(
        composed,
        headless.compositor.committed().properties,
        "the rest is the commit"
    );
}

#[test]
fn an_animation_is_evenly_spaced_at_predicted_present_times() {
    let shown = Signal::new(true);
    let mut s = Scenario::measuring(
        move || {
            div()
                .size(pct(100.0))
                .child(
                    div()
                        .width(120.0)
                        .height(40.0)
                        .background(Color::BLACK)
                        .duration_ms(500.0)
                        .ease(Easing::Linear)
                        .transition(crate::motion::Properties::OPACITY)
                        .opacity(if shown.get() { 1.0 } else { 0.0 }),
                )
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.play_motion();
    s.act("fade", || shown.set(false));
    s.frame("starts").expect("a frame");
    let start = clock::current_seconds();
    // A fake 120 Hz display: blocked acquires with jitter, the frames'
    // present times predicted from them, as the render thread does.
    let interval = Duration::from_secs_f64(1.0 / 120.0);
    let mut vsync = Vsync::new(interval);
    let base = clock::instant(start);
    let jitter = [0_u64, 300, 100, 250, 50, 200, 150, 0, 300, 100, 50, 250];
    let (_, headless) = s.parts();
    let Some(effect) = headless
        .compositor
        .committed()
        .compositing
        .moving
        .first()
        .and_then(|m| m.effect)
    else {
        panic!("the fade is the compositor's");
    };
    let mut opacities = Vec::new();
    for (k, j) in (0_u32..).zip(jitter) {
        // Each wake lands somewhere in the first 0.3 ms after a vblank.
        let woke = base
            .checked_add(interval * k + Duration::from_micros(j))
            .unwrap_or(base);
        vsync.acquired(woke, Duration::from_millis(4));
        let present = vsync.present_time(woke, Duration::from_millis(1));
        let at = clock::seconds(present);
        let scene = headless.compositor.compose(at);
        let e = usize::try_from(effect)
            .ok()
            .and_then(|i| scene.properties.effects.get(i));
        opacities.push(e.map_or(f32::NAN, |e| e.opacity));
    }
    let steps: Vec<f32> = opacities.windows(2).map(|w| w[0] - w[1]).collect();
    // 1/120 s of a 0.5 s linear fade from 1 to 0.
    let want = 1.0 / 120.0 / 0.5;
    for (i, d) in steps.iter().enumerate().skip(2) {
        assert!(
            (d - want).abs() < want * 0.1,
            "frame {i} moved {d}, not {want}: {opacities:?}"
        );
    }
}

#[test]
fn a_frame_the_ui_owes_still_moves_what_the_compositor_runs() {
    let shown = Signal::new(true);
    let renders = Rc::new(Cell::new(0));
    let counted = Rc::clone(&renders);
    let mut s = Scenario::measuring(
        move || {
            counted.set(counted.get() + 1);
            div()
                .size(pct(100.0))
                .child(
                    div()
                        .width(120.0)
                        .height(40.0)
                        .background(Color::BLACK)
                        .duration_ms(300.0)
                        .ease(Easing::Linear)
                        .transition(crate::motion::Properties::TRANSFORM)
                        .translate_y(crate::style::Calc::px(if shown.get() { 0.0 } else { 60.0 })),
                )
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.play_motion();
    s.act("move", || shown.set(false));
    s.frame("starts").expect("a frame");
    let start = clock::current_seconds();
    let before = renders.get();
    let (_, headless) = s.parts();
    // Compositor-only frames while the UI is busy: the box moves on.
    headless.draw(start + 0.1);
    let first = headless.presented();
    headless.draw(start + 0.2);
    let second = headless.presented();
    assert_eq!(renders.get(), before, "the UI ran nothing");
    let moved = |p: &Presented| p.moved.first().map_or(0.0, |m| m.1.1);
    assert!(
        moved(&second) > moved(&first) && moved(&first) > 0.0,
        "{first:?} then {second:?}"
    );
}

/// Wait until `done` holds, up to ten seconds: whether it happens, not how
/// fast. A render thread made afresh makes a device and builds every
/// pipeline up front, which on lavapipe (software Vulkan, CI) amid the
/// parallel tests takes seconds. `done` is given the time left, which
/// whatever it waits on (a read back) waits at most, so no try near the
/// end stretches the wait.
fn wait_for(mut done: impl FnMut(Duration) -> bool) -> bool {
    let until = Instant::now()
        .checked_add(Duration::from_secs(10))
        .unwrap_or_else(Instant::now);
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        if done(left) {
            return true;
        }
        // What `done` waited took from the time left.
        let left = until.saturating_duration_since(Instant::now());
        std::thread::sleep(Duration::from_millis(10).min(left));
    }
}

/// What the window shows once the render thread drew a commit after `after`.
fn shown_after(off: &crate::window::Offscreen, after: u64) -> Option<crate::image::Pixels> {
    assert!(
        wait_for(|_| off.presented().commit > after),
        "no commit after {after} was drawn"
    );
    off.capture()
}

/// What the window shows once a commit after `after` changed it from `was`
/// (a click's press may be committed before its release).
fn changed_after(
    off: &crate::window::Offscreen,
    after: u64,
    was: &crate::image::Pixels,
) -> Option<crate::image::Pixels> {
    let mut shown = None;
    wait_for(|left| {
        shown = (off.presented().commit > after)
            .then(|| off.capture_within(left))
            .flatten()
            .filter(|p| p != was);
        shown.is_some()
    });
    shown
}

#[test]
fn a_lost_device_or_a_render_panic_is_recovered_from_the_uis_arcs() {
    use crate::window::Input;
    // A box that toggles its colour on a click, over text.
    let started = crate::app::offscreen(
        || {
            let dark = Signal::new(false);
            move || {
                div()
                    .size(pct(100.0))
                    .child(
                        div()
                            .width(80.0)
                            .height(40.0)
                            .background(if dark.get() {
                                Color::hex(0x18181b)
                            } else {
                                Color::hex(0xe4e4e7)
                            })
                            .on_click(move |_| dark.update(|d| *d = !*d)),
                    )
                    .child(text("still here"))
                    .into_element()
            }
        },
        crate::app::RunOptions::default(),
        (160, 120),
        1.0,
        Duration::from_millis(8),
    );
    let Ok(mut off) = started else {
        eprintln!("no GPU: skipped");
        return;
    };
    let light = shown_after(&off, 0).expect("a first frame");
    let click = |off: &mut crate::window::Offscreen| {
        off.input(Input::MouseDown { x: 20.0, y: 20.0 });
        off.input(Input::MouseUp { x: 20.0, y: 20.0 });
    };

    // The device is lost: the render thread makes it again by itself, idle,
    // and draws the scene as committed whole. Nothing is read back until
    // a frame it drew says so (a read back would make the device itself).
    drop(off.take_frames());
    off.lose_device();
    let mut frames = Vec::new();
    assert!(
        wait_for(|_| {
            frames.extend(off.take_frames());
            frames.iter().any(|f| f.renewed)
        }),
        "the render thread made the lost device again before any read back"
    );
    let renewed = off.capture();
    assert_eq!(renewed.as_ref(), Some(&light), "the same frame on a new device");
    let before = off.presented().commit;
    click(&mut off);
    let dark = changed_after(&off, before, &light).expect("commits after the loss are drawn");

    // The render thread panics: made afresh, it draws the UI's full commit.
    // A commit the click still had in flight may be drawn before the panic,
    // and the fresh compositor holds nothing until the full commit comes, so
    // what it reads back on the way is not the answer: what it settles on is.
    let before = off.presented().commit;
    off.panic_render();
    let mut again = None;
    wait_for(|left| {
        again = (off.presented().commit > before)
            .then(|| off.capture_within(left))
            .flatten()
            .filter(|p| *p == dark);
        again.is_some()
    });
    assert_eq!(again.as_ref(), Some(&dark), "the full commit draws what was on screen");
    let before = off.presented().commit;
    click(&mut off);
    let back = changed_after(&off, before, &dark).expect("deltas after the full commit are drawn");
    assert_eq!(back, light, "deltas after the full commit apply to it");
}

/// Opus r10 I1 and Codex r10 1, through the live threads: 200 wheels of 3
/// px over a list, the render thread made to panic after 50. Whatever
/// fell before, during and after its recovery, up to its first full
/// commit (when its compositor has nothing to scroll), reaches a scroll:
/// the list ends 600 px down, none lost.
#[test]
fn wheels_around_a_render_panic_are_all_scrolled() {
    let started = crate::app::offscreen(
        || {
            || {
                div()
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(10_000.0))
                    .into_element()
            }
        },
        crate::app::RunOptions::default(),
        (100, 100),
        1.0,
        Duration::from_millis(8),
    );
    let Ok(mut off) = started else {
        eprintln!("no GPU: skipped");
        return;
    };
    let offset = |off: &crate::window::Offscreen| off.presented().scroll.first().map(|s| s.offset.1);
    assert!(wait_for(|_| offset(&off).is_some()), "the list shown");
    off.input(crate::window::Input::PointerMoved { x: 50.0, y: 50.0 });
    // Wheels a millisecond apart for 200 ms, through the panic, its
    // recovery and the full commit after it.
    for k in 0..200 {
        if k == 50 {
            off.panic_render();
        }
        off.wheel((0.0, 3.0), false);
        std::thread::sleep(Duration::from_millis(1));
    }
    wait_for(|_| offset(&off) == Some(600.0));
    assert_eq!(offset(&off), Some(600.0), "every wheel scrolled once");
}

/// A list 100 px tall over 5000 px, on a window of its size, run on the
/// app's threads with frames 40 ms apart (none drawn between two wheels).
fn a_list_on_threads() -> Option<crate::window::Offscreen> {
    let started = crate::app::offscreen(
        || {
            || {
                div()
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(5000.0))
                    .into_element()
            }
        },
        crate::app::RunOptions::default(),
        (100, 100),
        1.0,
        Duration::from_millis(40),
    );
    let Ok(off) = started else {
        eprintln!("no GPU: skipped");
        return None;
    };
    assert!(wait_for(|_| !off.presented().scroll.is_empty()), "the list shown");
    Some(off)
}

/// Codex r11 1: a trackpad's step the compositor took, still held by its
/// gesture's resampling when the render thread panics (no frame ticked
/// it), is scrolled all the same: diverting settles the gesture into the
/// outbox before it pulls, so the UI has it before the wheel after.
#[test]
fn a_trackpad_step_held_for_resampling_when_the_render_thread_panics_is_scrolled() {
    let Some(mut off) = a_list_on_threads() else { return };
    off.input(crate::window::Input::PointerMoved { x: 50.0, y: 50.0 });
    off.wheel((0.0, 30.0), true);
    off.panic_render();
    off.wheel((0.0, 5.0), false);
    let offset = |off: &crate::window::Offscreen| off.presented().scroll.first().map(|s| s.offset.1);
    wait_for(|_| offset(&off) == Some(35.0));
    assert_eq!(offset(&off), Some(35.0), "the trackpad's 30 and the wheel's 5");
}

/// A list 60 px tall (140 px to scroll) at the top of one 100 px tall
/// (60 px to scroll), on the app's threads.
fn nested_lists_on_threads() -> Option<crate::window::Offscreen> {
    let started = crate::app::offscreen(
        || {
            || {
                div()
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .child(
                        div()
                            .width(100.0)
                            .height(60.0)
                            .flex_shrink(0.0)
                            .overflow_y(Overflow::Scroll)
                            .child(div().height(200.0)),
                    )
                    .child(div().height(100.0).flex_shrink(0.0))
                    .into_element()
            }
        },
        crate::app::RunOptions::default(),
        (100, 100),
        1.0,
        Duration::from_millis(8),
    );
    let Ok(off) = started else {
        eprintln!("no GPU: skipped");
        return None;
    };
    assert!(wait_for(|_| off.presented().scroll.len() == 2), "both lists shown");
    Some(off)
}

/// Each list's offset as shown, outer first.
fn nested_offsets(off: &crate::window::Offscreen) -> Vec<f32> {
    off.presented().scroll.iter().map(|s| s.offset.1).collect()
}

/// Opus r11 I3: a notched wheel's gesture latched to the inner list stays
/// on it once it reaches its end (no chaining within a gesture), through a
/// device lost and made again: the compositor keeps its tables and its
/// latch, so input keeps going through it, and the outer list never
/// moves. A wheel the UI routed afresh meanwhile would have passed on to
/// the outer one.
#[test]
fn wheels_during_a_device_rebuild_keep_the_gestures_latch() {
    let Some(mut off) = nested_lists_on_threads() else {
        return;
    };
    off.input(crate::window::Input::PointerMoved { x: 50.0, y: 30.0 });
    // The inner list is at its end from the 47th on; the device is lost
    // past that, with wheels coming as it is made again.
    for k in 0..200 {
        if k == 100 {
            off.lose_device();
        }
        off.wheel((0.0, 3.0), false);
        std::thread::sleep(Duration::from_millis(1));
    }
    wait_for(|_| nested_offsets(&off) == [0.0, 140.0]);
    assert_eq!(nested_offsets(&off), [0.0, 140.0], "(outer, inner)");
}

/// The panic's path of the same gesture: the wheels the UI scrolls while
/// the render thread is made afresh follow the compositor's latch, handed
/// to the UI as it diverts, so they stay on the inner list too.
#[test]
fn wheels_around_a_render_panic_keep_the_gestures_latch() {
    let Some(mut off) = nested_lists_on_threads() else {
        return;
    };
    off.input(crate::window::Input::PointerMoved { x: 50.0, y: 30.0 });
    for k in 0..200 {
        if k == 20 {
            off.panic_render();
        }
        off.wheel((0.0, 3.0), false);
        std::thread::sleep(Duration::from_millis(1));
    }
    wait_for(|_| nested_offsets(&off) == [0.0, 140.0]);
    assert_eq!(nested_offsets(&off), [0.0, 140.0], "(outer, inner)");
}

/// Opus r12 P3: a container whose scrollbar fades with nothing else of it
/// moving (its scroll committed, nothing owed) stays in compose's visit
/// set until the fade ends: each frame shows the thumb fainter, then as
/// committed (hidden).
#[test]
fn a_fading_scrollbar_is_composed_each_frame_until_it_is_gone() {
    let mut s = Scenario::measuring(
        || {
            div()
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(5_000.0))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.hover(50.0, 50.0);
    assert!(s.wheel(30.0));
    // The UI takes the scroll and commits it: nothing is owed.
    s.frame("scrolled").expect("a frame");
    s.frame("committed").expect("a frame");
    let start = clock::current_seconds();
    let (_, headless) = s.parts();
    let c = &mut headless.compositor;
    let effect = c
        .committed()
        .compositing
        .scrollers
        .iter()
        .find_map(|sc| sc.thumb)
        .map(|t| t.effect)
        .expect("a scrollbar");
    let at = usize::try_from(effect).unwrap_or(usize::MAX);
    let shown = super::scroll::THUMB_SHOWN;
    let fade = super::scroll::THUMB_FADE;
    let mut opacities = Vec::new();
    for k in 0..=6_u8 {
        let t = start + shown + fade * f32::from(k) / 5.0;
        opacities.push(c.compose(t).properties.effects.get(at).map_or(f32::NAN, |e| e.opacity));
    }
    assert!(
        opacities.windows(2).all(|w| w[1] < w[0] || w[1] == 0.0),
        "fainter each frame: {opacities:?}"
    );
    assert!(opacities.first().is_some_and(|o| *o > 0.9), "{opacities:?}");
    assert_eq!(opacities.last().copied(), Some(0.0), "{opacities:?}");
    let rest = c.compose(start + shown + 2.0 * fade).properties.clone();
    assert_eq!(rest, c.committed().properties, "then as committed");
}

/// A trackpad's wheel event of `dy` (positive scrolls down) at `phase`.
fn pad_event(dy: f32, phase: WheelPhase) -> super::input_log::Event {
    super::input_log::Event::Wheel {
        delta: super::input_log::WheelDelta::Pixels(0.0, -dy),
        phase,
    }
}

/// Wait until the render thread serves again after a panic: the frames
/// it draws are numbered on from those before it, so a frame after `n`.
fn wait_restored(off: &crate::window::Offscreen, n: u64) {
    assert!(wait_for(|_| off.presented().frame > n), "a frame drawn after recovery");
    // And input goes through it (the full commit applied).
    assert!(wait_for(|_| off.through()), "input goes through the compositor again");
}

/// r12 matrix, the gesture latch through every switch of the route (ADR-0033:
/// one side owns it at a time, and it moves with the route). The nested
/// lists: the inner (60 tall, 140 to scroll) over the outer's top, the
/// outer (100 tall, 60 to scroll) alone under y 60.
///
/// Begun on the compositor, continued on the UI after a panic: the UI
/// takes the latch as input is diverted, and the rest stays on the inner
/// list though the pointer moved over the outer (round 11's direction).
#[test]
fn a_gesture_begun_on_the_compositor_keeps_its_list_on_the_ui() {
    let Some(mut off) = nested_lists_on_threads() else {
        return;
    };
    off.send(super::input_log::Event::Moved(50.0, 30.0));
    off.send(pad_event(30.0, WheelPhase::Began));
    assert!(
        wait_for(|_| nested_offsets(&off) == [0.0, 30.0]),
        "{:?}",
        nested_offsets(&off)
    );
    off.panic_render();
    off.send(super::input_log::Event::Moved(50.0, 90.0));
    off.send(pad_event(20.0, WheelPhase::Changed));
    wait_for(|_| nested_offsets(&off) == [0.0, 50.0]);
    assert_eq!(nested_offsets(&off), [0.0, 50.0], "(outer, inner)");
}

/// Codex r12 1: begun on the UI while diverted, continued on the
/// compositor once it serves again: restoring the route hands the UI's
/// latch to the compositor, so the rest stays on the inner list.
#[test]
fn a_gesture_begun_while_diverted_keeps_its_list_after_restore() {
    let Some(mut off) = nested_lists_on_threads() else {
        return;
    };
    off.send(super::input_log::Event::Moved(50.0, 30.0));
    let n = off.presented().frame;
    off.panic_render();
    off.send(pad_event(30.0, WheelPhase::Began));
    wait_restored(&off, n);
    assert!(
        wait_for(|_| nested_offsets(&off) == [0.0, 30.0]),
        "{:?}",
        nested_offsets(&off)
    );
    off.send(super::input_log::Event::Moved(50.0, 90.0));
    off.send(pad_event(20.0, WheelPhase::Changed));
    wait_for(|_| nested_offsets(&off) == [0.0, 50.0]);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(nested_offsets(&off), [0.0, 50.0], "(outer, inner)");
}

/// Opus r12 I1: a gesture latched on the compositor ends while diverted
/// and another begins there, over the outer list; once the compositor
/// serves again the new one goes on on the outer list. The compositor
/// handed its latch over as it diverted and kept none, so the old one's
/// cannot take the new gesture's steps.
#[test]
fn a_gesture_ended_and_begun_while_diverted_is_the_new_ones_after_restore() {
    let Some(mut off) = nested_lists_on_threads() else {
        return;
    };
    off.send(super::input_log::Event::Moved(50.0, 30.0));
    off.send(pad_event(10.0, WheelPhase::Began));
    assert!(
        wait_for(|_| nested_offsets(&off) == [0.0, 10.0]),
        "{:?}",
        nested_offsets(&off)
    );
    let n = off.presented().frame;
    off.panic_render();
    off.send(pad_event(0.0, WheelPhase::Ended));
    off.send(super::input_log::Event::Moved(50.0, 90.0));
    off.send(pad_event(10.0, WheelPhase::Began));
    wait_restored(&off, n);
    assert!(
        wait_for(|_| nested_offsets(&off) == [10.0, 10.0]),
        "{:?}",
        nested_offsets(&off)
    );
    off.send(pad_event(10.0, WheelPhase::Changed));
    wait_for(|_| nested_offsets(&off) == [20.0, 10.0]);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(nested_offsets(&off), [20.0, 10.0], "(outer, inner)");
}

/// The p99 of 100 pointer moves' hops through the render thread, 1 ms
/// apart, sent through `off`.
fn hop_p99(off: &mut crate::window::Offscreen) -> Duration {
    drop(off.take_forward_latencies());
    for k in 0..100_u16 {
        off.send(super::input_log::Event::Moved(f32::from(k % 90) + 1.0, 30.0));
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut hops = Vec::new();
    wait_for(|_| {
        hops.extend(off.take_forward_latencies().into_iter().map(|(_, d)| d));
        hops.len() >= 100
    });
    hops.sort();
    hops.get(98).copied().unwrap_or(Duration::MAX)
}

/// Codex r12 2: a device made again never holds input up. Making a
/// device blocks for hundreds of milliseconds (an adapter, every pipeline);
/// it is made on a thread of its own while the render thread goes on
/// handing input on, so the hop's p99 stays under a millisecond through a
/// rebuild as at rest (it was 221 ms).
#[test]
fn a_device_made_again_never_holds_input_up() {
    let Some(mut off) = nested_lists_on_threads() else {
        return;
    };
    let rest = hop_p99(&mut off);
    off.lose_device();
    let rebuilding = hop_p99(&mut off);
    eprintln!("hop p99 at rest {rest:?}, while the device is made again {rebuilding:?}");
    assert!(
        rebuilding < Duration::from_millis(1),
        "p99 at rest {rest:?}, rebuilding {rebuilding:?}"
    );
}

/// The render hop is timed from the platform's send, as ADR-0033 defines
/// it, not from the event's own time, which an OS's stamp or a script's
/// clock puts before the send: an event stamped 200 ms before it is sent
/// hops in far less than that (the fault matrix measured its sending
/// thread's late wakes, p99 7 ms under load, as the render thread's hop),
/// while the UI's take, timed from the event, counts all of it.
#[test]
fn an_inputs_hop_is_timed_from_its_send_not_its_stamp() {
    let Some(mut off) = nested_lists_on_threads() else {
        return;
    };
    drop(off.take_forward_latencies());
    drop(off.take_input_latencies());
    let early = std::time::Instant::now()
        .checked_sub(Duration::from_millis(200))
        .unwrap_or_else(std::time::Instant::now);
    off.send_at(super::input_log::Event::Moved(10.0, 30.0), early);
    let (mut hops, mut takes) = (Vec::new(), Vec::new());
    wait_for(|_| {
        hops.extend(off.take_forward_latencies().into_iter().map(|(_, d)| d));
        takes.extend(off.take_input_latencies().into_iter().map(|(_, d)| d));
        !hops.is_empty() && !takes.is_empty()
    });
    assert!(
        hops.first().is_some_and(|d| *d < Duration::from_millis(100)),
        "hop {hops:?}"
    );
    assert!(
        takes.first().is_some_and(|d| *d >= Duration::from_millis(200)),
        "take {takes:?}"
    );
}

/// Opus r12 P2: what the compositor accepted goes to the UI while no
/// device draws. A trackpad's step, held by its gesture's resampling for
/// the next frame's tick, and no frame comes while the device is lost:
/// a click then pulls it, so the click's handler reads the list at 30,
/// not where it was before the step.
#[test]
fn a_click_while_the_device_is_lost_comes_after_the_trackpads_step() {
    use std::sync::{Arc, Mutex};

    let read: Arc<Mutex<Vec<f32>>> = Arc::default();
    let into = Arc::clone(&read);
    let started = crate::app::offscreen(
        move || {
            let h = crate::scroll::ScrollHandle::new();
            move || {
                let into = Arc::clone(&into);
                div()
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .track_scroll(&h)
                    .on_click(move |_| {
                        into.lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(h.offset().1);
                    })
                    .child(div().height(5000.0))
                    .into_element()
            }
        },
        crate::app::RunOptions::default(),
        (100, 100),
        1.0,
        Duration::from_millis(40),
    );
    let Ok(mut off) = started else {
        eprintln!("no GPU: skipped");
        return;
    };
    assert!(wait_for(|_| !off.presented().scroll.is_empty()), "the list shown");
    off.send(super::input_log::Event::Moved(50.0, 50.0));
    off.lose_device_for(Duration::from_secs(2));
    off.send(pad_event(30.0, WheelPhase::Began));
    std::thread::sleep(Duration::from_millis(150));
    off.send(super::input_log::Event::Primary(true));
    off.send(super::input_log::Event::Primary(false));
    let clicked = || read.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    wait_for(|_| !clicked().is_empty());
    assert_eq!(clicked(), [30.0], "the click's handler read the list");
}

#[test]
fn a_tree_as_deep_as_gui_takes_runs_on_the_app_threads() {
    // Each level is a box around the next; the UI thread builds, lays out
    // and paints them recursively, on its own stack (`layout::STACK`).
    let depth = crate::layout::MAX_TREE_DEPTH;
    let started = crate::app::offscreen(
        move || {
            move || {
                let mut e = text("leaf").into_element();
                for _ in 0..depth {
                    e = div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .child(e)
                        .into_element();
                }
                div().size(pct(100.0)).child(e).into_element()
            }
        },
        crate::app::RunOptions::default(),
        (160, 120),
        1.0,
        Duration::from_millis(8),
    );
    let Ok(off) = started else {
        eprintln!("no GPU: skipped");
        return;
    };
    assert!(wait_for(|_| off.presented().commit > 0), "a tree {depth} deep is drawn");
}

#[test]
fn a_compositor_scroll_within_its_reach_never_passes_the_rows_built() {
    use crate::list::{ListState, virtual_list};
    // (frames the UI takes to answer, the wheel's speed in px/s): a UI that
    // answers at once under a fast fling, and one busy for six frames.
    for (late, speed) in [(0_u32, 12_000.0_f32), (6, 3_000.0)] {
        let state = ListState::new(24.0);
        let st = state;
        let mut s = Scenario::measuring(
            move || {
                div()
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            1_000_000,
                            |i| i,
                            |i| div().height(24.0).child(text(format!("row {i}"))).into_element(),
                        )
                        .height(300.0),
                    )
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("the first frame");
        s.hover(150.0, 150.0);
        s.frame("hovered").expect("a frame");
        let dt = 1.0 / 60.0;
        let step = speed * dt;
        let shown = |s: &mut Scenario| s.presented().scroll.iter().map(|x| x.offset.1).fold(0.0_f32, f32::max);
        let mut t = clock::current_seconds();
        {
            let (_, headless) = s.parts();
            headless.set_ui_late(Duration::from_secs_f32(dt).saturating_mul(late));
        }
        for cycle in 0..12 {
            // Compositor-only frames while the UI works on its frame.
            for frame in 0..=late {
                t += dt;
                clock::advance_to(Duration::from_secs_f32(t));
                let before = shown(&mut s);
                let (_, headless) = s.parts();
                assert!(headless.wheel((150.0, 150.0), (0.0, step), t), "the list scrolls");
                let moved = shown(&mut s) - before;
                // The first cycle has no speed to plan by: the fling had not begun.
                assert!(
                    cycle == 0 || (moved - step).abs() < 0.5,
                    "UI late {late} frames at {speed} px/s, cycle {cycle} frame {frame}: \
                     moved {moved} of {step}: the compositor reached the end of the rows built"
                );
            }
            s.frame("the UI's commit").expect("a frame");
        }
        assert!(state.anchor().row >= 90, "it went far: {:?}", state.anchor());
    }
}

#[test]
fn input_lands_where_the_compositor_showed_a_node_scaled_not_where_its_commit_scaled_it() {
    let big = Signal::new(false);
    let log: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let seen = Rc::clone(&log);
    let mut s = Scenario::measuring(
        move || {
            let seen = Rc::clone(&seen);
            div()
                .size(pct(100.0))
                .child(
                    div()
                        .width(100.0)
                        .height(40.0)
                        .background(Color::BLACK)
                        .on_drag(move |e| {
                            seen.borrow_mut().push(format!(
                                "{:?} {:.0},{:.0}",
                                e.phase,
                                e.x - e.bounds.x,
                                e.y - e.bounds.y
                            ));
                        })
                        .duration_ms(300.0)
                        .ease(Easing::Linear)
                        .transition(crate::motion::Properties::TRANSFORM)
                        .origin(crate::style::Origin::At([
                            crate::style::Calc::fraction(0.0),
                            crate::style::Calc::fraction(0.0),
                        ]))
                        .scale(if big.get() { 2.0 } else { 1.0 }),
                )
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.play_motion();
    s.act("grow", || big.set(true));
    s.frame("starts").expect("a frame");
    let start = clock::current_seconds();
    // The compositor runs the scale to its end; the UI's frame, laid out
    // as it began, has it at 1.
    let (_, headless) = s.parts();
    headless.draw(start + 1.0);
    let shown = headless.presented();
    assert!(
        shown.turned.iter().any(|(_, t)| (t.scale - 2.0).abs() < 1e-3),
        "the frame shown records the scale it showed: {shown:?}"
    );
    // A press 30 px in shows 15 px into the box, scaled 2x about its corner.
    s.send(crate::window::Input::PointerMoved { x: 30.0, y: 10.0 });
    s.send(crate::window::Input::MouseDown { x: 30.0, y: 10.0 });
    // Past the box as committed (100 wide), on it as shown (200 wide).
    s.send(crate::window::Input::PointerMoved { x: 150.0, y: 10.0 });
    s.send(crate::window::Input::MouseUp { x: 150.0, y: 10.0 });
    assert_eq!(*log.borrow(), ["Start 15,5", "Move 75,5", "End 75,5"]);
}

/// A wheel handler filling the window, under a 200 px scroll container of
/// 40 rows scaled by `scale` about its corner.
fn scaled_scroller(scale: f32) -> impl IntoElement {
    div().size(pct(100.0)).on_wheel(|_| true).child(
        div()
            .position(Position::Absolute)
            .top(0.0)
            .left(0.0)
            .width(200.0)
            .height(200.0)
            .overflow_y(Overflow::Scroll)
            .overflow_x(Overflow::Hidden)
            .scale(scale)
            .origin(crate::style::Origin::At([
                crate::style::Calc::fraction(0.0),
                crate::style::Calc::fraction(0.0),
            ]))
            .children((0..40).map(|i| div().height(24.0).child(text(format!("row {i}"))))),
    )
}

#[test]
fn a_wheel_over_a_scroller_scaled_away_goes_to_what_is_under_it() {
    for (scale, to_ui) in [(1.0, false), (0.0, true)] {
        let mut s =
            Scenario::measuring(move || scaled_scroller(scale).into_element(), 300, 300, 1.0).expect("a scenario");
        s.frame("first").expect("a frame");
        let (_, headless) = s.parts();
        let seen = headless.shown().seen();
        let wheeled = headless.compositor.wheel_input(super::WheelInput {
            point: (100.0, 100.0),
            delta: (0.0, 40.0),
            time: 0,
            precise: false,
            phase: WheelPhase::None,
            seen,
        });
        assert_eq!(
            matches!(wheeled, super::Wheeled::Ui(..)),
            to_ui,
            "scaled {scale}: a scroller scaled to nothing takes no wheel"
        );
    }
}

/// A wheel handler's box and a list scrolling down, side by side, each
/// 100 px square: the handler on the left, or, `swapped`, the list.
fn handler_and_list(swapped: bool) -> crate::element::Element {
    let (handler, list) = if swapped { (200.0, 0.0) } else { (0.0, 200.0) };
    div()
        .size(pct(100.0))
        .child(
            div()
                .position(Position::Absolute)
                .left(handler)
                .top(0.0)
                .width(100.0)
                .height(100.0)
                .on_wheel(|_| true),
        )
        .child(
            div()
                .position(Position::Absolute)
                .left(list)
                .top(0.0)
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(400.0)),
        )
        .into_element()
}

#[test]
fn the_platforms_wheel_is_routed_in_the_frame_shown_as_it_came() {
    use std::sync::{Arc, Mutex, PoisonError};

    use super::thread::{FromRender, Pace, RenderThread, ToRender};
    let swapped = Signal::new(false);
    let mut s = Scenario::measuring(move || handler_and_list(swapped.get()), 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    // On screen as the wheel came: the handler on the left.
    let seen = s.parts().1.compositor.presented();
    // A commit swaps them, and is shown, before the render thread takes
    // the wheel in.
    s.act("swap", || swapped.set(true));
    s.frame("swapped").expect("a frame");
    let now_shown = s.parts().1.compositor.presented();
    assert!(now_shown.frame > seen.frame);
    let compositor = std::mem::replace(&mut s.parts().1.compositor, Compositor::new(Glyphs::for_os()));
    let Ok(gpu) = crate::gpu::GpuRenderer::headless() else {
        eprintln!("no GPU: skipped");
        return;
    };
    // Each wheel handed to the UI: where, and the frame it came under.
    type Handed = Arc<Mutex<Vec<(f32, f32, u64)>>>;
    let handed: Handed = Arc::default();
    let to_ui = Arc::clone(&handed);
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut thread = RenderThread::new(
        compositor,
        gpu,
        rx,
        Box::new(move |m| {
            if let FromRender::Wheel {
                input: crate::window::Input::WheelAt { step, .. },
                shown,
                ..
            } = m
            {
                to_ui.lock().unwrap_or_else(PoisonError::into_inner).push((
                    step.point.0,
                    step.point.1,
                    numbered(shown),
                ));
            }
        }),
        Duration::from_millis(1),
        Pace::Ticker(Duration::from_millis(1)),
        None,
    );
    let at = Instant::now();
    let wheel = |at: Instant, shown: &Arc<Presented>| ToRender::Wheel {
        point: (50.0, 50.0),
        delta: (0.0, 40.0),
        at,
        sent: at,
        precise: false,
        phase: WheelPhase::None,
        shown: Seen::Frame(shown.frame),
        event: None,
    };
    // As the platform stamped it: over the handler, in the frame it saw.
    thread.handle(wheel(at, &seen));
    let took = |h: &Handed| h.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(
        took(&handed),
        [(50.0, 50.0, seen.frame)],
        "handed to the UI, under its frame"
    );
    // A wheel that came over what shows now, a gesture later: the list
    // there scrolls, and the UI is handed nothing.
    thread.handle(wheel(at + Duration::from_secs(1), &now_shown));
    assert_eq!(took(&handed).len(), 1, "the compositor scrolled the list");
}

#[test]
fn a_wheel_late_by_two_frames_is_routed_in_its_own_and_names_the_node_it_was_over() {
    let swapped = Signal::new(false);
    let mut s = Scenario::measuring(move || handler_and_list(swapped.get()), 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let seen = s.shown().seen();
    let Seen::Frame(first) = seen else {
        panic!("a frame presented")
    };
    // Two frames composed since the one it came under, the list on the
    // left in both.
    s.act("swap", || swapped.set(true));
    s.frame("swapped").expect("a frame");
    s.frame("again").expect("a frame");
    let c = &mut s.parts().1.compositor;
    assert!(c.presented().frame >= first + 2, "two frames later");
    let wheeled = c.wheel_input(WheelInput {
        point: (50.0, 50.0),
        delta: (0.0, 40.0),
        time: 0,
        precise: false,
        phase: WheelPhase::None,
        seen,
    });
    let Wheeled::Ui(Some(target), _) = wheeled else {
        panic!("over the handler as it was shown: {wheeled:?}");
    };
    assert_ne!(target.owner, None, "it names the node by its instance too");
}

/// Two lists side by side, 100 px square, each scrolling down 300 px.
fn two_lists() -> crate::element::Element {
    let list = |left: f32| {
        div()
            .position(Position::Absolute)
            .left(left)
            .top(0.0)
            .width(100.0)
            .height(100.0)
            .overflow_y(Overflow::Scroll)
            .child(div().height(400.0))
    };
    div()
        .size(pct(100.0))
        .child(list(0.0))
        .child(list(200.0))
        .into_element()
}

/// A wheel is routed in the frame it came under while that showed within
/// [`RETAIN`] of the newest frame's vblank, all of it as that frame showed
/// (its property trees, effects included, caught up through a frame
/// composed whole among them, and its tables). One no longer kept routes
/// nowhere: no other frame stands in for it.
#[test]
fn a_wheel_is_routed_in_any_frame_kept_as_it_showed_and_one_no_longer_kept_nowhere() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let mut shown: Vec<(u64, u64, crate::scene::Properties)> = Vec::new();
    // Twelve frames 62.5 ms apart: the newest's quarter second keeps the
    // last five, and the one showing up to its start.
    for n in 0..12_u8 {
        let at = 8.0 + f32::from(n) / 16.0;
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 10.0),
            time: crate::clock::nanos_of(at),
            precise: false,
            phase: WheelPhase::None,
            seen,
        });
        // Two frames before the last, one composed whole.
        if n == 9 {
            c.pending.all = true;
        }
        c.compose(at);
        c.did_present();
        let p = c.presented();
        shown.push((p.frame, p.present_at, c.shown.properties.clone()));
    }
    let (last, start) = shown.last().map_or((0, 0), |s| (s.0, s.1 - RETAIN));
    for w in shown.windows(2) {
        assert_ne!(w[0].2, w[1].2, "each frame scrolled on");
    }
    let mut kept_any = 0;
    for (i, (frame, _, props)) in shown.iter().enumerate() {
        // Kept: the newest, or one shown up to the start or after.
        let kept = shown.get(i + 1).is_none_or(|next| next.1 >= start);
        kept_any += usize::from(kept);
        match c.tables(Seen::Frame(*frame)) {
            Some(p) => {
                assert!(
                    kept,
                    "frame {frame}, {} before the last, is no longer kept",
                    last - frame
                );
                assert_eq!(&p.properties, props, "frame {frame}, {} before the last", last - frame);
            }
            None => assert!(!kept, "frame {frame}, {} before the last, is kept", last - frame),
        }
    }
    assert_eq!(
        kept_any, 6,
        "the last five and the one shown up to the quarter second's start"
    );
}

/// The UI stalls, holding the frame it last routed in, while the render
/// thread presents a hundred frames at 120 Hz, a list scrolling in each:
/// the frames alive are at most the bound ADR-0015 gives, the timeline's
/// quarter second (`ceil(RETAIN / interval) + 1`), the pool, the frame
/// composed last and the UI's two, whatever the stall's length. The
/// message that frames entered holds none.
#[test]
fn a_stalled_ui_keeps_a_bounded_number_of_frames_alive() {
    let alive = Arc::new(());
    // Two lists beside a thousand boxes that clip: trees far larger than
    // what a scrolled frame writes.
    let page = || {
        let list = |left: f32| {
            div()
                .position(Position::Absolute)
                .left(left)
                .top(0.0)
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .children((0..40).map(|_| div().height(10.0).on_click(|_| {})))
        };
        let clips = div()
            .position(Position::Absolute)
            .left(320.0)
            .children((0..1_000).map(|_| {
                div()
                    .width(1.0)
                    .height(1.0)
                    .overflow(Overflow::Hidden)
                    .child(div().width(8.0).height(1.0))
            }));
        div()
            .size(pct(100.0))
            .child(list(0.0))
            .child(list(200.0))
            .child(clips)
            .into_element()
    };
    let mut s = Scenario::measuring(page, 400, 100, 1.0).expect("a scenario");
    s.parts().1.compositor.alive = Some(Arc::clone(&alive));
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    // The pointer rests over the left list's rows: frames scrolling them
    // change what is under it, so the render thread would tell the UI.
    c.timeline().point(Some((50.0, 50.0)));
    // The stalled UI holds two frames, different ones: the one hover was
    // last worked out in (the first) and the one its last input was
    // routed in (taken halfway); and its queue, which the render thread
    // fills as frames enter and the UI never reads.
    let mut held = (c.presented(), None);
    let mut queue: Vec<super::thread::FromRender> = Vec::new();
    let mut tells = 0_usize;
    let interval = 1_000_000_000_u64 / 120;
    let bound = usize::try_from(RETAIN.div_ceil(interval)).unwrap_or(usize::MAX) + 1 + super::compose::POOL + 1 + 2;
    let start = clock::current_nanos() + 1_000_000_000;
    let mut most = 0_usize;
    let made_before = c.props_copied();
    for n in 0..100_u64 {
        if n == 50 {
            held.1 = Some(c.presented());
        }
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 1.0),
            time: start + n * interval,
            precise: false,
            phase: WheelPhase::None,
            seen,
        });
        c.compose(start + n * interval);
        // As the render thread tells the UI ([`super::thread`]): once
        // until it looks, and it never looks.
        if c.did_present() {
            tells += 1;
            if c.timeline().tell() {
                queue.push(super::thread::FromRender::Presented);
            }
        }
        // The token's own count aside, and the compositor's.
        most = most.max(Arc::strong_count(&alive).saturating_sub(2));
    }
    assert!(
        held.1.as_ref().is_some_and(|h| h.frame != held.0.frame),
        "the UI holds two frames"
    );
    assert!(tells > 1, "frames changed what is under the pointer: {tells}");
    assert_eq!(queue.len(), 1, "one message waits, whatever the stall");
    assert!(most <= bound, "{most} frames alive at most, the bound {bound}");
    // Frames were used again, not made anew: a fresh one copies the trees
    // whole, a reused one only what changed.
    let p = c.presented();
    let entries = p.properties.transforms.len() + p.properties.clips.len() + p.properties.effects.len();
    let copied = c.props_copied() - made_before;
    assert!(
        copied < 100 * entries / 2,
        "frames reused: {copied} entries copied over 100 frames of {entries}"
    );
    drop(held);
}

/// Opus r7 nit 6: before any frame is presented, the UI's snapshot and
/// the render thread's are of the same commit, by the same number, and
/// alike (tables and property trees), one constructor making both.
#[test]
fn before_the_first_present_both_threads_route_in_one_commits_snapshot() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("laid").expect("a frame");
    let scene = s.parts().1.compositor.scene.clone();
    let mut c = Compositor::new(Glyphs::for_os());
    let mut commit = Commit::default();
    commit.between(&Scene::default(), &scene);
    commit.sequence = 1;
    commit.full = true;
    c.apply(&mut commit);
    let render = c.tables(Seen::BeforeFirst).expect("the commit's snapshot");
    let ui = s.pipeline_mut().on_screen(&Shown::BeforeFirst);
    assert_eq!((render.commit, ui.commit), (1, 1), "one commit, one number");
    assert_eq!(render.hits, ui.hits, "the same boxes");
    assert_eq!(render.scrollers, ui.scrollers, "the same containers");
    assert_eq!(render.properties, ui.properties, "the same trees");
}

/// A list of rows that take the pointer, 100 wide, scrolling; beside it a
/// still box that takes it too.
fn hover_rows() -> crate::element::Element {
    div()
        .size(pct(100.0))
        .child(
            div()
                .position(Position::Absolute)
                .left(0.0)
                .top(0.0)
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .children((1..=40_u64).map(|i| div().key(i).height(10.0).on_hover(|_| {}))),
        )
        .child(
            div()
                .position(Position::Absolute)
                .left(200.0)
                .top(0.0)
                .width(100.0)
                .height(100.0)
                .on_hover(|_| {}),
        )
        .into_element()
}

/// Opus r7 I2: the compositor scrolls the list a frame at a time, 120
/// frames at 120 Hz. With no pointer in the window the UI is woken for
/// none of them; with the pointer at rest on the still box, for none; on
/// the rows, for each frame that brings another row under it (and only
/// those): the energy cost of hover with no input is what the pointer
/// sees change.
#[test]
fn a_compositor_scroll_wakes_the_ui_only_when_what_is_under_the_pointer_changes() {
    let wakes = |pointer: Option<(f32, f32)>| {
        let mut s = Scenario::measuring(hover_rows, 400, 100, 1.0).expect("a scenario");
        s.frame("first").expect("a frame");
        let c = s.compositor();
        c.timeline().point(pointer);
        let interval = 1_000_000_000_u64 / 120;
        let start = clock::current_nanos() + 1_000_000_000;
        let mut woke = 0_usize;
        for n in 0..120_u64 {
            let at = start + n * interval;
            let seen = c.present_last();
            c.wheel_input(WheelInput {
                point: (50.0, 50.0),
                delta: (0.0, 1.0),
                time: at,
                precise: false,
                phase: WheelPhase::None,
                seen,
            });
            c.compose(at);
            woke += usize::from(c.did_present());
        }
        woke
    };
    assert_eq!(wakes(None), 0, "no pointer in the window: no wake");
    assert_eq!(wakes(Some((250.0, 50.0))), 0, "at rest over what stays: no wake");
    let rows = wakes(Some((50.0, 50.0)));
    // 120 px scrolled under it: twelve rows of 10 px pass.
    assert!(
        (11..=13).contains(&rows),
        "one wake a row brought under it, not one a frame: {rows}"
    );
}

/// Codex r7 1: what shows now and when hover must be looked at next come
/// from one look at the timeline. B enters for a vblank just after `t`:
/// asked at `t`, A shows and B's vblank is the next; no frame can enter
/// between two separate reads and be skipped.
#[test]
fn what_shows_and_the_next_vblank_to_look_at_come_from_one_sample() {
    let on_screen = OnScreen::new();
    let mut let_go = std::collections::VecDeque::new();
    on_screen.point(Some((10.0, 10.0)));
    let rect = crate::geometry::Bounds {
        x: 0.0,
        y: 0.0,
        width: 32.0,
        height: 32.0,
    };
    on_screen.presented(Arc::new(Presented::boxed(1, 1_000, None)), &mut let_go);
    let b = on_screen.presented(Arc::new(Presented::boxed(2, 2_000, Some((7, rect)))), &mut let_go);
    assert!(b, "B puts a box under the pointer");
    let (shown, next) = on_screen.sample(1_999);
    assert_eq!(stamp(&shown), 1, "A shows");
    assert_eq!(next, Some(2_000), "B's vblank is when to look again");
    let (shown, next) = on_screen.sample(2_000);
    assert_eq!((stamp(&shown), next), (2, None), "B shows; nothing more to wait for");
}

/// A frame with a new hit table of 20,000 boxes enters under a pointer:
/// the render thread makes both frames' hit indexes to see whether to
/// wake the UI, and does so outside the timeline's lock, so the platform
/// stamping input meanwhile never waits on it. Measured: the longest a
/// stamp waits against the time the entry takes.
#[test]
fn a_frame_entering_hits_its_boxes_outside_the_timelines_lock() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
    let table = |frame: u64, at: u64, shift: f32| {
        let mut p = Presented::boxed(frame, at, Some((1, crate::geometry::Bounds::default())));
        let one = p.hits[0];
        p.hits = (0..20_000_u32)
            .map(|i| Hit {
                rect: crate::geometry::Bounds {
                    x: f32::from(u16::try_from(i % 200).unwrap_or(0)) * 10.0 + shift,
                    y: f32::from(u16::try_from(i / 200).unwrap_or(0)) * 10.0,
                    width: 8.0,
                    height: 8.0,
                },
                target: HitTarget {
                    key: NodeKey(u64::from(i)),
                    owner: None,
                },
                ..one
            })
            .collect();
        Arc::new(p)
    };
    let on_screen = OnScreen::new();
    let mut let_go = std::collections::VecDeque::new();
    on_screen.point(Some((4.0, 4.0)));
    on_screen.presented(table(1, 1_000, 0.0), &mut let_go);
    let b = table(2, 2_000, 5.0);
    let done = Arc::new(AtomicBool::new(false));
    let longest = Arc::new(AtomicU64::new(0));
    let stamper = {
        let (on_screen, done, longest) = (Arc::clone(&on_screen), Arc::clone(&done), Arc::clone(&longest));
        std::thread::spawn(move || {
            while !done.load(SeqCst) {
                let start = std::time::Instant::now();
                std::hint::black_box(on_screen.seen_at(1_500));
                let took = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
                longest.fetch_max(took, SeqCst);
            }
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(5));
    let start = std::time::Instant::now();
    let woke = on_screen.presented(b, &mut let_go);
    let entry = start.elapsed();
    done.store(true, SeqCst);
    stamper.join().expect("the stamping thread");
    let longest = std::time::Duration::from_nanos(longest.load(SeqCst));
    eprintln!("entry with two 20k indexes made: {entry:?}; longest stamp: {longest:?}");
    assert!(woke, "B moves the boxes off the pointer");
    assert!(
        longest * 2 < entry,
        "a stamp waited {longest:?} of an entry of {entry:?}"
    );
}

/// Opus r9 P1: a frame that brings another span of the same paragraph
/// under a resting pointer (its text scrolled within the box) changes
/// what decides the cursor and hover there, though the box under it is
/// the same: the UI is woken, and its vblank is when to look again.
#[test]
fn another_span_of_one_paragraph_under_a_resting_pointer_wakes_the_ui() {
    let on_screen = OnScreen::new();
    let mut let_go = std::collections::VecDeque::new();
    on_screen.point(Some((50.0, 10.0)));
    let rect = |x: f32| crate::geometry::Bounds {
        x,
        y: 0.0,
        width: 100.0,
        height: 20.0,
    };
    let paragraph = |frame: u64, at: u64, first: f32, second: f32| {
        let mut p = Presented::boxed(frame, at, Some((7, rect(0.0).union(&rect(100.0)))));
        p.texts = Arc::from(vec![(
            0,
            Arc::new(ShownText::Spans(Arc::from([(0, first), (1, second)].map(|(k, x)| {
                ShownSpan {
                    span: SpanId::At(k),
                    cursor: None,
                    rect: rect(x),
                }
            })))),
        )]);
        Arc::new(p)
    };
    on_screen.presented(paragraph(1, 1_000, 0.0, 100.0), &mut let_go);
    let b = on_screen.presented(paragraph(2, 2_000, 100.0, 0.0), &mut let_go);
    assert!(b, "the second span is under the pointer in B");
    assert_eq!(on_screen.sample(1_500).1, Some(2_000));
}

/// Review r10, Codex 2: a frame that changes only the cursor its box
/// shows under a resting pointer changes what the cursor is decided by
/// there: the UI is woken, and its vblank is when to look again.
#[test]
fn a_cursor_changed_alone_under_a_resting_pointer_wakes_the_ui() {
    let on_screen = OnScreen::new();
    let mut let_go = std::collections::VecDeque::new();
    on_screen.point(Some((10.0, 10.0)));
    let rect = crate::geometry::Bounds {
        x: 0.0,
        y: 0.0,
        width: 32.0,
        height: 32.0,
    };
    let with = |frame: u64, at: u64, cursor: crate::style::Cursor| {
        let mut p = Presented::boxed(frame, at, Some((7, rect)));
        let mut hits = p.hits.to_vec();
        for h in &mut hits {
            h.cursor = Some(cursor);
        }
        p.hits = hits.into();
        Arc::new(p)
    };
    on_screen.presented(with(1, 1_000, crate::style::Cursor::Grab), &mut let_go);
    let b = on_screen.presented(with(2, 2_000, crate::style::Cursor::Text), &mut let_go);
    assert!(b, "B shows another cursor under the pointer");
    assert_eq!(on_screen.sample(1_500).1, Some(2_000));
}

/// Frame B enters while the pointer is over the background in both A and
/// B (nothing to wake for); the pointer then moves, before B's vblank, to
/// where B puts a box. When the UI handles that move and samples, B's
/// vblank is when to look again: what was worked out as B entered is not
/// kept, the pointer and the frames waiting are weighed as they are now.
/// And moved off again, nothing is waited for.
#[test]
fn a_frame_entered_before_the_pointer_moved_onto_its_box_is_rechecked_at_its_vblank() {
    let on_screen = OnScreen::new();
    let mut let_go = std::collections::VecDeque::new();
    on_screen.point(Some((60.0, 40.0)));
    let rect = crate::geometry::Bounds {
        x: 150.0,
        y: 10.0,
        width: 70.0,
        height: 60.0,
    };
    on_screen.presented(Arc::new(Presented::boxed(1, 1_000, None)), &mut let_go);
    let b = on_screen.presented(Arc::new(Presented::boxed(2, 2_000, Some((7, rect)))), &mut let_go);
    assert!(!b, "B changes nothing where the pointer was as it entered");
    assert_eq!(on_screen.sample(1_500).1, None);
    on_screen.point(Some((180.0, 40.0)));
    let (shown, next) = on_screen.sample(1_500);
    assert_eq!(
        (stamp(&shown), next),
        (1, Some(2_000)),
        "B's box is under the pointer from its vblank"
    );
    on_screen.point(Some((60.0, 40.0)));
    assert_eq!(on_screen.sample(1_500).1, None, "moved off again");
    on_screen.point(None);
    assert_eq!(on_screen.sample(1_500).1, None, "no pointer, no wake");
}

/// The platform thread stalls 50 ms (twelve frames at 240 Hz) between a
/// wheel arriving and the render thread taking it: it is still routed in
/// the frame it came under, retention being a time, not a count of frames.
#[test]
fn a_wheel_after_a_50_ms_stall_at_240_hz_still_routes() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let interval = 1_000_000_000_u64 / 240;
    let start = clock::current_nanos() + 1_000_000_000;
    let mut came = None;
    for n in 0..48_u64 {
        let at = start + n * interval;
        c.compose(at);
        c.did_present();
        if n == 35 {
            came = Some((Seen::Frame(c.presented().frame), at));
        }
    }
    let (seen, at) = came.expect("stamped");
    let wheeled = c.wheel_input(WheelInput {
        point: (50.0, 50.0),
        delta: (0.0, 40.0),
        time: at,
        precise: false,
        phase: WheelPhase::None,
        seen,
    });
    assert!(matches!(wheeled, Wheeled::Scrolled), "routed in its frame: {wheeled:?}");
}

/// An ordinary list (no wheel handler) scrolls on the first wheel, made
/// before any frame was presented: the render thread routes it in the
/// commit it applied, as the UI routes any input then, one snapshot of
/// that commit (its scroll containers too) for both.
#[test]
fn a_list_scrolls_on_a_wheel_made_before_the_first_present() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("laid").expect("a frame");
    let scene = s.parts().1.compositor.scene.clone();
    let mut c = Compositor::new(Glyphs::for_os());
    let mut commit = Commit::default();
    commit.between(&Scene::default(), &scene);
    commit.sequence = 1;
    commit.full = true;
    c.apply(&mut commit);
    assert_eq!(c.timeline().latest(), None, "nothing presented");
    let wheeled = c.wheel_input(WheelInput {
        point: (50.0, 50.0),
        delta: (0.0, 40.0),
        time: clock::current_nanos(),
        precise: false,
        phase: WheelPhase::None,
        seen: Seen::BeforeFirst,
    });
    assert!(matches!(wheeled, Wheeled::Scrolled), "the list scrolls: {wheeled:?}");
}

/// Frame B presented for a vblank still to come is not yet what the display
/// shows: input stamped before that vblank gets A (its pixels, its clips),
/// and from it, B. What showed at a time is a pure question: asked about
/// A's time after B's, it is still A (a wheel stamped before B's vblank
/// and handled after it lands in A); before the first present, nothing.
#[test]
fn a_frame_presented_shows_from_its_vblank_not_from_its_present() {
    let frame = |frame: u64, present_at: u64| {
        Arc::new(Presented {
            frame,
            present_at,
            ..Presented::default()
        })
    };
    let on_screen = OnScreen::new();
    let mut let_go = std::collections::VecDeque::new();
    let mut present = |p| on_screen.presented(p, &mut let_go);
    assert!(matches!(on_screen.at(u64::MAX), Shown::BeforeFirst), "nothing yet");
    present(frame(1, 1_000));
    assert_eq!(stamp(&on_screen.at(1_000)), 1, "A from its vblank");
    assert!(
        matches!(on_screen.at(999), Shown::BeforeFirst),
        "before A's vblank, nothing"
    );
    present(frame(2, 17_000));
    assert_eq!(stamp(&on_screen.at(16_999)), 1, "B queued: A still shows");
    assert_eq!(stamp(&on_screen.at(17_000)), 2, "B from its vblank");
    assert_eq!(stamp(&on_screen.at(1_000)), 1, "A's time after B's: still A");
    assert_eq!(stamp(&on_screen.at(16_999)), 1, "and again");
    // A quarter second on, a time older than every frame kept finds the
    // frame gone, never another.
    for n in 3..=8_u64 {
        present(frame(n, RETAIN + n * 16_000));
    }
    assert!(
        matches!(on_screen.at(1_000), Shown::Gone(_)),
        "A is no longer kept: gone, not B"
    );
    assert_eq!(stamp(&on_screen.at(u64::MAX)), 8);
}

/// Frame A reached the screen; five frames after it are composed and fail
/// to present (their textures could not be taken). Input stamped under A
/// is still routed in A, all of it: failed presents never push out the
/// frame on screen.
#[test]
fn failed_presents_never_push_out_the_frame_on_screen() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("A").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let a = c.presented();
    assert_eq!(c.on_screen().map(|p| p.frame), Some(a.frame), "A is on screen");
    for n in 1..=5_u8 {
        let at = 1.0 + f32::from(n) * 0.01;
        c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 10.0),
            time: crate::clock::nanos_of(at),
            precise: false,
            phase: WheelPhase::None,
            seen: Seen::Frame(a.frame),
        });
        c.compose(at);
    }
    assert!(c.presented().frame >= a.frame + 5, "five composed since");
    assert_eq!(c.on_screen().map(|p| p.frame), Some(a.frame), "A is still on screen");
    let kept = c
        .tables(Seen::Frame(a.frame))
        .expect("A is kept for input stamped with it");
    assert!(Arc::ptr_eq(&a, &c.on_screen().expect("on screen")), "A's own snapshot");
    assert_eq!(kept.properties, a.properties, "as A showed");
}

/// A trackpad flick over list A lifting, and a gesture beginning 8 ms
/// later where nothing is hit: no hit is no same target, so it is no
/// momentum of A's, and A does not move.
#[test]
fn a_gesture_beginning_over_nothing_soon_after_a_lift_is_no_momentum() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    c.set_flings(false);
    let seen = c.present_last();
    let event = |c: &mut Compositor, x: f32, at: f32, dy: f32, began: bool, ended: bool| {
        c.wheel_input(WheelInput {
            point: (x, 50.0),
            delta: (0.0, dy),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(began, ended),
            seen,
        });
        c.tick(at + 0.05);
    };
    event(c, 50.0, 0.0, 40.0, true, false);
    event(c, 50.0, 0.016, 0.0, false, true);
    let a = downs(c)[0];
    // Past the window's edge: the hit table holds nothing there.
    event(c, 450.0, 0.024, 40.0, true, false);
    event(c, 450.0, 0.040, 40.0, false, false);
    assert_eq!(downs(c), [a, 0.0], "nothing hit: A stays");
}

/// A trackpad gesture over a list goes on after a modal layer opens
/// elsewhere: the compositor, as the UI, no longer scrolls the list below
/// the modal, though the gesture latched to it.
#[test]
fn a_compositor_gesture_latched_to_a_list_a_modal_opened_over_scrolls_it_no_more() {
    let modal = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            let list = div()
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(400.0));
            let root = div().size(pct(100.0)).child(list);
            if modal.get() {
                root.child(crate::element::deferred(
                    crate::element::Positioning::At { x: 200.0, y: 0.0 },
                    div()
                        .layer(crate::layer::Layer::new().modal(true))
                        .width(100.0)
                        .height(100.0),
                ))
            } else {
                root
            }
            .into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let event = |c: &mut Compositor, at: f32, began: bool| {
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 30.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(began, false),
            seen,
        });
        c.tick(at + 0.05);
    };
    let c = &mut s.parts().1.compositor;
    c.set_flings(false);
    event(c, 0.0, true);
    let scrolled = downs(c);
    assert!(scrolled[0] > 0.0, "the list took the gesture: {scrolled:?}");
    s.act("open", || modal.set(true));
    s.frame("modal").expect("a frame");
    let c = &mut s.parts().1.compositor;
    event(c, 0.016, false);
    assert_eq!(downs(c)[0], scrolled[0], "below the modal: no further");
}

/// A list 100 high over 400 at the left, and, once `modal` is set, a modal
/// layer opened beside it.
fn list_and_modal(modal: Signal<bool>) -> impl Fn() -> crate::element::Element + 'static {
    move || {
        let list = div()
            .width(100.0)
            .height(100.0)
            .overflow_y(Overflow::Scroll)
            .child(div().height(400.0));
        let root = div().size(pct(100.0)).child(list);
        if modal.get() {
            root.child(crate::element::deferred(
                crate::element::Positioning::At { x: 200.0, y: 0.0 },
                div()
                    .layer(crate::layer::Layer::new().modal(true))
                    .width(100.0)
                    .height(100.0),
            ))
        } else {
            root
        }
        .into_element()
    }
}

#[test]
fn a_fling_in_progress_when_a_modal_opens_stops() {
    let modal = Signal::new(false);
    let mut s = Scenario::measuring(list_and_modal(modal), 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    c.set_flings(true);
    let seen = c.present_last();
    for (k, at) in [0.0_f32, 0.016, 0.032, 0.048].into_iter().enumerate() {
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 4.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(k == 0, k == 3),
            seen,
        });
        c.tick(at + 0.008);
    }
    c.tick(0.1);
    let flung = downs(c)[0];
    c.tick(0.15);
    assert!(downs(c)[0] > flung, "the fling moves on: {flung} then {:?}", downs(c));
    s.act("open", || modal.set(true));
    s.frame("modal").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let opened = downs(c)[0];
    c.tick(0.3);
    c.tick(0.5);
    assert_eq!(downs(c)[0], opened, "below the modal its fling stops");
}

#[test]
fn a_wheel_stamped_with_a_frame_from_before_a_modal_moves_the_list_no_more() {
    let modal = Signal::new(false);
    let mut s = Scenario::measuring(list_and_modal(modal), 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let before = s.shown().seen();
    s.act("open", || modal.set(true));
    s.frame("modal").expect("a frame");
    for precise in [false, true] {
        let c = &mut s.parts().1.compositor;
        let was = downs(c)[0];
        let wheeled = c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 30.0),
            time: crate::clock::nanos_of(1.0),
            precise,
            phase: phase_of(true, false),
            seen: before,
        });
        c.tick(1.05);
        assert_eq!(downs(c)[0], was, "the list is below the modal now (precise: {precise})");
        assert!(!matches!(wheeled, Wheeled::Scrolled), "nothing scrolled: {wheeled:?}");
    }
}

#[test]
fn a_gesture_end_no_container_takes_flings_nothing() {
    let mut s = Scrolls::default();
    s.set_flings(true);
    let mut covered = node(2, 0.0, 500.0);
    covered.below_modal = true;
    let nodes = vec![node(1, 0.0, 500.0), covered];
    for (k, at) in [0.0_f32, 0.016, 0.032, 0.048].into_iter().enumerate() {
        assert!(
            s.gesture(&nodes, Some(WheelTarget(0)), y(4.0), (at, 0), false),
            "event {k}"
        );
        s.tick(&table(&nodes), at + 0.008);
    }
    // The end comes routed to the list a modal layer covers: refused.
    assert!(!s.gesture(&nodes, Some(WheelTarget(1)), (0, 0), (0.06, 0), true));
    s.tick(&table(&nodes), 0.2);
    let rested = s.offset(&nodes[0], 0);
    s.tick(&table(&nodes), 0.4);
    assert_eq!(s.offset(&nodes[0], 0), rested, "the first list's samples fling nothing");
    assert_eq!(s.offset(&nodes[1], 0), (0.0, 0.0));
}

/// A modal layer removes a list mid-fling and, closed, brings back the
/// same key before any frame ticks: the fling was the list's that left,
/// and ended with it.
#[test]
fn a_fling_on_a_list_a_modal_removed_stops_though_its_key_comes_back() {
    let mut s = Scrolls::default();
    s.set_flings(true);
    let nodes = vec![node(1, 0.0, 500.0)];
    for (k, at) in [0.0_f32, 0.016, 0.032, 0.048].into_iter().enumerate() {
        assert!(s.gesture(&nodes, Some(WheelTarget(0)), y(4.0), (at, 0), k == 3));
        s.tick(&table(&nodes), at + 0.008);
    }
    s.tick(&table(&nodes), 0.1);
    let flung = s.offset(&nodes[0], 0);
    s.tick(&table(&nodes), 0.12);
    assert_ne!(s.offset(&nodes[0], 0), flung, "it flings");
    // The modal's commit has no list; closing it, the next brings it back.
    s.committed(&table(&[]), 0, 0);
    s.committed(&table(&nodes), 0, 0);
    let back = s.offset(&nodes[0], 0);
    s.tick(&table(&nodes), 0.2);
    s.tick(&table(&nodes), 0.4);
    assert_eq!(s.offset(&nodes[0], 0), back, "the fling ended with the list that left");
}

/// A fling under way when the program scrolls its list (`scroll_to`: a
/// commit with a new generation): the fling stops there, as Chromium's
/// clobbering commit stops the compositor's own motion, and what it flung
/// the UI had not taken goes; the reader's own motion (the gesture's
/// events, 16 px) the UI had not taken lands on the place jumped to
/// (ADR-0032, I4).
#[test]
fn a_fling_stops_when_the_program_scrolls_its_list() {
    let mut s = Scrolls::default();
    s.set_flings(true);
    let mut nodes = vec![node(1, 0.0, 5_000.0)];
    for (k, at) in [0.0_f32, 0.016, 0.032, 0.048].into_iter().enumerate() {
        assert!(s.gesture(&nodes, Some(WheelTarget(0)), y(4.0), (at, 0), k == 3));
        s.tick(&table(&nodes), at + 0.008);
    }
    s.tick(&table(&nodes), 0.1);
    let flung = s.offset(&nodes[0], 0);
    s.tick(&table(&nodes), 0.12);
    assert_ne!(s.offset(&nodes[0], 0), flung, "it flings");
    // The program's scroll comes with the next commit.
    place(&mut nodes[0], 2_000.0, 5_000.0);
    nodes[0].generation = 1;
    s.committed(&table(&nodes), 0, 0);
    assert_eq!(
        s.offset(&nodes[0], 0),
        (0.0, 2_016.0),
        "where the program put it, the reader's own 16 px on top"
    );
    s.tick(&table(&nodes), 0.2);
    s.tick(&table(&nodes), 0.4);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 2_016.0), "the fling stopped there");
    let took = take(&mut s);
    assert_eq!(took[0].delta, y(16.0), "the UI takes the reader's own alone");
}

/// A trackpad gesture resampled between its events, then flung after the
/// lift, from 1 s of uptime and from 262,144 s (three days, where an f32
/// of seconds steps by 1/64 s): the list goes through the same places at
/// every frame, to 1e-3 px. Instants are exact nanoseconds; only spans
/// between them are seconds ([`crate::clock::span`]).
#[test]
fn a_gesture_and_its_fling_are_the_same_at_any_uptime() {
    let ms = |n: u64| n * 1_000_000;
    let path = |base: u64| {
        let mut s = Scrolls::default();
        s.set_flings(true);
        let nodes = vec![node(1, 0.0, 50_000.0)];
        let mut seen = Vec::new();
        for k in 0..6_u64 {
            let at = base + ms(8 * k) + 1_234_567;
            s.gesture(&nodes, Some(WheelTarget(0)), y(12.0), (at, base), k == 5);
            // Frames between events: resampled, or flung after the lift.
            for f in [ms(3), ms(7)] {
                s.tick(&table(&nodes), at + f);
                seen.push(s.offset(&nodes[0], 0).1);
            }
        }
        for f in 1..40_u64 {
            s.tick(&table(&nodes), base + ms(48) + ms(16 * f) + 333);
            seen.push(s.offset(&nodes[0], 0).1);
        }
        seen
    };
    let near = path(1_000_000_000);
    let far = path(262_144 * 1_000_000_000);
    assert!(near.last().is_some_and(|&y| y > 200.0), "it flings: {near:?}");
    for (k, (a, b)) in near.iter().zip(&far).enumerate() {
        assert!((a - b).abs() <= 1e-3, "frame {k}: {a} at 1 s, {b} at 262,144 s");
    }
}

/// The fault matrix's one late finding (round 12): a trackpad gesture of
/// whole pixels settled the inner list at 163.00002 in one run and 163 in
/// another, by where the frames that resampled it fell. Floating-point
/// addition does not associate, and the gesture's motion was summed in
/// pieces cut at the frames, by the compositor at each frame and by the UI
/// at each pull. Here the same events are resampled by frames at 80 phases
/// and pulled at every other frame: each run settles the list, on both
/// sides, exactly where its events put it.
#[test]
fn a_resampled_gesture_settles_exactly_where_its_events_put_it_whatever_the_frames() {
    let ms = |n: u64| n * 1_000_000;
    let deltas = [10.0_f32, 20.0, 20.0, 15.0, 8.0, 6.0, 4.0];
    let mut settled = std::collections::BTreeSet::new();
    for phase in 0..80_u64 {
        let mut s = Scrolls::default();
        let nodes = vec![node(1, 80.0, 50_000.0)];
        // The UI holds its list exactly, as its holders do ([`motion`]).
        let mut ui = motion::units(80.0);
        let mut frame = 0_u64;
        let mut tick = |s: &mut Scrolls, at: u64, ui: &mut i64| {
            s.tick(&table(&nodes), at);
            frame += 1;
            if frame.is_multiple_of(2) {
                *ui = take(s).iter().fold(*ui, |o, d| o + d.delta.1);
            }
        };
        let start = 1_000_000_000_u64;
        for (k, d) in (0_u64..).zip(deltas) {
            let at = start + ms(8) * k;
            s.gesture(&nodes, Some(WheelTarget(0)), y(d), (at, start), false);
            tick(&mut s, at + 100_000 * phase % ms(8), &mut ui);
            tick(&mut s, at + (100_000 * phase + 3_700_000) % ms(8), &mut ui);
        }
        // Long past its last event: settled, its rest applied.
        tick(&mut s, start + ms(400), &mut ui);
        tick(&mut s, start + ms(416), &mut ui);
        let ui = motion::px(take(&mut s).iter().fold(ui, |o, d| o + d.delta.1));
        let shown = s.offset(&nodes[0], 0).1;
        settled.insert((shown.to_bits(), ui.to_bits()));
        assert_eq!(shown, 163.0, "phase {phase}: the compositor's list");
        assert_eq!(ui, 163.0, "phase {phase}: the UI's list");
    }
    assert_eq!(settled.len(), 1);
}

/// Codex r13 4, as a property: random deltas (fine, coarse, back and
/// forth, of a list near its top, off the grid's pixels, and deep), taken
/// as the router takes them ([`motion::Carry`]), resampled by random frame
/// schedules and pulled at random frames: every schedule settles the
/// compositor's list and the UI's on the same unit, the one their events'
/// motion put it at. (Within its range: where an edge stops a resampled
/// step depends on when the frame fell, as in any compositor; the edge
/// tests hold that.)
#[test]
fn random_motion_under_random_frames_settles_on_the_same_unit() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let ms = |n: u64| n * 1_000_000;
    for trial in 0..60 {
        let start = [5_000.0_f32, 5_000.1, 36_000_000.0][trial % 3];
        let deltas: Vec<f32> = (0..12)
            .map(|_| {
                let r = next();
                let magnitude = [1.0 / 1024.0, 0.37, 7.5, 40.0][usize::try_from(r % 4).unwrap_or(0)];
                let sign = if r & 16 == 0 { 1.0 } else { -1.0 };
                let times = [1.0, 2.0, 3.0, 4.0, 5.0][usize::try_from((r >> 8) % 5).unwrap_or(0)];
                sign * magnitude * times
            })
            .collect();
        let mut carry = motion::Carry::default();
        let steps: Vec<motion::Exact> = deltas.iter().map(|&d| carry.take((0.0, d))).collect();
        let mut ends = std::collections::BTreeSet::new();
        for _schedule in 0..6 {
            let mut s = Scrolls::default();
            let nodes = vec![node(1, start, 40_000_000.0)];
            let mut ui = nodes[0].pos.1;
            let begin = 1_000_000_000_u64;
            for (k, step) in (0_u64..).zip(&steps) {
                let at = begin + ms(8) * k;
                s.gesture(&nodes, Some(WheelTarget(0)), *step, (at, begin), false);
                for _ in 0..next() % 3 {
                    s.tick(&table(&nodes), at + next() % ms(8));
                    if next() % 2 == 0 {
                        ui = take(&mut s).iter().fold(ui, |o, d| o + d.delta.1);
                    }
                }
            }
            s.tick(&table(&nodes), begin + ms(500));
            s.stop_gesture(begin + ms(500));
            ui = take(&mut s).iter().fold(ui, |o, d| o + d.delta.1);
            let shown = s.pos(&nodes[0], 0).1;
            assert_eq!(shown, ui, "trial {trial}: the compositor and the UI on one unit");
            ends.insert(shown);
        }
        let sum = steps.iter().fold(motion::units(start), |a, s| a + s.1);
        assert_eq!(
            ends.into_iter().collect::<Vec<_>>(),
            [sum],
            "trial {trial}: every schedule settles where the events put it"
        );
    }
}

/// A swipe down a list whose lift comes with a sideways delta (along an
/// axis the list does not scroll): the lift is no motion of the list's,
/// but it ends the gesture all the same, so the fling is decided from
/// what the gesture moved.
#[test]
fn a_lift_along_the_other_axis_still_flings() {
    let mut s = Scrolls::default();
    s.set_flings(true);
    let nodes = vec![node(1, 0.0, 5_000.0)];
    for at in [0.0_f32, 0.016, 0.032] {
        s.gesture(&nodes, Some(WheelTarget(0)), y(20.0), (at, 0), false);
        s.tick(&table(&nodes), at + 0.008);
    }
    let flung = s.gesture(
        &nodes,
        Some(WheelTarget(0)),
        motion::units2((4.0, 0.0)),
        (0.048, 0),
        true,
    );
    assert!(flung, "the lift flings");
    let at = s.offset(&nodes[0], 0);
    s.tick(&table(&nodes), 0.2);
    assert!(s.offset(&nodes[0], 0).1 > at.1, "on down");
}

/// The edge rule through the compositor's route, as the UI's has it: a
/// fresh wheel against the end of a lone list (nothing around it can move
/// that way) is the list's, held against its edge: its overlay scrollbar
/// shows and the UI is told it was scrolled. A sideways step of the
/// gesture latched to it, along the axis it does not scroll, is not its
/// and counts for nothing.
#[test]
fn the_compositors_wheel_takes_an_edge_by_the_uis_rule() {
    let h = crate::scroll::ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(300.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&h)
                .child(div().height(5_000.0))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    // Jumped on the scenario's clock, which the wheel's stamps are on.
    s.act("to its end", || h.scroll_to((0.0, f32::MAX)));
    s.frame("first").expect("a frame");
    let end = h.offset();
    assert!(end.1 > 4_000.0, "at its end: {end:?}");
    let wheels = h.wheels();
    let notch = |c: &mut Compositor, delta: (f32, f32), at: f32| {
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta,
            time: crate::clock::nanos_of(at),
            precise: false,
            phase: WheelPhase::None,
            seen,
        })
    };
    let c = s.compositor();
    let wheeled = notch(c, (0.0, 30.0), 1.0);
    assert!(matches!(wheeled, Wheeled::Scrolled), "taken: {wheeled:?}");
    let list = &c.scene.compositing.scrollers[0];
    assert_eq!(c.scrolls.thumb_opacity(list, 1.0), 1.0, "its scrollbar shows");
    s.frame("told").expect("a frame");
    assert_eq!(h.wheels(), wheels + 1, "the UI was told");
    assert_eq!(h.offset(), end, "nothing moved");
    let wheeled = notch(s.compositor(), (30.0, 0.0), 1.05);
    assert!(matches!(wheeled, Wheeled::Nothing), "a sideways step: {wheeled:?}");
    s.frame("sideways").expect("a frame");
    assert_eq!(h.wheels(), wheels + 1, "counts for nothing");
}

/// A wheel held against a list's end is news (a frame asked for, the UI
/// told) once a take: the turns after it until the UI takes it lay
/// nothing out again, each vsync; after it, the next is news again.
#[test]
fn a_wheel_held_against_an_edge_is_news_once_a_main_frame() {
    let mut s = Scrolls::default();
    let nodes = vec![node(1, 500.0, 500.0)];
    assert!(s.scroll(&nodes, WheelTarget(0), y(30.0), 0), "news");
    for k in 1..4_u8 {
        let at = f32::from(k) * 0.016;
        assert!(
            !s.scroll(&nodes, WheelTarget(0), y(30.0), clock::nanos_of(at)),
            "turn {k}: no more"
        );
    }
    let took = take(&mut s);
    assert_eq!((took.len(), took[0].steps), (1, 4), "the UI takes it, each step once");
    assert!(
        s.scroll(&nodes, WheelTarget(0), y(30.0), clock::nanos_of(0.08)),
        "news again"
    );
}

/// The reader wheels a list (moved, then held against its end) and the
/// program jumps it before the UI took the turns in (a commit with a new
/// generation): the turns' displacement and steps are the reader's, kept
/// on top of the place jumped to and told once (ADR-0032, I4); their speed
/// (a reach measured from where it no longer is) goes.
#[test]
fn a_jump_keeps_the_readers_untaken_turns_but_not_their_speed() {
    let mut s = Scrolls::default();
    let mut nodes = vec![node(1, 450.0, 500.0)];
    assert!(s.scroll(&nodes, WheelTarget(0), y(30.0), 0));
    assert!(s.scroll(&nodes, WheelTarget(0), y(30.0), clock::nanos_of(0.016)));
    assert!(
        !s.scroll(&nodes, WheelTarget(0), y(30.0), clock::nanos_of(0.02)),
        "held at its end"
    );
    place(&mut nodes[0], 100.0, 500.0);
    nodes[0].generation = 1;
    s.committed(&table(&nodes), 0, 0);
    assert!(
        s.reach(&table(&nodes), 0.03, 0.1).is_empty(),
        "no reach from before the jump"
    );
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 150.0), "the 50 px moved, on the jump");
    let took = take(&mut s);
    assert_eq!((took[0].delta, took[0].steps), (y(50.0), 3));
}

/// A list 100 px tall of 5,000 px, its offset in `h`, drawn once; and the
/// time on the clock then.
fn handled_list(h: crate::scroll::ScrollHandle) -> (Scenario, f32) {
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(300.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&h)
                .child(div().height(5_000.0))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    (s, clock::current_seconds())
}

/// A trackpad event over the list at `at` moving `dy`, the clock set to
/// it, a compositor frame drawn at it.
fn pad(s: &mut Scenario, at: f32, dy: f32, began: bool, ended: bool) -> Wheeled {
    swipe(s, (50.0, 50.0), at, (0.0, dy), phase_of(began, ended))
}

/// A trackpad event at `point` at `at` moving `delta`, at `phase` of the
/// platform's gesture, the clock set to it, a compositor frame drawn at it.
fn swipe(s: &mut Scenario, point: (f32, f32), at: f32, delta: (f32, f32), phase: WheelPhase) -> Wheeled {
    clock::advance_to(Duration::from_secs_f32(at));
    let c = s.compositor();
    let seen = c.present_last();
    let wheeled = c.wheel_input(WheelInput {
        point,
        delta,
        time: crate::clock::nanos_of(at),
        precise: true,
        phase,
        seen,
    });
    c.tick(at);
    wheeled
}

/// A trackpad gesture and the platform's momentum against the end of a
/// list, on a window otherwise idle: each event is taken as it comes, so
/// the reader's intent reaches the UI even when the main frame is granted
/// before the compositor draws again, and once a main frame: the events
/// after the first until that main frame ask for nothing.
#[test]
fn a_trackpad_against_an_edge_tells_the_ui_once_a_main_frame() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, _) = handled_list(h);
    // Jumped on the scenario's clock, which the events' stamps are on.
    s.act("to its end", || h.scroll_to((0.0, f32::MAX)));
    s.settle().expect("at its end");
    let t0 = clock::current_seconds() + 0.001;
    clock::advance_to(Duration::from_secs_f32(t0));
    s.compositor().set_flings(false);
    let wheels = h.wheels();
    let event = |s: &mut Scenario, at: f32, phase: WheelPhase| {
        let c = s.compositor();
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 30.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase,
            seen,
        })
    };
    // The fingers' first event, the main frame granted before any tick.
    assert!(
        matches!(event(&mut s, t0, WheelPhase::Began), Wheeled::Scrolled),
        "news"
    );
    assert!(s.compositor().scrolled(), "the UI is owed it");
    for k in 1..4_u8 {
        let phase = if k == 3 { WheelPhase::Ended } else { WheelPhase::Changed };
        let w = event(&mut s, t0 + f32::from(k) * 0.016, phase);
        assert!(matches!(w, Wheeled::Nothing), "event {k} asks for nothing: {w:?}");
    }
    s.frame("a main frame").expect("a frame");
    assert_eq!(h.wheels(), wheels + 4, "each step told once");
    // The platform's momentum, in the next main frame's time.
    assert!(matches!(
        event(&mut s, t0 + 0.07, WheelPhase::MomentumBegan),
        Wheeled::Scrolled
    ));
    assert!(matches!(
        event(&mut s, t0 + 0.086, WheelPhase::Momentum),
        Wheeled::Nothing
    ));
    s.frame("another").expect("a frame");
    assert_eq!(h.wheels(), wheels + 6, "each once again");
}

/// A sideways trackpad step of a gesture latched to a list that scrolls
/// only down is not the list's: nothing scrolled, no main frame asked for.
#[test]
fn a_sideways_trackpad_step_on_a_vertical_list_asks_for_nothing() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    pad(&mut s, t0, 30.0, true, false);
    s.frame("taken in").expect("a frame");
    let c = s.compositor();
    let seen = c.present_last();
    let w = c.wheel_input(WheelInput {
        point: (50.0, 50.0),
        delta: (30.0, 0.0),
        time: clock::nanos_of(t0 + 0.016),
        precise: true,
        phase: WheelPhase::Changed,
        seen,
    });
    assert!(matches!(w, Wheeled::Nothing), "not the list's: {w:?}");
    assert!(!c.scrolled(), "nothing owed the UI");
}

/// Fingers down on a list, moving it (a main frame takes that in), then
/// resting when the program scrolls the list: the fingers' next moves,
/// made before the jump's commit comes or after, move on from the jump
/// (ADR-0032: a jump is an assignment the UI made, and what it had not
/// taken when it made it is the reader's input after it, never dropped).
#[test]
fn a_finger_move_after_a_rest_and_a_jump_moves_on_from_it() {
    for before_commit in [false, true] {
        let h = crate::scroll::ScrollHandle::new();
        let (mut s, t0) = handled_list(h);
        s.compositor().set_flings(false);
        pad(&mut s, t0, 30.0, true, false);
        pad(&mut s, t0 + 0.016, 30.0, false, false);
        s.frame("taken in").expect("a frame");
        clock::advance_to(Duration::from_secs_f32(t0 + 0.6));
        s.act("jump", || h.scroll_to((0.0, 2_000.0)));
        if before_commit {
            pad(&mut s, t0 + 0.62, 30.0, false, false);
        }
        s.frame("jumped").expect("a frame");
        let wheeled = pad(&mut s, t0 + 0.64, 30.0, false, false);
        assert!(
            matches!(wheeled, Wheeled::Scrolled),
            "it moves ({before_commit}): {wheeled:?}"
        );
        s.compositor().tick(t0 + 1.0);
        let shown = downs(s.compositor())[0];
        assert!(shown > 2_000.0, "on from the jump ({before_commit}): {shown}");
        s.frame("after").expect("a frame");
        assert_eq!(h.offset().1, shown, "the UI's too ({before_commit})");
    }
}

/// A notched wheel turning through a jump: the jump cancels nothing of
/// the wheel's (Chrome's notch retargets from where it is): a notch the
/// UI had not taken when it jumped lands on the jump, and the next notch
/// scrolls on from there.
#[test]
fn a_notched_wheel_turning_through_a_jump_scrolls_from_it() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    let notch = |s: &mut Scenario, at: f32| {
        clock::advance_to(Duration::from_secs_f32(at));
        let c = s.compositor();
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 40.0),
            time: clock::nanos_of(at),
            precise: false,
            phase: WheelPhase::None,
            seen,
        })
    };
    notch(&mut s, t0);
    s.frame("turned").expect("a frame");
    clock::advance_to(Duration::from_secs_f32(t0 + 0.05));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    // A notch the UI has not taken when the jump's commit comes (its OS
    // time before the jump's, which nothing reads): on top of the jump.
    notch(&mut s, t0 + 0.04);
    s.frame("jumped").expect("a frame");
    assert_eq!(downs(s.compositor())[0], 2_040.0, "the notch on the jump");
    let wheeled = notch(&mut s, t0 + 0.1);
    assert!(
        matches!(wheeled, Wheeled::Scrolled),
        "the next notch scrolls: {wheeled:?}"
    );
    assert_eq!(downs(s.compositor())[0], 2_080.0, "on from there");
    s.frame("after").expect("a frame");
    assert_eq!(h.offset().1, 2_080.0, "the UI's too");
}

/// Nothing orders input and a jump by time, so uptime cannot change it: at
/// 262,144 s (where an f32 of seconds steps by 1/64 s) a swipe begun 5 ms
/// before a jump or 5 ms after, the UI not having taken it, goes on from
/// the jump (Codex r7, resolved by construction).
#[test]
fn input_and_a_jump_keep_their_order_at_long_uptime() {
    for (after, scrolls) in [(true, true), (false, true)] {
        let h = crate::scroll::ScrollHandle::new();
        let (mut s, _) = handled_list(h);
        s.compositor().set_flings(false);
        let t0 = Duration::from_secs(262_144);
        let jump = t0 + Duration::from_millis(100);
        let swipe_at = if after {
            jump + Duration::from_millis(5)
        } else {
            jump - Duration::from_millis(5)
        };
        let step = |s: &mut Scenario, at: Duration, phase| {
            clock::advance_to(at);
            let c = s.compositor();
            let seen = c.present_last();
            c.wheel_input(WheelInput {
                point: (50.0, 50.0),
                delta: (0.0, 30.0),
                time: u64::try_from(at.as_nanos()).unwrap_or(u64::MAX),
                precise: true,
                phase,
                seen,
            })
        };
        if !after {
            step(&mut s, swipe_at, WheelPhase::Began);
        }
        clock::advance_to(jump);
        s.act("jump", || h.scroll_to((0.0, 2_000.0)));
        // Begun after the jump, before its commit comes: on top of it.
        if after {
            step(&mut s, swipe_at, WheelPhase::Began);
        }
        s.frame("jumped").expect("a frame");
        let w = step(&mut s, jump + Duration::from_millis(20), WheelPhase::Changed);
        assert_eq!(
            matches!(w, Wheeled::Scrolled),
            scrolls,
            "begun after the jump {after}: {w:?}"
        );
    }
}

/// A wheel handler over a list declines a trackpad's gesture (the UI's
/// route scrolls the list), the fingers lift, the program jumps the list,
/// and the platform's momentum comes: the platform's events are the
/// reader's input, handled after the jump, so they move on from it, on
/// the UI's route as on the compositor's (ADR-0032: a jump stops only the
/// compositor's own fling; a jump that swallows a gesture begun before it
/// is a revisit condition).
#[test]
fn momentum_on_the_uis_route_after_a_jump_moves_on_from_it() {
    let h = crate::scroll::ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(300.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&h)
                .on_wheel(|_| false)
                .child(div().height(5_000.0))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let t0 = clock::current_seconds();
    let event = |s: &mut Scenario, at: f32, phase: WheelPhase| {
        clock::advance_to(Duration::from_secs_f32(t0 + at));
        let c = s.compositor();
        let seen = c.present_last();
        let w = c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 30.0),
            time: clock::nanos_of(t0 + at),
            precise: true,
            phase,
            seen,
        });
        let Wheeled::Ui(target, step) = w else {
            panic!("handed to the UI: {w:?}");
        };
        s.send(crate::window::Input::WheelAt {
            step,
            dx: 0.0,
            dy: 30.0,
            target,
        });
        s.frame("taken").expect("a frame");
    };
    event(&mut s, 0.0, WheelPhase::Began);
    event(&mut s, 0.016, WheelPhase::Changed);
    event(&mut s, 0.032, WheelPhase::Ended);
    assert!(h.offset().1 > 0.0, "the UI's route scrolled it");
    clock::advance_to(Duration::from_secs_f32(t0 + 0.04));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    s.frame("jumped").expect("a frame");
    event(&mut s, 0.05, WheelPhase::MomentumBegan);
    event(&mut s, 0.066, WheelPhase::Momentum);
    let after = h.offset().1;
    assert!(after > 2_000.0, "the momentum moves on from the jump: {after}");
    event(&mut s, 0.2, WheelPhase::MomentumEnded);
    event(&mut s, 0.3, WheelPhase::Began);
    assert!(h.offset().1 > after, "fingers again scroll on");
}

/// The program scrolls a list (an arrow key's `scroll_into_view`) and the
/// reader swipes over it at once, before the commit with the jump comes:
/// the swipe began after the jump, so it goes on from the place jumped to.
#[test]
fn a_swipe_begun_after_a_jump_goes_on_from_where_it_jumped() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    clock::advance_to(Duration::from_secs_f32(t0 + 0.01));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    pad(&mut s, t0 + 0.02, 30.0, true, false);
    s.frame("the jump comes").expect("a frame");
    let first = downs(s.compositor())[0];
    assert!(first > 2_000.0, "the swipe applied on top of the jump: {first}");
    let wheeled = pad(&mut s, t0 + 0.036, 30.0, false, false);
    assert!(matches!(wheeled, Wheeled::Scrolled), "the swipe goes on: {wheeled:?}");
    assert!(downs(s.compositor())[0] > first, "from there");
}

/// A wheel event over the list stamped `stamp` (seconds on the frame
/// clock), taken whenever the clock is now.
fn stamped(s: &mut Scenario, stamp: f32, delta: (f32, f32), precise: bool, phase: WheelPhase) -> Wheeled {
    let c = s.compositor();
    let seen = c.present_last();
    c.wheel_input(WheelInput {
        point: (50.0, 50.0),
        delta,
        time: clock::nanos_of(stamp),
        precise,
        phase,
        seen,
    })
}

/// Input the render thread takes after a jump's commit, whatever time the
/// OS gave it (here, before the jump's): it moves on from the jump, as
/// the reader saw it shown (Codex r7, the OS time read by nothing but a
/// gesture's resampling: resolved by construction).
#[test]
fn input_taken_after_a_jumps_commit_moves_from_it_whatever_its_os_time() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    clock::advance_to(Duration::from_secs_f32(t0 + 0.05));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    s.frame("jumped").expect("a frame");
    clock::advance_to(Duration::from_secs_f32(t0 + 0.08));
    let notch = stamped(&mut s, t0 + 0.04, (0.0, 40.0), false, WheelPhase::None);
    assert!(matches!(notch, Wheeled::Scrolled), "the notch moves: {notch:?}");
    assert_eq!(downs(s.compositor())[0], 2_040.0, "on from the jump");
    let began = stamped(&mut s, t0 + 0.045, (0.0, 30.0), true, WheelPhase::Began);
    assert!(matches!(began, Wheeled::Scrolled), "the gesture too: {began:?}");
    s.compositor().tick(t0 + 1.0);
    assert_eq!(downs(s.compositor())[0], 2_070.0, "all of it, on from the jump");
    s.frame("after").expect("a frame");
    assert_eq!(h.offset().1, 2_070.0, "the UI's too");
}

/// A push outward at an edge moves nothing now and nothing later
/// (ADR-0032 I3): a list at its end, the program jumps it to its middle,
/// and the reader turns the wheel on down before the commit with the jump
/// comes. The turn moved nothing at the end, so nothing lands on the jump
/// (its step is still told, once).
#[test]
fn a_turn_against_an_end_before_a_jump_commits_adds_nothing_to_it() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, _) = handled_list(h);
    s.act("to its end", || h.scroll_to((0.0, f32::MAX)));
    s.settle().expect("at its end");
    let t0 = clock::current_seconds() + 0.001;
    let end = s.compositor().scene.compositing.scrollers[0].max.1;
    assert_eq!(downs(s.compositor())[0], end, "at its end");
    clock::advance_to(Duration::from_secs_f32(t0 + 0.01));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    clock::advance_to(Duration::from_secs_f32(t0 + 0.02));
    let turned = stamped(&mut s, t0 + 0.02, (0.0, 40.0), false, WheelPhase::None);
    assert!(matches!(turned, Wheeled::Scrolled), "taken at the end: {turned:?}");
    assert_eq!(downs(s.compositor())[0], end, "nothing moved there");
    s.frame("the jump comes").expect("a frame");
    assert_eq!(downs(s.compositor())[0], 2_000.0, "nothing on the jump");
    s.frame("after").expect("a frame");
    assert_eq!(h.offset().1, 2_000.0, "the UI's too");
}

/// A turn made after a jump, before the jump's commit comes: the next main
/// frame takes it in on top of the jump, and the handle is told of it once
/// (cc's `PullDeltaForMainThread`: a delta is sent once, then held as
/// sent until the commit carrying it comes).
#[test]
fn a_turn_made_before_a_jumps_commit_is_taken_once_on_top_of_it() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    let wheels = h.wheels();
    clock::advance_to(Duration::from_secs_f32(t0 + 0.01));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    clock::advance_to(Duration::from_secs_f32(t0 + 0.02));
    let turned = stamped(&mut s, t0 + 0.02, (0.0, 40.0), false, WheelPhase::None);
    assert!(matches!(turned, Wheeled::Scrolled), "{turned:?}");
    s.frame("taken").expect("a frame");
    assert_eq!(h.offset().1, 2_040.0, "it lands on the jump");
    assert_eq!(h.wheels(), wheels + 1, "told once");
    s.frame("again").expect("a frame");
    assert_eq!(h.offset().1, 2_040.0, "once");
    s.frame("later").expect("a frame");
    assert_eq!(h.wheels(), wheels + 1, "and never again");
}

/// Fingers touching the trackpad over a flinging list without moving (a
/// touch-to-stop, AppKit's `MayBegin`): nothing takes a touch with no
/// delta, yet it begins a gesture, ending the one before and stopping its
/// fling.
#[test]
fn a_touch_with_no_movement_stops_the_fling_under_it() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(true);
    pad(&mut s, t0, 30.0, true, false);
    pad(&mut s, t0 + 0.016, 30.0, false, false);
    pad(&mut s, t0 + 0.032, 30.0, false, false);
    pad(&mut s, t0 + 0.048, 0.0, false, true);
    s.compositor().tick(t0 + 0.1);
    let flung = downs(s.compositor())[0];
    s.compositor().tick(t0 + 0.12);
    assert!(downs(s.compositor())[0] > flung, "it flings");
    let touched = pad(&mut s, t0 + 0.13, 0.0, true, false);
    assert!(matches!(touched, Wheeled::Nothing), "nothing takes it: {touched:?}");
    let stopped = downs(s.compositor())[0];
    s.compositor().tick(t0 + 0.3);
    assert_eq!(downs(s.compositor())[0], stopped, "the fling stopped");
}

/// Fingers down again on a list right after a lift are a gesture of their
/// own, however soon (the platform says fingers went down, not that its
/// momentum began): a jump that refused the first does not refuse it. On
/// macOS (the platform's momentum) as where the compositor flings.
#[test]
fn a_second_flick_soon_after_a_lift_is_a_gesture_of_its_own() {
    for flings in [false, true] {
        let h = crate::scroll::ScrollHandle::new();
        let (mut s, t0) = handled_list(h);
        s.compositor().set_flings(flings);
        pad(&mut s, t0, 30.0, true, false);
        pad(&mut s, t0 + 0.016, 30.0, false, false);
        pad(&mut s, t0 + 0.032, 0.0, false, true);
        clock::advance_to(Duration::from_secs_f32(t0 + 0.04));
        s.act("jump", || h.scroll_to((0.0, 2_000.0)));
        s.frame("jumped").expect("a frame");
        let wheeled = pad(&mut s, t0 + 0.05, 30.0, true, false);
        assert!(
            matches!(wheeled, Wheeled::Scrolled),
            "a new gesture (flings {flings}): {wheeled:?}"
        );
        assert!(downs(s.compositor())[0] > 2_000.0, "from the jump (flings {flings})");
    }
}

/// Fingers on a list lift, the program jumps the list, and the platform's
/// momentum begins with the pointer over a horizontal code block inside
/// it: the momentum is the lifted gesture's, whatever it is over, so it
/// moves the list on from the jump and never the code block.
#[test]
fn momentum_over_another_scroller_after_a_jump_stays_on_its_list() {
    let h = crate::scroll::ScrollHandle::new();
    let code = crate::scroll::ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(300.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&h)
                .child(
                    div()
                        .width(300.0)
                        .height(60.0)
                        .overflow_x(Overflow::Scroll)
                        .track_scroll(&code)
                        .child(div().width(3_000.0).height(60.0)),
                )
                .child(div().height(5_000.0))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let t0 = clock::current_seconds();
    s.compositor().set_flings(false);
    // Over the list below the code block, vertically.
    let below = (50.0, 80.0);
    swipe(&mut s, below, t0, (0.0, 20.0), WheelPhase::Began);
    swipe(&mut s, below, t0 + 0.016, (0.0, 20.0), WheelPhase::Changed);
    swipe(&mut s, below, t0 + 0.032, (0.0, 0.0), WheelPhase::Ended);
    s.frame("taken in").expect("a frame");
    clock::advance_to(Duration::from_secs_f32(t0 + 0.04));
    s.act("jump", || h.scroll_to((0.0, 1_000.0)));
    s.frame("jumped").expect("a frame");
    let code_at = code.offset();
    // The momentum, diagonal, with the pointer over the code block.
    let over = (50.0, 20.0);
    for (k, phase) in [WheelPhase::MomentumBegan, WheelPhase::Momentum, WheelPhase::Momentum]
        .into_iter()
        .enumerate()
    {
        let at = t0 + 0.05 + 0.016 * f32::from(u8::try_from(k).unwrap_or(0));
        swipe(&mut s, over, at, (20.0, 20.0), phase);
    }
    s.compositor().tick(t0 + 1.0);
    s.frame("after").expect("a frame");
    assert!(
        downs(s.compositor())[0] > 1_000.0,
        "the list moves on from where it was jumped to"
    );
    assert_eq!(code.offset(), code_at, "the code block is not scrolled");
}

/// A trackpad gesture over a list, its fingers lifted and the platform's
/// momentum begun, when the program scrolls the list (a jump) before the UI
/// took any of it: what the gesture moved lands on the jump, whole, and
/// the platform's momentum after moves on from there (ADR-0032: the
/// platform's events are the reader's input; only the compositor's own
/// fling stops at a jump). Fingers down again scroll on.
#[test]
fn a_platforms_momentum_after_the_program_scrolled_its_list_moves_on_from_it() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    let event =
        |s: &mut Scenario, at: f32, dy: f32, phase: WheelPhase| swipe(s, (50.0, 50.0), t0 + at, (0.0, dy), phase);
    // Fingers, then the platform's momentum begun as they lift.
    event(&mut s, 0.0, 30.0, WheelPhase::Began);
    event(&mut s, 0.016, 30.0, WheelPhase::Changed);
    event(&mut s, 0.032, 0.0, WheelPhase::Ended);
    event(&mut s, 0.04, 30.0, WheelPhase::MomentumBegan);
    let moved = downs(s.compositor())[0];
    assert!(moved > 0.0, "the gesture scrolls");
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    s.frame("jumped").expect("a frame");
    let jumped = downs(s.compositor())[0];
    assert_eq!(jumped, 2_000.0 + moved, "what it moved, on the jump");
    assert_eq!(h.offset().1, jumped, "the UI's too");
    let wheeled = event(&mut s, 0.056, 30.0, WheelPhase::Momentum);
    assert!(matches!(wheeled, Wheeled::Scrolled), "the momentum moves: {wheeled:?}");
    event(&mut s, 0.2, 0.0, WheelPhase::MomentumEnded);
    assert!(downs(s.compositor())[0] > jumped, "on from there");
    let wheeled = event(&mut s, 0.3, 30.0, WheelPhase::Began);
    assert!(
        matches!(wheeled, Wheeled::Scrolled),
        "fingers again scroll: {wheeled:?}"
    );
}

/// A virtual list's rows: how many, how many were spliced in at the top,
/// and how far the rest were rotated down (each keyed as it came, so its
/// key follows it).
type Rows = (u64, u64, u64);

/// A virtual list of `rows` ([`Rows`]), each 24 px tall, in a 300 px
/// viewport, settled at row `at`, under a trackpad flick lifted with
/// flings on; and the time it began.
fn flung_list(state: crate::list::ListState, rows: Signal<Rows>, at: u64) -> (Scenario, f32) {
    let mut s = Scenario::measuring(
        move || {
            let (count, inserted, rotated) = rows.get();
            let n = count.saturating_sub(inserted).max(1);
            div()
                .width(300.0)
                .child(
                    crate::list::virtual_list(
                        &state,
                        count,
                        move |j| match j.checked_sub(inserted) {
                            None => u64::MAX.saturating_sub(j),
                            Some(k) => k.wrapping_add(n).wrapping_sub(rotated).checked_rem(n).unwrap_or(k),
                        },
                        |i| div().height(24.0).child(text(format!("row {i}"))).into_element(),
                    )
                    .height(300.0),
                )
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("the first frame");
    if at > 0 {
        s.act("to the row", || state.scroll_to_row(at));
        s.settle().expect("there");
    }
    let t0 = clock::current_seconds();
    let c = s.compositor();
    c.set_flings(true);
    let seen = c.present_last();
    for k in 0..4_u8 {
        let at = t0 + f32::from(k) * 0.016;
        c.wheel_input(WheelInput {
            point: (150.0, 150.0),
            delta: (0.0, 60.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(k == 0, k == 3),
            seen,
        });
        c.tick(at + 0.008);
    }
    (s, t0)
}

/// Main frames `frames` (numbered from the flick's lift) a display
/// interval apart, each drawn at its time; the anchor's row after each.
fn fling_frames(s: &mut Scenario, state: crate::list::ListState, t0: f32, frames: std::ops::Range<u16>) -> Vec<u64> {
    frames
        .map(|f| {
            clock::advance_to(Duration::from_secs_f32(t0 + 0.06 + f32::from(f) / 60.0));
            s.frame("a main frame under the fling").expect("a frame");
            state.anchor().row
        })
        .collect()
}

/// A fling over a virtual list whose owner, from its layout hook, flushes
/// a scroll to a row it was asked for (as a message scroller does): that
/// is the program's jump, wherever it runs, so the fling stops and the
/// list lands on the row.
#[test]
fn a_scroll_to_a_row_flushed_in_a_layout_hook_stops_a_fling() {
    let state = crate::list::ListState::new(24.0);
    let pending = Rc::new(Cell::new(None::<u64>));
    let asked = Rc::clone(&pending);
    state.on_layout(move |_| {
        asked.take().is_some_and(|row| {
            state.scroll_to_row(row);
            true
        })
    });
    let (mut s, t0) = flung_list(state, Signal::new((1_000_000, 0, 0)), 0);
    let before = fling_frames(&mut s, state, t0, 0..10);
    assert!(before[9] > before[0], "it flings: {before:?}");
    pending.set(Some(5_000));
    s.act("a scroll asked for", || {});
    let after = fling_frames(&mut s, state, t0, 10..120);
    assert!(
        after[10..].iter().all(|&r| r == 5_000),
        "the fling stopped, on the row asked for: {after:?}"
    );
}

/// Rows spliced in above the place shown, and the rows reordered, during a
/// fling: the content moved under the place (rebases), so the fling goes
/// on from where it was, with no jump back.
#[test]
fn rows_spliced_or_reordered_under_a_fling_leave_it_going() {
    for reorder in [false, true] {
        let state = crate::list::ListState::new(24.0);
        let rows = Signal::new((1_000_000_u64, 0, 0));
        let (mut s, t0) = flung_list(state, rows, 0);
        let before = fling_frames(&mut s, state, t0, 0..10);
        let at = before[9];
        let shift = if reorder {
            s.act("reordered", || {
                rows.set((1_000_000, 0, 100));
                state.reorder(1_000_000, |p| Some((p + 100) % 1_000_000));
            });
            100
        } else {
            s.act("spliced", || {
                rows.set((1_000_050, 50, 0));
                state.splice(0, 0, 50);
            });
            50
        };
        let after = fling_frames(&mut s, state, t0, 10..60);
        assert!(
            after[0] >= at + shift,
            "no jump back (reorder: {reorder}): {at} then {after:?}"
        );
        assert!(
            after[30] > after[0] + 20,
            "the fling goes on (reorder: {reorder}): {after:?}"
        );
    }
}

/// A program's scroll to a far row during a fling, which in the window it
/// builds lies at the very offset the list had in the window before
/// (rows alike, at the same place in their windows): another place all
/// the same, a jump, so the fling stops (what of it the UI takes after
/// the jump it leaves out, what it has not taken the commit drops) and
/// the list lands on the row, the flick's own 240 px, which the UI had
/// not taken, on top of it (ADR-0032 I4: real input is never dropped).
#[test]
fn a_scroll_to_a_far_row_at_an_equal_local_offset_stops_a_fling() {
    let state = crate::list::ListState::new(24.0);
    let (mut s, t0) = flung_list(state, Signal::new((1_000_000, 0, 0)), 500);
    let handle = state.scroll().expect("its scroll handle");
    let local = handle.offset();
    s.act("far", || state.scroll_to_row(700));
    assert_eq!(handle.offset(), local, "the same offset in another window");
    let after = fling_frames(&mut s, state, t0, 0..120);
    assert!(
        after[1..].iter().all(|&r| r == 710),
        "the fling stopped, the flick's 10 rows on the row asked for: {after:?}"
    );
}

/// A trackpad flick over a long virtual list, lifted with flings on: the
/// list re-windows under the fling again and again (its offset rebased in
/// its own layout, not a program's scroll), and the fling goes on through
/// every re-window to its natural end, as Chrome's goes on through
/// anchoring's adjustments.
#[test]
fn a_fling_over_a_virtual_list_goes_on_through_its_re_windows() {
    let state = crate::list::ListState::new(24.0);
    let (mut s, t0) = flung_list(state, Signal::new((1_000_000, 0, 0)), 0);
    let rows = fling_frames(&mut s, state, t0, 0..240);
    let windows = rows.windows(2).filter(|w| w[1] > w[0]).count();
    assert!(
        rows[20] > rows[5] + 20,
        "it moves on past its first re-windows: {rows:?}"
    );
    assert!(windows > 20, "frame after frame: {rows:?}");
    assert_eq!(rows[238], rows[239], "to its natural end: {rows:?}");
}

/// A wheel held against the end of the list its gesture latched to moves
/// nothing, but the list took it: its overlay scrollbar shows (here, and
/// in the UI's own through the next main frame's delta), as when it moves.
#[test]
fn a_wheel_against_an_edge_shows_the_overlay_scrollbar() {
    let mut s = Scrolls::default();
    let nodes = vec![node(1, 500.0, 500.0)];
    assert_eq!(s.thumb_opacity(&nodes[0], 0.0), 0.0, "at rest, hidden");
    assert!(
        s.scroll(&nodes, WheelTarget(0), y(30.0), clock::nanos_of(1.0)),
        "taken, against its end"
    );
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 500.0), "nothing moved");
    assert_eq!(s.thumb_opacity(&nodes[0], 1.0), 1.0, "its scrollbar shows");
    let sent = take(&mut s);
    assert_eq!(sent.len(), 1, "the UI is told");
    assert!(
        sent[0].steps > 0 && sent[0].delta == (0, 0),
        "wheeled, not moved: {:?}",
        sent[0]
    );
    assert!(take(&mut s).is_empty(), "once");
    // Along an axis it does not scroll, it takes nothing.
    assert!(!s.scroll(
        &nodes,
        WheelTarget(0),
        motion::units2((30.0, 0.0)),
        clock::nanos_of(2.0)
    ));
}

/// The first commit of a new compositor, its scroll containers at rest,
/// allocates nothing: nothing held for any, there is nothing to sort out
/// (the set of those shown is not even filled).
#[test]
fn a_new_compositors_first_commit_at_rest_allocates_nothing() {
    let nodes: Vec<ScrollNode> = (0..64).map(|k| node(k, 0.0, 500.0)).collect();
    let mut s = Scrolls::default();
    let nodes = table(&nodes);
    let ((), allocations) = crate::heap::allocations(|| s.committed(&nodes, 0, 0));
    assert_eq!(allocations, 0);
}

/// A commit's scroll bookkeeping allocates nothing: with its scroll
/// containers at rest (nothing held for any), and with a scroll owed (the
/// set of those shown is one kept), 100 commits each.
#[test]
fn a_commits_scroll_bookkeeping_allocates_nothing() {
    crate::testing::both_passes(|pass| {
        let lists = || {
            let list = |left: f32| {
                div()
                    .position(Position::Absolute)
                    .left(left)
                    .top(0.0)
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(400.0))
            };
            div()
                .size(crate::pct(100.0))
                .child(list(0.0))
                .child(list(200.0))
                .into_element()
        };
        let mut s = Scenario::measuring(lists, 400, 100, 1.0).expect("a scenario");
        pass.scenario(&mut s);
        s.frame("first").expect("a frame");
        let c = s.compositor();
        let recommit = |c: &mut Compositor| {
            let taken = c.scrolls.outbox().takes;
            c.scrolls.committed(&c.scene.compositing, taken, c.commit);
        };
        let ((), at_rest) = crate::heap::allocations(|| {
            for _ in 0..100 {
                recommit(c);
            }
        });
        pass.measured(|| assert_eq!(at_rest, 0, "at rest"));
        let seen = c.present_last();
        let wheeled = c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 30.0),
            time: 0,
            precise: false,
            phase: WheelPhase::None,
            seen,
        });
        assert!(matches!(wheeled, Wheeled::Scrolled), "the list scrolls: {wheeled:?}");
        // Warm: the set of those shown grows once.
        recommit(c);
        let ((), owed) = crate::heap::allocations(|| {
            for _ in 0..100 {
                recommit(c);
            }
        });
        pass.measured(|| assert_eq!(owed, 0, "with a scroll owed"));
        // Opus r9 nit: main frames granted while it moves, each with how
        // far it may go, their buffer given back as a commit brings it.
        let horizon = Duration::from_millis(16);
        let seen = c.present_last();
        let again = c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 30.0),
            time: 16_000_000,
            precise: false,
            phase: WheelPhase::None,
            seen,
        });
        assert!(matches!(again, Wheeled::Scrolled), "a second notch: {again:?}");
        let warm = c.begin_main_frame(20_000_000_u64, horizon);
        assert_eq!(warm.len(), 1, "it moves: a reach");
        c.recycle_reach(warm);
        let ((), granted) = crate::heap::allocations(|| {
            for _ in 0..100 {
                let reach = c.begin_main_frame(20_000_000_u64, horizon);
                c.recycle_reach(reach);
            }
        });
        pass.measured(|| assert_eq!(granted, 0, "main frames while it moves"));
    });
}

/// A list at `left` that is the root of a component keyed 7, itself the
/// root of a wrapper keyed `key` (or, `None`, unkeyed).
fn wrapped_list(left: f32, key: Option<u64>) -> crate::element::Element {
    use crate::component::{Ctx, component};
    let inner = |left: f32| {
        component(left, |cx: Ctx<f32>| {
            move || {
                div()
                    .position(Position::Absolute)
                    .left(cx.props().get())
                    .top(0.0)
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(400.0))
                    .into_element()
            }
        })
        .key(7_u64)
    };
    let wrapper = component(left, move |cx: Ctx<f32>| move || inner(cx.props().get()).into_element());
    match key {
        Some(k) => wrapper.key(k).into_element(),
        None => wrapper.into_element(),
    }
}

/// Wrappers side by side, keyed 1 and 2 or unkeyed, whose roots are each
/// a component keyed 7: every component of the chain adds its own to the
/// path, so the lists are two scrollers, and two nodes to a screen reader.
#[test]
fn wrappers_whose_roots_are_keyed_alike_place_them_apart() {
    for keys in [(Some(1), Some(2)), (None, None)] {
        let mut s = Scenario::measuring(
            move || {
                div()
                    .size(pct(100.0))
                    .child(wrapped_list(0.0, keys.0))
                    .child(wrapped_list(200.0, keys.1))
                    .into_element()
            },
            400,
            100,
            1.0,
        )
        .expect("a scenario");
        s.frame("first").expect("a frame");
        let mut ts = crate::text::TextSystem::new();
        let tree = crate::access::tree(s.last(), None, 1.0, "w", &mut ts);
        let ids: Vec<_> = tree.nodes.iter().map(|(id, _)| *id).collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[..i].contains(id), "{keys:?}: AccessKit id {id:?} twice");
        }
        let c = &mut s.parts().1.compositor;
        let scrollers: Vec<NodeKey> = c.scene.compositing.scrollers.iter().map(|n| n.key).collect();
        assert_eq!(scrollers.len(), 2, "{keys:?}");
        assert_ne!(scrollers[0], scrollers[1], "{keys:?}: two scrollers");
    }
}

/// Each scroll container's offset down, in the order they were painted.
fn downs(c: &Compositor) -> Vec<f32> {
    c.scene
        .compositing
        .scrollers
        .iter()
        .map(|n| c.scrolls.offset(n, 0).1)
        .collect()
}

#[test]
fn a_notched_wheel_moved_onto_another_list_scrolls_that_one() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let seen = c.present_last();
    let turn = |c: &mut Compositor, x: f32, at: f32| {
        c.wheel_input(WheelInput {
            point: (x, 50.0),
            delta: (0.0, 40.0),
            time: crate::clock::nanos_of(at),
            precise: false,
            phase: WheelPhase::None,
            seen,
        })
    };
    turn(c, 50.0, 0.0);
    // Within 500 ms, the pointer moved onto the other list.
    turn(c, 250.0, 0.1);
    assert_eq!(downs(c), [40.0, 40.0], "each list took the turn over it");
    // Moved within the slop over the same list: still its gesture.
    turn(c, 255.0, 0.2);
    assert_eq!(downs(c), [40.0, 80.0]);
}

/// A trackpad flick over list A lifting with no momentum, and fingers down
/// over list B 50 ms later: within the momentum's wait, but over another
/// list, so a gesture of its own, which scrolls B.
#[test]
fn a_quick_second_flick_over_another_list_is_a_gesture_of_its_own() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    c.set_flings(false);
    let seen = c.present_last();
    let event = |c: &mut Compositor, x: f32, at: f32, dy: f32, began: bool, ended: bool| {
        c.wheel_input(WheelInput {
            point: (x, 50.0),
            delta: (0.0, dy),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(began, ended),
            seen,
        });
        c.tick(at + 0.05);
    };
    event(c, 50.0, 0.0, 40.0, true, false);
    event(c, 50.0, 0.016, 40.0, false, false);
    event(c, 50.0, 0.032, 0.0, false, true);
    let a = downs(c)[0];
    assert!(a > 0.0, "A scrolled: {:?}", downs(c));
    event(c, 250.0, 0.082, 40.0, true, false);
    event(c, 250.0, 0.098, 40.0, false, false);
    assert_eq!(downs(c)[0], a, "A no further: {:?}", downs(c));
    assert!(downs(c)[1] > 0.0, "B took the flick over it: {:?}", downs(c));
}

#[test]
fn a_trackpad_gesture_resting_long_is_one_until_its_fingers_lift() {
    let mut s = Scenario::measuring(nested_lists, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let seen = c.present_last();
    let swipe = |c: &mut Compositor, at: f32, began: bool| {
        c.wheel_input(WheelInput {
            point: (20.0, 20.0),
            delta: (0.0, 100.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(began, false),
            seen,
        });
        c.tick(at + 0.05);
    };
    // Down over the inner list, to its end; resting 600 ms; moving again,
    // the fingers never lifted: the inner list's gesture still.
    swipe(c, 0.0, true);
    swipe(c, 0.016, false);
    swipe(c, 0.6, false);
    let inner_outer = |c: &Compositor| {
        let of = |outer: bool| {
            c.scene
                .compositing
                .scrollers
                .iter()
                .find(|n| n.parent.is_none() == outer)
                .map_or(-1.0, |n| c.scrolls.offset(n, 0).1)
        };
        (of(false), of(true))
    };
    assert_eq!(
        inner_outer(c).1,
        0.0,
        "the outer list does not move: {:?}",
        inner_outer(c)
    );
    // Fingers down again: a new gesture, the inner list at its end passes
    // it to the outer one.
    swipe(c, 1.0, true);
    swipe(c, 1.016, false);
    assert!(inner_outer(c).1 > 0.0, "{:?}", inner_outer(c));
}

/// macOS as winit gives it: fingers down, moving, lifting, then the
/// platform's momentum begun at once (a `Started` like the fingers'),
/// moving, ending. The momentum is the gesture's still: the inner list at
/// its end does not pass it to the outer. Fingers down after it, however
/// soon, begin another.
#[test]
fn a_trackpads_momentum_begun_as_its_fingers_lift_stays_on_the_gestures_list() {
    let mut s = Scenario::measuring(nested_lists, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    c.set_flings(false);
    let seen = c.present_last();
    let event = |c: &mut Compositor, at: f32, dy: f32, phase: WheelPhase| {
        c.wheel_input(WheelInput {
            point: (20.0, 20.0),
            delta: (0.0, dy),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase,
            seen,
        });
        c.tick(at + 0.05);
    };
    let outer = |c: &Compositor| {
        c.scene
            .compositing
            .scrollers
            .iter()
            .find(|n| n.parent.is_none())
            .map_or(-1.0, |n| c.scrolls.offset(n, 0).1)
    };
    // Fingers: the inner list to its end.
    event(c, 0.0, 100.0, WheelPhase::Began);
    event(c, 0.016, 100.0, WheelPhase::Changed);
    event(c, 0.032, 0.0, WheelPhase::Ended);
    // Momentum, begun 8 ms after the lift.
    event(c, 0.04, 60.0, WheelPhase::MomentumBegan);
    event(c, 0.056, 40.0, WheelPhase::Momentum);
    event(c, 0.2, 0.0, WheelPhase::MomentumEnded);
    assert_eq!(outer(c), 0.0, "the momentum stays on the inner list");
    // Fingers down 50 ms after the momentum ended: a new gesture, passed
    // by the inner list at its end to the outer.
    event(c, 0.25, 100.0, WheelPhase::Began);
    event(c, 0.266, 100.0, WheelPhase::Changed);
    assert!(outer(c) > 0.0, "fingers after momentum begin another gesture");
}

/// A list 100 px tall (400 px of content) holding at its top another, 60
/// px tall (200 px of content).
fn nested_lists() -> crate::element::Element {
    div()
        .width(100.0)
        .height(100.0)
        .overflow_y(Overflow::Scroll)
        .child(
            div()
                .width(100.0)
                .height(60.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(200.0)),
        )
        .child(div().height(340.0))
        .into_element()
}

#[test]
fn a_compositor_wheel_gesture_stays_on_its_container_and_the_next_passes_one_at_its_end() {
    let mut s = Scenario::measuring(nested_lists, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let (_, headless) = s.parts();
    let c = &mut headless.compositor;
    let seen = c.present_last();
    let wheel = |c: &mut Compositor, at: f32| {
        c.wheel_input(WheelInput {
            point: (20.0, 20.0),
            delta: (0.0, 100.0),
            time: crate::clock::nanos_of(at),
            precise: false,
            phase: WheelPhase::None,
            seen,
        })
    };
    let offsets = |c: &Compositor| {
        let of = |outer: bool| {
            c.scene
                .compositing
                .scrollers
                .iter()
                .find(|n| n.parent.is_none() == outer)
                .map_or(-1.0, |n| c.scrolls.offset(n, 0).1)
        };
        (of(false), of(true))
    };
    // One gesture over the inner list: to its end, and on it there.
    for at in [0.0, 0.1, 0.2] {
        wheel(c, at);
    }
    assert_eq!(offsets(c), (140.0, 0.0), "(inner, outer) within the gesture");
    // The next gesture passes the inner list, at its end, to the outer.
    assert_eq!(wheel(c, 1.0), Wheeled::Scrolled);
    assert_eq!(offsets(c), (140.0, 100.0), "(inner, outer) in a new gesture");
}

/// Keyed lists A (key 1, at the left, while `a` holds) and B (key 2).
fn keyed_lists(a: Signal<bool>) -> impl Fn() -> crate::element::Element + 'static {
    move || {
        let list = |key: u64, left: f32| {
            div()
                .key(key)
                .position(Position::Absolute)
                .left(left)
                .top(0.0)
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(400.0))
        };
        div()
            .size(pct(100.0))
            .children(a.get().then(|| list(1, 0.0)))
            .child(list(2, 200.0))
            .into_element()
    }
}

/// A scroller is the instance it is, not its key: a trackpad gesture
/// begun over list A, A gone for one commit and back with its key, then
/// the gesture going on over B. The A that came back is another instance,
/// so the gesture's latch is to nothing on screen, and A does not scroll.
#[test]
fn a_gesture_latched_to_a_list_that_left_and_came_back_by_its_key_scrolls_it_no_more() {
    let a = Signal::new(true);
    let mut s = Scenario::measuring(keyed_lists(a), 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let event = |s: &mut Scenario, x: f32, at: f32, began: bool| {
        let c = &mut s.parts().1.compositor;
        c.set_flings(false);
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (x, 50.0),
            delta: (0.0, 40.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(began, false),
            seen,
        });
        c.tick(at + 0.05);
    };
    // A is first in tree order.
    let a_key = s.parts().1.compositor.scene.compositing.scrollers[0].key;
    let a_offset = |s: &mut Scenario| {
        let c = &s.parts().1.compositor;
        c.scene
            .compositing
            .scrollers
            .iter()
            .find(|n| n.key == a_key)
            .map(|n| c.scrolls.offset(n, 0).1)
    };
    event(&mut s, 50.0, 0.0, true);
    event(&mut s, 50.0, 0.016, false);
    assert!(a_offset(&mut s).is_some_and(|y| y > 0.0), "A scrolled");
    a.set(false);
    s.frame("A gone").expect("a frame");
    assert_eq!(a_offset(&mut s), None, "A left");
    a.set(true);
    s.frame("A back").expect("a frame");
    // Back where the layout kept it, as another instance.
    let back = a_offset(&mut s);
    assert!(back.is_some(), "A is back");
    // The fingers never lifted: the same gesture, now over B.
    event(&mut s, 250.0, 0.032, false);
    event(&mut s, 250.0, 0.048, false);
    assert_eq!(a_offset(&mut s), back, "the A that came back is not the gesture's");
}

/// A list made by component `which` (two types), keyed 5: the same path
/// for either.
fn list_of_type(which: bool) -> crate::element::Element {
    use crate::component::component;
    let list = || {
        div()
            .width(100.0)
            .height(100.0)
            .overflow_y(Overflow::Scroll)
            .child(div().height(400.0))
            .into_element()
    };
    if which {
        component((), move |_| move || list()).key(5_u64).into_element()
    } else {
        component(0_u8, move |_| move || list()).key(5_u64).into_element()
    }
}

/// One component type replaced by another at the same key, in one commit,
/// while the first's list flings: the new list is another scroller, and
/// the fling ended with the list that left.
#[test]
fn a_list_whose_component_is_replaced_at_its_key_mid_fling_does_not_move() {
    let which = Signal::new(true);
    let mut s = Scenario::measuring(
        move || div().size(pct(100.0)).child(list_of_type(which.get())).into_element(),
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    c.set_flings(true);
    let seen = c.present_last();
    for (k, at) in [0.0_f32, 0.016, 0.032, 0.048].into_iter().enumerate() {
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 4.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(k == 0, k == 3),
            seen,
        });
        c.tick(at + 0.008);
    }
    c.tick(0.1);
    let flung = downs(c)[0];
    c.tick(0.15);
    assert!(downs(c)[0] > flung, "the fling moves on: {flung} then {:?}", downs(c));
    s.act("replace", || which.set(false));
    s.frame("replaced").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let swapped = downs(c)[0];
    c.tick(0.3);
    c.tick(0.5);
    assert_eq!(downs(c)[0], swapped, "the new list does not move");
    assert_eq!(swapped, 0.0, "it starts at the top, not where the old one was");
}

/// A gesture latched to a UI wheel handler that goes mid-gesture: the
/// latch is checked against what is there on each event, so the rest of
/// the gesture, over a list now, is routed afresh and scrolls it.
#[test]
fn a_gesture_whose_ui_target_goes_scrolls_the_list_it_goes_on_over() {
    let handler = Signal::new(true);
    let mut s = Scenario::measuring(
        move || {
            let list = div()
                .position(Position::Absolute)
                .left(0.0)
                .top(0.0)
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(400.0));
            let over = handler.get().then(|| {
                div()
                    .position(Position::Absolute)
                    .left(0.0)
                    .top(0.0)
                    .width(100.0)
                    .height(100.0)
                    .on_wheel(|_| true)
            });
            div().size(pct(100.0)).child(list).child(over).into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let wheel = |c: &mut Compositor, at: f32, began: bool| {
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 10.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(began, false),
            seen,
        })
    };
    let c = &mut s.parts().1.compositor;
    assert!(matches!(wheel(c, 0.0, true), Wheeled::Ui(..)), "the handler takes it");
    s.act("remove the handler", || handler.set(false));
    s.frame("gone").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let wheeled = wheel(c, 0.016, false);
    c.tick(0.05);
    assert!(
        matches!(wheeled, Wheeled::Scrolled),
        "routed afresh to the list: {wheeled:?}"
    );
    assert!(downs(c)[0] > 0.0, "the list scrolled: {:?}", downs(c));
}

/// A wheel gesture latched to a node the UI takes it on (a wheel
/// handler's) when a modal layer opens over it with a list under the
/// pointer: the latch's target is below the modal, so gone, and the
/// gesture goes on in what is at the pointer now, the modal's list.
#[test]
fn a_gesture_latched_to_a_handler_a_modal_covered_goes_on_in_the_modals_list() {
    let modal = Signal::new(false);
    let page = move || {
        let root = div()
            .size(pct(100.0))
            .child(div().width(100.0).height(100.0).on_wheel(|_| true));
        if modal.get() {
            root.child(crate::element::deferred(
                crate::element::Positioning::At { x: 0.0, y: 0.0 },
                div()
                    .layer(crate::layer::Layer::new().modal(true))
                    .width(400.0)
                    .height(100.0)
                    .child(
                        div()
                            .width(100.0)
                            .height(100.0)
                            .overflow_y(Overflow::Scroll)
                            .child(div().height(400.0)),
                    ),
            ))
        } else {
            root
        }
        .into_element()
    };
    let mut s = Scenario::measuring(page, 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let step = |c: &mut Compositor, began: bool, at: f32| {
        let seen = c.present_last();
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 30.0),
            time: crate::clock::nanos_of(at),
            precise: true,
            phase: phase_of(began, false),
            seen,
        })
    };
    assert!(matches!(step(c, true, 1.0), Wheeled::Ui(Some(_), _)), "the handler's");
    s.act("open", || modal.set(true));
    s.frame("modal").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let wheeled = step(c, false, 1.016);
    assert!(
        matches!(wheeled, Wheeled::Scrolled),
        "the modal's list took it on: {wheeled:?}"
    );
}

/// A list 200 px square, and over its top left a list 100 px square in a
/// card rounded 40 that clips it.
fn list_under_a_rounded_list() -> crate::element::Element {
    div()
        .size(pct(100.0))
        .child(
            div()
                .position(Position::Absolute)
                .left(0.0)
                .top(0.0)
                .width(200.0)
                .height(200.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(800.0)),
        )
        .child(
            div()
                .position(Position::Absolute)
                .left(0.0)
                .top(0.0)
                .width(100.0)
                .height(100.0)
                .radius(crate::style::Corners::all(40.0))
                .overflow(Overflow::Hidden)
                .child(
                    div()
                        .width(100.0)
                        .height(100.0)
                        .overflow_y(Overflow::Scroll)
                        .child(div().height(400.0)),
                ),
        )
        .into_element()
}

/// A wheel over the cut-away corner of a rounded card clipping a list, the
/// card over another list: the compositor tests the card's clip as paint
/// keeps it (corners included, [`crate::clip::keeps`]), so the list under
/// the corner takes the wheel; inside the card, its own list does.
#[test]
fn a_wheel_over_a_rounded_lists_cut_away_corner_scrolls_the_list_under_it() {
    let mut s = Scenario::measuring(list_under_a_rounded_list, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let seen = c.present_last();
    let turn = |c: &mut Compositor, at: (f32, f32), t: f32| {
        c.wheel_input(WheelInput {
            point: at,
            delta: (0.0, 40.0),
            time: crate::clock::nanos_of(t),
            precise: false,
            phase: WheelPhase::None,
            seen,
        })
    };
    turn(c, (3.0, 3.0), 0.0);
    assert_eq!(downs(c), [40.0, 0.0], "the corner: the list under it");
    // Long after, a gesture of its own, inside the card.
    turn(c, (50.0, 50.0), 5.0);
    assert_eq!(downs(c), [40.0, 40.0], "inside the card: its list");
}

/// A list under a card rounded 8 that clips, holding a wrapper of its box
/// rounded 8 that clips, holding a list that starts ten to the left of
/// them; clicks pass through the card and the wrapper, so only their clips
/// decide what is under a point.
fn list_under_two_coincident_rounded_clips() -> crate::element::Element {
    let rounded = || {
        div()
            .width(100.0)
            .height(100.0)
            .radius(crate::style::Corners::all(8.0))
            .overflow(Overflow::Hidden)
            .pass_through()
    };
    div()
        .size(pct(100.0))
        .child(
            div()
                .position(Position::Absolute)
                .left(0.0)
                .top(0.0)
                .width(200.0)
                .height(200.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(800.0)),
        )
        .child(
            rounded().position(Position::Absolute).left(20.0).top(20.0).child(
                rounded().child(
                    div()
                        .margin_left(-10.0)
                        .width(120.0)
                        .height(100.0)
                        .overflow_y(Overflow::Scroll)
                        .child(div().height(400.0)),
                ),
            ),
        )
        .into_element()
}

/// A wheel by two coincident rounded edges at x = 20: in the device pixel
/// before them, which they keep nothing of, the list under them takes it;
/// in the pixel they keep whole, the card's list, as the UI's hit testing
/// and paint have it.
#[test]
fn a_wheel_by_two_coincident_rounded_edges_goes_where_the_pixel_it_lands_in_shows() {
    let mut s = Scenario::measuring(list_under_two_coincident_rounded_clips, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let seen = c.present_last();
    let turn = |c: &mut Compositor, at: (f32, f32), t: f32| {
        c.wheel_input(WheelInput {
            point: at,
            delta: (0.0, 40.0),
            time: crate::clock::nanos_of(t),
            precise: false,
            phase: WheelPhase::None,
            seen,
        })
    };
    turn(c, (19.9, 70.0), 0.0);
    assert_eq!(downs(c), [40.0, 0.0], "the pixel before: the list under it");
    turn(c, (20.1, 70.0), 5.0);
    assert_eq!(downs(c), [40.0, 40.0], "the pixel kept whole: the card's list");
}

/// A thousand clipped rows, each narrower than their list: a wheel beside
/// them tests no row's clip chain.
#[test]
fn a_wheel_beside_a_thousand_clipped_rows_walks_no_rows_clip_chain() {
    let rows = || {
        div()
            .size(pct(100.0))
            .child(
                div()
                    .width(200.0)
                    .height(300.0)
                    .overflow_y(Overflow::Scroll)
                    .children((0..1000).map(|_| div().width(100.0).height(20.0).flex_shrink(0.0).on_click(|_| {}))),
            )
            .into_element()
    };
    let mut s = Scenario::measuring(rows, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let seen = c.present_last();
    assert!(
        c.tables(seen).is_some_and(|p| p.hits.len() > 10),
        "the rows are hit boxes"
    );
    c.wheel_input(WheelInput {
        point: (150.0, 100.0),
        delta: (0.0, 40.0),
        time: 0,
        precise: false,
        phase: WheelPhase::None,
        seen,
    });
    assert_eq!(downs(c), [40.0], "the list");
    assert_eq!(c.chain_walks(), 0, "no row's clip chain walked");
}

/// Two notches the program made at one instant (a scenario's, the clock
/// standing still): the one clock stamps them a nanosecond apart, an
/// order, not a span of time, so they measure no speed and the main frame
/// is told of no reach (10 px over 1 ns would be 10^10 px a second, a
/// list asked to build half a billion pixels ahead).
#[test]
fn notches_stamped_a_nanosecond_apart_measure_no_speed() {
    let mut s = Scrolls::default();
    let list = vec![node(1, 0.0, 1_000.0)];
    s.scroll(&list, WheelTarget(0), y(10.0), 2);
    s.scroll(&list, WheelTarget(0), y(10.0), 3);
    let tables = table(&list);
    let sent = s.reach(&tables, 3_u64, 0.5);
    assert!(sent.is_empty(), "no speed measured: {sent:?}");
    assert_eq!(take(&mut s)[0].delta, y(20.0), "both told");
}

/// A UI stalled (no commit answers) while the reader turns a notched wheel
/// 10,000 times, back and forth against the end, a key pressed every 100
/// notches (handed on with a pull, ADR-0033) and main frames pulling all
/// along (I3): each notch and each main frame costs a constant number of
/// visits, what is held is one sum a container for each hand-off the UI
/// has not committed (which its queue holds anyway) whatever the count of
/// notches, and where the list shows is where the notches, each clamped
/// as it came, put it, before the commit answering them and after.
/// Opus r11 I1: 20 000 scroll containers on a page, each with a thumb. A
/// frame the compositor composes at rest visits none of them (ADR-0032
/// I7: its work touches only containers that moved or whose table
/// changed), and one that scrolls one container visits that one: never
/// each thumb against each other (the quadratic thumb lookup took 60 ms a
/// frame here, and held input 34 ms at p99).
#[test]
fn composing_twenty_thousand_scrollers_at_rest_visits_none() {
    const N: usize = 20_000;
    let mut s = Scenario::measuring(
        || {
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .children((0..N).map(|_| {
                    div()
                        .width(4.0)
                        .height(0.01)
                        .flex_shrink(0.0)
                        .overflow_y(Overflow::Scroll)
                        .child(div().width(8.0).height(1.0))
                }))
                .into_element()
        },
        100,
        300,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    let c = s.compositor();
    assert_eq!(c.presented().scroll.len(), N, "every container shown");
    let mut at = clock::current_nanos();
    // The commit's own frame (its table changed): then at rest.
    at += 16_000_000;
    c.compose(at);
    at += 16_000_000;
    let before = super::scroll::visits();
    c.compose(at);
    let rest = super::scroll::visits() - before;
    assert!(rest <= 4, "a frame at rest visited {rest} containers");
    assert_eq!(c.presented().scroll.len(), N, "still every container shown");
    // Timed (ADR-0033's record): composes at rest, the slowest of 100.
    let mut slowest = std::time::Duration::ZERO;
    for _ in 0..100 {
        at += 16_000_000;
        let t = std::time::Instant::now();
        c.compose(at);
        slowest = slowest.max(t.elapsed());
    }
    eprintln!("{N} scrollers: a compose at rest takes at most {slowest:?}");
}

#[test]
fn a_stalled_ui_owed_ten_thousand_notches_costs_a_constant_a_notch() {
    let mut s = Scrolls::default();
    let list = vec![node(1, 0.0, 1_000.0)];
    let tables = table(&list);
    let mut expected = 0.0_f32;
    let mut pulls = 0;
    for k in 0..10_000_u64 {
        // Five down, three up: against the end and back, each clamped.
        let dy = if k % 8 < 5 { 40.0 } else { -40.0 };
        let before = super::scroll::visits();
        s.scroll(&list, WheelTarget(0), y(dy), 1_000 + k * 1_000_000);
        let work = super::scroll::visits() - before;
        assert!(work <= 4, "notch {k} cost {work} visits");
        expected = (expected + dy).clamp(0.0, 1_000.0);
        if k % 100 == 49 {
            let _ = take(&mut s);
            pulls += 1;
        }
        if k % 100 == 99 {
            let before = super::scroll::visits();
            s.reach(&tables, k * 1_000_000, 0.0);
            let _ = take(&mut s);
            pulls += 1;
            let work = super::scroll::visits() - before;
            // Its reach (two), and the one sum pulled: a constant,
            // whatever the notches.
            assert!(work <= 5, "main frame {} cost {work} visits", k / 100 + 1);
        }
        let entries = s.outbox().entries();
        assert!(entries <= pulls + 1, "{entries} sums held after {pulls} pulls");
    }
    assert_eq!(s.offset(&list[0], 0), (0.0, expected), "where the notches put it");
    // The UI took all; its commit comes with the base it reached.
    let at = vec![node(1, expected, 1_000.0)];
    commit(&mut s, &at);
    assert_eq!(s.offset(&at[0], 0), (0.0, expected), "the commit shows the same");
    assert_eq!(s.owed(1), (0.0, 0.0), "nothing owed");
}

/// The UI took half of 10,000 notches (back and forth, then past the end
/// and part way back), the commit built after that take has not come,
/// more notches came: what shows is where each, clamped as it came, put
/// it (600), and the commit answering the take shows the same, its base
/// the UI's (at 0 after the back and forth) and what came after on top,
/// each clamped in turn (where the sum clamped once would put it at 800).
#[test]
fn a_stalled_ui_owed_ten_thousand_notches_keeps_their_order_on_the_commit() {
    let mut s = Scrolls::default();
    let list = vec![node(1, 0.0, 1_000.0)];
    let mut expected = 0.0_f32;
    let mut base = 0.0_f32;
    for k in 0..10_000_u64 {
        if k == 5_000 {
            let _ = take(&mut s);
            base = expected;
        }
        let dy = match k {
            5_000..5_030 => 40.0,
            5_030..5_040 => -40.0,
            _ if k % 2 == 0 => 10.0,
            _ => -10.0,
        };
        s.scroll(&list, WheelTarget(0), y(dy), 1_000 + k * 1_000_000);
        expected = (expected + dy).clamp(0.0, 1_000.0);
    }
    assert_eq!((base, expected), (0.0, 600.0));
    assert!(s.outbox().entries() <= 4, "{} sums held", s.outbox().entries());
    assert_eq!(s.offset(&list[0], 0), (0.0, expected), "where the notches put it");
    commit(&mut s, &[node(1, base, 1_000.0)]);
    assert_eq!(s.offset(&list[0], 0), (0.0, expected), "the commit shows the same");
}

/// 20,000 scroll containers, a trackpad gesture held on the last, on the
/// compositor's own path (no wheel handler), its samples and ticks
/// interleaved with commits that move the list (the scroller table comes
/// anew with each): each sample, tick and commit costs a constant number
/// of lookups in the tables' index, and the index, made by paint with the
/// tables, is not made again while no container came or went.
#[test]
fn a_gesture_held_on_the_last_of_twenty_thousand_scrollers_costs_lookups_alone() {
    let page = || {
        div()
            .width(300.0)
            .height(300.0)
            .children((0..20_000).map(|_| {
                div()
                    .width(1.0)
                    .height(0.01)
                    .flex_shrink(0.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(1.0))
            }))
            .child(
                div()
                    .width(200.0)
                    .height(100.0)
                    .flex_shrink(0.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(5_000.0)),
            )
            .into_element()
    };
    let mut s = Scenario::measuring(page, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    s.compositor().set_flings(false);
    let tables = |s: &mut Scenario| {
        let c = &s.compositor().scene.compositing;
        (Arc::clone(&c.scrollers), Arc::clone(&c.index))
    };
    let (scrollers, index) = tables(&mut s);
    assert!(scrollers.len() > 20_000, "every container is a scroller");
    let t0 = clock::current_seconds();
    for k in 0..12_u8 {
        let (reads, visits) = (super::table_reads(), super::scroll::visits());
        let phase = if k == 0 { WheelPhase::Began } else { WheelPhase::Changed };
        let w = swipe(&mut s, (100.0, 250.0), t0 + f32::from(k) * 0.008, (0.0, 4.0), phase);
        assert!(matches!(w, Wheeled::Scrolled), "sample {k}: {w:?}");
        let reads = super::table_reads() - reads;
        assert!(reads <= 4, "sample {k} and its tick: {reads} lookups");
        // Every scroller the sample and its tick looked at, by identity
        // (ADR-0032 I7, Codex r7 #9): the one moving, not the 20,000.
        let visits = super::scroll::visits() - visits;
        assert!(visits <= 8, "sample {k} and its tick: {visits} scrollers visited");
        if k % 3 == 2 {
            let (reads, visits) = (super::table_reads(), super::scroll::visits());
            s.frame("a commit").expect("a frame");
            let reads = super::table_reads() - reads;
            // Its frame's: a main frame granted, the commit, a tick.
            assert!(reads <= 8, "commit after sample {k}: {reads} lookups");
            let visits = super::scroll::visits() - visits;
            assert!(
                visits <= 16,
                "grant, commit and tick after sample {k}: {visits} visited"
            );
        }
    }
    let (now, again) = tables(&mut s);
    assert!(!Arc::ptr_eq(&now, &scrollers), "the commits carried the list moved");
    assert!(Arc::ptr_eq(&again, &index), "the index was not made again");
    let last = downs(s.compositor()).last().copied().unwrap_or_default();
    assert!(last > 0.0, "the last list scrolled: {last}");
}

/// A trackpad gesture over a wheel handler on a page of 20,000 clickable
/// nodes, end to end: the render thread holds it on the handler and hands
/// each sample on, the UI finds the handler's node. Each sample costs a
/// constant number of lookups on both threads, not a walk of the hit
/// table or of the frame: the render thread's in the index paint made
/// with the tables.
#[test]
fn a_held_gesture_over_a_large_page_costs_a_lookup_a_sample_on_both_threads() {
    let wheeled = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    let seen_by = std::rc::Rc::clone(&wheeled);
    let page = move || {
        let seen_by = std::rc::Rc::clone(&seen_by);
        div()
            .width(300.0)
            .height(300.0)
            .children((0..20_000).map(|_| div().width(1.0).height(0.01).on_click(|_| {})))
            .child(div().width(200.0).height(100.0).on_wheel(move |_| {
                seen_by.set(seen_by.get() + 1);
                true
            }))
            .into_element()
    };
    let mut s = Scenario::measuring(page, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let seen = s.shown().seen();
    assert!(
        s.compositor().tables(seen).is_some_and(|p| p.hits.len() > 20_000),
        "every node is a hit box"
    );
    let t0 = clock::current_seconds();
    for k in 0..12_u8 {
        let c = s.compositor();
        let (held, ui) = (super::table_reads(), crate::interact::index_visits());
        let w = c.dispatch_wheel(WheelInput {
            point: (100.0, 250.0),
            delta: (0.0, 4.0),
            time: clock::nanos_of(t0 + f32::from(k) * 0.008),
            precise: true,
            phase: if k == 0 { WheelPhase::Began } else { WheelPhase::Changed },
            seen,
        });
        let super::Dispatched::Ui(input) = w else {
            panic!("sample {k} handed to the UI: {w:?}");
        };
        s.send(input);
        let held = super::table_reads() - held;
        let ui = crate::interact::index_visits() - ui;
        assert!(held <= 2, "sample {k}: the render thread made {held} lookups");
        if k >= 2 {
            assert!(ui <= 2, "sample {k}: the UI looked at {ui} nodes");
        }
    }
    assert_eq!(wheeled.get(), 12, "each sample reached the handler");
}

/// Two samples of a smooth wheel (precise, no phases: each its own input)
/// with no compositor tick between them, a jump made between the two, the
/// UI having taken neither: both land on the jump, exactly, each once
/// (Codex r6 1, by construction: nothing is split by time).
#[test]
fn smooth_samples_with_no_tick_between_land_on_a_jump_exactly() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    clock::advance_to(Duration::from_secs_f32(t0 + 0.01));
    stamped(&mut s, t0 + 0.01, (0.0, 30.0), true, WheelPhase::None);
    clock::advance_to(Duration::from_secs_f32(t0 + 0.02));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    clock::advance_to(Duration::from_secs_f32(t0 + 0.03));
    stamped(&mut s, t0 + 0.03, (0.0, 30.0), true, WheelPhase::None);
    s.frame("the jump comes").expect("a frame");
    s.compositor().tick(t0 + 0.5);
    assert_eq!(downs(s.compositor())[0], 2_060.0, "both, on the jump");
    s.frame("after").expect("a frame");
    assert_eq!(h.offset().1, 2_060.0, "the UI's too");
}

/// Fingers touch (AppKit's `MayBegin`), the program jumps the list and
/// its commit comes, then the fingers begin moving (`Began`): the moves
/// come after the jump, so they move on from it (Codex r6 4, by
/// construction: no gesture's start is compared with a jump).
#[test]
fn a_touch_then_a_jump_then_a_begin_moves_on_from_the_jump() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    let touched = swipe(&mut s, (50.0, 50.0), t0, (0.0, 0.0), WheelPhase::Touched);
    assert!(matches!(touched, Wheeled::Nothing), "a touch: {touched:?}");
    clock::advance_to(Duration::from_secs_f32(t0 + 0.01));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    s.frame("jumped").expect("a frame");
    let began = swipe(&mut s, (50.0, 50.0), t0 + 0.02, (0.0, 30.0), WheelPhase::Began);
    assert!(matches!(began, Wheeled::Scrolled), "it moves: {began:?}");
    swipe(&mut s, (50.0, 50.0), t0 + 0.03, (0.0, 30.0), WheelPhase::Changed);
    s.compositor().tick(t0 + 1.0);
    assert_eq!(downs(s.compositor())[0], 2_060.0, "on from the jump");
}

/// A precise wheel with no phases (a smooth wheel): a sample, the program
/// jumps the list, another sample, all before the commit with the jump
/// comes: neither taken by the UI when it jumped, both move the list on
/// from where it jumped.
#[test]
fn a_smooth_sample_after_a_jump_survives_its_commit() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    swipe(&mut s, (50.0, 50.0), t0 + 0.01, (0.0, 30.0), WheelPhase::None);
    clock::advance_to(Duration::from_secs_f32(t0 + 0.02));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    swipe(&mut s, (50.0, 50.0), t0 + 0.03, (0.0, 30.0), WheelPhase::None);
    s.frame("the jump comes").expect("a frame");
    s.compositor().tick(t0 + 0.2);
    let at = downs(s.compositor())[0];
    assert!(at > 2_000.0, "the second sample moved on from the jump: {at}");
    s.frame("after").expect("a frame");
    assert!(h.offset().1 > 2_000.0, "the UI's too: {:?}", h.offset());
}

/// Fingers touch the trackpad over a list without moving (a Began with no
/// delta, which reaches no scroller), the program jumps the list and its
/// commit comes, then the fingers move: the move comes after the jump, so
/// it moves on from it.
#[test]
fn a_still_touch_before_a_jump_then_a_move_moves_on_from_it() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    let touched = swipe(&mut s, (50.0, 50.0), t0, (0.0, 0.0), WheelPhase::Began);
    assert!(matches!(touched, Wheeled::Nothing), "a still touch: {touched:?}");
    clock::advance_to(Duration::from_secs_f32(t0 + 0.01));
    s.act("jump", || h.scroll_to((0.0, 2_000.0)));
    s.frame("jumped").expect("a frame");
    let moved = swipe(&mut s, (50.0, 50.0), t0 + 0.02, (0.0, 30.0), WheelPhase::Changed);
    assert!(matches!(moved, Wheeled::Scrolled), "it moves: {moved:?}");
    s.compositor().tick(t0 + 1.0);
    assert_eq!(downs(s.compositor())[0], 2_030.0, "on from the jump");
    s.frame("after").expect("a frame");
    assert_eq!(h.offset().1, 2_030.0, "the UI's too");
}

/// Fingers move a list (Began +10, Changed +30), lift (Ended) and touch
/// again (`MayBegin`) before any compositor tick resampled them: the
/// gesture's whole motion stays, shown at once and taken by the UI
/// (ADR-0032 I6, Codex r7 #5).
#[test]
fn a_gesture_ended_and_touched_again_before_a_tick_keeps_its_motion() {
    let h = crate::scroll::ScrollHandle::new();
    let (mut s, t0) = handled_list(h);
    s.compositor().set_flings(false);
    let events = [
        (10.0, WheelPhase::Began),
        (30.0, WheelPhase::Changed),
        (0.0, WheelPhase::Ended),
        (0.0, WheelPhase::Touched),
    ];
    for (k, (dy, phase)) in (0_u8..).zip(events) {
        let at = t0 + f32::from(k) * 0.008;
        clock::advance_to(Duration::from_secs_f32(at));
        stamped(&mut s, at, (0.0, dy), true, phase);
    }
    assert_eq!(downs(s.compositor())[0], 40.0, "the gesture's whole motion");
    s.frame("taken").expect("a frame");
    assert_eq!(h.offset().1, 40.0, "the UI's too");
    assert_eq!(downs(s.compositor())[0], 40.0, "and on its commit");
}

/// The UI took 30 px in and built a commit with them, which never came
/// (a render thread awaiting a full commit after a panic drops a delta,
/// ADR-0032 I4); it took 10 more, and the next commit, its base holding
/// both, says so: neither counts twice, and what it had not taken (5)
/// stays on top (Chromium's `ApplySentScrollDeltasFromAbortedCommit`).
#[test]
fn a_commit_that_never_came_leaves_its_takes_to_the_next() {
    let mut s = Scrolls::default();
    let list = vec![node(1, 0.0, 500.0)];
    s.scroll(&list, WheelTarget(0), y(30.0), 1);
    assert_eq!(take(&mut s)[0].delta, y(30.0));
    s.scroll(&list, WheelTarget(0), y(10.0), 2);
    assert_eq!(take(&mut s)[0].delta, y(10.0));
    s.scroll(&list, WheelTarget(0), y(5.0), 3);
    assert_eq!(s.offset(&list[0], 0), (0.0, 45.0), "shown before");
    let next = vec![node(1, 40.0, 500.0)];
    commit(&mut s, &next);
    assert_eq!(s.offset(&next[0], 0), (0.0, 45.0), "each once");
    assert_eq!(s.owed(1), (0.0, 5.0), "the untaken on top");
}

/// Codex r10 2: two pulls the UI has not taken, +30 then −20, and a commit
/// that shrinks the range to 10 under them. The UI takes each in turn,
/// clamped as it lands (0 → 10 → 0); the compositor shows what the UI
/// will hold, each pull clamped in the order handed on, not their sum
/// clamped once (10).
#[test]
fn pulls_outstanding_at_a_commit_land_in_order_each_clamped() {
    let mut s = Scrolls::default();
    let list = vec![node(1, 0.0, 500.0)];
    s.scroll(&list, WheelTarget(0), y(30.0), 1);
    let _ = take(&mut s);
    s.scroll(&list, WheelTarget(0), y(-20.0), 2);
    let _ = take(&mut s);
    let shrunk = vec![node(1, 0.0, 10.0)];
    s.committed(&table(&shrunk), 0, 0);
    assert_eq!(s.offset(&shrunk[0], 0), (0.0, 0.0), "as the UI will hold it");
}

/// ADR-0032's vertical slice on a 1,000,000-row virtual list, every frame
/// checked against a cold one: real input on both sides of a grant's cut;
/// a push past the top edge and back (I3); a commit aborted and the next
/// carrying its take (I4); wheel then click "Top" during a 500 ms stall,
/// then a wheel after the click (I2: the jump swallows the first, the
/// second moves from it); a jump to row 5,000 made while the wheels the
/// UI has not taken wait (I5, #2: they land on top of it, and the offset
/// and the rows go out in one commit); and what is held stays one sum a
/// container however many wheels a stall holds.
#[test]
fn a_million_row_list_is_ordered_by_what_the_ui_saw() {
    use crate::list::{ListState, virtual_list};
    let state = ListState::new(24.0);
    let page = move || {
        div()
            .width(300.0)
            .child(
                div()
                    .height(40.0)
                    .on_click(move |_| state.scroll_to_row(0))
                    .child(text("Top")),
            )
            .child(
                virtual_list(
                    &state,
                    1_000_000,
                    |i| i,
                    |i| {
                        div()
                            .height(if i % 3 == 0 { 40.0 } else { 24.0 })
                            .child(text(format!("row {i}")))
                            .into_element()
                    },
                )
                .height(360.0),
            )
            .into_element()
    };
    let mut s = Scenario::measuring(page, 300, 400, 1.0).expect("a scenario");
    s.check_cold(true);
    s.frame("first").expect("a frame");
    s.hover(150.0, 200.0);
    s.settle().expect("a frame");
    let top = |s: &mut Scenario| {
        s.settle().expect("a frame");
        s.settle().expect("a frame");
        state.view_top()
    };
    // Real input on both sides of a grant's cut: the later on top of the
    // commit answering the grant.
    assert!(s.wheel(100.0));
    s.frame_granted(|s| assert!(s.wheel(50.0))).expect("a frame");
    assert_eq!(state.view_top(), 100.0, "the frame took the wheels up to its grant");
    assert_eq!(top(&mut s), 150.0, "both, once each");
    // Wheels turned on through every frame, one before each grant and
    // one between it and the take (Codex r8 1): each frame takes all
    // before its grant, so the rows lag the screen by one wheel, never
    // more.
    for k in 1..=20_u8 {
        assert!(s.wheel(10.0));
        s.frame_granted(|s| assert!(s.wheel(10.0))).expect("a frame");
        let shown = f64::from(downs(s.compositor())[0]);
        assert_eq!(shown - state.view_top(), 10.0, "frame {k}: one wheel behind");
    }
    assert_eq!(top(&mut s), 550.0, "all of them");
    s.wheel(-400.0);
    assert_eq!(top(&mut s), 150.0);
    // Past the top edge and back: the push outward adds nothing.
    s.wheel(-1_000.0);
    s.wheel(40.0);
    assert_eq!(top(&mut s), 40.0, "from the edge");
    // A commit aborted: the next carries what the UI took for it.
    s.wheel(25.0);
    let shown = downs(s.compositor())[0];
    s.frame_aborted().expect("a frame");
    assert_eq!(
        downs(s.compositor())[0],
        shown,
        "the take kept while the full commit is awaited"
    );
    s.wheel(5.0);
    assert_eq!(top(&mut s), 70.0, "each once");
    // Wheel, then click Top, the UI stalled 500 ms: the click's jump comes
    // after the wheel; a wheel after the click moves from it.
    s.wheel(300.0);
    let down = s.queue(crate::window::Input::MouseDown { x: 150.0, y: 20.0 });
    let up = s.queue(crate::window::Input::MouseUp { x: 150.0, y: 20.0 });
    s.wheel(40.0);
    clock::advance_to(Duration::from_nanos(clock::current_nanos() + 500_000_000));
    s.handle(down);
    s.handle(up);
    assert_eq!(top(&mut s), 40.0, "at Top, the wheel after on it");
    // A jump to row 5,000 while wheels the UI has not taken wait, many of
    // them: they land on top of it, and what is held stays bounded.
    s.hover(150.0, 200.0);
    for _ in 0..1_000 {
        s.wheel(1.0);
        s.wheel(-1.0);
    }
    s.wheel(20.0);
    let held = s.compositor().scrolls.outbox().entries();
    assert!(held <= 4, "{held} sums held for 2,001 wheels");
    s.act("to row 5,000", || state.scroll_to_row(5_000));
    s.wheel(10.0);
    let got = top(&mut s);
    assert_eq!(got, state.offset_of(5_000) + 30.0, "the jump, the wheels on top");
    let h = state.scroll().expect("a scroll container");
    assert_eq!(
        downs(s.compositor())[0],
        h.offset().1,
        "the offset shown is the one its rows were built for"
    );
}

/// The compositor's own motion: `by` its fling moved container `id`,
/// generation `generation`, not yet pulled.
fn flung_by(s: &mut Scrolls, id: u64, generation: u64, by: f32) {
    let m = s.outbox().pending.entry(id, NodeKey(id), None);
    m.generation = generation;
    m.synthetic.1 = m.synthetic.1.saturating_add(motion::units(by));
}

/// Codex r8 4: a fling ran +30 the UI has not taken, and the range shrank
/// to 10 under it. What survives the clamp is still the fling's, so the
/// UI, which jumped the list to Top before it took it, drops it
/// (`take_scroll`) and the list stays at Top.
#[test]
fn a_fling_cut_by_a_range_that_shrank_is_still_the_flings() {
    let mut s = Scrolls::default();
    flung_by(&mut s, 1, 0, 30.0);
    let nodes = vec![node(1, 0.0, 10.0)];
    s.committed(&table(&nodes), 0, 0);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 10.0), "clamped to the range");
    let took = take(&mut s);
    assert_eq!(took.len(), 1);
    assert_eq!(
        (took[0].delta, took[0].synthetic),
        (y(10.0), y(10.0)),
        "all of it the fling's"
    );
}

/// Codex r8 5: a trackpad gesture from 80 in a range of 100, its
/// prediction resampled past the end. What the edge stopped is not the
/// gesture's to settle, so the content rests at the end, not 5 px back
/// from it, and a turn back moves it from the end at once.
#[test]
fn a_prediction_past_an_edge_rests_at_the_edge() {
    let mut s = Scrolls::default();
    let nodes = vec![node(1, 80.0, 100.0)];
    s.gesture(&nodes, Some(WheelTarget(0)), y(10.0), (0.0, 0), false);
    s.tick(&table(&nodes), 0.0);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 90.0));
    s.gesture(&nodes, Some(WheelTarget(0)), y(10.0), (0.010, 0), false);
    // Resampled 5 ms ahead: 25 asked, 20 moved, the end stops the rest.
    s.tick(&table(&nodes), 0.015);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 100.0));
    // Past its prediction, and settled: it stays at the end.
    s.tick(&table(&nodes), 0.1);
    s.stop_gesture(clock::Moment::nanos(0.1));
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 100.0), "at the end");
    s.gesture(&nodes, Some(WheelTarget(0)), y(-10.0), (0.2, 1), false);
    s.tick(&table(&nodes), 0.2);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 90.0), "a turn back moves from the end");
}

/// Codex r8 6: 5,000 containers moved in one run cost a commit a few
/// visits apiece, not one per pair of them, under the outbox's lock.
#[test]
fn a_commit_of_five_thousand_moved_containers_is_linear() {
    let mut s = Scrolls::default();
    let nodes: Vec<_> = (1..=5_000).map(|k| node(k, 0.0, 500.0)).collect();
    for k in 1..=5_000 {
        flung_by(&mut s, k, 0, 1.0);
    }
    let tables = table(&nodes);
    let before = super::scroll::visits();
    s.committed(&tables, 0, 0);
    let work = super::scroll::visits() - before;
    assert!(work <= 20 * 5_000, "{work} visits for 5,000 containers");
    assert_eq!(s.offset(&nodes[4_999], 0), (0.0, 1.0));
}

/// Opus r8 nit: the render thread panicked and a new compositor took the
/// outbox over (`Scrolls::with`), with none of the old one's state. A
/// commit that jumped the list still stops the fling the UI had not
/// taken: the jump is told from the outbox's own entries.
#[test]
fn a_compositor_rebuilt_after_a_panic_still_tells_a_jump() {
    let mut old = Scrolls::default();
    flung_by(&mut old, 1, 0, 30.0);
    let Accepted { outbox, fling, flings } = old.into_accepted();
    let mut s = Scrolls::with(outbox, fling, flings);
    let mut nodes = vec![node(1, 200.0, 500.0)];
    nodes[0].generation = 1;
    s.committed(&table(&nodes), 0, 0);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 200.0), "the fling stopped at the jump");
}

/// Codex r8 2, under ADR-0033: a wheel over a list whose handler declines
/// it (the render thread hands it to the UI), then a click on "Top",
/// while the UI is stalled. The platform sends both to the render thread,
/// which hands them on in the order they came, down the one queue the UI
/// takes them from: the wheel's fallback scroll first, then the click's
/// jump, and the list ends at Top. (Under ADR-0032 the click went to the
/// UI straight and could arrive first.)
#[test]
fn a_click_after_a_wheel_handed_to_the_ui_is_handled_after_it() {
    use std::sync::{Arc, Mutex, PoisonError};

    use super::thread::{FromRender, Pace, RenderThread, ToRender};
    use crate::window::Input;
    let h = crate::scroll::ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(300.0)
                .child(
                    div()
                        .height(40.0)
                        .on_click(move |_| h.scroll_to((0.0, 0.0)))
                        .child(text("Top")),
                )
                .child(
                    div()
                        .height(100.0)
                        .overflow_y(Overflow::Scroll)
                        .track_scroll(&h)
                        .on_wheel(|_| false)
                        .child(div().height(5_000.0)),
                )
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.hover(150.0, 80.0);
    s.frame("hovered").expect("a frame");
    let shown = s.shown();
    let Seen::Frame(frame) = shown.seen() else {
        panic!("a frame presented")
    };
    let compositor = std::mem::replace(&mut s.parts().1.compositor, Compositor::new(Glyphs::for_os()));
    let Ok(gpu) = crate::gpu::GpuRenderer::headless() else {
        eprintln!("no GPU: skipped");
        return;
    };
    let handed: Arc<Mutex<Vec<FromRender>>> = Arc::default();
    let to_ui = Arc::clone(&handed);
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut thread = RenderThread::new(
        compositor,
        gpu,
        rx,
        Box::new(move |m| to_ui.lock().unwrap_or_else(PoisonError::into_inner).push(m)),
        Duration::from_millis(1),
        Pace::Ticker(Duration::from_millis(1)),
        None,
    );
    let at = Instant::now();
    thread.handle(ToRender::Wheel {
        point: (150.0, 80.0),
        delta: (0.0, 30.0),
        at,
        sent: at,
        precise: false,
        phase: WheelPhase::None,
        shown: Seen::Frame(frame),
        event: None,
    });
    for input in [
        Input::MouseDown { x: 150.0, y: 20.0 },
        Input::MouseUp { x: 150.0, y: 20.0 },
    ] {
        thread.handle(ToRender::Input {
            input,
            at,
            sent: at,
            shown: Seen::Frame(frame),
            event: None,
        });
    }
    // The UI, stalled until now, takes its queue in order.
    let mut handled = Vec::new();
    for m in std::mem::take(&mut *handed.lock().unwrap_or_else(PoisonError::into_inner)) {
        match m {
            FromRender::Wheel { input, pulled, .. } | FromRender::Input { input, pulled, .. } => {
                handled.push(if matches!(input, Input::WheelAt { .. }) {
                    "wheel"
                } else {
                    "click"
                });
                s.pipeline_mut().take_pulled(&pulled);
                s.pipeline_mut().input_shown(input, &shown);
            }
            _ => {}
        }
    }
    assert_eq!(handled, ["wheel", "click", "click"], "in the platform's order");
    assert_eq!(h.offset(), (0.0, 0.0), "at Top");
}

/// Codex r9 I1: list A is wheeled 40, the UI (behind) removes it and mounts
/// list B at the same key and owner before it takes the wheel in. The
/// wheel moved A, a mount gone: B takes none of it, in the UI and on
/// screen.
#[test]
fn remounted_list_does_not_take_previous_lists_wheel() {
    use crate::window::Input;
    let stage = Signal::new(0_u8);
    let a = crate::scroll::ScrollHandle::new();
    let b = crate::scroll::ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            let root = div().width(200.0);
            if stage.get() == 1 {
                return root.into_element();
            }
            root.child(
                div()
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .track_scroll(if stage.get() == 0 { &a } else { &b })
                    .child(div().height(1_000.0)),
            )
            .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.settle().expect("a frame");
    s.hover(50.0, 50.0);
    let old = || Input::Modifiers(crate::events::Modifiers::default());
    let hide = s.queue(old());
    let show = s.queue(old());
    assert!(s.wheel(40.0));
    s.act("remove A", || stage.set(1));
    s.handle(hide);
    assert!(
        s.last()
            .nodes
            .iter()
            .all(|n| n.scroll(crate::inspect::OUTSIDE).is_none())
    );
    s.act("mount B", || stage.set(2));
    s.handle(show);
    assert!(
        s.last()
            .nodes
            .iter()
            .any(|n| n.scroll(crate::inspect::OUTSIDE).is_some())
    );
    assert_eq!(b.offset().1, 0.0);
    s.send(old());
    s.settle().expect("a frame");
    assert_eq!((b.offset().1, s.presented().scroll[0].offset.1), (0.0, 0.0));
}

/// Codex r9 I2: 256 containers moved one wheel each, each in a pull of its
/// own (an input handed on between every two wheels, ADR-0033), none
/// committed: a commit visits each entry a few times, not every pull for
/// each container.
#[test]
fn a_commit_of_sparse_runs_is_linear_in_entries() {
    let mut s = Scrolls::default();
    let nodes: Vec<_> = (1..=256).map(|k| node(k, 0.0, 500.0)).collect();
    for k in 1..=256_u64 {
        flung_by(&mut s, k, 0, 1.0);
        let _ = take(&mut s);
    }
    assert_eq!(s.outbox().sent.len(), 256);
    let tables = table(&nodes);
    let before = super::scroll::visits();
    s.committed(&tables, 0, 0);
    let work = super::scroll::visits() - before;
    assert!(work <= 8 * 256, "{work} visits for 256 entries in 256 pulls");
    assert_eq!(s.offset(&nodes[255], 0), (0.0, 1.0));
}

/// The fault matrix's finding: the compositor made afresh after a render
/// panic draws on the timeline the one before drew on, and the frames
/// that one presented come back to its pool as the timeline lets them
/// go. Their scroll lists were versioned, and their property trees
/// numbered, by the counters of the one before, which the new one runs
/// past again: a frame reused so kept a list from before the panic when
/// its version matched (the lists shown where they were before it). Each
/// frame shows the lists as composed, before the panic and after.
#[test]
fn a_frame_drawn_before_a_render_panic_is_never_reused_by_its_numbers() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let mut at = 8.0_f32;
    // Twelve frames 62.5 ms apart (the timeline keeps five), the first
    // `wheels` of them scrolled.
    let scroll = |s: &mut Scenario, at: &mut f32, wheels: usize| {
        for n in 0..12 {
            let c = &mut s.parts().1.compositor;
            let seen = c.present_last();
            if n < wheels {
                c.wheel_input(WheelInput {
                    point: (50.0, 50.0),
                    delta: (0.0, 10.0),
                    time: crate::clock::nanos_of(*at),
                    precise: false,
                    phase: WheelPhase::None,
                    seen,
                });
            }
            c.compose(*at);
            c.did_present();
            let p = c.presented();
            assert_eq!(
                p.scroll.to_vec(),
                c.scroll_shown.list,
                "frame {} shows the lists as composed",
                p.frame
            );
            *at += 1.0 / 16.0;
        }
    };
    // Two lists' worth of change, then frames at rest: every frame the
    // timeline keeps holds the same version.
    scroll(&mut s, &mut at, 2);
    // The render thread panics: one made afresh on the same timeline
    // takes in the UI's full commit.
    {
        let c = &mut s.parts().1.compositor;
        let accepted = c.take_accepted();
        *c = Compositor::after(Glyphs::for_os(), c.timeline(), accepted);
    }
    s.pipeline_mut().recommit();
    crate::clock::advance_to(Duration::from_secs_f32(at));
    s.frame("after the panic").expect("a frame");
    // Its versions run past those as it scrolls on.
    scroll(&mut s, &mut at, 12);
}

/// A list 100 tall over `extent`, jumped to `start`.
fn a_list_at(start: f32, extent: f32) -> Scenario {
    let h = crate::ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&h)
                .child(div().height(extent))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    s.act("jump", || h.scroll_to((0.0, start)));
    s.settle().expect("a frame");
    // A jump lands on the grid ([`motion`]).
    assert_eq!(h.offset().1, motion::px(motion::units(start)));
    s
}

/// A trackpad step of `dy` at `t` nanoseconds over the list.
fn pad_at(s: &mut Scenario, dy: f32, t: u64, phase: WheelPhase) {
    let seen = Seen::Frame(s.presented().frame);
    s.compositor().wheel_input(WheelInput {
        point: (50.0, 50.0),
        delta: (0.0, dy),
        time: t,
        precise: true,
        phase,
        seen,
    });
}

/// Where the compositor shows the list at `t`.
fn shown_at(s: &mut Scenario, t: u64) -> f32 {
    s.compositor().compose(t);
    s.compositor()
        .presented()
        .scroll
        .first()
        .map_or(f32::NAN, |l| l.offset.1)
}

/// Codex r13 1, Opus r13 I1: a fling the compositor runs, settled by a
/// fault (a panic's divert, a frame not drawn) one tick after it began,
/// ends where the fling with no fault does: settling hands the fling on, it
/// never stops it (failed: 53.01 for 295.25).
#[test]
fn a_fling_settled_by_a_fault_ends_where_the_fault_free_fling_does() {
    let ms = |n: u64| n * 1_000_000;
    let run = |fault: bool| {
        let mut s = a_list_at(0.0, 40_000_000.0);
        s.compositor().set_flings(true);
        for (i, phase) in (0_u64..).zip([
            WheelPhase::Began,
            WheelPhase::Changed,
            WheelPhase::Changed,
            WheelPhase::Ended,
        ]) {
            pad_at(&mut s, 10.0, ms(16) * i, phase);
        }
        s.compositor().tick(ms(64));
        if fault {
            s.compositor().settle_gesture(ms(65));
        }
        for k in 5..150_u64 {
            s.compositor().tick(ms(16) * k);
        }
        shown_at(&mut s, ms(2_400))
    };
    let (faulted, free) = (run(true), run(false));
    assert!(free > 200.0, "it flings: {free}");
    assert_eq!(faulted, free);
}

/// The fault matrix (round 13): a gesture settled after each event (no
/// frame drawn: the device lost, the window hidden) still flings as it is
/// lifted, as fast as its events went, and ends where it does with frames
/// (failed: 40 for 294.58, the lift flinging nothing).
#[test]
fn a_gesture_settled_with_its_fingers_down_still_flings_as_they_lift() {
    let ms = |n: u64| n * 1_000_000;
    let run = |settle: bool| {
        let mut s = a_list_at(0.0, 40_000_000.0);
        s.compositor().set_flings(true);
        for (i, phase) in (0_u64..).zip([
            WheelPhase::Began,
            WheelPhase::Changed,
            WheelPhase::Changed,
            WheelPhase::Ended,
        ]) {
            pad_at(&mut s, 10.0, ms(16) * i, phase);
            if settle {
                s.compositor().settle_gesture(ms(16) * i + 1);
            } else {
                s.compositor().tick(ms(16) * i + 1);
            }
        }
        for k in 4..200_u64 {
            s.compositor().tick(ms(16) * k);
        }
        shown_at(&mut s, ms(3_200))
    };
    let (settled, drawn) = (run(true), run(false));
    assert!(drawn > 200.0, "it flings: {drawn}");
    assert_eq!(settled, drawn);
}

/// The fault matrix (round 13): a fling no frame moved (the device lost)
/// that the next gesture stops is brought up to that gesture's time, as a
/// fling drawn every frame up to it is (failed: 30 for 148.46, the fling
/// moving nothing).
#[test]
fn a_fling_no_frame_moved_stops_where_a_drawn_one_does() {
    let ms = |n: u64| n * 1_000_000;
    let run = |draw: bool| {
        let mut s = a_list_at(0.0, 40_000_000.0);
        s.compositor().set_flings(true);
        for (i, phase) in (0_u64..).zip([WheelPhase::Began, WheelPhase::Changed, WheelPhase::Ended]) {
            pad_at(
                &mut s,
                if phase == WheelPhase::Ended { 0.0 } else { 15.0 },
                ms(16) * i,
                phase,
            );
        }
        if draw {
            for k in 3..25_u64 {
                s.compositor().tick(ms(8) * k);
            }
        }
        pad_at(&mut s, 0.0, ms(200), WheelPhase::Began);
        shown_at(&mut s, ms(200))
    };
    let (undrawn, drawn) = (run(false), run(true));
    assert!(drawn > 100.0, "it flung: {drawn}");
    assert_eq!(undrawn, drawn);
}

/// The fault matrix (round 13): a gesture handed to the UI with its latch
/// (input diverted) and lifted there flings, once handed back, to where
/// the gesture lifted on the compositor does: its motion goes with its
/// latch both ways (failed: 30 for 216.88, the lift flinging nothing).
#[test]
fn a_gesture_lifted_while_the_ui_routes_it_flings_as_one_lifted_here() {
    let ms = |n: u64| n * 1_000_000;
    let run = |handed: bool| {
        let mut s = a_list_at(0.0, 40_000_000.0);
        s.compositor().set_flings(true);
        for (i, phase) in (0_u64..).zip([WheelPhase::Began, WheelPhase::Changed, WheelPhase::Changed]) {
            pad_at(&mut s, 10.0, ms(16) * i, phase);
            s.compositor().tick(ms(16) * i + 1);
        }
        if handed {
            // Diverted: its motion so far and its latch go to the UI,
            // which takes the lift and hands them back.
            s.compositor().settle_gesture(ms(40));
            let mut g = s.compositor().hand_gesture().expect("a gesture latched");
            let mut motion = g.2.take().expect("its motion with it");
            let to = motion.to.expect("the container it moved");
            motion.record(motion.began, ms(48), (0, 0), true, to);
            // The platform's momentum after the lift is no part of its
            // speed (the matrix's [(6, held loss), (7, panic)]: 1,629.56
            // for 1,467.06; here 271.04 for 216.88).
            motion.record(motion.began, ms(52), y(8.0), false, to);
            g.2 = Some(motion);
            s.compositor().take_gesture(g);
        } else {
            pad_at(&mut s, 0.0, ms(48), WheelPhase::Ended);
        }
        for k in 4..250_u64 {
            s.compositor().tick(ms(16) * k);
        }
        shown_at(&mut s, ms(4_000))
    };
    let (handed, here) = (run(true), run(false));
    assert!(here > 200.0, "it flings: {here}");
    assert_eq!(handed, here);
}

/// The fault matrix (round 13): a fling ends at the same place whichever
/// frame finds it stopped, so a run whose frames fell otherwise (a fault,
/// a slower machine) ends where any does (failed: 294.71 every 7 ms,
/// 295.25 every 16 ms).
#[test]
fn a_fling_ends_at_the_same_place_whatever_its_frames() {
    let ms = |n: u64| n * 1_000_000;
    let run = |every: u64| {
        let mut s = a_list_at(0.0, 40_000_000.0);
        s.compositor().set_flings(true);
        for (i, phase) in (0_u64..).zip([
            WheelPhase::Began,
            WheelPhase::Changed,
            WheelPhase::Changed,
            WheelPhase::Ended,
        ]) {
            pad_at(&mut s, 10.0, ms(16) * i, phase);
        }
        let mut t = ms(48);
        while t < ms(3_000) {
            t += every;
            s.compositor().tick(t);
        }
        shown_at(&mut s, ms(3_000))
    };
    let (sixteen, seven, odd) = (run(ms(16)), run(ms(7)), run(ms(16) + 333_333));
    assert!(sixteen > 200.0, "it flings: {sixteen}");
    assert_eq!(seven, sixteen);
    assert_eq!(odd, sixteen);
}

/// Codex r13 3: motion finer than any grid is carried, never lost: 1,024
/// steps of 1/1,024 px move the list 1 px (failed: 0).
#[test]
fn steps_finer_than_a_pixel_add_up_exactly() {
    let mut s = a_list_at(0.0, 40_000_000.0);
    for i in 0..1024_u64 {
        let phase = if i == 0 { WheelPhase::Began } else { WheelPhase::Changed };
        pad_at(&mut s, 1.0 / 1024.0, i * 1_000_000, phase);
    }
    s.compositor().settle_gesture(1_024_000_000);
    assert_eq!(shown_at(&mut s, 1_024_000_000), 1.0);
}

/// Codex r13 4: deep in a long list (36,000,000 px, where an f32 steps by
/// 4 px), forty 1 px steps move it 40 px whether a frame falls between
/// them or not (failed: 36,000,000 with a frame each, 36,000,040 without).
#[test]
fn deep_in_a_long_list_every_step_moves_it_whatever_the_frames() {
    let ms = |n: u64| n * 1_000_000;
    let run = |frames: bool| {
        let mut s = a_list_at(36_000_000.0, 40_000_000.0);
        for i in 0..40_u64 {
            let t = ms(16) * i;
            pad_at(
                &mut s,
                1.0,
                t,
                if i == 0 { WheelPhase::Began } else { WheelPhase::Changed },
            );
            if frames {
                s.compositor().tick(t);
            }
        }
        s.compositor().settle_gesture(ms(640));
        shown_at(&mut s, ms(640))
    };
    assert_eq!(run(true), 36_000_040.0);
    assert_eq!(run(false), 36_000_040.0);
}

/// Codex r13 4: from a jump to 0.1 px (which lands on the grid, the
/// nearest unit), a gesture out and back settles exactly where the jump
/// put it whether a frame fell between its steps or not (failed:
/// 0.099998474 for 0.1).
#[test]
fn a_gesture_out_and_back_from_a_jump_settles_where_it_began_whatever_the_frames() {
    let run = |draw: bool| {
        let mut s = a_list_at(0.1, 40_000_000.0);
        pad_at(&mut s, 32.0, 0, WheelPhase::Began);
        if draw {
            s.compositor().tick(0_u64);
        }
        pad_at(&mut s, -32.0, 16_000_000, WheelPhase::Changed);
        s.compositor().settle_gesture(32_000_000);
        shown_at(&mut s, 32_000_000)
    };
    assert_eq!(run(true), run(false));
    assert_eq!(run(true), motion::px(motion::units(0.1)));
}

/// Codex r13 2: past the render thread's last recovery the UI is the one
/// owner of input and takes it straight (ADR-0033, `Failed`): it does not
/// quit (failed: the UI never took the `PointerLeft`).
#[test]
fn past_the_last_recovery_the_ui_still_takes_input() {
    use crate::window::Input;
    let Ok(mut w) = crate::app::offscreen(
        || || div().into_element(),
        crate::app::RunOptions::default(),
        (100, 100),
        1.0,
        Duration::from_millis(8),
    ) else {
        eprintln!("no GPU: skipped");
        return;
    };
    assert!(wait_for(|_| w.presented().frame > 0), "a first frame");
    for _ in 0..4 {
        w.panic_render();
    }
    assert!(wait_for(|_| !w.through()), "input goes straight to the UI");
    drop(w.capture_within(Duration::from_secs(5)));
    drop(w.take_input_latencies());
    w.input(Input::PointerLeft);
    let took = wait_for(|_| w.take_input_latencies().iter().any(|(i, _)| *i == Input::PointerLeft));
    assert!(took, "the UI took no input after the render thread's last recovery");
}

/// Codex r14 1: past the render thread's last recovery nothing waits for a
/// frame the render thread grants, as none comes: a move with the button
/// held, a key's repeat and a wheel each reach the app, and what the wheel
/// scrolled is laid out, on the UI's own frames (failed: the held move
/// never came, `Err(Timeout)` for 40).
#[test]
fn past_the_last_recovery_a_drag_a_key_and_a_wheel_go_on_with_no_grant() {
    use crate::events::{Key, KeyEvent, Modifiers, NamedKey};
    use crate::window::Input;
    use std::sync::mpsc;

    #[derive(Debug, PartialEq)]
    enum Heard {
        Moved(f32),
        Key,
        Click(f32),
    }
    let (tx, rx) = mpsc::channel();
    let Ok(mut w) = crate::app::offscreen(
        move || {
            let h = crate::scroll::ScrollHandle::new();
            move || {
                let (moved, key, click) = (tx.clone(), tx.clone(), tx.clone());
                div()
                    .width(100.0)
                    .height(100.0)
                    .focusable()
                    .overflow_y(Overflow::Scroll)
                    .track_scroll(&h)
                    .on_pointer_move(move |e| {
                        moved.send(Heard::Moved(e.x)).ok();
                    })
                    .on_key_down(move |_| {
                        key.send(Heard::Key).ok();
                        true
                    })
                    .on_click(move |_| {
                        click.send(Heard::Click(h.offset().1)).ok();
                    })
                    .child(div().height(400.0))
                    .into_element()
            }
        },
        crate::app::RunOptions::default(),
        (100, 100),
        1.0,
        Duration::from_millis(8),
    ) else {
        eprintln!("no GPU: skipped");
        return;
    };
    assert!(wait_for(|_| w.presented().frame > 0), "a first frame");
    for _ in 0..4 {
        w.panic_render();
    }
    assert!(wait_for(|_| !w.through()), "input goes straight to the UI");
    drop(w.capture_within(Duration::from_secs(5)));
    let heard = |want: &dyn Fn(&Heard) -> bool| {
        let until = std::time::Instant::now() + Duration::from_secs(1);
        while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(h) if want(&h) => return Some(h),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
        None
    };
    w.input(Input::PointerMoved { x: 30.0, y: 30.0 });
    assert_eq!(heard(&|h| matches!(h, Heard::Moved(_))), Some(Heard::Moved(30.0)));
    w.input(Input::MouseDown { x: 30.0, y: 30.0 });
    w.input(Input::PointerMoved { x: 40.0, y: 30.0 });
    assert_eq!(
        heard(&|h| matches!(h, Heard::Moved(_))),
        Some(Heard::Moved(40.0)),
        "the held move, with no grant to come"
    );
    w.input(Input::MouseUp { x: 40.0, y: 30.0 });
    assert!(heard(&|h| matches!(h, Heard::Click(_))).is_some(), "the click");
    w.input(Input::KeyRepeat(KeyEvent {
        key: Key::Named(NamedKey::ArrowDown),
        text: None,
        modifiers: Modifiers::default(),
    }));
    assert_eq!(heard(&|h| *h == Heard::Key), Some(Heard::Key), "the key's repeat");
    w.wheel((0.0, 40.0), false);
    std::thread::sleep(Duration::from_millis(100));
    w.input(Input::MouseDown { x: 40.0, y: 30.0 });
    w.input(Input::MouseUp { x: 40.0, y: 30.0 });
    let click = heard(&|h| matches!(h, Heard::Click(_)));
    assert!(
        matches!(click, Some(Heard::Click(y)) if y > 0.0),
        "the wheel scrolled, laid out on the UI's frames: {click:?}"
    );
}

/// Codex r13 5: a mailbox whose scope is disposed and last sender dropped
/// lets go of its handler at once, not at a delivery to its address that
/// never comes (failed: what only the handler held was still alive).
#[test]
fn a_retired_mailbox_lets_go_of_its_handler_at_once() {
    use std::sync::{Arc, mpsc};
    let (tx, rx) = mpsc::channel();
    let Ok(_w) = crate::app::offscreen(
        move || {
            let scope = crate::reactive::Scope::new();
            let owned = Arc::new(());
            let retired = Arc::downgrade(&owned);
            let old = scope.run(|| {
                crate::mailbox(move |()| {
                    let _ = &owned;
                })
            });
            drop(old);
            scope.dispose();
            let (answer, replied) = mpsc::channel();
            let probe = crate::mailbox(move |()| {
                answer.send(retired.upgrade().is_some()).ok();
            });
            tx.send((probe, replied)).ok();
            || div().into_element()
        },
        crate::app::RunOptions::default(),
        (100, 100),
        1.0,
        Duration::from_millis(8),
    ) else {
        eprintln!("no GPU: skipped");
        return;
    };
    let (probe, replied) = rx.recv_timeout(Duration::from_secs(5)).expect("the page's probe");
    assert!(probe.send(()));
    assert_eq!(
        replied.recv_timeout(Duration::from_secs(5)).ok(),
        Some(false),
        "the retired handler's capture is gone"
    );
}
