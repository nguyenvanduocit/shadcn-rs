//! ADR-0015: the compositor draws what the UI committed, scrolls and moves
//! by itself between commits without losing or doubling a pixel, and input
//! lands on what was on screen.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::scroll::{Scrolls, WheelTarget};
use super::*;
use crate::app::Scenario;
use crate::clock::{self, Vsync};
use crate::color::Color;
use crate::element::{IntoElement, div, text};
use crate::prelude::*;
use crate::reactive::Signal;
use crate::scene::Properties;

fn node(key: u64, offset: f32, max: f32) -> ScrollNode {
    ScrollNode {
        key: NodeKey(key),
        owner: None,
        instance: key,
        content: 1,
        offset: (0.0, offset),
        applied: (0.0, -offset),
        max: (0.0, max),
        rtl: false,
        generation: 0,
        thumb: None,
        parent: None,
        wheel: false,
        wheel_above: false,
        below_modal: false,
    }
}

#[test]
fn a_scroll_is_owed_until_a_commit_takes_it_in_and_never_twice() {
    let mut s = Scrolls::default();
    let mut nodes = vec![node(1, 0.0, 500.0)];
    assert!(s.scroll(&nodes, WheelTarget(0), (0.0, 30.0), 0.0));
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 30.0), "shown at once");
    // Main frame 1 takes the 30 px; 10 more come while the UI lays out.
    let sent = s.send(&nodes, 1, 0.0, 0.0);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].delta, (0.0, 30.0));
    assert!(s.scroll(&nodes, WheelTarget(0), (0.0, 10.0), 0.0));
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 40.0));
    // A commit that answers an older frame takes nothing off.
    s.committed(&nodes, 0);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 40.0), "nothing lost");
    // Commit 1 has the 30 px in its offset: only the 10 are still owed.
    nodes[0].offset.1 = 30.0;
    s.committed(&nodes, 1);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 40.0), "nothing doubled");
    let sent = s.send(&nodes, 2, 0.0, 0.0);
    assert_eq!(sent[0].delta, (0.0, 10.0));
    nodes[0].offset.1 = 40.0;
    s.committed(&nodes, 2);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 40.0));
    assert!(s.send(&nodes, 3, 0.0, 0.0).is_empty(), "nothing owed");
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
    assert!(s.scroll(&list_a, WheelTarget(0), (0.0, 30.0), 0.0));
    let sent = s.send(&list_a, 1, 0.0, 0.0);
    assert_eq!((sent[0].owner, sent[0].delta), (a, (0.0, 30.0)), "A's delta names A");
    // More while the UI lays out; its commit has B where A was.
    assert!(s.scroll(&list_a, WheelTarget(0), (0.0, 10.0), 0.0));
    let list_b = vec![ScrollNode {
        owner: b,
        instance: 2,
        ..node(1, 0.0, 500.0)
    }];
    s.committed(&list_b, 0);
    assert_eq!(s.offset(&list_b[0], 0), (0.0, 0.0), "B is shown at its own offset");
    assert!(s.send(&list_b, 2, 0.0, 0.0).is_empty(), "and is sent nothing of A's");
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
            assert!(s.scroll(&first, WheelTarget(0), (0.0, 10.0), f32::from(k) * 0.01));
        }
        let sent = s.send(&first, 1, 0.04, 0.1);
        assert!(sent[0].reach.1 > 0.0, "a list scrolling has a reach");
        assert!(s.thumb_opacity(&first[0], 0.04) > 0.0, "and its scrollbar shows");
        if hidden {
            s.committed(&[], 1);
            assert!(!s.animating(0.05), "hidden, its scrollbar is gone");
        }
        let again = vec![ScrollNode {
            owner,
            instance: 2,
            ..node(1, 0.0, 500.0)
        }];
        s.committed(&again, 1);
        assert!(!s.animating(0.05), "the first one's scrollbar is gone ({hidden})");
        assert_eq!(s.thumb_opacity(&again[0], 0.05), 0.0, "shown again, no scrollbar");
        assert!(s.send(&again, 2, 0.05, 0.1).is_empty(), "nor any reach ({hidden})");
    }
}

#[test]
fn the_program_setting_an_offset_drops_what_the_compositor_scrolled() {
    let mut s = Scrolls::default();
    let mut nodes = vec![node(1, 0.0, 500.0)];
    assert!(s.scroll(&nodes, WheelTarget(0), (0.0, 30.0), 0.0));
    let _sent = s.send(&nodes, 1, 0.0, 0.0);
    // The program scrolled to 200 meanwhile (`scroll_to`): it wins.
    nodes[0].offset.1 = 200.0;
    nodes[0].generation = 1;
    s.committed(&nodes, 0);
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 200.0));
}

#[test]
fn a_scroll_stops_at_the_edges() {
    let mut s = Scrolls::default();
    let nodes = vec![node(1, 490.0, 500.0)];
    assert!(s.scroll(&nodes, WheelTarget(0), (0.0, 30.0), 0.0));
    assert_eq!(s.offset(&nodes[0], 0), (0.0, 500.0));
    assert!(!s.scroll(&nodes, WheelTarget(0), (0.0, 30.0), 0.0), "at the end");
    assert_eq!(
        s.send(&nodes, 1, 0.0, 0.0)[0].delta,
        (0.0, 10.0),
        "only what moved is owed"
    );
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
    p.push(0.000, (0.0, 10.0));
    p.push(0.010, (0.0, 10.0));
    p.push(0.020, (0.0, 10.0));
    let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
    assert!(close(p.at(0.015), (0.0, 25.0)), "between events: {:?}", p.at(0.015));
    // 1 px a millisecond, at most min(8 ms, half the gap = 5 ms) ahead.
    assert!(close(p.at(0.022), (0.0, 32.0)), "{:?}", p.at(0.022));
    assert!(close(p.at(0.030), (0.0, 35.0)), "capped: {:?}", p.at(0.030));
    assert!(close(p.at(0.060), (0.0, 30.0)), "past 35 ms, the events' own total");
    assert!(close(p.velocity(), (0.0, 1000.0)), "{:?}", p.velocity());
}

#[test]
fn a_fling_slows_and_ends() {
    let f = Fling::new(1.0, (0.0, 2000.0)).expect("fast enough");
    let early = f.at(1.1);
    let later = f.at(1.2);
    assert!(early.1 > 0.0 && later.1 > early.1, "carries on");
    assert!(later.1 - early.1 < early.1, "slower and slower");
    assert!(f.moving(1.2));
    assert!(!f.moving(3.0), "ends");
    // Never further than velocity × τ.
    assert!(f.at(10.0).1 <= 2000.0 * 0.325 + 1e-3);
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
        s.last().nodes.iter().find_map(|n| n.scroll).map(|sc| sc.offset),
        Some((0.0, 0.0))
    );
    s.click(100.0, 12.0);
    assert_eq!(clicked.get_untracked(), 2, "row 2 was on screen there");
    // The UI takes the scroll in with its next frame, once.
    s.frame("scrolled").expect("a frame");
    assert_eq!(
        s.last().nodes.iter().find_map(|n| n.scroll).map(|sc| sc.offset),
        Some((0.0, 48.0))
    );
    s.click(100.0, 12.0);
    assert_eq!(clicked.get_untracked(), 2, "and only once");
    s.frame("again").expect("a frame");
    assert_eq!(
        s.last().nodes.iter().find_map(|n| n.scroll).map(|sc| sc.offset),
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
        let st = state.clone();
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
        let seen = headless.compositor.presented().frame;
        let wheeled = headless.compositor.wheel_input(super::WheelInput {
            point: (100.0, 100.0),
            delta: (0.0, 40.0),
            at: 0.0,
            precise: false,
            began: false,
            ended: false,
            seen,
        });
        assert_eq!(
            matches!(wheeled, super::Wheeled::Ui(_)),
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
            if let FromRender::Wheel { step, shown, .. } = m {
                to_ui
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((step.point.0, step.point.1, shown.frame));
            }
        }),
        Arc::new(Mutex::new(Arc::clone(&now_shown))),
        Duration::from_millis(1),
        Pace::Ticker(Duration::from_millis(1)),
        None,
    );
    let at = Instant::now();
    let wheel = |at: Instant, shown: &Arc<Presented>| ToRender::Wheel {
        point: (50.0, 50.0),
        delta: (0.0, 40.0),
        at,
        precise: false,
        began: false,
        ended: false,
        shown: Arc::clone(shown),
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
    let seen = s.parts().1.compositor.presented().frame;
    // Two frames composed since the one it came under, the list on the
    // left in both.
    s.act("swap", || swapped.set(true));
    s.frame("swapped").expect("a frame");
    s.frame("again").expect("a frame");
    let c = &mut s.parts().1.compositor;
    assert!(c.presented().frame >= seen + 2, "two frames later");
    let wheeled = c.wheel_input(WheelInput {
        point: (50.0, 50.0),
        delta: (0.0, 40.0),
        at: 0.0,
        precise: false,
        began: false,
        ended: false,
        seen,
    });
    let Wheeled::Ui(Some(target)) = wheeled else {
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

/// A wheel is routed in the frame it came under while that is one of the
/// last [`ROUTED`] composed, as each showed it: made again from the
/// composes after it, over a whole one among them, the ring wrapped many
/// times. One older is routed in the last composed.
#[test]
fn a_wheel_is_routed_in_any_of_the_last_frames_as_each_showed_and_an_older_in_the_last() {
    let mut s = Scenario::measuring(two_lists, 400, 100, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let mut shown: Vec<(u64, crate::scene::Properties)> = Vec::new();
    for n in 0..12_u8 {
        let at = f32::from(n) * 0.01;
        let seen = c.presented().frame;
        c.wheel_input(WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 10.0),
            at,
            precise: false,
            began: false,
            ended: false,
            seen,
        });
        // Two frames before the last, one composed whole.
        if n == 9 {
            c.pending.all = true;
        }
        c.compose(at);
        shown.push((c.presented().frame, c.shown.properties.clone()));
    }
    let (last, now) = shown.last().cloned().expect("frames");
    for w in shown.windows(2) {
        assert_ne!(w[0].1, w[1].1, "each frame scrolled on");
    }
    for (frame, props) in &shown {
        let (_, p, _) = c.tables(*frame);
        let kept = last - frame < u64::try_from(ROUTED).unwrap_or(u64::MAX);
        let want = if kept { props } else { &now };
        assert_eq!(
            (&p.transforms, &p.clips),
            (&want.transforms, &want.clips),
            "frame {frame}, {} before the last: {}",
            last - frame,
            if kept { "as it showed" } else { "the last" }
        );
    }
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
    let seen = c.presented().frame;
    let event = |c: &mut Compositor, x: f32, at: f32, dy: f32, began: bool, ended: bool| {
        c.wheel_input(WheelInput {
            point: (x, 50.0),
            delta: (0.0, dy),
            at,
            precise: true,
            began,
            ended,
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
        let seen = c.presented().frame;
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 30.0),
            at,
            precise: true,
            began,
            ended: false,
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
    let seen = c.presented().frame;
    for (k, at) in [0.0_f32, 0.016, 0.032, 0.048].into_iter().enumerate() {
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 4.0),
            at,
            precise: true,
            began: k == 0,
            ended: k == 3,
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
    let before = s.parts().1.compositor.presented().frame;
    s.act("open", || modal.set(true));
    s.frame("modal").expect("a frame");
    for precise in [false, true] {
        let c = &mut s.parts().1.compositor;
        let was = downs(c)[0];
        let wheeled = c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 30.0),
            at: 1.0,
            precise,
            began: true,
            ended: false,
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
            s.gesture(&nodes, Some(WheelTarget(0)), (0.0, 4.0), at, false),
            "event {k}"
        );
        s.tick(&nodes, at + 0.008);
    }
    // The end comes routed to the list a modal layer covers: refused.
    assert!(!s.gesture(&nodes, Some(WheelTarget(1)), (0.0, 0.0), 0.06, true));
    s.tick(&nodes, 0.2);
    let rested = s.offset(&nodes[0], 0);
    s.tick(&nodes, 0.4);
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
        assert!(s.gesture(&nodes, Some(WheelTarget(0)), (0.0, 4.0), at, k == 3));
        s.tick(&nodes, at + 0.008);
    }
    s.tick(&nodes, 0.1);
    let flung = s.offset(&nodes[0], 0);
    s.tick(&nodes, 0.12);
    assert_ne!(s.offset(&nodes[0], 0), flung, "it flings");
    // The modal's commit has no list; closing it, the next brings it back.
    s.committed(&[], 0);
    s.committed(&nodes, 0);
    let back = s.offset(&nodes[0], 0);
    s.tick(&nodes, 0.2);
    s.tick(&nodes, 0.4);
    assert_eq!(s.offset(&nodes[0], 0), back, "the fling ended with the list that left");
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
    let seen = c.presented().frame;
    let turn = |c: &mut Compositor, x: f32, at: f32| {
        c.wheel_input(WheelInput {
            point: (x, 50.0),
            delta: (0.0, 40.0),
            at,
            precise: false,
            began: false,
            ended: false,
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
    let seen = c.presented().frame;
    let event = |c: &mut Compositor, x: f32, at: f32, dy: f32, began: bool, ended: bool| {
        c.wheel_input(WheelInput {
            point: (x, 50.0),
            delta: (0.0, dy),
            at,
            precise: true,
            began,
            ended,
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
    let seen = c.presented().frame;
    let swipe = |c: &mut Compositor, at: f32, began: bool| {
        c.wheel_input(WheelInput {
            point: (20.0, 20.0),
            delta: (0.0, 100.0),
            at,
            precise: true,
            began,
            ended: false,
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
    let seen = c.presented().frame;
    let event = |c: &mut Compositor, at: f32, dy: f32, began: bool, ended: bool| {
        c.wheel_input(WheelInput {
            point: (20.0, 20.0),
            delta: (0.0, dy),
            at,
            precise: true,
            began,
            ended,
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
    event(c, 0.0, 100.0, true, false);
    event(c, 0.016, 100.0, false, false);
    event(c, 0.032, 0.0, false, true);
    // Momentum, begun 8 ms after the lift.
    event(c, 0.04, 60.0, true, false);
    event(c, 0.056, 40.0, false, false);
    event(c, 0.2, 0.0, false, true);
    assert_eq!(outer(c), 0.0, "the momentum stays on the inner list");
    // Fingers down 50 ms after the momentum ended: a new gesture, passed
    // by the inner list at its end to the outer.
    event(c, 0.25, 100.0, true, false);
    event(c, 0.266, 100.0, false, false);
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
    let seen = c.presented().frame;
    let wheel = |c: &mut Compositor, at: f32| {
        c.wheel_input(WheelInput {
            point: (20.0, 20.0),
            delta: (0.0, 100.0),
            at,
            precise: false,
            began: false,
            ended: false,
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
        let seen = c.presented().frame;
        c.wheel_input(WheelInput {
            point: (x, 50.0),
            delta: (0.0, 40.0),
            at,
            precise: true,
            began,
            ended: false,
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
    let seen = c.presented().frame;
    for (k, at) in [0.0_f32, 0.016, 0.032, 0.048].into_iter().enumerate() {
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 4.0),
            at,
            precise: true,
            began: k == 0,
            ended: k == 3,
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
        let seen = c.presented().frame;
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 10.0),
            at,
            precise: true,
            began,
            ended: false,
            seen,
        })
    };
    let c = &mut s.parts().1.compositor;
    assert!(matches!(wheel(c, 0.0, true), Wheeled::Ui(_)), "the handler takes it");
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
        let seen = c.presented().frame;
        c.wheel_input(WheelInput {
            point: (50.0, 20.0),
            delta: (0.0, 30.0),
            at,
            precise: true,
            began,
            ended: false,
            seen,
        })
    };
    assert!(matches!(step(c, true, 1.0), Wheeled::Ui(Some(_))), "the handler's");
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
    let seen = c.presented().frame;
    let turn = |c: &mut Compositor, at: (f32, f32), t: f32| {
        c.wheel_input(WheelInput {
            point: at,
            delta: (0.0, 40.0),
            at: t,
            precise: false,
            began: false,
            ended: false,
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
/// rounded 8 that clips, holding a list; clicks pass through the card and
/// the wrapper, so only their clips decide what is under a point.
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
                        .width(100.0)
                        .height(100.0)
                        .overflow_y(Overflow::Scroll)
                        .child(div().height(400.0)),
                ),
            ),
        )
        .into_element()
}

/// A wheel on two coincident rounded edges that each cover 60% of its
/// pixel: the chain keeps their product, 36%, so the list under it takes
/// the wheel, as the UI's hit testing and paint have it.
#[test]
fn a_wheel_on_two_coincident_rounded_edges_scrolls_the_list_under_them() {
    let mut s = Scenario::measuring(list_under_two_coincident_rounded_clips, 300, 300, 1.0).expect("a scenario");
    s.frame("first").expect("a frame");
    let c = &mut s.parts().1.compositor;
    let seen = c.presented().frame;
    let turn = |c: &mut Compositor, at: (f32, f32), t: f32| {
        c.wheel_input(WheelInput {
            point: at,
            delta: (0.0, 40.0),
            at: t,
            precise: false,
            began: false,
            ended: false,
            seen,
        })
    };
    turn(c, (20.1, 70.0), 0.0);
    assert_eq!(downs(c), [40.0, 0.0], "36% of the pixel: the list under it");
    turn(c, (20.6, 70.0), 5.0);
    assert_eq!(downs(c), [40.0, 40.0], "all of it: the card's list");
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
    let seen = c.presented().frame;
    assert!(c.tables(seen).0.hits.len() > 10, "the rows are hit boxes");
    c.wheel_input(WheelInput {
        point: (150.0, 100.0),
        delta: (0.0, 40.0),
        at: 0.0,
        precise: false,
        began: false,
        ended: false,
        seen,
    });
    assert_eq!(downs(c), [40.0], "the list");
    assert_eq!(c.chain_walks(), 0, "no row's clip chain walked");
}
