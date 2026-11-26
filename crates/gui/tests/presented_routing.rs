//! Input is routed in the frame that was on screen when it came (the
//! presented frame it is stamped with), not in a commit made since:
//! `/tmp/m10/hit-routing-interface.md`'s shared fixture. The compositor
//! owns a wheel gesture: its first event picks, in the presented frame, a
//! scroll container that can move that way (passing one at its end to the
//! one around it) and latches it; the rest of the gesture follows the
//! latch, whatever is under the pointer by then; a program's jump refuses
//! the rest of a phased gesture begun before it, never a notched wheel's.
//!
//! The scroll agent's gesture cases. A live run and its replay (the same
//! inputs, each with the presented frame it was stamped with) agree.

#![expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]

use std::time::Duration;

use gui::app::Scenario;
use gui::clock;
use gui::compositor::{Seen, WheelInput, WheelPhase, Wheeled};
use gui::prelude::*;
use gui::{ScrollHandle, Signal, div};

/// The three scroll containers: the page around, and two lists 100 px
/// square side by side at its top, each scrolling down 300 px.
#[derive(Clone, Copy)]
struct Lists {
    page: ScrollHandle,
    left: ScrollHandle,
    right: ScrollHandle,
}

/// The page (400×100, its content 300 tall), the two lists in it; when
/// `swapped`, the right list shows on the left and the left one on the
/// right (a commit that changes what is under the pointer).
fn scene(l: Lists, swapped: Signal<bool>) -> Scenario {
    Scenario::measuring(
        move || {
            let list = |h: &ScrollHandle, left: f32| {
                div()
                    .position(Position::Absolute)
                    .left(left)
                    .top(0.0)
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .track_scroll(h)
                    .child(div().height(400.0))
            };
            let (a, b) = if swapped.get() { (200.0, 0.0) } else { (0.0, 200.0) };
            div()
                .width(400.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&l.page)
                .child(
                    div()
                        .position(Position::Relative)
                        .width(400.0)
                        .height(300.0)
                        .child(list(&l.left, a))
                        .child(list(&l.right, b)),
                )
                .into_element()
        },
        400,
        100,
        1.0,
    )
    .expect("a scenario")
}

/// A wheel input as the platform thread stamps it: at `at` seconds on the
/// clock, under the presented frame `seen`.
fn input(at: f32, dy: f32, precise: bool, phase: WheelPhase, seen: u64) -> WheelInput {
    WheelInput {
        point: (50.0, 50.0),
        delta: (0.0, dy),
        time: clock::nanos_of(at),
        precise,
        phase,
        seen: Seen::Frame(seen),
    }
}

/// Where the three containers are once the UI took in what the
/// compositor did: (page, left, right).
fn offsets(s: &mut Scenario, l: Lists) -> (f32, f32, f32) {
    s.frame("taken in").expect("a frame");
    (l.page.offset().1, l.left.offset().1, l.right.offset().1)
}

fn lists() -> Lists {
    Lists {
        page: ScrollHandle::new(),
        left: ScrollHandle::new(),
        right: ScrollHandle::new(),
    }
}

/// What a run gives: each wheel as stamped, what it became, and the
/// offsets at the end.
type Run = (Vec<WheelInput>, Vec<Wheeled>, (f32, f32, f32));

/// Frame A is presented (the left list under the pointer); a commit B
/// swaps the lists and is presented before the render thread takes in
/// wheels stamped under A. `feed` gives each wheel its stamp: the first
/// run's (`None`: A's frame for those before B, B's after), or a replay's.
fn run(phased: bool, replayed: Option<&[WheelInput]>) -> Run {
    let l = lists();
    let swapped = Signal::new(false);
    let mut s = scene(l, swapped);
    s.frame("A").expect("a frame");
    let t0 = clock::current_seconds();
    let a = s.compositor().presented().frame;
    s.act("B swaps the lists", || swapped.set(true));
    s.frame("B").expect("a frame");
    let b = s.compositor().presented().frame;
    assert!(b > a, "B is presented after A");
    // The first two under A, the rest under B: one gesture.
    let (first, rest) = if phased {
        (WheelPhase::Began, WheelPhase::Changed)
    } else {
        (WheelPhase::None, WheelPhase::None)
    };
    let script = [
        input(t0 + 0.001, 30.0, phased, first, a),
        input(t0 + 0.017, 30.0, phased, rest, a),
        input(t0 + 0.033, 30.0, phased, rest, b),
        input(t0 + 0.049, 30.0, phased, rest, b),
    ];
    let inputs = replayed.map_or_else(|| script.to_vec(), <[WheelInput]>::to_vec);
    let mut became = Vec::new();
    for w in &inputs {
        clock::advance_to(Duration::from_nanos(w.time));
        let c = s.compositor();
        became.push(c.wheel_input(*w));
        c.tick(clock::current_seconds());
    }
    (inputs, became, offsets(&mut s, l))
}

/// A notched wheel stamped under A, taken in after B is presented, goes
/// to A's list under the pointer; its later turns, under B (the other list
/// under the pointer now), follow the latch. A replay of the same inputs
/// gives the same.
#[test]
fn a_notched_wheel_stamped_under_a_goes_to_as_list_and_its_turns_follow_it() {
    let (inputs, became, (page, left, right)) = run(false, None);
    assert!(
        became.iter().all(|w| *w == Wheeled::Scrolled),
        "each turn scrolls: {became:?}"
    );
    assert_eq!((page, right), (0.0, 0.0), "nothing else moved");
    assert_eq!(left, 120.0, "all four turns on A's list");
    let replay = run(false, Some(&inputs));
    assert_eq!(replay.1, became, "the replay routes as the run did");
    assert_eq!(replay.2, (page, left, right), "and ends where it did");
}

/// The same for a trackpad's gesture: its first event, stamped under A,
/// latches A's list; its moves under B follow it.
#[test]
fn a_trackpads_gesture_begun_under_a_stays_on_as_list() {
    let (inputs, _, (page, left, right)) = run(true, None);
    assert_eq!((page, right), (0.0, 0.0), "nothing else moved");
    assert!(left > 0.0, "A's list took the gesture");
    let replay = run(true, Some(&inputs));
    assert_eq!(replay.2, (page, left, right), "the replay ends where the run did");
}

/// The first wheel of a gesture over a list at its end goes to a container
/// around it that can move that way (the page); its later turns follow
/// the page, even once the list could move.
#[test]
fn a_gestures_first_wheel_picks_an_ancestor_that_can_move() {
    let l = lists();
    let swapped = Signal::new(false);
    let mut s = scene(l, swapped);
    s.act("the left list to its end", || l.left.scroll_to((0.0, 300.0)));
    s.frame("A").expect("a frame");
    let t0 = clock::current_seconds();
    let seen = s.compositor().presented().frame;
    for k in 0..3_u8 {
        let at = t0 + 0.001 + 0.016 * f32::from(k);
        clock::advance_to(Duration::from_secs_f32(at));
        let w = s
            .compositor()
            .wheel_input(input(at, 30.0, false, WheelPhase::None, seen));
        assert_eq!(w, Wheeled::Scrolled, "turn {k}");
    }
    let (page, left, _) = offsets(&mut s, l);
    assert_eq!(page, 90.0, "the page took the gesture");
    assert_eq!(left, 300.0, "the list at its end did not");
}

/// A trackpad's gesture on A's list, its fingers resting, when the program
/// jumps the list: the rest of the gesture, and the platform's momentum
/// after the lift, are the reader's input after the jump, so they move on
/// from it, on A's list alone; a notched wheel's turn after scrolls on
/// from there (ADR-0032: a jump is an assignment; only the compositor's
/// own fling stops at one).
#[test]
fn a_jump_lets_the_rest_of_the_gesture_and_a_new_notch_go_on_from_it() {
    let l = lists();
    let swapped = Signal::new(false);
    let mut s = scene(l, swapped);
    s.frame("A").expect("a frame");
    let t0 = clock::current_seconds();
    let step = |s: &mut Scenario, at: f32, dy: f32, precise: bool, phase: WheelPhase| {
        clock::advance_to(Duration::from_secs_f32(t0 + at));
        let seen = s.compositor().presented().frame;
        let c = s.compositor();
        let w = c.wheel_input(input(t0 + at, dy, precise, phase, seen));
        c.tick(t0 + at);
        w
    };
    step(&mut s, 0.0, 20.0, true, WheelPhase::Began);
    step(&mut s, 0.016, 20.0, true, WheelPhase::Changed);
    s.frame("moved").expect("a frame");
    // The fingers rest; the program jumps the list.
    clock::advance_to(Duration::from_secs_f32(t0 + 0.3));
    s.act("jump", || l.left.scroll_to((0.0, 100.0)));
    s.frame("jumped").expect("a frame");
    let (page, _, right) = offsets(&mut s, l);
    assert_eq!(
        step(&mut s, 0.35, 20.0, true, WheelPhase::Changed),
        Wheeled::Scrolled,
        "the fingers' move"
    );
    step(&mut s, 0.36, 0.0, true, WheelPhase::Ended);
    for (k, phase) in [
        WheelPhase::MomentumBegan,
        WheelPhase::Momentum,
        WheelPhase::MomentumEnded,
    ]
    .into_iter()
    .enumerate()
    {
        let at = 0.37 + 0.016 * f32::from(u8::try_from(k).unwrap_or(0));
        step(&mut s, at, 20.0, true, phase);
    }
    let after = offsets(&mut s, l);
    assert!(after.1 > 100.0, "on from where the program put it: {after:?}");
    assert_eq!((after.0, after.2), (page, right), "A's list alone");
    assert_eq!(
        step(&mut s, 0.5, 40.0, false, WheelPhase::None),
        Wheeled::Scrolled,
        "a new notch"
    );
    assert_eq!(offsets(&mut s, l).1, after.1 + 40.0, "on from there");
}

/// The shared fixture's layered scene: a scroll container (100×60) in a
/// group that filters (a layer) scaled three times about its corner at
/// (9.825, 19.825), so its clips' edges fall between pixels and the
/// composite resamples them; in it two rounded cards (`radius`, 60
/// square) that clip, and in both a black field (80×60) that scrolls and
/// counts its clicks. The page is white. Without `filtered` the group
/// is drawn straight to the canvas, no layer between.
fn layered(
    outer: ScrollHandle,
    inner: ScrollHandle,
    radius: Signal<f32>,
    filtered: Signal<bool>,
    clicks: Signal<u32>,
) -> Scenario {
    Scenario::measuring(
        move || {
            let r = radius.get();
            let filters = if filtered.get() {
                gui::style::Filters::repeat(1, Filter::Brightness(1.0))
            } else {
                gui::style::Filters::default()
            };
            let card = |c: gui::element::Div| {
                div()
                    .width(60.0)
                    .height(60.0)
                    .radius(Corners::all(r))
                    .overflow(Overflow::Hidden)
                    .child(c)
            };
            let field = div()
                .margin_left(-10.0)
                .width(80.0)
                .height(60.0)
                .background(gui::color::Color::BLACK)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&inner)
                .on_click(move |_| clicks.set(clicks.get_untracked().saturating_add(1)))
                .child(div().width(80.0).height(200.0));
            let around = div()
                .width(100.0)
                .height(60.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&outer)
                .child(div().width(100.0).height(200.0).child(card(card(field))));
            let layer = div()
                .scale(3.0)
                .origin(Origin::At([Calc::fraction(0.0), Calc::fraction(0.0)]))
                .filters(filters)
                .child(around);
            div()
                .width(400.0)
                .height(200.0)
                .background(gui::color::Color::WHITE)
                .padding_left(9.825)
                .padding_top(19.825)
                .child(layer)
                .into_element()
        },
        400,
        200,
        1.0,
    )
    .expect("a scenario")
}

/// What a read-back pixel shows of the black field over white.
fn shown(shot: &gui::image::Pixels, x: u32, y: u32) -> f32 {
    shot.pixel(x, y).map_or(0.0, |p| 1.0 - f32::from(p[0]) / 255.0)
}

/// The pixel on row 60 of the frame presented that shows a little over
/// half of the field (read back): its column, row and centre.
fn part_shown(s: &mut Scenario) -> (u32, u32, (f32, f32)) {
    let shot = s.shot(false).expect("read back");
    let y = 60_u32;
    let x = (0..40_u32)
        .find(|&x| (0.5..0.7).contains(&shown(&shot, x, y)))
        .expect("a pixel part shown");
    let centre = |v: u32| f32::from(u16::try_from(v).expect("small")) + 0.5;
    (x, y, (centre(x), centre(y)))
}

/// A notched wheel at `at`, stamped under the frame `seen`, one
/// millisecond on: what the compositor made of it.
fn wheel_under(s: &mut Scenario, at: (f32, f32), seen: u64) -> Wheeled {
    let t = clock::current_seconds() + 0.001;
    clock::advance_to(Duration::from_secs_f32(t));
    s.compositor().wheel_input(WheelInput {
        point: at,
        delta: (0.0, 10.0),
        time: clock::nanos_of(t),
        precise: false,
        phase: WheelPhase::None,
        seen: Seen::Frame(seen),
    })
}

/// Frame A is drawn through a layer that resamples; commit B draws the
/// same boxes straight to the canvas, and is presented. A wheel stamped
/// under A, on a pixel A showed a little over half of the field and B
/// shows less of, routes by A's layers with A's tables: the frame's
/// hit-test data is all of that one frame, its layers too.
#[test]
fn a_wheel_stamped_under_an_older_frame_reads_that_frames_layers() {
    let (outer, inner) = (ScrollHandle::new(), ScrollHandle::new());
    let (radius, filtered, clicks) = (Signal::new(8.0_f32), Signal::new(true), Signal::new(0_u32));
    let mut s = layered(outer, inner, radius, filtered, clicks);
    s.frame("A").expect("a frame");
    let (x, y, at) = part_shown(&mut s);
    let a = s.presented();
    assert!(a.stages.iter().any(|l| l.resamples()), "A is drawn through a layer");
    s.act("B draws to the canvas", || filtered.set(false));
    s.frame("B").expect("a frame");
    let b = s.presented();
    assert!(b.frame > a.frame, "B is presented");
    assert!(!b.stages.iter().any(|l| l.resamples()), "B has no layer to resample");
    let shot = s.shot(false).expect("B read back");
    assert!(shown(&shot, x, y) < 0.5, "B shows less than half of the field there");
    assert_eq!(wheel_under(&mut s, at, a.frame), Wheeled::Scrolled, "the wheel scrolls");
    s.frame("scrolled").expect("a frame");
    assert!(inner.offset().1 > 0.0, "the field took the wheel, as A showed it");
    assert_eq!(outer.offset().1, 0.0, "the container around it did not");
}

/// The devtools send input as the platform does, through the frame the
/// screen shows: a click sent at a point A shows the field on lands on
/// it while commit B, which cuts that pixel away, is laid out and not
/// yet presented.
#[test]
fn the_devtools_send_input_through_the_frame_shown() {
    let (outer, inner) = (ScrollHandle::new(), ScrollHandle::new());
    let (radius, filtered, clicks) = (Signal::new(8.0_f32), Signal::new(true), Signal::new(0_u32));
    let mut session = gui::devtools::Session::new(layered(outer, inner, radius, filtered, clicks)).expect("a session");
    let (_, _, at) = part_shown(session.scenario());
    session.scenario().act("B rounds the corners more", || radius.set(20.0));
    session.run(&format!("click {},{}", at.0, at.1)).expect("clicked");
    assert_eq!(clicks.get_untracked(), 1, "the click lands on the field as A showed it");
}

/// Frame A is presented with the cards' corners 8 round; the pointer is
/// on a pixel of the corner's arc that shows a little over half of the
/// field (read back from A). Commit B rounds the corners to 20, which cuts
/// that pixel away; the UI lays it out as input comes, but it is not yet
/// presented. A click, the devtools' pick and the first wheel all land in
/// the field, as A showed it; B, once presented, shows less than half
/// there (the input did not follow B).
#[test]
fn input_before_b_is_presented_lands_as_as_pixels_show_by_click_devtools_and_wheel() {
    let (outer, inner) = (ScrollHandle::new(), ScrollHandle::new());
    let (radius, clicks) = (Signal::new(8.0_f32), Signal::new(0_u32));
    let filtered = Signal::new(true);
    let mut session = gui::devtools::Session::new(layered(outer, inner, radius, filtered, clicks)).expect("a session");
    let s = session.scenario();
    let (x, y, at) = part_shown(s);
    let seen = s.presented().frame;
    s.act("B rounds the corners more", || radius.set(20.0));
    s.send(gui::window::Input::PointerMoved { x: 399.0, y: 199.0 });
    // The click.
    s.click(at.0, at.1);
    assert_eq!(clicks.get_untracked(), 1, "the click lands on the field as A showed it");
    // The devtools' pick: the box the field's centre picks, inside it.
    let field = session.run("at 100 110").expect("the field's centre");
    let picked = session.run(&format!("at {} {}", at.0, at.1)).expect("the pick");
    assert_eq!(
        picked.lines().next(),
        field.lines().next(),
        "the devtools pick the field as A showed it"
    );
    // The first wheel, stamped under A.
    let s = session.scenario();
    assert_eq!(s.presented().frame, seen, "B is not presented yet");
    assert_eq!(wheel_under(s, at, seen), Wheeled::Scrolled, "the wheel scrolls");
    s.frame("B").expect("a frame");
    assert!(inner.offset().1 > 0.0, "the field took the wheel");
    assert_eq!(outer.offset().1, 0.0, "the container around it did not");
    let b = s.shot(false).expect("B read back");
    assert!(shown(&b, x, y) < 0.5, "B shows less than half of the field there");
}
