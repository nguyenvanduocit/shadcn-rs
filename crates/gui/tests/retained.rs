//! ADR-0016 end to end: a scroll, a fade or a move changes property tree
//! entries and nothing else, and a hover redraws only the chunk it changed.
//! What a frame records, lays out, plans, uploads and redraws is in its
//! report (`FrameReport::paint`, `laid_out`, `drawn`).

use std::time::Duration;

use gui::prelude::*;
use gui::style::Calc;
use gui::{Color, Scenario, Signal, div, text};

fn rows() -> impl IntoElement {
    div()
        .size(pct(100.0))
        .overflow_y(Overflow::Scroll)
        .overflow_x(Overflow::Hidden)
        .children((0..80).map(|i| {
            div()
                .height(24.0)
                .background(if i % 2 == 0 { Color::hex(0xf4f4f5) } else { Color::WHITE })
                .child(text(format!("row {i}")))
        }))
}

#[test]
fn a_scroll_records_nothing_again_and_lays_out_nothing() {
    let mut s = Scenario::measuring(|| rows().into_element(), 300, 200, 2.0).expect("a scenario");
    s.frame("first").expect("a frame");
    s.hover(100.0, 100.0);
    s.frame("rest").expect("a frame");
    for step in 0..4 {
        // ADR-0015: the compositor scrolls and draws the scroll by itself.
        let shown = s.presented().frame;
        assert!(s.wheel(40.0), "step {step}: the compositor scrolls");
        assert_eq!(
            s.presented().frame,
            shown + 1,
            "step {step}: and draws it without the UI"
        );
        let r = s.frame(&format!("scrolled {step}")).expect("a frame");
        assert_eq!(r.paint.recorded, 0, "a scroll records no chunk again: {}", r.paint);
        assert_eq!(r.layout.placements, 0, "a scroll lays out nothing");
        assert!(
            !r.drawn.submitted,
            "the UI's frame shows what the compositor drew: {}",
            r.drawn
        );
        assert_eq!(r.drawn.encoded, 0, "and encodes no chunk again: {}", r.drawn);
    }
}

/// A box faded or moved by a transition: every frame of it writes property
/// entries only.
fn moving(shown: Signal<bool>, opacity: bool) -> impl IntoElement {
    let on = shown.get();
    let b = div()
        .width(120.0)
        .height(40.0)
        .background(Color::hex(0x18181b))
        .duration_ms(200.0)
        .ease(Easing::Linear)
        .child(text("moving"));
    let b = if opacity {
        b.transition(Properties::OPACITY).opacity(if on { 1.0 } else { 0.2 })
    } else {
        b.transition(Properties::TRANSFORM)
            .translate_y(Calc::px(if on { 0.0 } else { 40.0 }))
    };
    div().size(pct(100.0)).padding(8.0).child(b)
}

#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
fn animates_without_recording(opacity: bool) {
    let shown = Signal::new(true);
    let mut s = Scenario::measuring(move || moving(shown, opacity).into_element(), 300, 200, 2.0).expect("a scenario");
    s.frame("first").expect("a frame");
    s.frame("rest").expect("a frame");
    s.play_motion();
    s.act("toggle", || shown.set(false));
    s.frame("starts").expect("a frame");
    for step in 0..5 {
        s.wait(Duration::from_millis(30));
        let r = s.frame(&format!("step {step}")).expect("a frame");
        assert_eq!(r.paint.recorded, 0, "step {step} records no chunk: {}", r.paint);
        assert_eq!(r.layout.placements, 0, "step {step} lays out nothing");
        assert!(r.drawn.submitted, "step {step} is drawn");
        // The render side plans from what changed: a move signs the box and
        // its text again; a fade, once it has given them a layer (the text
        // overlaps the box), signs neither and composites the layer kept.
        if step > 0 || !opacity {
            assert!(
                !r.drawn.planned_whole,
                "step {step} is planned from what changed: {}",
                r.drawn
            );
            let signed = if opacity { 0 } else { 2 };
            assert_eq!(r.drawn.signed, signed, "step {step}: {}", r.drawn);
        }
    }
}

#[test]
fn a_fade_records_nothing_again_and_lays_out_nothing() {
    animates_without_recording(true);
}

#[test]
fn a_move_records_nothing_again_and_lays_out_nothing() {
    animates_without_recording(false);
}

#[test]
fn an_unchanged_frame_submits_nothing() {
    let mut s = Scenario::measuring(|| rows().into_element(), 300, 200, 2.0).expect("a scenario");
    s.frame("first").expect("a frame");
    s.frame("again").expect("a frame");
    let r = s.frame("still").expect("a frame");
    assert!(!r.drawn.submitted, "nothing changed, nothing submitted: {}", r.drawn);
    assert_eq!(r.drawn.uploaded, 0);
    assert_eq!(r.drawn.damage, 0);
    assert!(!r.drawn.planned_whole, "nor planned whole");
    assert_eq!(r.drawn.signed, 0, "nor any chunk signed: {}", r.drawn);
}

#[test]
fn a_hover_redraws_its_chunk_and_its_shadow_alone() {
    let mut s = Scenario::measuring(
        || {
            div()
                .size(pct(100.0))
                .padding(40.0)
                .gap(16.0)
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .child(text("A page with a button"))
                .child({
                    let i = Interaction::new();
                    div()
                        .interaction(i)
                        .width(120.0)
                        .height(36.0)
                        .radius(8.0)
                        .shadow([
                            Shadow {
                                x: 0.0,
                                y: 4.0,
                                blur: 6.0,
                                spread: -1.0,
                                color: Color::BLACK.opacity(0.1),
                            },
                            Shadow {
                                x: 0.0,
                                y: 2.0,
                                blur: 4.0,
                                spread: -2.0,
                                color: Color::BLACK.opacity(0.1),
                            },
                        ])
                        .background(move || Color::hex(if i.hovered() { 0x3f3f46 } else { 0x18181b }))
                        .child(text("Button"))
                })
                .child(text("Something below it"))
                .into_element()
        },
        400,
        300,
        2.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    s.hover(5.0, 290.0);
    s.frame("away").expect("a frame");
    s.frame("rest").expect("a frame");
    let (x, y) = s.locate("Button").expect("the button");
    s.hover(x, y);
    let r = s.frame("hovered").expect("a frame");
    assert_eq!(r.paint.recorded, 1, "the button alone is recorded again: {}", r.paint);
    assert_eq!(r.drawn.encoded, 1, "and encoded again: {}", r.drawn);
    assert!(
        !r.drawn.planned_whole && r.drawn.signed == 1,
        "and it alone signed again: {}",
        r.drawn
    );
    assert!(r.drawn.uploaded <= 1024, "{} bytes uploaded", r.drawn.uploaded);
    // The button (240x72 device pixels) and its shadow's reach (4 px down,
    // 6 px of blur: 3σ + 1 = 10 px each way, 2x) and nothing else.
    assert_eq!(r.drawn.damage_rects, 1, "{}", r.drawn);
    let most = (240 + 2 * 22) * (72 + 2 * 22 + 8);
    assert!(
        r.drawn.damage > 240 * 72 && r.drawn.damage <= most,
        "{} pixels redrawn, at most {most}",
        r.drawn.damage
    );
}
