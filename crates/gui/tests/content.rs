//! `ContentStyle` (ADR-0022, research §7.10): a container provides the
//! colour, type and icon size of its subtree's content; only text, icons
//! and text fields read it, each for what it does not set itself. A
//! container's hover colour reaches its text with only paint dirty: no
//! instance renders, nothing lays out, nothing below it resolves, and only
//! the text and icons that show the colour are recorded again.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a scenario that cannot run, or a node not found, is a failed test"
)]

use std::cell::Cell;
use std::rc::Rc;

use gui::app::Scenario;
use gui::icon::Icon;
use gui::layout::{Content, Frame};
use gui::prelude::*;
use gui::{Color, Interaction, Signal, div, icon, span, text};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

fn rest() -> Color {
    Color::hex(0x0033_4466)
}
fn lit() -> Color {
    Color::hex(0x00e6_1a1a)
}
fn own() -> Color {
    Color::hex(0x001a_9933)
}

/// The colour the text node reading `label` is drawn in.
fn text_color(frame: &Frame, label: &str) -> Color {
    frame
        .nodes
        .iter()
        .find_map(|n| match &n.content {
            Content::Text { content, text, .. } if &**content == label => Some(text.color),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text {label:?}"))
}

/// The colour of the one icon.
fn icon_color(frame: &Frame) -> Color {
    frame
        .nodes
        .iter()
        .find_map(|n| match &n.content {
            Content::Icon { color, .. } => Some(*color),
            _ => None,
        })
        .expect("an icon")
}

/// A card whose colour is lit while hovered: a label straight in it, a
/// nested box holding text and an icon that set nothing, and a box that
/// provides a colour of its own to the text in it.
fn card(renders: &Rc<Cell<u32>>) -> gui::Element {
    renders.set(renders.get().saturating_add(1));
    let i = Interaction::new();
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .child(
            div()
                .interaction(i)
                .padding(8.0)
                .text_size(14.0)
                .text_color(move || Some(if i.hovered() { lit() } else { rest() }))
                .child(text("label"))
                .child(div().padding(4.0).child(text("nested")).child(icon(Icon::Plus)))
                .child(div().text_color(own()).child(text("own"))),
        )
        .child(text("outside"))
        .into_element()
}

/// Hover the card and leave it, counting what each frame did; with
/// `cold`, each frame is checked against one built cold instead (whose
/// own build the counts would include).
fn hover_card(cold: bool) {
    let renders = Rc::new(Cell::new(0));
    let counted = Rc::clone(&renders);
    let mut s = Scenario::measuring(move || card(&counted), 300, 300, 1.0).expect("a scenario");
    s.check_cold(cold);
    s.settle().expect("first frame");
    assert_eq!(text_color(s.last(), "label"), rest());
    assert_eq!(text_color(s.last(), "nested"), rest());
    assert_eq!(text_color(s.last(), "own"), own());
    assert_eq!(text_color(s.last(), "outside"), Color::BLACK);
    let before = renders.get();
    s.take_reports();

    for (step, on) in [true, false, true].into_iter().enumerate() {
        if on {
            let (x, y) = s.locate("label").expect("the label");
            s.hover(x, y);
        } else {
            let (x, y) = s.locate("outside").expect("the text outside");
            s.hover(x, y);
        }
        let r = s.frame(&format!("hover {step}")).expect("a frame");
        assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
        if !cold {
            hover_counts(&r);
        }
        let want = if on { lit() } else { rest() };
        assert_eq!(text_color(s.last(), "label"), want);
        assert_eq!(text_color(s.last(), "nested"), want);
        assert_eq!(icon_color(s.last()), want);
        assert_eq!(
            text_color(s.last(), "own"),
            own(),
            "a box's own colour stays its subtree's"
        );
        assert_eq!(text_color(s.last(), "outside"), Color::BLACK);
    }
    if !cold {
        // A cold build renders it once more each frame.
        assert_eq!(renders.get(), before, "the card never rendered again");
    }
}

/// What a hover frame of the card did: only paint is dirty.
fn hover_counts(r: &gui::inspect::FrameReport) {
    assert_eq!(r.layout.calls, 0, "nothing lays out");
    assert_eq!(r.visits.resolved, 1, "the card alone resolves: {}", r.visits);
    assert_eq!(r.bindings, 1, "its colour binding alone runs");
    // Below the card: the label, the nested box, its text and icon,
    // and the box with a colour of its own take the new colour
    // unresolved; the text in that box is not reached.
    assert_eq!(r.visits.repainted, 5, "{}", r.visits);
    assert_eq!(
        r.paint.recorded, 3,
        "the label, the nested text and the icon alone record again: {}",
        r.paint
    );
}

#[test]
fn a_hover_colour_reaches_its_text_with_only_paint_dirty() {
    hover_card(false);
}

#[test]
fn a_hover_colour_gives_what_a_cold_build_does() {
    hover_card(true);
}

/// What a container provides at once, typed, and what its text reads of
/// it: each field it sets, the enclosing provider's for the rest.
#[test]
fn a_provided_content_style_reaches_only_what_it_does_not_set() {
    let mut s = Scenario::measuring(
        || {
            div()
                .text_color(rest())
                .font_weight(700)
                .child(
                    div()
                        .provide_content(ContentStyle {
                            color: Some(lit()),
                            text: TextStyle {
                                size: Some(20.0),
                                tabular_nums: Some(true),
                                ..TextStyle::default()
                            },
                            icon_size: Some(24.0),
                        })
                        .child(text("provided"))
                        .child(text("set").text_color(own()).text_size(10.0))
                        .child(icon(Icon::Plus)),
                )
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.settle().expect("a frame");
    let frame = s.last();
    let style = |label: &str| {
        frame
            .nodes
            .iter()
            .find_map(|n| match &n.content {
                Content::Text { content, text, .. } if &**content == label => Some(text.clone()),
                _ => None,
            })
            .expect("the text")
    };
    let provided = style("provided");
    assert_eq!(provided.color, lit());
    assert_eq!(provided.size, 20.0);
    assert!(
        provided.tabular_nums,
        "tabular_nums is a field of the provided text style"
    );
    assert_eq!(provided.weight, 700, "the enclosing provider's weight");
    let set = style("set");
    assert_eq!((set.color, set.size), (own(), 10.0), "a text's own values win");
    assert!(set.tabular_nums);
    let i = frame
        .nodes
        .iter()
        .find(|n| matches!(n.content, Content::Icon { .. }))
        .expect("the icon");
    assert_eq!(
        (i.bounds.width, i.bounds.height),
        (24.0, 24.0),
        "the provided icon size"
    );
}

/// A closure given to a span's setter is bound to its paragraph's node: a
/// colour it reads repaints the paragraph, a size lays it out, and the
/// instance that built it does not render again.
fn span_binding(cold: bool) {
    let hot = Signal::new(false);
    let big = Signal::new(false);
    let renders = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&renders);
    let mut s = Scenario::measuring(
        move || {
            counted.set(counted.get().saturating_add(1));
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .child(
                    text("a ")
                        .text_color(rest())
                        .span(span("hot").text_color(move || Some(if hot.get() { lit() } else { rest() })))
                        .span(span(" big").text_size(move || if big.get() { 24.0 } else { 16.0 })),
                )
                .child(text("other"))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(cold);
    s.settle().expect("a frame");
    let run_color = |frame: &Frame| {
        frame
            .nodes
            .iter()
            .find_map(|n| match &n.content {
                Content::Text { runs, .. } if !runs.is_empty() => runs.first().map(|r| r.text.color),
                _ => None,
            })
            .expect("the paragraph")
    };
    assert_eq!(run_color(s.last()), rest());
    let before = renders.get();
    s.take_reports();

    s.act("hot", || hot.set(true));
    let r = s.frame("hot").expect("a frame");
    assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
    if !cold {
        assert_eq!(r.visits.resolved, 1, "the paragraph alone resolves: {}", r.visits);
        assert_eq!(r.bindings, 1, "the span's colour binding alone runs");
        assert_eq!(r.layout.calls, 0, "a colour lays out nothing");
        assert_eq!(r.paint.recorded, 1, "the paragraph alone records again: {}", r.paint);
    }
    assert_eq!(run_color(s.last()), lit());

    s.act("big", || big.set(true));
    let r = s.frame("big").expect("a frame");
    assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
    if !cold {
        assert_eq!(r.visits.resolved, 1, "the paragraph alone resolves: {}", r.visits);
        assert!(r.layout.misses > 0, "a size lays out");
    }
    if !cold {
        // A cold build renders it once more each frame.
        assert_eq!(renders.get(), before, "the instance never rendered again");
    }
}

#[test]
fn a_span_binding_resolves_its_paragraph_not_its_instance() {
    span_binding(false);
}

#[test]
fn a_span_binding_gives_what_a_cold_build_does() {
    span_binding(true);
}

/// A paragraph of a plain span and a bold one, toggled `hot` four times:
/// what each frame allocated before paint, with the colour bound to the
/// first span (`span_binds`) or to the paragraph itself.
fn colour_frames(span_binds: bool) -> Vec<u64> {
    let hot = Signal::new(false);
    let colour = move || Some(if hot.get() { lit() } else { rest() });
    let mut s = Scenario::measuring(
        move || {
            let p = text("a ").text_color(rest());
            let p = if span_binds { p } else { p.text_color(colour) };
            let first = span("hot");
            let first = if span_binds { first.text_color(colour) } else { first };
            p.span(first).span(span(" bold").font_weight(700)).into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    (0..4)
        .map(|step| {
            s.act("hot", || hot.set(step % 2 == 0));
            let r = s.frame(&format!("hot {step}")).expect("a frame");
            assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
            assert_eq!(r.layout.calls, 0, "a colour lays out nothing");
            assert_eq!(r.visits.resolved, 1, "the paragraph alone resolves: {}", r.visits);
            r.footprint.built.map_or(u64::MAX, |t| t.allocations)
        })
        .collect()
}

/// A span's colour binding resolves its paragraph at the cost of the
/// paragraph's own: its trackers, its style and the paragraph's behaviour
/// are kept, so the frame allocates what a bound paragraph colour does.
#[test]
fn a_span_colour_allocates_what_its_paragraphs_does() {
    assert_eq!(colour_frames(true), colour_frames(false));
}

/// A span that stops binding lets go of what its closure read, while
/// another span of its paragraph still binds: writing that signal then
/// resolves nothing.
#[test]
fn a_span_that_stops_binding_reads_nothing() {
    let binds = Signal::new(true);
    let hot = Signal::new(false);
    let big = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            let first = span("hot");
            let first = if binds.get() {
                first.text_color(move || Some(if hot.get() { lit() } else { rest() }))
            } else {
                first.text_color(own())
            };
            text("a ")
                .span(first)
                .span(span(" big").text_size(move || if big.get() { 24.0 } else { 16.0 }))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    s.act("bound", || hot.set(true));
    let r = s.frame("bound").expect("a frame");
    assert_eq!(
        r.visits.resolved, 1,
        "the bound span resolves its paragraph: {}",
        r.visits
    );
    s.act("unbind", || binds.set(false));
    s.frame("unbind").expect("a frame");
    s.act("stale", || hot.set(false));
    let r = s.frame("stale").expect("a frame");
    assert_eq!(
        r.visits.resolved, 0,
        "the closure it dropped reads nothing: {}",
        r.visits
    );
    assert_eq!(r.bindings, 0, "no binding runs");
}
