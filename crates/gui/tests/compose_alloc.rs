//! A frame the compositor composes over a scene at rest allocates nothing
//! (ADR-0015): its sources, its working sets and the frame it presents
//! are last frames' memory.

use gui::app::Scenario;
use gui::prelude::*;
use gui::{div, pct};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

/// Two lists side by side, each with an overlay scrollbar, the left one's
/// content a paragraph with a span that takes clicks and a text field:
/// what a frame records of the text it showed ([`gui::compositor::ShownText`]).
/// The right list is 120 wide while `wide` holds.
fn two_lists(wide: gui::Signal<bool>, field: gui::TextField) -> gui::Element {
    let list = |left: f32| {
        div()
            .position(Position::Absolute)
            .left(left)
            .top(0.0)
            .width(if left > 0.0 && wide.get() { 120.0 } else { 100.0 })
            .height(100.0)
            .overflow_y(Overflow::Scroll)
            .child(div().height(400.0))
    };
    div()
        .size(pct(100.0))
        .child(
            list(0.0)
                .child(
                    gui::element::text("")
                        .key(1)
                        .span(gui::element::span("link").key(1).on_click(|_| {})),
                )
                .child(gui::text_input(&field).width(80.0).height(24.0)),
        )
        .child(list(200.0))
        .into_element()
}

#[test]
fn a_frame_composed_over_a_scene_at_rest_allocates_nothing() {
    gui::testing::both_passes(composed);
}

#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
fn composed(pass: gui::testing::Pass) {
    let (wide, field) = (gui::Signal::new(false), gui::TextField::new("abc"));
    let mut s = Scenario::measuring(move || two_lists(wide, field), 400, 100, 1.0).expect("a scenario");
    pass.scenario(&mut s);
    s.frame("first").expect("a frame");
    // A main frame that makes the compositor's tables again (a list
    // widened) and changes no text keeps the record of the text shown:
    // nothing shaped or copied for it.
    let first = std::sync::Arc::clone(&s.compositor().presented().texts);
    s.act("widen the right list", || wide.set(true));
    s.frame("widened").expect("a frame");
    assert!(
        std::sync::Arc::ptr_eq(&first, &s.compositor().presented().texts),
        "a main frame changing no text"
    );
    let c = s.compositor();
    // Warm, each frame presented: the timeline's quarter second of frames
    // kept for late input filled, and frames it let go coming round.
    for n in 0..40_u8 {
        c.compose(f32::from(n) * 0.01);
        c.did_present();
    }
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let texts = std::sync::Arc::clone(&c.presented().texts);
    assert_eq!(texts.len(), 2, "the paragraph's spans and the field's text");
    for n in 40..44_u8 {
        let at = f32::from(n) * 0.01;
        let ((), heap) = gui::heap::measure(|| {
            c.compose(at);
            c.did_present();
        });
        let heap = heap.expect("counted");
        pass.measured(|| {
            assert_eq!(
                (heap.allocations, heap.allocated),
                (0, 0),
                "frame {n} composed at rest allocated"
            );
        });
        // The text each frame showed is the one record, shared, not made
        // again a frame.
        assert!(std::sync::Arc::ptr_eq(&texts, &c.presented().texts), "frame {n}");
    }
}

/// A list that scrolls (its rows unclipped, one of them masked by a
/// gradient), a card whose corners a signal rounds, and 20,000 boxes that
/// clip: property trees of more than 20,000 entries.
fn many_clips(radius: gui::Signal<f32>, list: gui::ScrollHandle) -> gui::Element {
    let card = div()
        .width(60.0)
        .height(40.0)
        .radius(Corners::all(radius.get()))
        .overflow(Overflow::Hidden)
        .child(div().width(80.0).height(40.0));
    let fade = ScrollFade {
        axis: Axis::Vertical,
        start: true,
        end: true,
        size: Some(10.0),
    };
    let row = div().width(80.0).height(60.0).background(gui::Color::BLACK);
    // Both edges fade in a 100 high list: a gradient there is.
    let masked = match fade.gradient(60.0, 30.0, 100.0) {
        Some(mask) => row.mask(mask),
        None => row,
    };
    let rows = div()
        .width(100.0)
        .height(100.0)
        .overflow_y(Overflow::Scroll)
        .track_scroll(&list)
        .child(masked)
        .child(div().height(400.0));
    div()
        .size(pct(100.0))
        .child(rows)
        .child(card)
        .children((0..20_000).map(|_| {
            div()
                .width(4.0)
                .height(0.01)
                .overflow(Overflow::Hidden)
                .child(div().width(8.0).height(1.0))
        }))
        .into_element()
}

/// Each frame's property trees, which input reads, take in only the
/// entries the frame changed: over trees of 20,000 clips, a commit that
/// rounds one card's corners copies a few entries, not the trees, and a
/// frame the compositor scrolls (a masked effect moving with it) copies a
/// few and, composed and presented onto the timeline, allocates nothing.
#[test]
fn a_frames_property_trees_copy_what_changed_not_the_trees() {
    gui::testing::both_passes(copied);
}

#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
fn copied(pass: gui::testing::Pass) {
    let (radius, list) = (gui::Signal::new(4.0_f32), gui::ScrollHandle::new());
    let mut s = Scenario::measuring(move || many_clips(radius, list), 400, 300, 1.0).expect("a scenario");
    pass.scenario(&mut s);
    s.frame("first").expect("a frame");
    // Each commit a third of a second after the last: the timeline lets
    // the frame before go (a quarter second on), to be used again.
    let mut later = 0.0_f32;
    let mut later_by = |s: f32| {
        later += s;
        gui::clock::advance_to(std::time::Duration::from_secs_f32(later));
    };
    for n in 0..8_u8 {
        later_by(0.3);
        s.act("warm", || radius.set(4.0 + f32::from(n) * 0.5));
        s.frame("warm").expect("a frame");
    }
    let clips = s.presented().properties.clips.len();
    assert!(clips > 20_000, "{clips} clips");
    assert!(
        s.presented().properties.effects.iter().any(|e| e.mask.is_some()),
        "a masked effect"
    );
    // A commit that rounds the card's corners.
    for n in 0..4_u8 {
        let before = s.compositor().props_copied();
        later_by(0.3);
        s.act("round", || radius.set(10.0 + f32::from(n)));
        s.frame("rounded").expect("a frame");
        let copied = s.compositor().props_copied().saturating_sub(before);
        pass.measured(|| assert!(copied <= 16, "frame {n}: {copied} entries copied of {clips}"));
    }
    // The compositor scrolls the list, frame by frame.
    let c = s.compositor();
    let t0 = later + 1.0;
    for n in 0..32_u8 {
        let at = t0 + f32::from(n) * 0.016;
        gui::clock::advance_to(std::time::Duration::from_secs_f32(at));
        let seen = c.on_screen().map_or(gui::compositor::Seen::BeforeFirst, |p| {
            gui::compositor::Seen::Frame(p.frame)
        });
        let wheeled = c.wheel_input(gui::compositor::WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 4.0),
            time: gui::clock::nanos_of(at),
            precise: false,
            phase: gui::compositor::WheelPhase::None,
            seen,
        });
        assert_eq!(
            wheeled,
            gui::compositor::Wheeled::Scrolled,
            "frame {n}: the list scrolls"
        );
        let before = c.props_copied();
        let ((), heap) = gui::heap::measure(|| {
            c.compose(at);
            c.did_present();
        });
        // Drawn, as the render thread draws each frame it composes.
        c.drew();
        let heap = heap.expect("counted");
        let copied = c.props_copied().saturating_sub(before);
        // Past the frames that warm the buffers (each compose's record of
        // what it wrote comes round again after a dozen, sized by the most
        // a scrolled frame writes once each has come round). The frame
        // recycled is one the timeline let go of, presented several frames
        // back: it takes in what each frame since wrote (the list's scroll,
        // the masked effect moving with it), a few entries each.
        if n >= 24 {
            pass.measured(|| {
                assert!(copied <= 32, "scrolled frame {n}: {copied} entries copied of {clips}");
                assert_eq!(
                    (heap.allocations, heap.allocated),
                    (0, 0),
                    "scrolled frame {n} allocated"
                );
            });
        }
    }
}

/// Codex r7 nit: the display-rate cost of the memory bound. The compositor
/// scrolls a list over trees of 20,000 clips at 240 Hz for half a second.
/// The first quarter second fills the timeline: each frame is a new
/// snapshot, a whole copy of the trees. After that, frames come back
/// from the timeline and copy what changed, allocating nothing. The
/// copies made are at most the frames the timeline keeps at 240 Hz and
/// the pool: `ceil(RETAIN / interval) + 1 + POOL + 2`.
#[test]
fn the_first_quarter_second_at_240_hz_makes_at_most_the_bound_of_copies() {
    gui::testing::both_passes(at_240_hz);
}

#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "sums of a few hundred frames' counts and nanoseconds, far from any bound"
)]
fn at_240_hz(pass: gui::testing::Pass) {
    let (radius, list) = (gui::Signal::new(4.0_f32), gui::ScrollHandle::new());
    let mut s = Scenario::measuring(move || many_clips(radius, list), 400, 300, 1.0).expect("a scenario");
    pass.scenario(&mut s);
    s.frame("first").expect("a frame");
    let p = s.presented();
    let entries = p.properties.transforms.len() + p.properties.clips.len() + p.properties.effects.len();
    let c = s.compositor();
    let interval = 1_000_000_000_u64 / 240;
    let kept = usize::try_from(gui::compositor::RETAIN.div_ceil(interval)).unwrap_or(usize::MAX) + 1;
    let t0 = gui::clock::current_nanos() + 1_000_000_000;
    let (mut first, mut after) = ((0_usize, 0_u64, 0_usize), (0_usize, 0_u64, 0_usize));
    for n in 0..120_u64 {
        let at = t0 + n * interval;
        gui::clock::advance_to(std::time::Duration::from_nanos(at));
        let seen = c.on_screen().map_or(gui::compositor::Seen::BeforeFirst, |p| {
            gui::compositor::Seen::Frame(p.frame)
        });
        c.wheel_input(gui::compositor::WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 1.0),
            time: at,
            precise: false,
            phase: gui::compositor::WheelPhase::None,
            seen,
        });
        let before = c.props_copied();
        let ((), heap) = gui::heap::measure(|| {
            c.compose(at);
            c.did_present();
        });
        c.drew();
        let heap = heap.expect("counted");
        let copied = c.props_copied().saturating_sub(before);
        // Until the timeline lets its first frame go (its quarter second
        // full, and a frame or two more), each frame is a new snapshot.
        let filling = usize::try_from(n).unwrap_or(usize::MAX) < kept + 4;
        let into = if filling { &mut first } else { &mut after };
        into.0 += usize::try_from(heap.allocations).unwrap_or(usize::MAX);
        into.1 += heap.allocated;
        into.2 += copied;
    }
    eprintln!(
        "240 Hz over {entries} entries: filling ({} frames) {} allocations, {} bytes, {} entries copied; after {} allocations, {} bytes, {} entries copied",
        kept + 4,
        first.0,
        first.1,
        first.2,
        after.0,
        after.1,
        after.2
    );
    pass.measured(|| {
        assert!(
            first.2 <= (kept + 5) * entries,
            "at most the bound of whole copies: {} entries copied, {} bound",
            first.2,
            (kept + 5) * entries
        );
        assert!(after.2 <= 120 * 32, "after, what changed: {} entries copied", after.2);
        assert_eq!(after.0, 0, "after, nothing allocated");
    });
}
