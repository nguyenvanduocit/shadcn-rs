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
use crate::reactive::Signal;
use crate::scene::Properties;
use crate::styled::Styled;

fn node(key: u64, offset: f32, max: f32) -> ScrollNode {
    ScrollNode {
        key: NodeKey(key),
        content: 1,
        offset: (0.0, offset),
        applied: (0.0, -offset),
        max: (0.0, max),
        rtl: false,
        generation: 0,
        thumb: None,
        parent: None,
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
    div().size_full().overflow_y_scroll().children((0..40).map(move |i| {
        div()
            .h_px(24.0)
            .bg(if i % 2 == 0 { Color::hex(0xf4f4f5) } else { Color::WHITE })
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
                .size_full()
                .child(
                    div()
                        .w_px(120.0)
                        .h_px(40.0)
                        .bg(Color::hex(0x18181b))
                        .duration_ms(200.0)
                        .ease_linear()
                        .transition_opacity()
                        .opacity(if on { 1.0 } else { 0.2 }),
                )
                .child(div().h_px(150.0).child(rows(clicked)))
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
    same_as_committed(&mut s, clock::current_seconds(), "scrolled");
}

#[test]
fn an_animation_is_evenly_spaced_at_predicted_present_times() {
    let shown = Signal::new(true);
    let mut s = Scenario::measuring(
        move || {
            div()
                .size_full()
                .child(
                    div()
                        .w_px(120.0)
                        .h_px(40.0)
                        .bg(Color::BLACK)
                        .duration_ms(500.0)
                        .ease_linear()
                        .transition_opacity()
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
                .size_full()
                .child(
                    div()
                        .w_px(120.0)
                        .h_px(40.0)
                        .bg(Color::BLACK)
                        .duration_ms(300.0)
                        .ease_linear()
                        .transition_transform()
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

/// Wait until `done` holds, up to two seconds.
fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now()
        .checked_add(Duration::from_secs(2))
        .unwrap_or_else(Instant::now);
    while Instant::now() < until {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// What the window shows once the render thread drew a commit after `after`.
fn shown_after(off: &crate::window::Offscreen, after: u64) -> Option<crate::image::Pixels> {
    assert!(
        wait_for(|| off.presented().commit > after),
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
    wait_for(|| {
        shown = (off.presented().commit > after)
            .then(|| off.capture())
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
                    .size_full()
                    .child(
                        div()
                            .w_px(80.0)
                            .h_px(40.0)
                            .bg(if dark.get() {
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

    // The device is lost: made again, and the scene as committed drawn whole.
    off.lose_device();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(off.capture().as_ref(), Some(&light), "the same frame on a new device");
    let before = off.presented().commit;
    click(&mut off);
    let dark = changed_after(&off, before, &light).expect("commits after the loss are drawn");

    // The render thread panics: made afresh, it draws the UI's full commit.
    let before = off.presented().commit;
    off.panic_render();
    let again = shown_after(&off, before).expect("a frame after the panic");
    assert_eq!(again, dark, "the full commit draws what was on screen");
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
                    e = div().flex_col().child(e).into_element();
                }
                div().size_full().child(e).into_element()
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
    assert!(wait_for(|| off.presented().commit > 0), "a tree {depth} deep is drawn");
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
                    .w_px(300.0)
                    .child(
                        virtual_list(
                            &st,
                            1_000_000,
                            |i| i,
                            |i| div().h_px(24.0).child(text(format!("row {i}"))).into_element(),
                        )
                        .h_px(300.0),
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
