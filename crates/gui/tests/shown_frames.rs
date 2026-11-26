//! Review round 9's reproducers, kept as regression tests: hover follows
//! the pointer and the frame on screen whenever either changes; a
//! non-positional event leaves hover and the cursor to the frame on
//! screen; text is hit in the frame that showed it; the hit index finds
//! what a walk of the table finds whatever the frames did before.

#![expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
#![expect(clippy::float_cmp, reason = "coordinates set exactly by the test")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::thread;
use std::time::{Duration, Instant};

use gui::app::Scenario;
use gui::element::{span, text};
use gui::interact::hit_presented;
use gui::prelude::*;
use gui::window::Input;
use gui::{Signal, div};

/// A frame drawn while the pointer was over the background enters; the
/// pointer then moves, before that frame's vblank, to where the frame put
/// a box. By the vblank, with no input more, the box is hovered.
#[test]
fn a_frame_entered_before_the_pointer_moved_onto_its_box_is_hovered_at_its_vblank() {
    let over = Arc::new(AtomicBool::new(false));
    let observe = Arc::clone(&over);
    let interval = Duration::from_millis(250);
    let mut w = gui::offscreen(
        move || {
            let show = Signal::new(false);
            move || {
                let observe = Arc::clone(&observe);
                let base = div().width(300.).height(100.).on_pointer_move(move |e| {
                    if e.x == 60. {
                        show.set(true);
                    }
                });
                if show.get() {
                    base.child(
                        div()
                            .position(Position::Absolute)
                            .left(150.)
                            .top(10.)
                            .width(70.)
                            .height(60.)
                            .on_hover(move |v| observe.store(v, SeqCst)),
                    )
                    .into_element()
                } else {
                    base.into_element()
                }
            }
        },
        Default::default(),
        (300, 100),
        1.,
        interval,
    )
    .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while w.presented().frame == 0 {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    w.input(Input::PointerMoved { x: 30., y: 40. });
    thread::sleep(interval * 4);
    let a = w.presented().commit;
    drop(w.take_frames());
    w.input(Input::PointerMoved { x: 60., y: 40. });
    let b = loop {
        if let Some(b) = w.take_frames().into_iter().find(|f| f.shows > a) {
            break b;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    };
    assert!(Instant::now() + Duration::from_millis(15) < b.present_at);
    w.input(Input::PointerMoved { x: 180., y: 40. });
    thread::sleep(b.present_at.saturating_duration_since(Instant::now()) + interval * 2);
    assert!(w.presented().commit > a);
    let by_vblank = over.load(SeqCst);
    w.input(Input::PointerMoved { x: 180., y: 40. });
    thread::sleep(interval);
    assert!(over.load(SeqCst), "the same coordinates now pick B's box");
    assert!(by_vblank, "pointer is over B's box after its vblank");
}

/// A click queued under a frame lands on the span that frame showed,
/// though only the spans' text changed (the paragraph's box the same)
/// before and after it: a frame that changes only a paragraph's spans
/// carries them.
#[test]
fn a_click_queued_under_a_frame_picks_the_span_it_showed_when_only_the_spans_text_changed() {
    let long = Signal::new(false);
    let picked = Signal::new(0u8);
    let mut s = Scenario::measuring(
        move || {
            let first = if long.get() { "aaaaaaaaaaaa" } else { "aaaa" };
            div()
                .width(300.)
                .height(60.)
                .child(
                    text("")
                        .key(1)
                        .width(300.)
                        .height(40.)
                        .span(span(first).on_click(move |_| picked.set(1)))
                        .span(span("bbbbbbbb").on_click(move |_| picked.set(2))),
                )
                .into_element()
        },
        300,
        60,
        1.,
    )
    .unwrap();
    s.frame("A").unwrap();
    s.act("lengthen the first span", || long.set(true));
    s.frame("B").unwrap();
    // Past the short first span's end, inside the long one's.
    s.click(60., 10.);
    assert_eq!(picked.get_untracked(), 1);
    let down = s.queue(Input::MouseDown { x: 60., y: 10. });
    let up = s.queue(Input::MouseUp { x: 60., y: 10. });
    s.act("shorten the first span", || {
        picked.set(0);
        long.set(false);
    });
    s.frame("C").unwrap();
    s.handle(down);
    s.handle(up);
    assert_eq!(picked.get_untracked(), 1, "queued under B");
}

/// A click queued under frame A lands on the span A showed there, though
/// the paragraph was laid out again (its first span wider) before it is
/// handled.
#[test]
fn a_click_queued_under_a_picks_the_span_a_showed() {
    let wide = Signal::new(false);
    let picked = Signal::new(0u8);
    let mut s = Scenario::measuring(
        move || {
            let size = if wide.get() { 28. } else { 12. };
            div()
                .width(300.)
                .height(60.)
                .child(
                    text("")
                        .key(1)
                        .text_size(size)
                        .span(span("aaaa").on_click(move |_| picked.set(1)))
                        .span(span("bbbbbbbb").on_click(move |_| picked.set(2))),
                )
                .into_element()
        },
        300,
        60,
        1.,
    )
    .unwrap();
    s.frame("A").unwrap();
    s.click(35., 14.);
    assert_eq!(picked.get_untracked(), 2);
    let down = s.queue(Input::MouseDown { x: 35., y: 14. });
    let up = s.queue(Input::MouseUp { x: 35., y: 14. });
    s.act("widen first span", || {
        picked.set(0);
        wide.set(true);
    });
    s.frame("B").unwrap();
    s.handle(down);
    s.handle(up);
    assert_eq!(picked.get_untracked(), 2, "queued under A");
}

/// A click queued under frame A puts the caret where A showed the text,
/// though the field's type was enlarged before it is handled.
#[test]
fn a_click_queued_under_a_puts_the_caret_where_a_showed_the_text() {
    let wide = Signal::new(false);
    let field = gui::TextField::new("abcdefghijklmnopqrst");
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            gui::text_input(&f)
                .width(300.)
                .height(60.)
                .text_size(if wide.get() { 28. } else { 12. })
                .into_element()
        },
        300,
        60,
        1.,
    )
    .unwrap();
    s.frame("A").unwrap();
    s.click(50., 15.);
    let expected = field.edit.with_untracked(|e| e.caret);
    let down = s.queue(Input::MouseDown { x: 50., y: 15. });
    let up = s.queue(Input::MouseUp { x: 50., y: 15. });
    s.act("enlarge type", || wide.set(true));
    s.frame("B").unwrap();
    s.handle(down);
    s.handle(up);
    assert_eq!(field.edit.with_untracked(|e| e.caret), expected);
}

/// A click queued under a frame puts the caret where that frame showed
/// the text, though the field's text was replaced (its box the same)
/// before and after it: a frame that changes only a field's text carries
/// that text, and a click under it is hit in it.
#[test]
fn a_click_queued_under_a_frame_puts_the_caret_in_the_text_that_frame_showed() {
    let field = gui::TextField::new("WWWWWWWWWWWWWWWWWWWW");
    let f = field;
    let mut s = Scenario::measuring(
        move || gui::text_input(&f).width(300.).height(60.).into_element(),
        300,
        60,
        1.,
    )
    .unwrap();
    s.frame("A").unwrap();
    s.act("narrow the text", || field.set_text("iiiiiiiiiiiiiiiiiiii"));
    s.frame("B").unwrap();
    s.click(50., 15.);
    let expected = field.edit.with_untracked(|e| e.caret);
    let down = s.queue(Input::MouseDown { x: 50., y: 15. });
    let up = s.queue(Input::MouseUp { x: 50., y: 15. });
    s.act("widen the text", || field.set_text("WWWWWWWWWWWWWWWWWWWW"));
    s.frame("C").unwrap();
    s.handle(down);
    s.handle(up);
    assert_eq!(field.edit.with_untracked(|e| e.caret), expected, "queued under B");
}

/// A modifier stamped with an older frame does not roll hover and the
/// cursor back to that frame: they follow the frame on screen.
#[test]
fn a_late_modifier_leaves_the_cursor_of_the_frame_on_screen() {
    use gui::compositor::{Headless, MainFrame, Presented, Shown};
    use gui::style::Cursor;
    let mut p = gui::app::Pipeline::new(
        || {
            div()
                .width(40.)
                .height(40.)
                .cursor(Cursor::Grab)
                .on_hover(|_| {})
                .into_element()
        },
        false,
    );
    let mut h = Headless::new(
        gui::gpu::GpuRenderer::headless().unwrap(),
        gui::text::Glyphs::for_os(),
        false,
    );
    p.draw_with(&mut h, MainFrame::at(1000_u64), (64, 64), 1., &mut Default::default());
    let b = h.shown();
    let a = Shown::Frame(Arc::new(Presented::default()));
    p.input_shown(Input::PointerMoved { x: 10., y: 10. }, &a);
    p.converge(&b);
    assert_eq!(p.cursor(), Cursor::Grab);
    p.input_shown(Input::Modifiers(Default::default()), &a);
    p.converge(&b);
    assert_eq!(p.cursor(), Cursor::Grab, "B still shows");
}

/// One box that takes clicks over the window's corner.
fn control() -> Scenario {
    let mut s = Scenario::measuring(
        || div().width(40.).height(40.).on_click(|_| {}).into_element(),
        64,
        64,
        1.,
    )
    .expect("scenario");
    s.frame("control").expect("frame");
    s
}

/// A space whose bounds were not finite in the first frame hit and are in
/// the next is found in it, as an index made fresh finds it.
#[test]
fn a_space_whose_bounds_become_finite_is_hit_as_a_fresh_index_hits_it() {
    let s = control();
    let mut p = (*s.presented()).clone();
    let mut h = p.hits[0];
    h.transform = 1;
    p.hits = vec![h].into();
    p.properties.transforms.truncate(1);
    p.properties.transforms.push(Default::default());
    p.properties.transforms[1].offset.0 = f32::MAX;
    let pick = |p: &_| hit_presented(s.last(), Some(p), 10., 10.);
    assert_eq!(pick(&p), None);
    p.frame += 1;
    p.properties.transforms[1].offset.0 = 0.;
    let warm = pick(&p);
    p.hits = p.hits.to_vec().into();
    let cold = pick(&p);
    assert!(cold.is_some());
    assert_eq!(warm, cold);
}

/// What the space tree costs over 20,000 spaces, each a box of its own 50
/// px apart in a row (ADR-0015 records the numbers, from a release run
/// with `--nocapture`): the first hit of a table (the index made and the
/// frame placed), a fresh frame that scrolls them all (refit), one that
/// scatters them into two piles (packed again), and a thousand misses in
/// each frame. The counted bound is `clip`'s unit test; this one times it.
#[test]
fn the_space_tree_over_twenty_thousand_spaces_costs_what_adr_0015_records() {
    let s = control();
    let mut p = (*s.presented()).clone();
    let mut h = *p.hits.last().expect("the box");
    h.rect = gui::geometry::Bounds {
        x: 0.,
        y: 0.,
        width: 40.,
        height: 40.,
    };
    let n = 20_000_u16;
    p.hits = (1..=u32::from(n))
        .map(|transform| gui::compositor::Hit { transform, ..h })
        .collect();
    p.properties.transforms.truncate(1);
    let root = p.properties.transforms[0];
    for i in 0..n {
        let mut t = root;
        t.offset.0 = f32::from(i) * 50.;
        p.properties.transforms.push(t);
    }
    let pick = |p: &_, x: f32| hit_presented(s.last(), Some(p), x, 10.);
    let timed = |p: &_, x: f32| {
        let start = Instant::now();
        let got = pick(p, x);
        (got, start.elapsed())
    };
    let misses = |p: &_, x: f32| {
        let start = Instant::now();
        for _ in 0..1000 {
            assert_eq!(std::hint::black_box(pick(p, x)), None);
        }
        start.elapsed() / 1000
    };
    let (_, first) = timed(&p, 145.);
    let row = misses(&p, 145.);
    p.frame += 1;
    for t in p.properties.transforms.iter_mut().skip(1) {
        t.offset.1 = 1.;
    }
    let (_, refit) = timed(&p, 145.);
    let scrolled = misses(&p, 145.);
    p.frame += 1;
    for (i, t) in p.properties.transforms.iter_mut().skip(1).enumerate() {
        t.offset.0 = if i % 2 == 0 { 0. } else { 200. };
    }
    let (_, repacked) = timed(&p, 100.);
    let piles = misses(&p, 100.);
    eprintln!(
        "20k spaces: first hit {first:?}, miss {row:?}; fresh frame (refit) {refit:?}, miss {scrolled:?}; \
         scattered (packed again) {repacked:?}, miss {piles:?}"
    );
}

/// A field of twenty letters, 300 by 40, its type `Password` while
/// `masked` holds and `Text` otherwise.
fn field_typed(f: gui::TextField, masked: Signal<bool>) -> Scenario {
    use gui::element::InputType;
    Scenario::measuring(
        move || {
            gui::text_input(&f)
                .width(300.)
                .height(40.)
                .input_type(if masked.get() {
                    InputType::Password
                } else {
                    InputType::Text
                })
                .into_element()
        },
        300,
        60,
        1.,
    )
    .expect("scenario")
}

/// Review r10, Codex 1: a press queued under a frame that showed the
/// field masked puts the caret where that frame showed it, though the
/// value is revealed before it is handled (the value itself unchanged).
#[test]
fn a_press_queued_under_a_masked_frame_puts_the_caret_where_it_showed() {
    let masked = Signal::new(true);
    let f = gui::TextField::new("abcdefghijklmnopqrst");
    let mut s = field_typed(f, masked);
    s.frame("masked").expect("frame");
    s.click(50., 15.);
    let expected = f.edit.with_untracked(|e| e.caret);
    let down = s.queue(Input::MouseDown { x: 50., y: 15. });
    s.act("reveal unchanged value", || masked.set(false));
    s.frame("revealed").expect("frame");
    s.handle(down);
    assert_eq!(f.edit.with_untracked(|e| e.caret), expected);
}

/// Opus r10 I2: the same as a click (down and up), on a field of ten
/// letters clicked at x = 25 under the masked frame.
#[test]
fn a_click_queued_under_a_password_frame_puts_the_caret_where_a_showed_it() {
    let masked = Signal::new(true);
    let f = gui::TextField::new("abcdefghij");
    let mut s = field_typed(f, masked);
    s.frame("A").expect("frame");
    s.click(25., 15.);
    let expected = f.edit.with_untracked(|e| e.caret);
    let down = s.queue(Input::MouseDown { x: 25., y: 15. });
    let up = s.queue(Input::MouseUp { x: 25., y: 15. });
    s.act("show the password", || masked.set(false));
    s.frame("B").expect("frame");
    s.handle(down);
    s.handle(up);
    assert_eq!(f.edit.with_untracked(|e| e.caret), expected);
}

/// The other way: a click queued under a frame that showed the value in
/// the clear puts the caret where that frame showed it, though the field
/// is masked before it is handled.
#[test]
fn a_click_queued_under_a_clear_frame_puts_the_caret_where_it_showed_after_masking() {
    let masked = Signal::new(false);
    let f = gui::TextField::new("abcdefghijklmnopqrst");
    let mut s = field_typed(f, masked);
    s.frame("A").expect("frame");
    s.click(50., 15.);
    let expected = f.edit.with_untracked(|e| e.caret);
    let down = s.queue(Input::MouseDown { x: 50., y: 15. });
    let up = s.queue(Input::MouseUp { x: 50., y: 15. });
    s.act("mask the value", || masked.set(true));
    s.frame("B").expect("frame");
    s.handle(down);
    s.handle(up);
    assert_eq!(f.edit.with_untracked(|e| e.caret), expected);
}

/// Review r10, Codex 1: a press queued under a frame that showed a
/// composition at the start of the field (a press inside it puts the
/// caret at 0) still does, though the composition was cancelled before it
/// is handled.
#[test]
fn a_press_queued_under_a_composing_frame_puts_the_caret_where_it_showed() {
    let f = gui::TextField::new("abcdefghijklmnopqrst");
    let mut s = Scenario::measuring(
        move || gui::text_input(&f).width(300.).height(40.).into_element(),
        300,
        60,
        1.,
    )
    .expect("scenario");
    s.frame("field").expect("frame");
    s.click(1., 15.);
    s.act("compose at start", || {
        f.edit.update(|e| {
            e.place(0, false);
            e.compose(gui::Preedit {
                text: "WWWWWW".into(),
                cursor: None,
            });
        });
    });
    s.frame("preedit").expect("frame");
    let down = s.queue(Input::MouseDown { x: 30., y: 15. });
    s.click(30., 15.);
    assert_eq!(f.edit.with_untracked(|e| e.caret), 0);
    s.act("cancel composition", || f.edit.update(|e| e.end_composition()));
    s.frame("cancelled").expect("frame");
    s.handle(down);
    assert_eq!(f.edit.with_untracked(|e| e.caret), 0);
}

/// Review r10, Codex 2: a resting pointer over a `Grab` box; the box's
/// cursor alone changes, and no frame shows that yet. The cursor is a
/// value the frame shows: it stays `Grab` until a frame with the new one
/// is on screen, whatever input comes meanwhile.
#[test]
fn a_cursor_changed_alone_shows_with_its_frame() {
    use gui::compositor::{Headless, MainFrame};
    use gui::style::Cursor;
    let grab = Signal::new(true);
    let mut p = gui::app::Pipeline::new(
        move || {
            div()
                .width(40.)
                .height(40.)
                .cursor(if grab.get() { Cursor::Grab } else { Cursor::Text })
                .on_hover(|_| {})
                .into_element()
        },
        false,
    );
    let mut h = Headless::new(
        gui::gpu::GpuRenderer::headless().expect("gpu"),
        gui::text::Glyphs::for_os(),
        false,
    );
    p.draw_with(&mut h, MainFrame::at(1000_u64), (64, 64), 1., &mut Default::default());
    let a = h.shown();
    p.input_shown(Input::PointerMoved { x: 10., y: 10. }, &a);
    assert_eq!(p.cursor(), Cursor::Grab);
    p.act("cursor only", || grab.set(false));
    p.input_shown(Input::Modifiers(Default::default()), &a);
    p.converge(&a);
    assert_eq!(p.cursor(), Cursor::Grab, "the frame on screen shows a grab");
    p.draw_with(&mut h, MainFrame::at(2000_u64), (64, 64), 1., &mut Default::default());
    let b = h.shown();
    p.converge(&b);
    assert_eq!(p.cursor(), Cursor::Text, "the frame on screen shows text");
}

/// Review r10, Codex 4: a press routed in frame A whose handler panics,
/// caught by the app, leaves hover and the cursor on the frame they were
/// worked out in (B, empty), not on A.
#[test]
fn a_dispatch_that_unwinds_leaves_the_frame_on_screen() {
    use gui::compositor::{Headless, MainFrame, Presented, Shown};
    use gui::style::Cursor;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let mut p = gui::app::Pipeline::new(
        || {
            div()
                .width(40.)
                .height(40.)
                .cursor(Cursor::Grab)
                .on_mouse_down(|_| std::panic::panic_any("handler"))
                .into_element()
        },
        false,
    );
    let mut h = Headless::new(
        gui::gpu::GpuRenderer::headless().expect("gpu"),
        gui::text::Glyphs::for_os(),
        false,
    );
    p.draw_with(&mut h, MainFrame::at(1000_u64), (64, 64), 1., &mut Default::default());
    let a = h.shown();
    let b = Shown::Frame(Arc::new(Presented {
        frame: 99,
        ..Default::default()
    }));
    p.input_shown(Input::PointerMoved { x: 10., y: 10. }, &b);
    assert_eq!(p.cursor(), Cursor::Default);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            p.input_shown(Input::MouseDown { x: 10., y: 10. }, &a)
        }))
        .is_err()
    );
    p.converge(&b);
    assert_eq!(p.cursor(), Cursor::Default);
}

/// Opus r10 P1: a click queued under frame A on a keyed span reaches that
/// span's handler, though a span was put before it since (the spans'
/// places shift); and a span taken away since gets nothing.
#[test]
fn a_click_queued_under_a_keyed_span_reaches_it_after_spans_were_put_before_it() {
    let extra = Signal::new(false);
    let gone = Signal::new(false);
    let picked = Signal::new(0u8);
    let mut s = Scenario::measuring(
        move || {
            let mut t = text("").key(1).width(300.).height(40.);
            if extra.get() {
                t = t.span(span("x").key(3).on_click(move |_| picked.set(3)));
            }
            t = t.span(span("aaaa").key(1).on_click(move |_| picked.set(1)));
            if !gone.get() {
                t = t.span(span("bbbbbbbb").key(2).on_click(move |_| picked.set(2)));
            }
            div().width(300.).height(60.).child(t).into_element()
        },
        300,
        60,
        1.,
    )
    .expect("scenario");
    s.frame("A").expect("frame");
    s.click(60., 10.);
    assert_eq!(picked.get_untracked(), 2, "the second span under A");
    let down = s.queue(Input::MouseDown { x: 60., y: 10. });
    let up = s.queue(Input::MouseUp { x: 60., y: 10. });
    s.act("put a span first", || {
        picked.set(0);
        extra.set(true);
    });
    s.frame("B").expect("frame");
    s.handle(down);
    s.handle(up);
    assert_eq!(picked.get_untracked(), 2, "the span A showed there");
    let down = s.queue(Input::MouseDown { x: 60., y: 10. });
    let up = s.queue(Input::MouseUp { x: 60., y: 10. });
    s.act("take the span away", || {
        picked.set(0);
        gone.set(true);
    });
    s.frame("C").expect("frame");
    s.handle(down);
    s.handle(up);
    assert_eq!(picked.get_untracked(), 0, "a span gone since gets nothing");
}

/// Draw `p`'s frame at `t` into `h`, 300 by 60.
fn draw(p: &mut gui::app::Pipeline, h: &mut gui::compositor::Headless, t: u64) {
    p.draw_with(
        h,
        gui::compositor::MainFrame::at(t),
        (300, 60),
        1.,
        &mut Default::default(),
    );
}

/// A headless renderer.
fn headless() -> gui::compositor::Headless {
    gui::compositor::Headless::new(
        gui::gpu::GpuRenderer::headless().expect("gpu"),
        gui::text::Glyphs::for_os(),
        false,
    )
}

/// Review r11, Codex 1: a click routed in frame A gets the bounds A showed
/// its node with: at x = 75 of a control A showed 100 wide, the right
/// half, though the control was laid out 200 wide since.
#[test]
fn a_click_routed_in_a_frame_gets_the_bounds_that_frame_showed() {
    let wide = Signal::new(false);
    let right = Signal::new(None);
    let mut p = gui::app::Pipeline::new(
        move || {
            div()
                .child(
                    div()
                        .width(if wide.get() { 200. } else { 100. })
                        .height(40.)
                        .on_click(move |e| right.set(Some(e.x - e.bounds.x >= e.bounds.width / 2.))),
                )
                .into_element()
        },
        false,
    );
    let mut h = headless();
    draw(&mut p, &mut h, 1000);
    let a = h.shown();
    let click = |p: &mut gui::app::Pipeline| {
        p.input_shown(Input::MouseDown { x: 75., y: 10. }, &a);
        p.input_shown(Input::MouseUp { x: 75., y: 10. }, &a);
    };
    click(&mut p);
    assert_eq!(right.get_untracked(), Some(true));
    p.act("width", || {
        wide.set(true);
        right.set(None);
    });
    click(&mut p);
    assert_eq!(right.get_untracked(), Some(true));
}

/// Review r11, Codex 2: a node removed since the frame on screen showed
/// it still shows that frame's cursor until a frame without it shows.
#[test]
fn a_node_removed_since_shows_its_frames_cursor_until_a_frame_without_it() {
    use gui::style::Cursor;
    let visible = Signal::new(true);
    let mut p = gui::app::Pipeline::new(
        move || {
            let mut root = div().width(300.).height(60.);
            if visible.get() {
                root = root.child(div().width(40.).height(40.).cursor(Cursor::Grab).on_hover(|_| {}));
            }
            root.into_element()
        },
        false,
    );
    let mut h = headless();
    draw(&mut p, &mut h, 1000);
    let a = h.shown();
    p.input_shown(Input::PointerMoved { x: 10., y: 10. }, &a);
    assert_eq!(p.cursor(), Cursor::Grab);
    p.act("remove child", || visible.set(false));
    p.input_shown(Input::Modifiers(Default::default()), &a);
    p.converge(&a);
    assert_eq!(p.cursor(), Cursor::Grab, "A still shows the box");
    draw(&mut p, &mut h, 2000);
    let b = h.shown();
    p.converge(&b);
    assert_eq!(p.cursor(), Cursor::Default, "B shows it gone");
}

/// Review r11 (brief): the cursor of a span the frame on screen showed is
/// that frame's, whatever is laid out now: a paragraph whose link was
/// taken away since still shows the link's cursor until a frame without it
/// does.
#[test]
fn a_span_taken_away_since_shows_its_frames_cursor_until_a_frame_without_it() {
    use gui::style::Cursor;
    let gone = Signal::new(false);
    let mut p = gui::app::Pipeline::new(
        move || {
            let mut t = text("").width(300.).height(40.);
            if !gone.get() {
                t = t.span(
                    span("a link to follow")
                        .cursor(|| Some(Cursor::Pointer))
                        .on_click(|_| {}),
                );
            }
            div().width(300.).height(60.).child(t).into_element()
        },
        false,
    );
    let mut h = headless();
    draw(&mut p, &mut h, 1000);
    let a = h.shown();
    p.input_shown(Input::PointerMoved { x: 10., y: 10. }, &a);
    let link = p.cursor();
    assert_ne!(link, Cursor::Default, "the link shows its cursor");
    p.act("take the link away", || gone.set(true));
    p.input_shown(Input::Modifiers(Default::default()), &a);
    p.converge(&a);
    assert_eq!(p.cursor(), link, "A still shows the link");
    draw(&mut p, &mut h, 2000);
    let b = h.shown();
    p.converge(&b);
    assert_eq!(p.cursor(), Cursor::Default, "B shows it gone");
}

/// Review r11, Codex 3: the input method's candidate window follows the
/// caret the frame on screen shows, not one moved since.
#[test]
fn the_input_methods_area_follows_the_caret_on_screen() {
    let f = gui::TextField::new("abcdefghijklmnopqrst");
    let mut p = gui::app::Pipeline::new(
        move || gui::text_input(&f).width(300.).height(40.).into_element(),
        false,
    );
    let mut h = headless();
    draw(&mut p, &mut h, 1000);
    let first = h.shown();
    p.input_shown(Input::MouseDown { x: 1., y: 15. }, &first);
    p.input_shown(Input::MouseUp { x: 1., y: 15. }, &first);
    draw(&mut p, &mut h, 2000);
    let a = h.shown();
    p.converge(&a);
    let before = p.ime_target().expect("a field focused").area;
    p.act("move caret", || f.edit.update(|e| e.place(18, false)));
    p.input_shown(Input::Modifiers(Default::default()), &a);
    p.converge(&a);
    assert_eq!(p.ime_target().expect("a field focused").area, before);
    draw(&mut p, &mut h, 3000);
    let b = h.shown();
    p.converge(&b);
    assert_ne!(
        p.ime_target().expect("a field focused").area,
        before,
        "the frame with the caret moved shows"
    );
}

/// Review r11, Codex 4: a move routed in frame A whose pointer handler
/// panics, after its hover handler ran, leaves hover and the key it is
/// worked out by together: converging on B (empty), checked before at
/// the same place, takes the hover back off.
#[test]
fn a_move_that_unwinds_leaves_hover_and_its_key_together() {
    use gui::compositor::{Presented, Shown};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let over = Signal::new(false);
    let mut p = gui::app::Pipeline::new(
        move || {
            div()
                .width(40.)
                .height(40.)
                .on_hover(move |on| over.set(on))
                .on_pointer_move(|_| std::panic::panic_any("move"))
                .into_element()
        },
        false,
    );
    let mut h = headless();
    draw(&mut p, &mut h, 1000);
    let a = h.shown();
    let b = Shown::Frame(Arc::new(Presented {
        frame: 99,
        ..Default::default()
    }));
    let moved = || Input::PointerMoved { x: 10., y: 10. };
    p.input_shown(moved(), &b);
    assert!(!over.get_untracked());
    assert!(catch_unwind(AssertUnwindSafe(|| p.input_shown(moved(), &a))).is_err());
    p.converge(&b);
    assert!(!over.get_untracked(), "empty frame still shows");
}

/// Opus r11 I1: a text area of 30 lines the compositor scrolled down 60
/// px, with no main frame since: a click on its fourth visible line puts
/// the caret where that frame showed it, as when a main frame followed.
#[test]
fn a_click_in_a_text_area_the_compositor_scrolled_lands_where_it_showed() {
    let caret_after = |main_frame: bool| {
        let value: String = (0..30).map(|n| format!("line{n:02}\n")).collect();
        let f = gui::TextField::new(value.trim_end());
        let mut s = Scenario::measuring(
            move || gui::text_input(&f).multiline().width(300.).height(80.).into_element(),
            300,
            80,
            1.,
        )
        .expect("scenario");
        s.frame("field").expect("frame");
        s.hover(50., 40.);
        assert!(s.wheel(60.), "the field scrolls");
        if main_frame {
            s.frame("after the wheel").expect("frame");
        }
        s.click(50., 40.);
        let caret = f.edit.with_untracked(|e| e.caret);
        let text = f.edit.with_untracked(|e| e.text().to_owned());
        text.get(..caret).map_or(0, |t| t.matches('\n').count())
    };
    let reference = caret_after(true);
    assert!(reference >= 3, "the click is below the third line: {reference}");
    assert_eq!(caret_after(false), reference, "the frame the compositor scrolled");
}

/// Review r12, Codex 1: a text area inside a container the compositor
/// scrolled 60 px: a click on the same visible point of the text area
/// puts the caret where it does unscrolled (the container's scroll is not
/// counted again as the field's own).
#[test]
fn a_text_area_in_a_scrolled_container_keeps_its_text_where_it_showed() {
    fn caret(scroll: bool) -> usize {
        let f = gui::TextField::new("zero\none\ntwo\nthree\nfour\nfive\nsix\nseven");
        let mut s = Scenario::measuring(
            move || {
                div()
                    .width(300.)
                    .height(160.)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(80.))
                    .child(gui::text_input(&f).multiline().width(180.).height(80.))
                    .child(div().height(400.))
                    .into_element()
            },
            300,
            160,
            1.,
        )
        .expect("scenario");
        s.frame("initial").expect("frame");
        if scroll {
            s.hover(220., 90.);
            assert!(s.wheel(60.));
            assert!(s.presented().scroll.iter().any(|s| s.offset.1 == 60.));
        }
        s.click(10., if scroll { 40. } else { 100. });
        f.edit.with_untracked(|e| e.caret)
    }
    assert_eq!(caret(true), caret(false));
}

/// Review r12, Codex 2: an `on_hover(true)` that panics leaves no hover
/// state half-written: converging on an empty frame after it takes the
/// hovered flag off.
#[test]
fn a_hover_handler_that_panics_leaves_hover_state_whole() {
    use gui::compositor::{Presented, Shown};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let i = gui::Interaction::new();
    let mut p = gui::app::Pipeline::new(
        move || {
            div()
                .width(40.)
                .height(40.)
                .interaction(i)
                .on_hover(|on| assert!(!on, "enter callback"))
                .into_element()
        },
        false,
    );
    let mut h = headless();
    draw(&mut p, &mut h, 1000);
    let empty = Shown::Frame(Arc::new(Presented {
        frame: 99,
        ..Default::default()
    }));
    let moved = || Input::PointerMoved { x: 10., y: 10. };
    p.input_shown(moved(), &empty);
    let a = h.shown();
    assert!(catch_unwind(AssertUnwindSafe(|| p.input_shown(moved(), &a))).is_err());
    p.converge(&empty);
    assert!(!i.hovered(), "the empty frame hovers nothing");
}

/// Review r12, Opus I1: the first `on_hover` panics inside a converge; a
/// second converge on the same frame hovers the box.
#[test]
fn a_converge_whose_hover_handler_panics_hovers_on_the_next() {
    use gui::compositor::{Presented, Shown};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::AtomicU32;
    let calls = Arc::new(AtomicU32::new(0));
    let i = gui::Interaction::new();
    let c = Arc::clone(&calls);
    let mut p = gui::app::Pipeline::new(
        move || {
            let c = Arc::clone(&c);
            div()
                .width(40.)
                .height(40.)
                .interaction(i)
                .on_hover(move |_| {
                    if c.fetch_add(1, SeqCst) == 0 {
                        std::panic::panic_any("first hover");
                    }
                })
                .into_element()
        },
        false,
    );
    let mut h = headless();
    draw(&mut p, &mut h, 1000);
    let empty = Shown::Frame(Arc::new(Presented {
        frame: 99,
        ..Default::default()
    }));
    p.input_shown(Input::PointerMoved { x: 10., y: 10. }, &empty);
    let a = h.shown();
    assert!(catch_unwind(AssertUnwindSafe(|| p.converge(&a))).is_err());
    p.converge(&a);
    assert!(i.hovered(), "the box under the pointer is hovered");
    // Hover was written whole before its handler ran: leaving takes it
    // off, and tells the handler so.
    p.input_shown(Input::PointerLeft, &a);
    assert!(!i.hovered(), "left");
    assert_eq!(calls.load(SeqCst), 2, "the leave is told");
}

/// Review r13, Codex 1: an effect reading two nodes' hovered flags sees
/// the whole chain written (a child is never hovered without its parent),
/// and one that panics leaves hover whole: moving out takes both off.
#[test]
fn an_effect_on_hover_sees_the_whole_chain_and_cannot_tear_it() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let child = gui::Interaction::new();
    let parent = gui::Interaction::new();
    let mut s = Scenario::measuring(
        move || {
            div()
                .size(80.)
                .interaction(parent)
                .child(div().size(40.).interaction(child).on_click(|_| {}))
                .into_element()
        },
        80,
        80,
        1.,
    )
    .expect("scenario");
    s.frame("initial").expect("frame");
    let _effect = gui::Effect::new(move || {
        assert!(!child.hovered() || parent.hovered(), "partial chain");
    });
    let _caught = catch_unwind(AssertUnwindSafe(|| s.hover(10., 10.)));
    s.hover(100., 100.);
    assert_eq!((child.hovered(), parent.hovered()), (false, false));
}

/// Review r13, Codex 2 and Opus I1: A's leave handler panics as the
/// pointer crosses to B: B is still told it was entered (every handler of
/// a change is told, the first panic resumed after).
#[test]
fn a_panicking_leave_does_not_skip_the_next_enter() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let entered = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(80.)
                .height(40.)
                .child(div().size(40.).on_hover(|on| assert!(on, "left A")))
                .child(
                    div()
                        .position(Position::Absolute)
                        .left(40.)
                        .top(0.)
                        .size(40.)
                        .on_hover(move |on| entered.set(on)),
                )
                .into_element()
        },
        80,
        40,
        1.,
    )
    .expect("scenario");
    s.frame("two siblings").expect("frame");
    s.hover(10., 10.);
    assert!(catch_unwind(AssertUnwindSafe(|| s.hover(50., 10.))).is_err());
    s.hover(51., 10.);
    assert!(entered.get_untracked(), "B never received its enter");
}

/// Review r13, Codex 3 and Opus I2: asking for the input method's area
/// again while nothing changed costs no layout of the field's text: 20
/// equal asks of a 20,000-line field take about what they take of a
/// 20-line one. (Times printed; the bound is loose enough for a debug
/// build, and 20 relayouts of 20,000 lines are well past it.)
#[test]
fn an_unchanged_input_method_area_is_not_worked_out_again() {
    let mut took = Vec::new();
    for lines in [20, 20_000] {
        let f = gui::TextField::new("line\n".repeat(lines));
        let mut p = gui::app::Pipeline::new(
            move || gui::text_input(&f).multiline().width(300.).height(60.).into_element(),
            false,
        );
        let mut h = headless();
        draw(&mut p, &mut h, 1000);
        let shown = h.shown();
        p.input_shown(Input::MouseDown { x: 5., y: 5. }, &shown);
        p.converge(&shown);
        let expected = p.ime_target().expect("a field focused");
        let start = Instant::now();
        for _ in 0..20 {
            assert_eq!(p.ime_target(), Some(expected));
        }
        let t = start.elapsed();
        println!("{lines} lines, 20 equal IME queries: {t:?}");
        took.push(t);
    }
    assert!(
        took[1] < took[0] * 20 + Duration::from_millis(5),
        "20 equal asks of 20,000 lines took {:?}, of 20 lines {:?}",
        took[1],
        took[0]
    );
}
