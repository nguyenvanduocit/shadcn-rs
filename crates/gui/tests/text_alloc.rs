//! Text drawn again allocates nothing once warm, whatever it draws: run
//! backgrounds and a run's own decorations (found from the glyphs' places
//! and put down before them, the glyphs never moved), a line-through a
//! box of another font hands down (set by that font's ascent), a
//! decorated paragraph resolved again (its spans' decorations found, not
//! made), a line cut short by
//! an ellipsis or a line clamp (one text clamped at two widths too), and chunks of very different piece counts
//! recorded by turns (each keeps room for its own pieces only).
//!
//! What is measured is the frame from paint's record onward: recording the
//! chunks again, committing them and the frame's end and report (layout's
//! own copies are another test's).

use gui::app::Scenario;
use gui::prelude::*;
use gui::{Color, Interaction, div, span, text};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

const REST: Color = Color::WHITE;
const LIT: Color = Color::BLACK;

/// A column of eight rows, each drawn by `row` with an interaction whose
/// hover lights it, so hovering a row draws its text again.
fn rows(row: fn(usize, Interaction) -> gui::Element) -> gui::Element {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .children((0..8).map(move |r| row(r, Interaction::new())))
        .into_element()
}

/// The bytes a frame's thread allocated from paint's record onward:
/// recording, then the frame's end and report (the commit's hand-off aside).
fn from_record(r: &gui::inspect::FrameReport) -> (u64, u64) {
    let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
    let recorded = bytes(r.footprint.recorded);
    let after = bytes(r.footprint.thread)
        .saturating_sub(bytes(r.footprint.built))
        .saturating_sub(bytes(r.footprint.committed));
    (recorded, after)
}

/// Hover the points `at` (two rows) by turns once warm: each frame draws
/// what they hold again (at least `records` chunks recorded again), and
/// nothing from the record onward allocates. The scenario, its last frame
/// hovering `at[1]`.
#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn hover(
    build: impl Fn() -> gui::Element + 'static,
    at: [(f32, f32); 2],
    size: (u32, u32),
    records: usize,
) -> Scenario {
    let mut s = Scenario::measuring(build, size.0, size.1, 1.0).unwrap();
    gui::reactive::stop_trace();
    s.check_cold(false);
    s.settle().unwrap();
    for _ in 0..10 {
        for (x, y) in at {
            s.hover(x, y);
            s.settle().unwrap();
            s.frame("warm").unwrap();
        }
    }
    for (step, (x, y)) in at.into_iter().cycle().take(16).enumerate() {
        s.hover(x, y);
        let r = s.frame(&format!("hover {step}")).unwrap();
        assert!(
            r.paint.recorded >= records,
            "at least {records} chunks record again: {}",
            r.paint
        );
        let (recorded, after) = from_record(&r);
        assert_eq!(recorded, 0, "{recorded} bytes allocated recording hover {step}");
        assert_eq!(after, 0, "{after} bytes allocated from the record on, hover {step}");
    }
    s
}

/// A row whose paragraph has a run with a rounded, padded background:
/// drawn again, its background goes down before its glyphs, which are
/// never moved, and nothing allocates.
#[test]
fn drawing_run_backgrounds_again_allocates_nothing() {
    fn row(r: usize, i: Interaction) -> gui::Element {
        text(format!("row {r} with "))
            .span(
                span("a chip")
                    .background(Color::hex(0x3b_82_f6))
                    .radius(4.0)
                    .padding_x(4.0),
            )
            .span(" and more")
            .interaction(i)
            .height(24.0)
            .background(move || if i.hovered() { LIT } else { REST })
            .into_element()
    }
    hover(|| rows(row), [(40.0, 84.0), (40.0, 108.0)], (400, 400), 2);
}

/// A row whose runs declare their own underline (skipping descenders'
/// ink) and line-through, drawn again, allocating nothing.
#[test]
fn drawing_a_runs_own_decorations_again_allocates_nothing() {
    fn row(r: usize, i: Interaction) -> gui::Element {
        text(format!("row {r}: "))
            .span(span("jumpy grapes").underline(true).underline_offset(3.0))
            .span(" and ")
            .span(span("struck out").line_through(true))
            .interaction(i)
            .height(24.0)
            .background(move || if i.hovered() { LIT } else { REST })
            .into_element()
    }
    hover(|| rows(row), [(40.0, 84.0), (40.0, 108.0)], (400, 400), 2);
}

/// Rows struck in 32px Geist Mono around a 16px Inter paragraph: the
/// line sits by the declaring box's font, read from a probe shaped in it,
/// and drawn again nothing allocates.
#[test]
fn drawing_a_line_through_another_font_hands_down_again_allocates_nothing() {
    fn row(r: usize, i: Interaction) -> gui::Element {
        div()
            .font_family(FontFamily::GEIST_MONO)
            .text_size(32.0)
            .line_through(true)
            .interaction(i)
            .height(40.0)
            .background(move || if i.hovered() { LIT } else { REST })
            .child(
                text(format!("struck row {r}"))
                    .font_family(FontFamily::INTER)
                    .text_size(16.0),
            )
            .into_element()
    }
    hover(|| rows(row), [(40.0, 140.0), (40.0, 180.0)], (400, 400), 2);
}

/// Rows of an underlined paragraph with an underlined span, whose colour
/// follows the hover: hovering resolves the paragraph again, providing its
/// span's style (its own underline alone, found held, never made anew),
/// and nothing allocates, from the frame's start (resolve and layout)
/// through its end.
#[test]
fn resolving_a_decorated_paragraph_with_an_underlined_span_again_allocates_nothing() {
    fn row(r: usize, i: Interaction) -> gui::Element {
        text(format!("row {r}: "))
            .span(span("underlined too").underline(true).underline_offset(2.0))
            .underline(true)
            .interaction(i)
            .height(24.0)
            .text_color(move || if i.hovered() { LIT } else { Color::hex(0x33_33_33) })
            .background(move || if i.hovered() { REST } else { LIT })
            .into_element()
    }
    let mut s = Scenario::measuring(|| rows(row), 400, 400, 1.0).unwrap();
    gui::reactive::stop_trace();
    s.check_cold(false);
    s.settle().unwrap();
    let at = [(40.0, 84.0), (40.0, 108.0)];
    for _ in 0..10 {
        for (x, y) in at {
            s.hover(x, y);
            s.settle().unwrap();
            s.frame("warm").unwrap();
        }
    }
    for (step, (x, y)) in at.into_iter().cycle().take(16).enumerate() {
        s.hover(x, y);
        let r = s.frame(&format!("hover {step}")).unwrap();
        assert!(r.visits.resolved >= 2, "both rows resolve again: {}", r.visits);
        let bytes = |t: Option<gui::heap::ThreadHeap>| t.map_or(u64::MAX, |t| t.allocated);
        let built = bytes(r.footprint.built);
        assert_eq!(
            built, 0,
            "{built} bytes allocated resolving and laying out hover {step}"
        );
        let (recorded, after) = from_record(&r);
        assert_eq!(recorded, 0, "{recorded} bytes allocated recording hover {step}");
        assert_eq!(after, 0, "{after} bytes allocated from the record on, hover {step}");
    }
}

/// Rows cut short, one by an ellipsis on one line and one by a two-line
/// clamp, each underlined: drawn again, allocating nothing.
#[test]
fn drawing_an_ellipsis_or_a_line_clamp_again_allocates_nothing() {
    fn row(r: usize, i: Interaction) -> gui::Element {
        let long = format!("cut {r} {}", "a quick jumpy fox goes by ".repeat(12));
        let t = text(long)
            .interaction(i)
            .width(160.0)
            .underline(true)
            .background(move || if i.hovered() { LIT } else { REST });
        if r.is_multiple_of(2) {
            t.truncate(true).height(24.0).into_element()
        } else {
            t.line_clamp(2).height(48.0).into_element()
        }
    }
    hover(|| rows(row), [(40.0, 120.0), (40.0, 156.0)], (400, 600), 2);
}

/// Rows clamped to two lines, all of one text in two widths and
/// alignments: each width finds its own soft wrap opportunity (its line
/// broken apart where its ellipsis leaves room), and neither breaks the
/// other's again, so drawing them by turns allocates nothing, and shows
/// what a first frame does.
#[test]
fn drawing_one_clamped_text_at_two_widths_again_allocates_nothing() {
    fn row(r: usize, i: Interaction) -> gui::Element {
        let (width, align) = if r.is_multiple_of(2) {
            (150.0, TextAlign::Left)
        } else {
            (190.0, TextAlign::Center)
        };
        text("the same words in every row ".repeat(12))
            .interaction(i)
            .width(width)
            .text_align(align)
            .line_clamp(2)
            .height(48.0)
            .background(move || if i.hovered() { LIT } else { REST })
            .into_element()
    }
    let at = [(40.0, 120.0), (40.0, 168.0)];
    let warm = hover(|| rows(row), at, (400, 600), 2).shot(false).unwrap();
    let mut fresh = Scenario::measuring(|| rows(row), 400, 600, 1.0).unwrap();
    fresh.hover(at[1].0, at[1].1);
    fresh.settle().unwrap();
    fresh.frame("fresh").unwrap();
    let fresh = fresh.shot(false).unwrap();
    assert!(fresh.data().iter().any(|&b| b != 255), "the frame shows text");
    assert!(warm == fresh, "warm frames show what a fresh one does");
}

/// Two blocks of 200 Arabic labels each, clamped to one line (each finds
/// its soft wrap opportunity on a line broken apart, and keeps a prefix
/// shaped alone: two pieces apart), each label of a block re-recorded as
/// its row is hovered, by turns: 800 pieces apart in use, more than the
/// cache's floor, and drawing them again allocates nothing.
#[test]
fn drawing_more_clamped_labels_than_the_cache_floor_again_allocates_nothing() {
    fn block(lit: Interaction, from: usize) -> gui::Element {
        div()
            .display(Display::Flex)
            .flex_wrap(true)
            .width(400.0)
            .children((from..from + 200).map(move |n| {
                text(format!("{n} {}", "\u{633}\u{633}\u{633} ".repeat(6)))
                    .direction(Direction::Rtl)
                    .line_clamp(1)
                    .size(8.0)
                    .line_height(10.0)
                    .width(40.0)
                    .height(10.0)
                    .background(move || if lit.hovered() { LIT } else { REST })
                    .into_element()
            }))
            .into_element()
    }
    fn page() -> gui::Element {
        let i = Interaction::new();
        let j = Interaction::new();
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .child(text("the first").interaction(i).height(24.0))
            .child(text("the second").interaction(j).height(24.0))
            .child(block(i, 0))
            .child(block(j, 200))
            .into_element()
    }
    hover(page, [(40.0, 12.0), (40.0, 36.0)], (400, 800), 400);
}

/// Two blocks of a thousand labels each, every label of a block
/// re-recorded as its row is hovered, by turns: a thousand chunks recorded
/// again each frame, more than paint's floor of displays kept, and the
/// displays the last repaint freed take the next, allocating nothing.
#[test]
fn recording_a_thousand_chunks_again_allocates_nothing() {
    fn block(lit: Interaction, from: usize) -> gui::Element {
        div()
            .display(Display::Flex)
            .flex_wrap(true)
            .width(400.0)
            .children((from..from + 1000).map(move |n| {
                text(format!("{n}"))
                    .size(6.0)
                    .line_height(8.0)
                    .width(16.0)
                    .height(8.0)
                    .background(move || if lit.hovered() { LIT } else { REST })
                    .into_element()
            }))
            .into_element()
    }
    fn page() -> gui::Element {
        let i = Interaction::new();
        let j = Interaction::new();
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .child(text("the first").interaction(i).height(24.0))
            .child(text("the second").interaction(j).height(24.0))
            .child(block(i, 0))
            .child(block(j, 1000))
            .into_element()
    }
    hover(page, [(40.0, 12.0), (40.0, 36.0)], (400, 800), 2_000);
}

/// A textarea of two thousand lines (pieces) that re-records with each
/// hover, by turns with one-line rows: the chunks' piece counts differ a
/// thousandfold, and drawing them again allocates nothing.
#[test]
fn chunks_of_very_different_piece_counts_allocate_nothing() {
    fn page() -> gui::Element {
        let i = Interaction::new();
        let j = Interaction::new();
        let lines: String = (0..2_000).map(|n| format!("line {n}\n")).collect();
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .child(
                text("the row")
                    .interaction(i)
                    .height(24.0)
                    .background(move || if i.hovered() { LIT } else { REST }),
            )
            .child(
                text("the rest")
                    .interaction(j)
                    .height(24.0)
                    .background(move || if j.hovered() { LIT } else { REST }),
            )
            .child(text(lines).background(move || if i.hovered() { REST } else { LIT }))
            .into_element()
    }
    let at = [(40.0, 12.0), (40.0, 36.0)];
    let mut s = hover(page, at, (300, 400), 2);
    // The textarea's chunk itself records again (below the rows, at 48):
    // its thousands of pieces are what is drawn again.
    for (step, (x, y)) in at.into_iter().cycle().take(4).enumerate() {
        s.hover(x, y);
        let r = s.frame(&format!("again {step}")).unwrap();
        assert!(
            r.painted.iter().any(|p| p.recorded && p.bounds.y >= 47.0),
            "hover {step}: the textarea records again: {:?}",
            r.painted
        );
    }
}

/// A long field's runs for a screen reader allocate by the run (each its
/// few lists, 256 entries at most), never by the character: told whole,
/// cut short by an ellipsis, or with its lines past a clamp hidden.
#[test]
fn a_long_fields_runs_for_a_screen_reader_allocate_by_the_run_not_the_character() {
    use gui::style::ResolvedText;
    let line = "ab cd ef gh ".repeat(400);
    // A short line, then the long one, hidden by the clamp.
    let lines = format!("ab\n{line}");
    let root = ResolvedText::root(LIT);
    let truncated = ResolvedText {
        truncate: true,
        ..root.clone()
    };
    let clamped = ResolvedText {
        line_clamp: Some(1),
        ..root.clone()
    };
    for (what, content, t) in [
        ("whole", &line, &root),
        ("truncated", &line, &truncated),
        ("clamped", &lines, &clamped),
    ] {
        let mut ts = gui::text::TextSystem::new();
        // Shaped once; then only the runs are made.
        drop(ts.text_runs(content, t, Some(3000.0)));
        let (runs, heap) = gui::heap::measure(|| ts.text_runs(content, t, Some(3000.0)));
        let allocations = heap.unwrap().allocations;
        let entries: usize = runs.iter().map(|r| r.lengths.len()).sum();
        assert!(entries >= content.len() - 2, "{what}: every character told ({entries})");
        // Each run's four lists of up to 256, grown a few times: far
        // fewer than one a character.
        let bound = u64::try_from(content.len()).unwrap() / 4;
        assert!(
            allocations <= bound,
            "{what}: {allocations} allocations for {} characters in {} runs (at most {bound})",
            content.len(),
            runs.len()
        );
    }
}
