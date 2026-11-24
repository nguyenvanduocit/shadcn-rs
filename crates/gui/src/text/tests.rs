use super::*;

crate::engine::engine_local! {
    /// Inline boxes looked up by id on this thread ([`box_range`]).
    [Fork] pub(super) static BOX_LOOKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// A truncated line's drawn units looked at as a screen reader's runs
    /// read it ([`TextSystem::text_runs`]).
    [Fork] pub(super) static UNIT_LOOKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Word boundaries looked at as a clamp tells which of its prefixes
    /// end a word ([`TextSystem::kept_fit`]).
    [Fork] pub(super) static BOUND_LOOKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn style(size: f32, line_height: f32, weight: u16) -> ResolvedText {
    ResolvedText {
        size,
        line_height,
        weight,
        ..ResolvedText::root(Color::BLACK)
    }
}

#[test]
fn wrapping_to_a_narrow_width_adds_lines() {
    let mut ts = TextSystem::new();
    let t = ResolvedText::root(Color::BLACK);
    let (w, h) = ts.measure("hello wide world", &t, None);
    assert!(w > 50.0 && (h - 24.0).abs() < 0.1, "one line: {w}x{h}");
    let (_, h2) = ts.measure("hello wide world", &t, Some(w / 2.0));
    assert!(h2 >= 48.0, "wraps to two or more lines: {h2}");
}

/// Inter's `opsz` follows the size and `wght` the weight, set at run time on
/// the one variable face: shadcn's `h3` ("The Joke Tax", `text-2xl
/// font-semibold tracking-tight`) is as wide as Chrome draws it.
#[test]
fn the_joke_tax_at_24px_semibold_is_as_wide_as_in_chrome() {
    let mut ts = TextSystem::new();
    let t = ResolvedText {
        tracking: -0.025,
        ..style(24.0, 32.0, 600)
    };
    // The line's advance, before `measure` rounds it up to a layout unit
    // (1/64 px) as Blink does.
    let p = ts.pieces(Paragraph::from("The Joke Tax"), &t);
    ts.break_piece(p[0].hash, Breaking::of(&t, None));
    let advance = ts.shaped[&p[0].hash].layout.width();
    assert!((advance - 141.06).abs() < 0.01, "{advance}");
    let (w, _) = ts.measure("The Joke Tax", &t, None);
    assert!((w - advance).abs() <= 1.0 / 64.0, "{w}");
    let big = ResolvedText {
        tracking: -0.025,
        ..style(24.0, 32.0, 400)
    };
    assert!(
        ts.measure("The Joke Tax", &big, None).0 < w,
        "a lighter weight is narrower"
    );
}

#[test]
fn a_new_width_breaks_lines_again_but_shapes_nothing() {
    let mut ts = TextSystem::new();
    let t = ResolvedText::root(Color::BLACK);
    let text = "Parley shapes a paragraph once and breaks it at any width after.";
    ts.measure(text, &t, Some(300.0));
    assert_eq!(ts.take_shaped(), 1);
    for w in [100.0, 180.0, 640.0, 72.5] {
        let (_, h) = ts.measure(text, &t, Some(w));
        assert!(h >= 24.0);
        ts.min_content(text, &t);
        ts.caret(text, &t, Some(w), 10);
    }
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale: 2.0,
        size: (800, 600),
        cull: None,
        snap: true,
    };
    ts.draw(&mut target, text, &t, Some(222.0), (0.0, 0.0), 1.0);
    ts.end_frame();
    assert_eq!(ts.take_shaped(), 0, "a resize reshapes nothing");
}

#[test]
fn a_keystroke_in_a_long_textarea_reshapes_one_paragraph() {
    let mut ts = TextSystem::new();
    let t = ResolvedText::root(Color::BLACK);
    let mut text: String = (0..10_000).map(|i| format!("line {i} of a long text area\n")).collect();
    ts.measure(text.as_str(), &t, Some(400.0));
    assert_eq!(ts.take_shaped(), 10_001, "every paragraph, and the empty last line");
    ts.end_frame();
    let at = text.find("line 5000 ").unwrap_or(0) + 4;
    text.insert(at, 'x');
    let (_, h) = ts.measure(text.as_str(), &t, Some(400.0));
    assert!((h - 10_001.0 * 24.0).abs() < 1.0, "{h}");
    ts.caret(text.as_str(), &t, Some(400.0), at + 1);
    assert_eq!(ts.take_shaped(), 1, "one keystroke reshapes one paragraph");
}

#[test]
fn the_caret_moves_by_grapheme_and_by_word() {
    let mut ts = TextSystem::new();
    let t = ResolvedText::root(Color::BLACK);
    // An `e` with a combining acute accent is one grapheme of three bytes.
    let s = "e\u{301}x";
    assert_eq!(ts.step(s, &t, None, 0, Step::Right), 3);
    assert_eq!(ts.step(s, &t, None, 3, Step::Left), 0);
    // A family emoji joined by ZWJs is one grapheme.
    let family = "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}b";
    let after = family.len() - 1;
    assert_eq!(ts.step(family, &t, None, 1, Step::Right), after);
    assert_eq!(ts.step(family, &t, None, after, Step::Left), 1);
    // Backspace takes one character of a combined grapheme, all of an emoji.
    assert_eq!(ts.step(s, &t, None, 3, Step::DeleteBack), 1);
    assert_eq!(ts.step(family, &t, None, after, Step::DeleteBack), 1);
    // Word by word.
    let words = "hello wide world";
    assert_eq!(ts.step(words, &t, None, 0, Step::WordRight), 5);
    assert_eq!(ts.step(words, &t, None, 5, Step::WordRight), 10);
    assert_eq!(ts.step(words, &t, None, 16, Step::WordLeft), 11);
    assert_eq!(ts.step(words, &t, None, 11, Step::DeleteWordBack), 6);
    // Across a hard line break, and up and down a line.
    let lines = "ab\ncd";
    assert_eq!(ts.step(lines, &t, None, 2, Step::Right), 3);
    assert_eq!(ts.step(lines, &t, None, 3, Step::Left), 2);
    assert_eq!(ts.step(lines, &t, None, 4, Step::Up), 1);
    assert_eq!(ts.step(lines, &t, None, 1, Step::Down), 4);
    assert_eq!(ts.step(lines, &t, None, 4, Step::LineStart), 3);
}

#[test]
fn hit_caret_and_selection_agree_across_paragraphs() {
    let mut ts = TextSystem::new();
    let t = ResolvedText::root(Color::BLACK);
    let s = "first\nsecond";
    let c = ts.caret(s, &t, None, 8);
    assert!((c.y - 24.0).abs() < 0.01, "{c:?}");
    assert_eq!(ts.hit(s, &t, None, c.x + 0.1, c.y + 5.0), 8);
    let boxes = ts.selection(s, &t, None, 2, 9);
    assert_eq!(boxes.len(), 2, "{boxes:?}");
    assert!(boxes.iter().any(|b| b.y >= 24.0));
}

#[test]
fn min_content_is_the_widest_word() {
    let mut ts = TextSystem::new();
    let t = ResolvedText::root(Color::BLACK);
    let word = ts.measure("extraordinary", &t, None).0;
    let min = ts.min_content("an extraordinary day", &t);
    assert!((min - word).abs() < 0.02, "{min} vs {word}");
}

#[test]
fn a_clamped_or_truncated_paragraph_ends_in_an_ellipsis() {
    let mut ts = TextSystem::new();
    let t = ResolvedText {
        truncate: true,
        ..ResolvedText::root(Color::BLACK)
    };
    let text = "A long line of text that does not fit in its box at all";
    let (w, h) = ts.measure(text, &t, Some(120.0));
    assert!(w <= 120.0 && w > 100.0 && (h - 24.0).abs() < 0.01, "{w}x{h}");
    let clamped = ResolvedText {
        line_clamp: Some(2),
        ..ResolvedText::root(Color::BLACK)
    };
    let (_, h) = ts.measure(text, &clamped, Some(120.0));
    assert!((h - 48.0).abs() < 0.01, "{h}");
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale: 1.0,
        size: (200, 100),
        cull: None,
        snap: true,
    };
    ts.draw(&mut target, text, &clamped, Some(120.0), (0.0, 0.0), 1.0);
    let ys: std::collections::BTreeSet<i32> = items
        .iter()
        .filter_map(|i| match i {
            Item::Glyph { y, .. } => Some(*y),
            _ => None,
        })
        .collect();
    assert_eq!(ys.len(), 2, "two baselines: {ys:?}");
    // The second line ends in the ellipsis: its last glyph, on its baseline.
    let dots = ellipsis_glyph(&clamped);
    let glyphs = glyphs_of(&items);
    let last = glyphs.last().copied().expect("glyphs");
    assert_eq!(last.0.glyph, dots, "{glyphs:?}");
    assert_eq!(Some(last.2), ys.last().copied());
    assert_eq!(glyphs.iter().filter(|g| g.0.glyph == dots).count(), 1, "{glyphs:?}");
    // Truncated, likewise on its one line.
    let truncated = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &t, Some(120.0)));
    assert_eq!(truncated.last().map(|g| g.0.glyph), Some(dots), "{truncated:?}");
}

#[test]
fn balance_evens_the_lines_and_pretty_leaves_no_orphan() {
    let mut ts = TextSystem::new();
    let text = "Balanced lines are about as wide as each other";
    let auto = ResolvedText::root(Color::BLACK);
    let balanced = ResolvedText {
        wrap_style: WrapStyle::Balance,
        ..auto.clone()
    };
    let (wa, ha) = ts.measure(text, &auto, Some(300.0));
    let (wb, hb) = ts.measure(text, &balanced, Some(300.0));
    assert!((ha - hb).abs() < 0.01 && wb < wa, "{wa} {wb}");
    let pretty = ResolvedText {
        wrap_style: WrapStyle::Pretty,
        ..auto.clone()
    };
    let orphan = "Words wrap so that the last line holds one word alone";
    let w = ts.measure(orphan, &auto, None).0 - 20.0;
    let p = ts.pieces(Paragraph::from(orphan), &pretty);
    ts.break_piece(p[0].hash, Breaking::of(&pretty, Some(w)));
    let s = &ts.shaped[&p[0].hash];
    assert!(!orphan_line(s), "pretty leaves no word alone");
}

#[test]
fn pretty_rebreaks_the_last_four_lines_of_a_long_paragraph() {
    // Chromium's `ScoreLineBreaker` slides a four-line window to a
    // paragraph's end: a paragraph of six or more lines loses its orphan
    // too, its lines before the last four wrapping as they would.
    let mut ts = TextSystem::new();
    let auto = ResolvedText {
        align: TextAlign::Center,
        ..ResolvedText::root(Color::BLACK)
    };
    let pretty = ResolvedText {
        wrap_style: WrapStyle::Pretty,
        ..auto.clone()
    };
    let text = "Pretty text wraps its paragraph as usual until the last few lines, then sends a word or two \
                down so that the last line of a long paragraph never holds one short word alone at its end";
    let ranges = |s: &Shaped| -> Vec<std::ops::Range<usize>> { s.layout.lines().map(|l| l.text_range()).collect() };
    let mut seen = None;
    for w in (150_u16..400).step_by(3) {
        let w = f32::from(w);
        let p = ts.pieces(Paragraph::from(text), &auto);
        ts.break_piece(p[0].hash, Breaking::of(&auto, Some(w)));
        let s = &ts.shaped[&p[0].hash];
        if s.layout.len() >= 6 && orphan_line(s) && last_short(&s.layout, w) {
            seen = Some((w, ranges(s)));
            break;
        }
    }
    let (w, greedy) = seen.expect("a width at which the paragraph ends in an orphan");
    let p = ts.pieces(Paragraph::from(text), &pretty);
    ts.break_piece(p[0].hash, Breaking::of(&pretty, Some(w)));
    let s = &ts.shaped[&p[0].hash];
    assert!(!orphan_line(s), "pretty leaves no word alone at {w}px");
    let lines = ranges(s);
    assert_eq!(lines.len(), greedy.len(), "as many lines");
    let kept = greedy.len() - PRETTY_MAX_LINES;
    assert_eq!(lines[..kept], greedy[..kept], "the lines before the last four");
    assert_ne!(lines[kept..], greedy[kept..], "the last four re-broken");
    // Every line, narrowed or not, is centred in the whole box.
    for line in s.layout.lines() {
        let m = line.metrics();
        let ink = m.advance - m.hanging_advance;
        let mid = m.offset + m.inline_min_coord + ink / 2.0 + s.shift;
        assert!((mid - w / 2.0).abs() < 0.01, "centred: {mid} in {w}");
    }
}

#[test]
fn pretty_leaves_a_last_word_wider_than_a_third_of_the_box() {
    // Blink re-breaks only for a short last line: one word as wide as a
    // third of the box or more stays where greedy breaking puts it.
    let mut ts = TextSystem::new();
    let auto = ResolvedText::root(Color::BLACK);
    let pretty = ResolvedText {
        wrap_style: WrapStyle::Pretty,
        ..auto.clone()
    };
    let text = "These are a few words that come first incomprehensibilities";
    let last = ts.measure("incomprehensibilities", &auto, None).0;
    let w = ts.measure("These are a few words that come first", &auto, None).0 + 4.0;
    assert!(last * 3.0 >= w && last < w, "{last} in {w}");
    let p = ts.pieces(Paragraph::from(text), &pretty);
    ts.break_piece(p[0].hash, Breaking::of(&pretty, Some(w)));
    let s = &ts.shaped[&p[0].hash];
    assert_eq!(s.layout.len(), 2);
    assert!(orphan_line(s), "its last word left alone");
}

/// Whether the last line of `s` holds a single word.
fn orphan_line(s: &Shaped) -> bool {
    s.layout.lines().next_back().is_some_and(|line| {
        s.content
            .get(line.text_range())
            .map_or(0, |t| t.split_whitespace().count())
            < 2
    })
}

/// Whether the last line of `layout` is shorter than a third of `width`.
/// The width of the text a line holds: its advance without the space that
/// hangs at its end.
fn content(m: &LineMetrics) -> f32 {
    m.advance - m.hanging_advance
}

fn last_short(layout: &Layout<u32>, width: f32) -> bool {
    layout
        .lines()
        .next_back()
        .is_some_and(|line| content(line.metrics()) * 3.0 < width)
}

/// How many words each line of `s` holds.
fn words_per_line(s: &Shaped) -> Vec<usize> {
    s.layout
        .lines()
        .map(|l| {
            s.content
                .get(l.text_range())
                .map_or(0, |t| t.split_whitespace().count())
        })
        .collect()
}

/// Geist Mono at 16px, wrapping as `wrap_style` says, and the advance of
/// one of its characters.
fn mono(ts: &mut TextSystem, wrap_style: WrapStyle) -> (ResolvedText, f32) {
    let t = ResolvedText {
        family: FontFamily::GEIST_MONO,
        wrap_style,
        ..style(16.0, 24.0, 400)
    };
    let advance = ts.measure("abcdefghij", &t, None).0 / 10.0;
    (t, advance)
}

/// `s` broken as `t` asks at `width`.
fn broken_piece<'a>(ts: &'a mut TextSystem, text: &str, t: &ResolvedText, width: f32) -> &'a Shaped {
    let p = ts.pieces(Paragraph::from(text), t);
    ts.break_piece(p[0].hash, Breaking::of(t, Some(width)));
    &ts.shaped[&p[0].hash]
}

#[test]
fn pretty_scores_break_points_where_no_one_width_fits() {
    // Nine six-character words and a two-character one in a box 20.5
    // characters wide: greedy breaks them 3/3/3/1. No single width for the
    // last four lines sends a word down without adding a line, but Blink's
    // score picks 3/3/2/2, each line inside the box.
    let mut ts = TextSystem::new();
    let text = format!("{} ab", ["abcdef"; 9].join(" "));
    let (auto, a) = mono(&mut ts, WrapStyle::Auto);
    let w = 20.5 * a;
    assert_eq!(words_per_line(broken_piece(&mut ts, &text, &auto, w)), [3, 3, 3, 1]);
    let (pretty, _) = mono(&mut ts, WrapStyle::Pretty);
    let s = broken_piece(&mut ts, &text, &pretty, w);
    assert_eq!(words_per_line(s), [3, 3, 2, 2]);
    for line in s.layout.lines() {
        assert!(content(line.metrics()) <= w, "inside the box");
    }
}

#[test]
fn pretty_breaks_a_long_paragraph_once_and_its_window_again() {
    // Ten thousand lines ending in an orphan: greedy's one pass, and work
    // bounded by the four-line window to re-break its end; not a pass over
    // the whole paragraph for each step of a search.
    const LINES: usize = 10_000;
    let mut ts = TextSystem::new();
    let text = format!("{} ab", vec!["abcdef"; 3 * (LINES - 1)].join(" "));
    let (pretty, a) = mono(&mut ts, WrapStyle::Pretty);
    let w = 20.5 * a;
    let p = ts.pieces(Paragraph::from(text.as_str()), &pretty);
    let before = ts.lines_broken;
    ts.break_piece(p[0].hash, Breaking::of(&pretty, Some(w)));
    let broken = ts.lines_broken - before;
    let s = &ts.shaped[&p[0].hash];
    assert_eq!(s.layout.len(), LINES, "as many lines");
    assert!(!orphan_line(s), "no orphan");
    let words = words_per_line(s);
    assert!(
        words[..LINES - 2].iter().all(|n| *n == 3),
        "greedy's lines before the window"
    );
    assert_eq!(words[LINES - 2..], [2, 2]);
    // The paragraph's lines once, then the window probed (a line per
    // grapheme at most: 4 lines of at most 21 characters) and its four
    // lines broken again.
    let most = u64::try_from(LINES).unwrap_or(u64::MAX) + 4 * 21 + 4;
    assert!(broken <= most, "{broken} lines broken for {LINES}");
}

#[test]
fn pretty_leaves_a_clamped_paragraph_whose_last_line_is_hidden() {
    // Pretty only when every line is shown: a clamp that hides the last
    // line leaves the lines as they wrap; one that shows them all does not.
    let mut ts = TextSystem::new();
    let text = format!("{} ab", ["abcdef"; 9].join(" "));
    let (pretty, a) = mono(&mut ts, WrapStyle::Pretty);
    let w = 20.5 * a;
    for (clamp, want) in [(3, vec![3, 3, 3]), (4, vec![3, 3, 2, 2]), (9, vec![3, 3, 2, 2])] {
        let t = ResolvedText {
            line_clamp: Some(clamp),
            ..pretty.clone()
        };
        let p = ts.pieces(Paragraph::from(text.as_str()), &t);
        let mut how = Breaking::of(&t, Some(w));
        how.lines = Some(u32::from(clamp));
        ts.break_piece(p[0].hash, how);
        assert_eq!(words_per_line(&ts.shaped[&p[0].hash]), want, "clamped to {clamp}");
    }
}

#[test]
fn balanced_lines_are_aligned_in_the_whole_box() {
    // Balance narrows every line; each is still centred in the box, as
    // Blink places them.
    let mut ts = TextSystem::new();
    let t = ResolvedText {
        wrap_style: WrapStyle::Balance,
        align: TextAlign::Center,
        ..ResolvedText::root(Color::BLACK)
    };
    let w = 300.0;
    let s = broken_piece(&mut ts, "Balanced lines are about as wide as each other", &t, w);
    assert_eq!(s.layout.len(), 2);
    for line in s.layout.lines() {
        let m = line.metrics();
        let mid = m.offset + m.inline_min_coord + content(m) / 2.0;
        assert!((mid - w / 2.0).abs() < 0.01, "centred: {mid} in {w}");
    }
}

#[test]
fn pretty_narrowed_lines_are_centred_and_end_aligned_in_the_whole_box() {
    // The 3/3/2/2 window, centred and end aligned, left to right and right
    // to left: each line's text sits where its alignment puts it in the
    // box, not in the narrowed width.
    // Right to left in Hebrew: Parley hangs a line's trailing space only at
    // the paragraph's bidi level (no UAX #9 L1), so Latin words in a
    // right-to-left paragraph keep greedy's lines.
    let mut ts = TextSystem::new();
    let latin = format!("{} ab", ["abcdef"; 9].join(" "));
    let hebrew = "שלום עולם זה טקסט ארוך מאוד שלא נכנס בשורה אחת ולכן הוא נשבר לכמה שורות עד המילה האחרונה שלו";
    let (pretty, a) = mono(&mut ts, WrapStyle::Pretty);
    let rtl_pretty = ResolvedText {
        direction: Direction::Rtl,
        wrap_style: WrapStyle::Pretty,
        ..style(16.0, 24.0, 400)
    };
    let auto = ResolvedText {
        wrap_style: WrapStyle::Auto,
        ..rtl_pretty.clone()
    };
    let hebrew_w = (120_u16..400)
        .step_by(3)
        .map(f32::from)
        .find(|w| {
            let s = broken_piece(&mut ts, hebrew, &auto, *w);
            s.layout.len() >= 2 && orphan_line(s) && last_short(&s.layout, *w)
        })
        .expect("a width at which the Hebrew paragraph ends in an orphan");
    for (base, text, w) in [(&pretty, latin.as_str(), 20.5 * a), (&rtl_pretty, hebrew, hebrew_w)] {
        let direction = base.direction;
        for align in [TextAlign::Center, TextAlign::End] {
            let t = ResolvedText { align, ..base.clone() };
            let s = broken_piece(&mut ts, text, &t, w);
            assert!(!orphan_line(s), "{direction:?} {align:?}: re-broken");
            for line in s.layout.lines() {
                let m = line.metrics();
                // Right to left, the space at a line's end hangs at its left.
                let rtl = direction == Direction::Rtl;
                let left = m.offset + m.inline_min_coord + if rtl { m.hanging_advance } else { 0.0 };
                let right = left + content(m);
                let (got, want) = match (align, rtl) {
                    (TextAlign::Center, _) => ((left + right) / 2.0, w / 2.0),
                    (_, false) => (right, w),
                    (_, true) => (left, 0.0),
                };
                assert!((got - want).abs() < 0.01, "{direction:?} {align:?}: {got} for {want}");
            }
        }
    }
}

#[test]
fn pretty_scores_each_paragraph_a_forced_break_ends() {
    // A forced break inside a piece (U+2028) ends a paragraph as Blink
    // scores it: the first loses its orphan though the piece's last lines
    // have none.
    let mut ts = TextSystem::new();
    let nine = ["abcdef"; 9].join(" ");
    let text = format!("{nine} ab\u{2028}{nine}");
    let (pretty, a) = mono(&mut ts, WrapStyle::Pretty);
    let s = broken_piece(&mut ts, &text, &pretty, 20.5 * a);
    assert_eq!(words_per_line(s), [3, 3, 2, 2, 3, 3, 3]);
}

#[test]
fn pretty_hangs_a_trailing_space_in_every_width_it_compares() {
    // The 3/3/3/1 paragraph with a space at its end: the space hangs, as
    // Blink's widths leave it out, and the paragraph re-breaks as without.
    let mut ts = TextSystem::new();
    let text = format!("{} ab ", ["abcdef"; 9].join(" "));
    let (pretty, a) = mono(&mut ts, WrapStyle::Pretty);
    let s = broken_piece(&mut ts, &text, &pretty, 20.5 * a);
    assert_eq!(words_per_line(s), [3, 3, 2, 2]);
}

#[test]
fn pretty_scores_a_window_of_thousands_of_opportunities_in_bounded_work() {
    // 12,001 CJK characters, 4,000 a line, the last alone: every character
    // is a break opportunity. The window holds more segments than are
    // scored, so its lines stay greedy's, and no start is scored.
    let mut ts = TextSystem::new();
    let t = ResolvedText {
        wrap_style: WrapStyle::Pretty,
        ..style(16.0, 24.0, 400)
    };
    let one = ts.measure("一", &t, None).0;
    let text = "一".repeat(12_001);
    let w = 4_000.0 * one + one / 2.0;
    let (lines, visits) = (ts.lines_broken, ts.scores_visited);
    let s = broken_piece(&mut ts, &text, &t, w);
    let per_line: Vec<usize> = s.layout.lines().map(|l| l.text_range().len() / "一".len()).collect();
    assert_eq!(per_line, [4_000, 4_000, 4_000, 1], "greedy's");
    assert_eq!(ts.scores_visited - visits, 0, "nothing scored");
    // Past greedy's four: its last line probed, the window probed to one
    // segment past the cap, and its four lines broken again.
    let extra = ts.lines_broken - lines - 4;
    let most = u64::try_from(1 + PRETTY_MAX_SEGMENTS + 1 + 4).unwrap_or(u64::MAX);
    assert!(extra <= most, "{extra} lines past greedy's");
}

#[test]
fn best_breaks_scores_each_end_against_one_line_of_starts() {
    // As many segments as a window scores, ten a line: each end is scored
    // against the starts within a line of it, not every start before it.
    let segments = vec![(10.0_f32, 1.0_f32); PRETTY_MAX_SEGMENTS];
    let mut visits = 0;
    let widths = best_breaks(&segments, 99.5, 16.0, &mut visits).expect("scored");
    assert!(widths.iter().all(|w| *w <= 99.5 + 2.0 * FIT_SLACK), "{widths:?}");
    let bound = u64::try_from(PRETTY_MAX_SEGMENTS * 11).unwrap_or(u64::MAX);
    assert!(visits <= bound, "{visits} starts scored");
}

#[test]
fn a_narrowed_rtl_line_is_painted_hit_and_selected_where_it_is() {
    // Right to left, pretty: the re-broken lines start at the box's right
    // edge; their glyphs, carets, hits and selection boxes agree.
    let mut ts = TextSystem::new();
    let auto = ResolvedText {
        direction: Direction::Rtl,
        ..style(16.0, 24.0, 400)
    };
    let pretty = ResolvedText {
        wrap_style: WrapStyle::Pretty,
        ..auto.clone()
    };
    let text = "שלום עולם זה טקסט ארוך מאוד שלא נכנס בשורה אחת ולכן הוא נשבר לכמה שורות עד המילה האחרונה שלו";
    let mut seen = None;
    for w in (120_u16..400).step_by(3) {
        let w = f32::from(w);
        let s = broken_piece(&mut ts, text, &auto, w);
        if s.layout.len() >= 2 && orphan_line(s) && last_short(&s.layout, w) {
            seen = Some(w);
            break;
        }
    }
    let w = seen.expect("a width at which the paragraph ends in an orphan");
    let s = broken_piece(&mut ts, text, &pretty, w);
    assert!(!orphan_line(s), "pretty leaves no word alone at {w}px");
    let ranges: Vec<_> = s.layout.lines().map(|l| l.text_range()).collect();
    let rows: Vec<_> = s.rows.iter().map(|r| (r.top, r.height)).collect();
    let glyphs = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &pretty, Some(w)));
    for (range, (top, height)) in ranges.iter().zip(rows) {
        let boxes = ts.selection(text, &pretty, Some(w), range.start, range.end);
        let (l, r) = boxes
            .iter()
            .filter(|b| b.y < top + height / 2.0 && b.y + b.height > top + height / 2.0)
            .fold((f32::MAX, f32::MIN), |(l, r), b| (l.min(b.x), r.max(b.x + b.width)));
        assert!(
            (r - w).abs() < 1.0,
            "the line starts at the box's right edge: {r} in {w}"
        );
        // Its glyphs lie within its selection.
        let on_line = glyphs.iter().filter(|g| {
            let y = f32::from(i16::try_from(g.2).unwrap_or(i16::MAX));
            y > top && y <= top + height
        });
        for g in on_line {
            let x = f32::from(i16::try_from(g.1).unwrap_or(i16::MAX));
            assert!(x >= l - 1.0 && x <= r + 1.0, "glyph at {x} in {l}..{r}");
        }
        // Its start's caret at its right, and a hit there finds its start.
        let caret = ts.caret(text, &pretty, Some(w), range.start);
        assert!((caret.x - r).abs() < 1.0, "caret {} at {r}", caret.x);
        let hit = ts.hit(text, &pretty, Some(w), r - 0.5, top + height / 2.0);
        assert_eq!(hit, range.start, "a hit at its right edge");
    }
}

#[test]
fn baselines_round_as_blink_places_them() {
    // Geist Mono at 14px on a 22.75px line: its 18px content box 2px below
    // each line's top (ADR-0005).
    let mut ts = TextSystem::new();
    let t = ResolvedText {
        family: FontFamily::GEIST_MONO,
        ..style(14.0, 22.75, 400)
    };
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale: 1.0,
        size: (400, 100),
        cull: None,
        snap: true,
    };
    ts.draw(&mut target, "ab\ncd", &t, None, (0.0, 0.0), 1.0);
    let ys: Vec<i32> = items
        .iter()
        .filter_map(|i| match i {
            Item::Glyph { y, .. } => Some(*y),
            _ => None,
        })
        .collect();
    // Ascent 14.4 rounds to 14, descent 3.6 to 4: 2px (floored 2.375) + 14.
    assert_eq!(ys.first().copied(), Some(16), "{ys:?}");
    assert_eq!(ys.last().copied(), Some(39), "22.75 + 16 rounds to 39: {ys:?}");
}

#[test]
fn a_runs_padding_takes_room_in_the_line() {
    let mut ts = TextSystem::new();
    let t = ResolvedText::root(Color::BLACK);
    let content = "say code now";
    let plain = ts.measure(content, &t, None).0;
    let runs = [Run {
        range: 4..8,
        text: t.clone(),
        background: Some(Color::BLACK),
        radius: 2.0,
        padding: Edges {
            left: 3.0,
            right: 5.0,
            top: 1.0,
            bottom: 1.0,
        },
    }];
    let padded = ts.measure(Paragraph::new(content, &runs), &t, None).0;
    assert!((padded - plain - 8.0).abs() < 0.02, "{plain} {padded}");
}

#[test]
fn inline_boxes_sit_in_the_line_and_wrap_with_it() {
    let mut ts = TextSystem::new();
    let t = ResolvedText::root(Color::BLACK);
    let content = "hello \u{FFFC}world";
    let boxes = [InlineSlot {
        at: 6,
        width: 20.0,
        height: 16.0,
        baseline: None,
        id: 7,
    }];
    let p = Paragraph {
        content,
        runs: &[],
        boxes: &boxes,
    };
    let placed = ts.placed_inline_boxes(p, &t, None);
    assert_eq!(placed.len(), 1);
    let PlacedBox { id, bounds: b, .. } = placed[0];
    let hello = ts.measure("hello ", &t, None).0;
    assert_eq!(id, 7);
    assert!(
        (b.x - hello).abs() < 1.0 && (b.width - 20.0).abs() < 0.01,
        "{b:?} after {hello}"
    );
}

#[test]
fn a_line_an_inline_box_grows_keeps_blinks_strut() {
    // A 14px/22.75 paragraph (`text-sm leading-relaxed`) holding a 24px
    // `size="xs"` button whose label's baseline is 16px down: Chrome's line
    // is 24 tall, 16 above the baseline (the button's, equal to the strut's
    // rounded ascent and floored half-leading) and 6.75 below (the strut's),
    // not Parley's unrounded 24.43.
    let mut ts = TextSystem::new();
    let t = style(14.0, 22.75, 400);
    let content = "browser. \u{FFFC} Server";
    let button = |height: f32, baseline: Option<f32>| {
        [InlineSlot {
            at: 9,
            width: 60.0,
            height,
            baseline,
            id: 1,
        }]
    };
    let plain = ts.metrics("browser. Server", &t, None);
    assert_eq!(plain.height, 22.75);
    assert_eq!(plain.first_baseline, Some(16.0), "Blink's strut: 16 above, 6.75 below");
    let boxes = button(24.0, Some(16.0));
    let p = Paragraph {
        content,
        runs: &[],
        boxes: &boxes,
    };
    let m = ts.metrics(p, &t, None);
    assert_eq!(m.height, 24.0, "16 above the baseline and 8 below, the button's");
    assert_eq!(m.first_baseline, Some(16.0));
    let placed = ts.placed_inline_boxes(p, &t, None);
    assert_eq!(placed[0].bounds.y, 0.0, "the button's top on the line's");
    // A short box with no baseline stands on it, inside the strut.
    let boxes = button(10.0, None);
    let p = Paragraph {
        content,
        runs: &[],
        boxes: &boxes,
    };
    let m = ts.metrics(p, &t, None);
    assert_eq!((m.height, m.first_baseline), (22.75, Some(16.0)));
    assert_eq!(
        ts.placed_inline_boxes(p, &t, None)[0].bounds.y,
        6.0,
        "its bottom on the baseline"
    );
}

fn glyph_key(ts: &mut TextSystem, ch: &str, size: f32, scale: f32) -> GlyphKey {
    let t = style(size, size * 1.5, 400);
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale,
        size: (100, 100),
        cull: None,
        snap: true,
    };
    ts.draw(&mut target, ch, &t, None, (0.0, 0.0), 1.0);
    items
        .iter()
        .find_map(|i| match i {
            Item::Glyph { key, .. } => Some(*key),
            _ => None,
        })
        .expect("a glyph")
}

#[test]
fn hinting_changes_glyphs_at_1x_and_not_from_2x() {
    let mut ts = TextSystem::new();
    let key = glyph_key(&mut ts, "e", 13.0, 1.0);
    // Hinted at 1x where Chrome hints (FreeType, DirectWrite); not on
    // macOS, where Chrome's CoreText does not.
    assert_eq!(key.flags & GlyphKey::HINTED != 0, !cfg!(target_os = "macos"));
    let hinted = GlyphKey {
        flags: key.flags | GlyphKey::HINTED,
        ..key
    };
    let plain = GlyphKey {
        flags: key.flags & !GlyphKey::HINTED,
        ..key
    };
    let mut glyphs = ts.glyphs();
    let hinted = glyphs.raster(&hinted, Coverage::Gray, Color::BLACK).expect("a raster");
    let plain = glyphs.raster(&plain, Coverage::Gray, Color::BLACK).expect("a raster");
    assert_ne!(hinted.data, plain.data, "the autohinter moves the outline at 1x");
    let key2 = glyph_key(&mut ts, "e", 13.0, 2.0);
    assert_eq!(key2.flags & GlyphKey::HINTED, 0, "unhinted from 2x");
}

#[test]
fn subpixel_coverage_fringes_by_stripe_order() {
    let mut ts = TextSystem::new();
    let key = glyph_key(&mut ts, "l", 16.0, 1.0);
    let mut glyphs = ts.glyphs();
    let rgb = glyphs
        .raster(&key, Coverage::Lcd(settings::Subpixel::Rgb), Color::BLACK)
        .expect("rgb");
    let bgr = glyphs
        .raster(&key, Coverage::Lcd(settings::Subpixel::Bgr), Color::BLACK)
        .expect("bgr");
    assert_eq!(rgb.content, GlyphContent::Lcd);
    assert_eq!((rgb.width, rgb.height), (bgr.width, bgr.height));
    let px = rgb.data.as_chunks::<4>().0;
    let mirrored = bgr
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(px)
        .all(|(b, r)| b[0] == r[2] && b[2] == r[0]);
    assert!(mirrored, "BGR swaps red and blue");
    assert!(px.iter().any(|p| p[0] != p[2]), "a stem's edges fringe");
}

#[test]
fn metrics_give_the_baselines_draw_places_glyphs_on() {
    // Inter, Blink's rounding (ADR-0005): 12px on 16px has its baseline 12
    // below the line's top, 24px on 32px 24, 16px on 24px 18 (Chrome's
    // `align-items: baseline`, `chrome_layout/flex_baseline`).
    let mut ts = TextSystem::new();
    for (size, line, want) in [(12.0, 16.0, 12.0), (24.0, 32.0, 24.0), (16.0, 24.0, 18.0)] {
        let t = style(size, line, 400);
        let m = ts.metrics("Base", &t, None);
        assert_eq!(m.first_baseline, Some(want), "{size}/{line}");
        assert_eq!(m.last_baseline, Some(want), "one line: {size}/{line}");
        let mut items = Vec::new();
        let mut target = Target {
            items: &mut items,
            scale: 1.0,
            size: (400, 100),
            cull: None,
            snap: true,
        };
        ts.draw(&mut target, "Base", &t, None, (0.0, 0.0), 1.0);
        let y = items.iter().find_map(|i| match i {
            Item::Glyph { y, .. } => Some(*y),
            _ => None,
        });
        assert_eq!(
            y,
            Some(f32_to_i32(want)),
            "draw puts {size}/{line} on the same baseline"
        );
    }
    // Lines below the first: the last baseline is on the last line, and
    // an empty paragraph has none.
    let t = style(16.0, 24.0, 400);
    let m = ts.metrics("one\ntwo\n", &t, None);
    assert_eq!((m.first_baseline, m.last_baseline), (Some(18.0), Some(66.0)), "{m:?}");
    let m = ts.metrics("", &t, None);
    assert_eq!((m.first_baseline, m.last_baseline, m.height), (None, None, 24.0));
    // A cut paragraph has the baselines of the lines it shows.
    let clamped = ResolvedText {
        line_clamp: Some(2),
        ..t
    };
    let m = ts.metrics("one two three four five six", &clamped, Some(60.0));
    assert_eq!(
        (m.first_baseline, m.last_baseline, m.height),
        (Some(18.0), Some(42.0), 48.0)
    );
}

/// The rectangles `draw` records for `p` at `scale` from (0, 0), in device
/// pixels (left, top, right, bottom) as the renderer reaches them: each
/// field times the scale, the far edges their sums.
fn decoration_rects(p: Paragraph<'_>, t: &ResolvedText, scale: f32) -> Vec<[f32; 4]> {
    let mut ts = TextSystem::new();
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale,
        size: (800, 200),
        cull: None,
        snap: true,
    };
    ts.draw(&mut target, p, t, None, (0.0, 0.0), 1.0);
    items
        .iter()
        .filter_map(|i| match i {
            Item::Rect { bounds: b, .. } => Some([
                b.x * scale,
                b.y * scale,
                b.x * scale + b.width * scale,
                b.y * scale + b.height * scale,
            ]),
            _ => None,
        })
        .collect()
}

#[test]
fn an_underline_sits_where_chrome_draws_it() {
    // Laid out in CSS pixels: the top below the baseline, the resolved
    // (unrounded) thickness.
    assert_eq!(underline(14.0, Some(3.0)), (3.0, 1.4));
    assert_eq!(underline(14.0, None), (1.0, 1.4));
    assert_eq!(underline(8.0, None), (1.0, 1.0));
    assert_eq!(underline(48.0, None), (3.0, 4.8));
    assert_eq!(underline(16.0, Some(4.3)), (4.0, 1.6));
    // MarkerLinkButtonDemo at 2x, 14px, `underline-offset-3`: Chrome's line
    // fills device rows 46-47 under a baseline at 40, solid.
    assert_eq!(
        line_rect(0.0, 10.0, 20.0 + 3.0, 1.4, 2.0).map(|b| (b.y * 2.0, b.height * 2.0)),
        Some((46.0, 2.0))
    );
    // TypographyDemo at 2x, 16px, `underline-offset-4`: two device rows
    // (Chrome's reference shot), the 1.6px line rounded down in CSS pixels.
    assert_eq!(
        line_rect(0.0, 10.0, 281.0, 1.6, 2.0).map(|b| (b.y * 2.0, b.height * 2.0)),
        Some((562.0, 2.0))
    );
    // 48px: 4.8 rounds down to 4 CSS pixels; the top snaps to the nearest.
    assert_eq!(
        line_rect(0.0, 10.0, 10.4, 4.8, 1.0).map(|b| (b.y, b.height)),
        Some((10.0, 4.0))
    );
    assert_eq!(
        line_rect(0.0, 10.0, 10.5, 4.8, 1.0).map(|b| (b.y, b.height)),
        Some((11.0, 4.0))
    );
}

#[test]
fn a_line_through_sits_a_third_of_the_ascent_above_the_baseline() {
    // Inter's ascent is 0.96875 em: 15.5px at 16px, its box's top 16px
    // above the baseline; the centre two thirds of 15.5 below that.
    let (above, thickness) = line_through(15.5, 15.5, 16.0);
    assert!((above - (-16.0 + 31.0 / 3.0 - 0.8)).abs() < 1e-5, "{above}");
    assert!((thickness - 1.6).abs() < 1e-6);
    // Drawn: a 16px/24px line's baseline at 18 (ascent 16 + half-leading
    // 2); top 18 - 6.47 = 11.53 snaps to 12, one CSS pixel thick: one
    // device row at 1x, two at 2x.
    let t = style(16.0, 24.0, 400).struck();
    let one = decoration_rects(Paragraph::from("strike"), &t, 1.0);
    assert_eq!(one.len(), 1, "{one:?}");
    assert_eq!((one[0][1], one[0][3]), (12.0, 13.0), "{one:?}");
    let two = decoration_rects(Paragraph::from("strike"), &t, 2.0);
    assert_eq!((two[0][1], two[0][3]), (24.0, 26.0), "{two:?}");
    // A line-through never skips ink.
    let ink = decoration_rects(Paragraph::from("gpy-ABH"), &t, 2.0);
    assert_eq!(ink.len(), 1, "{ink:?}");
}

/// The rectangles `draw` records for `p` at `scale`, as
/// [`decoration_rects`] gives them, each with its colour.
fn decoration_rects_colored(p: Paragraph<'_>, t: &ResolvedText, scale: f32) -> Vec<([f32; 4], Color)> {
    let mut ts = TextSystem::new();
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale,
        size: (800, 200),
        cull: None,
        snap: true,
    };
    ts.draw(&mut target, p, t, None, (0.0, 0.0), 1.0);
    items
        .iter()
        .filter_map(|i| match i {
            Item::Rect { bounds: b, color } => Some((
                [
                    b.x * scale,
                    b.y * scale,
                    b.x * scale + b.width * scale,
                    b.y * scale + b.height * scale,
                ],
                *color,
            )),
            _ => None,
        })
        .collect()
}

/// A run in `content` at `range` styled `text`, plain otherwise.
fn run(range: Range<usize>, text: ResolvedText) -> Run {
    Run {
        range,
        text,
        background: None,
        radius: 0.0,
        padding: Edges::default(),
    }
}

#[test]
fn a_paragraphs_underline_keeps_its_font_under_a_bigger_run() {
    // A 16px underlined paragraph with a 24px run that declares no
    // underline: one line under both, its thickness and offset the
    // paragraph's (Blink's decorating box).
    let t = style(16.0, 24.0, 400).underlined();
    let content = "small BIG small";
    let runs = [run(
        6..9,
        ResolvedText {
            size: 24.0,
            decorations: TextDecorations::NONE,
            ..t
        },
    )];
    for scale in [1.0, 2.0] {
        let rects = decoration_rects(Paragraph::new(content, &runs), &t, scale);
        assert!(rects.len() >= 3, "{rects:?}");
        for r in &rects {
            assert_eq!((r[1], r[3]), (rects[0][1], rects[0][3]), "at {scale}: {rects:?}");
        }
        // 1.6px rounds down to one CSS pixel.
        assert_eq!(rects[0][3] - rects[0][1], scale, "{rects:?}");
        // Continuous: each piece starts where the one before ends.
        for w in rects.windows(2) {
            assert!((w[1][0] - w[0][2]).abs() <= 1.0, "{rects:?}");
        }
    }
    // A run that adds the underline itself draws it by its own size.
    let plain = style(16.0, 24.0, 400);
    let own = [run(
        6..9,
        ResolvedText {
            size: 24.0,
            ..plain.clone()
        }
        .underlined(),
    )];
    let rects = decoration_rects(Paragraph::new(content, &own), &plain, 1.0);
    // 2.4px: two pixels thick, three below the baseline (gap ceil(1.2) = 2).
    assert_eq!(rects.len(), 1, "{rects:?}");
    assert_eq!(rects[0][3] - rects[0][1], 2.0, "{rects:?}");
}

#[test]
fn a_paragraph_and_a_run_that_both_underline_draw_both_lines() {
    // CSS: the paragraph's line by its 16px font across the whole line,
    // and the run's own by its 24px font across the run alone.
    let t = style(16.0, 24.0, 400).underlined();
    let red = Color::hex(0xff_00_00);
    let content = "small BIG small";
    let runs = [run(
        6..9,
        ResolvedText {
            size: 24.0,
            color: red,
            ..t.clone().undecorated()
        }
        .underlined(),
    )];
    let alone = decoration_rects(
        Paragraph::new(
            content,
            &[run(
                6..9,
                ResolvedText {
                    size: 24.0,
                    decorations: TextDecorations::NONE,
                    ..t
                },
            )],
        ),
        &t,
        1.0,
    );
    let both = decoration_rects_colored(Paragraph::new(content, &runs), &t, 1.0);
    // The paragraph's line, in its colour, as it is with no run line.
    let paragraph: Vec<[f32; 4]> = both.iter().filter(|(_, c)| *c == t.color).map(|(r, _)| *r).collect();
    assert_eq!(paragraph, alone, "{both:?}");
    // The run's: two pixels thick, two below the baseline (the
    // paragraph's is one thick, one below), within the run.
    let own: Vec<[f32; 4]> = both.iter().filter(|(_, c)| *c == red).map(|(r, _)| *r).collect();
    assert_eq!(own.len(), 1, "{both:?}");
    assert_eq!(own[0][3] - own[0][1], 2.0, "{own:?}");
    assert_eq!(own[0][1] - paragraph[0][1], 1.0, "{both:?}");
    let (left, right) = (paragraph[0][0], paragraph[paragraph.len() - 1][2]);
    assert!(own[0][0] > left + 20.0 && own[0][2] < right - 20.0, "{both:?}");
}

#[test]
fn a_run_cannot_take_off_its_paragraphs_underline() {
    // `underline(false)` on a span: the paragraph's line goes on under
    // it, in the paragraph's colour.
    let t = style(16.0, 24.0, 400).underlined().struck();
    let own = ContentStyle {
        color: Some(Color::hex(0x00_00_ff)),
        text: crate::style::TextStyle {
            underline: Some(false),
            line_through: Some(false),
            ..Default::default()
        },
        ..Default::default()
    };
    let span = Run::style_in(&t, &own);
    assert!(span.decorations.is_empty(), "the span declares none of its own");
    // A node under the paragraph keeps its decorations too.
    let below = t.provide(&own);
    assert!(
        below.decorations.underline() && below.decorations.line_through(),
        "{below:?}"
    );
    let content = "plain span plain";
    let runs = [run(6..10, span)];
    for scale in [1.0, 2.0] {
        let plain = decoration_rects_colored(Paragraph::from(content), &t, scale);
        let under = decoration_rects_colored(Paragraph::new(content, &runs), &t, scale);
        assert!(under.iter().all(|(_, c)| *c == t.color), "{under:?}");
        // The same pixels (drawn a run at a time, as CSS paints each
        // fragment of text).
        let covered = |rects: Vec<([f32; 4], Color)>| {
            let mut rects: Vec<[f32; 4]> = rects.into_iter().map(|(r, _)| r).collect();
            rects.sort_by(|a, b| {
                (a[1], a[0])
                    .partial_cmp(&(b[1], b[0]))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut out: Vec<[f32; 4]> = Vec::new();
            for r in rects {
                match out.last_mut() {
                    Some(l) if l[1] == r[1] && l[3] == r[3] && r[0] <= l[2] => l[2] = l[2].max(r[2]),
                    _ => out.push(r),
                }
            }
            out
        };
        assert_eq!(covered(under), covered(plain), "at {scale}");
    }
}

#[test]
fn an_underline_skips_the_ink_of_descenders() {
    let t = style(16.0, 24.0, 400).underlined();
    let plain = style(16.0, 24.0, 400);
    let content = "a g p y a";
    for scale in [1.0, 2.0] {
        // No descender: one line.
        let abc = decoration_rects(Paragraph::from("abc"), &t, scale);
        assert_eq!(abc.len(), 1, "{abc:?}");
        // g, p and y each cross the line: it breaks around each.
        let gpy = decoration_rects(Paragraph::from(content), &t, scale);
        assert_eq!(gpy.len(), 4, "at {scale}: {gpy:?}");
        // A run over all of it declaring the underline draws the same
        // line as the paragraph declaring it.
        let runs = [run(0..content.len(), t.clone())];
        let by_run = decoration_rects(Paragraph::new(content, &runs), &plain, scale);
        assert_eq!(by_run, gpy, "at {scale}");
        // Each gap is the glyph's ink in the line's band (half a pixel in
        // from the line's unsnapped edges, as Blink measures it), widened
        // by the dilation (the thickness, 1.6px) each side, to within a
        // device pixel. The ink is read from the glyph rasterized eight
        // times finer, apart from skip-ink's reading of the outline.
        let mut ts = TextSystem::new();
        let base = ts.metrics(content, &t, None).first_baseline.expect("a line");
        let (below, thickness) = underline(16.0, None);
        let band = ((base + below + 0.5) * scale, (base + below + thickness - 0.5) * scale);
        let mut items = Vec::new();
        let mut target = Target {
            items: &mut items,
            scale,
            size: (800, 200),
            cull: None,
            snap: true,
        };
        ts.draw(&mut target, content, &t, None, (0.0, 0.0), 1.0);
        let mut glyphs = ts.glyphs();
        let placed: Vec<(GlyphKey, i32, i32)> = items
            .iter()
            .filter_map(|i| match i {
                Item::Glyph { key, x, y, .. } => Some((*key, *x, *y)),
                _ => None,
            })
            .collect();
        const FINE: f32 = 8.0;
        let dilation = 1.6 * scale;
        for (k, at) in [2, 4, 6].into_iter().enumerate() {
            let (key, x, y) = placed[at];
            let pen = i32_to_f32(x) + f32::from(key.bin) / 4.0;
            let fine = GlyphKey {
                ppem: (f32::from_bits(key.ppem) * FINE).to_bits(),
                bin: 0,
                flags: 0,
                ..key
            };
            let image = glyphs
                .raster(&fine, Coverage::Gray, Color::BLACK)
                .expect("g, p and y have ink");
            assert_eq!(image.content, GlyphContent::Mask);
            let (mut left, mut right) = (f32::MAX, f32::MIN);
            for row in 0..image.height {
                // The row's centre, in device pixels.
                let centre = i32_to_f32(y) + (crate::convert::u32_to_f32(row) + 0.5 - i32_to_f32(image.top)) / FINE;
                if centre < band.0 || centre > band.1 {
                    continue;
                }
                for col in 0..image.width {
                    let i = usize::try_from(row * image.width + col).unwrap_or(0);
                    if image.data[i] >= 128 {
                        let at = pen + (i32_to_f32(image.left) + crate::convert::u32_to_f32(col)) / FINE;
                        left = left.min(at);
                        right = right.max(at + 1.0 / FINE);
                    }
                }
            }
            let (gap_left, gap_right) = (gpy[k][2], gpy[k + 1][0]);
            assert!(
                (gap_left - (left - dilation)).abs() <= 1.0 && (gap_right - (right + dilation)).abs() <= 1.0,
                "at {scale}, glyph {k}: gap {gap_left}..{gap_right}, ink {left}..{right} widened by {dilation}"
            );
        }
    }
}

#[test]
fn a_decoration_lands_on_device_pixels_at_any_scale() {
    for scale in [1.0_f32, 1.25, 1.5, 2.0, 2.5, 3.0] {
        for i in 0..2000_u16 {
            let top = f32::from(i) * 0.37 + 0.13;
            let Some(b) = line_rect(top * 0.7, top * 1.3 + 1.0, top, 1.6, scale) else {
                continue;
            };
            for v in [b.x * scale, b.y * scale, b.width * scale, b.height * scale] {
                assert_eq!(v, v.round(), "{v} at {scale}");
            }
            let bottom = b.y * scale + b.height * scale;
            assert_eq!(bottom, bottom.round(), "{bottom} at {scale}");
        }
    }
    // Drawn text at 1.5x and 3x: every edge a whole device pixel.
    let t = style(14.0, 20.0, 400).underlined().struck();
    for scale in [1.5, 3.0] {
        for r in decoration_rects(Paragraph::from("Struck and underlined, gpy"), &t, scale) {
            for v in r {
                assert_eq!(v, v.round(), "{r:?} at {scale}");
            }
        }
    }
}

/// What `draw` records for `p` in `t` at `width`, 1x, from (0, 0).
fn drawn(ts: &mut TextSystem, p: Paragraph<'_>, t: &ResolvedText, width: Option<f32>) -> Vec<Item> {
    let mut items = Vec::new();
    let mut target = Target {
        items: &mut items,
        scale: 1.0,
        size: (800, 200),
        cull: None,
        snap: true,
    };
    ts.draw(&mut target, p, t, width, (0.0, 0.0), 1.0);
    items
}

/// The glyphs of `items`, in order: their keys and places.
fn glyphs_of(items: &[Item]) -> Vec<(GlyphKey, i32, i32)> {
    items
        .iter()
        .filter_map(|i| match i {
            Item::Glyph { key, x, y, .. } => Some((*key, *x, *y)),
            _ => None,
        })
        .collect()
}

/// The glyph `…` is drawn with in `t`.
fn ellipsis_glyph(t: &ResolvedText) -> u32 {
    let items = drawn(&mut TextSystem::new(), Paragraph::from(ELLIPSIS), t, None);
    let glyphs = glyphs_of(&items);
    assert_eq!(glyphs.len(), 1, "one glyph: {glyphs:?}");
    glyphs[0].0.glyph
}

#[test]
fn backgrounds_and_underlines_go_under_the_glyphs_and_line_throughs_over() {
    // Chrome paints a line's inline backgrounds, then its underlines,
    // then its glyphs, then its line-throughs (measured, Chrome 145: an
    // underline raised into the glyphs is covered by them, a line-through
    // covers them).
    let blue = Color::hex(0x00_00_ff);
    let red = Color::hex(0xff_00_00);
    let t = style(16.0, 24.0, 400).underlined().struck();
    let content = "plain chip jumpy";
    let chip = |text: ResolvedText, background| Run {
        background,
        radius: 4.0,
        padding: Edges {
            left: 4.0,
            right: 4.0,
            top: 0.0,
            bottom: 0.0,
        },
        ..run(6..10, text)
    };
    let red_text = ResolvedText {
        color: red,
        ..style(16.0, 24.0, 400)
    };
    let runs = [chip(red_text.clone().underlined().struck(), Some(blue))];
    let mut ts = TextSystem::new();
    let base = ts.metrics(content, &t, None).first_baseline.expect("a line");
    let items = drawn(&mut ts, Paragraph::new(content, &runs), &t, None);
    let (mut backgrounds, mut unders, mut glyphs, mut throughs) = (vec![], vec![], vec![], vec![]);
    for (at, item) in items.iter().enumerate() {
        match item {
            Item::RoundedRect { color, .. } if *color == blue => backgrounds.push(at),
            Item::Rect { bounds, .. } if bounds.y > base => unders.push(at),
            Item::Rect { .. } => throughs.push(at),
            Item::Glyph { .. } => glyphs.push(at),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(backgrounds.len(), 1, "{items:?}");
    // The paragraph's underline and the run's (in three pieces at least:
    // j and y cut it), its line-through and the run's.
    assert!(unders.len() >= 3 && throughs.len() >= 2, "{items:?}");
    let last = |v: &[usize]| v.iter().copied().max().unwrap_or(0);
    let first = |v: &[usize]| v.iter().copied().min().unwrap_or(usize::MAX);
    assert!(last(&backgrounds) < first(&unders), "backgrounds first: {items:?}");
    assert!(last(&unders) < first(&glyphs), "underlines under the glyphs: {items:?}");
    assert!(last(&glyphs) < first(&throughs), "line-throughs over them: {items:?}");
    // The glyphs are those of the paragraph drawn bare (the same runs, no
    // background or line), each where it was: nothing moved them.
    let bare = [chip(red_text, None)];
    let plain = drawn(
        &mut TextSystem::new(),
        Paragraph::new(content, &bare),
        &style(16.0, 24.0, 400),
        None,
    );
    assert_eq!(glyphs_of(&items), glyphs_of(&plain));
}

#[test]
fn each_line_puts_down_its_background_underline_glyphs_and_line_through_in_turn() {
    // Wrapped: line 1's background, underline, glyphs, line-through, then
    // line 2's, and so on (Chrome paints each line's fragments in turn).
    let blue = Color::hex(0x00_00_ff);
    let red = Color::hex(0xff_00_00);
    // A red underline from one box, a black line-through from another.
    let t = ResolvedText {
        color: red,
        ..style(16.0, 24.0, 400)
    }
    .underlined();
    let t = ResolvedText {
        color: Color::BLACK,
        ..t
    }
    .struck();
    let content = "aa bb cc dd ee ff gg hh";
    let runs = [Run {
        background: Some(blue),
        radius: 2.0,
        ..run(0..content.len(), t.clone().undecorated())
    }];
    let items = drawn(&mut TextSystem::new(), Paragraph::new(content, &runs), &t, Some(60.0));
    let mut kinds = String::new();
    for i in &items {
        let k = match i {
            Item::RoundedRect { color, .. } if *color == blue => 'b',
            Item::Rect { color, .. } if *color == red => 'u',
            Item::Rect { color, .. } if *color == Color::BLACK => 't',
            Item::Glyph { .. } => 'g',
            other => panic!("unexpected {other:?}"),
        };
        if !kinds.ends_with(k) {
            kinds.push(k);
        }
    }
    let lines = glyphs_of(&items)
        .iter()
        .map(|g| g.2)
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    assert!(lines >= 3, "it wraps: {lines}");
    assert_eq!(kinds, "bugt".repeat(lines));
    // The glyphs where they are drawn bare.
    let bare = drawn(
        &mut TextSystem::new(),
        Paragraph::from(content),
        &style(16.0, 24.0, 400),
        Some(60.0),
    );
    assert_eq!(glyphs_of(&items), glyphs_of(&bare));
}

#[test]
fn an_ellipsis_is_not_decorated() {
    // Chrome 145 draws a block's underline under the text an ellipsis
    // ends, not under the ellipsis.
    let t = ResolvedText {
        truncate: true,
        ..style(16.0, 24.0, 400)
    }
    .underlined();
    let text = "A long line of text that does not fit in its box at all";
    let items = drawn(&mut TextSystem::new(), Paragraph::from(text), &t, Some(120.0));
    let glyphs = glyphs_of(&items);
    let dots = ellipsis_glyph(&t);
    // One ellipsis, the last glyph, and the line's last.
    assert_eq!(glyphs.iter().filter(|g| g.0.glyph == dots).count(), 1, "{glyphs:?}");
    let (key, x, _) = *glyphs.last().expect("glyphs");
    assert_eq!(key.glyph, dots, "{glyphs:?}");
    assert!(glyphs.iter().all(|g| g.1 <= x), "{glyphs:?}");
    let right = items
        .iter()
        .filter_map(|i| match i {
            Item::Rect { bounds, .. } => Some(bounds.right()),
            _ => None,
        })
        .fold(f32::MIN, f32::max);
    assert!(right > 60.0, "the kept text is underlined: {right}");
    assert!(right <= i32_to_f32(x) + 1.0, "the ellipsis at {x} is not: {right}");
}

#[test]
fn an_rtl_line_keeps_its_start_and_ends_in_an_ellipsis_at_the_left() {
    // css-overflow-3 §5.1: a right-to-left line overflows at its end edge,
    // the left. It keeps its logical start (the right) and the ellipsis
    // goes left of what is kept, at the box's left.
    let rtl = ResolvedText {
        direction: Direction::Rtl,
        nowrap: true,
        ..style(16.0, 24.0, 400)
    };
    let text = "שלום עולם זה טקסט ארוך מאוד שלא נכנס";
    let width = 90.0;
    let whole = glyphs_of(&drawn(&mut TextSystem::new(), Paragraph::from(text), &rtl, Some(width)));
    let cut = ResolvedText { truncate: true, ..rtl };
    let glyphs = glyphs_of(&drawn(&mut TextSystem::new(), Paragraph::from(text), &cut, Some(width)));
    let dots = ellipsis_glyph(&cut);
    let (ellipsis, kept): (Vec<&(GlyphKey, i32, i32)>, Vec<_>) = glyphs.iter().partition(|g| g.0.glyph == dots);
    assert_eq!(ellipsis.len(), 1, "{glyphs:?}");
    let ex = ellipsis[0].1;
    assert!(kept.len() >= 3, "{glyphs:?}");
    assert!(kept.iter().all(|g| g.1 > ex), "the ellipsis is at the left: {glyphs:?}");
    assert!((0..12).contains(&ex), "at the box's left edge: {ex}");
    // What is kept is the overflowing line's right end, where it was: its
    // start, flush with the box's right edge.
    let leftmost = kept.iter().map(|g| g.1).min().unwrap_or(0);
    let tail: Vec<_> = whole.iter().filter(|g| g.1 >= leftmost).copied().collect();
    assert_eq!(kept.into_iter().copied().collect::<Vec<_>>(), tail);
    let right = whole.iter().map(|g| g.1).max().unwrap_or(0);
    assert!(right > 60 && right <= 90, "the start is at the right: {right}");
}

#[test]
fn a_padded_chip_the_cut_leaves_nothing_of_paints_no_box() {
    // The chip's first glyph does not fit before the ellipsis: neither its
    // padding nor its background shows behind it.
    let blue = Color::hex(0x00_00_ff);
    let t = ResolvedText {
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let runs = [Run {
        background: Some(blue),
        radius: 4.0,
        padding: Edges {
            left: 6.0,
            right: 6.0,
            top: 0.0,
            bottom: 0.0,
        },
        ..run(0..4, t.clone())
    }];
    let mut ts = TextSystem::new();
    let ellipsis = ts.measure(ELLIPSIS, &t, None).0;
    let content = "chip and more text that does not fit";
    let items = drawn(&mut ts, Paragraph::new(content, &runs), &t, Some(ellipsis + 8.0));
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, Item::RoundedRect { .. } | Item::Rect { .. })),
        "{items:?}"
    );
    let glyphs = glyphs_of(&items);
    assert_eq!(glyphs.len(), 1, "the ellipsis alone: {glyphs:?}");
    // Nor its padding's room: the ellipsis starts the line.
    assert!(
        glyphs[0].1 * 4 + i32::from(glyphs[0].0.bin) <= 1,
        "the ellipsis at the line's start: {glyphs:?}"
    );
    // Wide enough for "ch": the chip's box shows, ending under the cut.
    let items = drawn(&mut ts, Paragraph::new(content, &runs), &t, Some(ellipsis + 30.0));
    let boxes: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            Item::RoundedRect { bounds, .. } => Some(*bounds),
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 1, "{items:?}");
    let dots = glyphs_of(&items).last().map_or(0, |g| g.1);
    assert!(boxes[0].right() <= i32_to_f32(dots) + 0.5, "{boxes:?} before {dots}");
}

#[test]
fn an_underline_skips_an_atomic_inline() {
    // CSS decorates no atomic inline: "before [box] after", underlined,
    // has no line across the box.
    let t = style(16.0, 24.0, 400).underlined();
    let content = "before \u{FFFC} after";
    let boxes = [InlineSlot {
        at: 7,
        width: 40.0,
        height: 16.0,
        baseline: None,
        id: 7,
    }];
    let p = Paragraph {
        content,
        runs: &[],
        boxes: &boxes,
    };
    let mut ts = TextSystem::new();
    let b = ts.placed_inline_boxes(p, &t, None)[0].bounds;
    let items = drawn(&mut ts, p, &t, None);
    let lines: Vec<Bounds> = items
        .iter()
        .filter_map(|i| match i {
            Item::Rect { bounds, .. } => Some(*bounds),
            _ => None,
        })
        .collect();
    assert!(lines.len() >= 2, "{lines:?}");
    assert!(lines.iter().any(|l| l.right() <= b.x + 0.5), "{lines:?}");
    assert!(lines.iter().any(|l| l.x >= b.right() - 0.5), "{lines:?}");
    for l in &lines {
        assert!(l.right() <= b.x + 0.5 || l.x >= b.right() - 0.5, "{l:?} crosses {b:?}");
    }
}

/// An underlined, unwrapped paragraph of 12,000 descenders and a short run
/// of its own after them: the skip-ink buffers hold 12,000 gaps, then a
/// few.
fn descenders() -> (String, [Run; 1], ResolvedText) {
    let t = style(16.0, 24.0, 400).underlined();
    let mut content = "gpy".repeat(4_000);
    let from = content.len();
    content.push_str(" x");
    let runs = [run(from.saturating_add(1)..content.len(), style(16.0, 24.0, 400))];
    (content, runs, t)
}

#[test]
fn a_steady_draw_keeps_its_skip_ink_room() {
    // Drawn each frame, the buffers are what it uses: never cut back to
    // the short run's (and grown again the next frame).
    let (content, runs, t) = descenders();
    let mut ts = TextSystem::new();
    let mut room = None;
    for frame in 0..3 * SCRATCH_AGE {
        drawn(&mut ts, Paragraph::new(&content, &runs), &t, None);
        ts.end_frame();
        let now = (ts.scratch.cuts.buf.capacity(), ts.scratch.segments.buf.capacity());
        assert!(now.0 >= 12_000, "frame {frame}: {now:?}");
        assert_eq!(*room.get_or_insert(now), now, "frame {frame}: its room changed");
    }
}

#[test]
fn a_one_off_draw_lets_its_room_go() {
    // Drawn once, then plain text each frame: what the big draw left in
    // the buffers does not keep their room.
    let (content, runs, t) = descenders();
    let mut ts = TextSystem::new();
    drawn(&mut ts, Paragraph::new(&content, &runs), &t, None);
    ts.end_frame();
    assert!(ts.scratch.inked.buf.capacity() >= 12_000);
    for _ in 0..2 * SCRATCH_AGE {
        drawn(&mut ts, Paragraph::from("plain"), &style(16.0, 24.0, 400), None);
        ts.end_frame();
    }
    // The large ones (past `SCRATCH_BYTES`); the small are kept anyway.
    let rooms = [ts.scratch.inked.buf.capacity(), ts.scratch.cuts.buf.capacity()];
    assert!(rooms.iter().all(|r| *r < 1_000), "{rooms:?}");
}

#[test]
fn an_enclosing_boxs_decoration_keeps_its_colour_size_and_offset() {
    // CSS: a decoration is drawn by its decorating box, whatever its
    // descendants' colour and size.
    let red = Color::hex(0xff_00_00);
    let parent = ResolvedText {
        color: red,
        underline_offset: Some(3.0),
        ..style(16.0, 24.0, 400)
    }
    .underlined();
    let own = ContentStyle {
        color: Some(Color::hex(0x00_00_ff)),
        text: crate::style::TextStyle {
            size: Some(24.0),
            underline_offset: Some(6.0),
            ..Default::default()
        },
        ..Default::default()
    };
    let child = parent.provide(&own);
    let by_parent = TextDecoration {
        lines: crate::style::DecorationLines::UNDERLINE,
        color: red,
        size: 16.0,
        family: FontFamily::INTER,
        weight: 400,
        italic: false,
        offset: Some(3.0),
    };
    assert_eq!(child.decorations.to_vec(), [by_parent]);
    // Drawn: the child's paragraph's underline is the parent's, one pixel
    // thick (1.6px at 16px) and three below the baseline, in red.
    let rects = decoration_rects_colored(Paragraph::from("abc"), &child, 1.0);
    let mut ts = TextSystem::new();
    let base = ts.metrics("abc", &child, None).first_baseline.expect("a line");
    assert_eq!(rects.len(), 1, "{rects:?}");
    assert_eq!(rects[0].1, red);
    assert_eq!(rects[0].0[3] - rects[0].0[1], 1.0, "{rects:?}");
    assert_eq!(rects[0].0[1], (base + 3.0).round(), "{rects:?}");
}

#[test]
fn a_node_that_declares_an_underline_inside_an_underlined_box_draws_both() {
    // css-text-decor-3 §2.1: each decorating box draws its own line. A red
    // 1px line from the container (16px) and a blue 2px one, 6px down,
    // from the node (24px) that declares its own.
    let red = Color::hex(0xff_00_00);
    let blue = Color::hex(0x00_00_ff);
    let parent = ResolvedText {
        color: red,
        ..style(16.0, 24.0, 400)
    }
    .underlined();
    let own = ContentStyle {
        color: Some(blue),
        text: crate::style::TextStyle {
            size: Some(24.0),
            underline: Some(true),
            underline_offset: Some(6.0),
            ..Default::default()
        },
        ..Default::default()
    };
    let child = parent.provide(&own);
    let boxes: Vec<_> = child
        .decorations
        .to_vec()
        .iter()
        .map(|d| (d.color, d.size, d.offset))
        .collect();
    assert_eq!(boxes, [(red, 16.0, None), (blue, 24.0, Some(6.0))]);
    let rects = decoration_rects_colored(Paragraph::from("abc"), &child, 1.0);
    let reds: Vec<_> = rects.iter().filter(|(_, c)| *c == red).collect();
    let blues: Vec<_> = rects.iter().filter(|(_, c)| *c == blue).collect();
    assert_eq!((reds.len(), blues.len()), (1, 1), "{rects:?}");
    assert_eq!(reds[0].0[3] - reds[0].0[1], 1.0, "{rects:?}");
    assert_eq!(blues[0].0[3] - blues[0].0[1], 2.0, "{rects:?}");
    assert!(blues[0].0[1] > reds[0].0[1], "{rects:?}");
    // However deep: ten nested boxes, each with its own line, draw ten.
    let mut deep = child.clone();
    for i in 0..8_u8 {
        deep = deep.provide(&ContentStyle {
            color: Some(Color::hex(u32::from(i) << 16 | 0x10)),
            text: crate::style::TextStyle {
                underline: Some(true),
                ..Default::default()
            },
            ..Default::default()
        });
    }
    let chain = deep.decorations.to_vec();
    assert_eq!(chain.len(), 10);
    assert_eq!((chain[0].color, chain[1].color), (red, blue), "outermost first");
    assert_eq!(chain[9].color, Color::hex(7 << 16 | 0x10));
    let rects = decoration_rects_colored(Paragraph::from("abc"), &deep, 1.0);
    for i in 0..8_u8 {
        assert!(
            rects.iter().any(|(_, c)| *c == Color::hex(u32::from(i) << 16 | 0x10)),
            "box {i}: {rects:?}"
        );
    }
    assert!(rects.iter().any(|(_, c)| *c == red), "{rects:?}");
    // Resolved again, the same boxes give the same handle: hash-consed.
    let mut again = child;
    for i in 0..8_u8 {
        again = again.provide(&ContentStyle {
            color: Some(Color::hex(u32::from(i) << 16 | 0x10)),
            text: crate::style::TextStyle {
                underline: Some(true),
                ..Default::default()
            },
            ..Default::default()
        });
    }
    assert_eq!(again.decorations, deep.decorations);
}

#[test]
fn an_inherited_line_through_sits_by_the_declaring_boxs_font() {
    // A struck 32px Geist Mono box around a 16px Inter paragraph: the line
    // sits by Geist Mono's ascent at 32px, as Blink's decorating box does,
    // not by Inter's scaled to 32px.
    let parent = ResolvedText {
        family: FontFamily::GEIST_MONO,
        ..style(32.0, 40.0, 400)
    }
    .struck();
    let child = parent.provide(&ContentStyle {
        text: crate::style::TextStyle {
            size: Some(16.0),
            family: Some(FontFamily::INTER),
            line_height: Some(crate::style::LineHeight::Px(24.0)),
            ..Default::default()
        },
        ..Default::default()
    });
    let mut ts = TextSystem::new();
    let ascent = |ts: &mut TextSystem, t: &ResolvedText| {
        let p = ts.piece(Paragraph::from(PROBE), t, 0, PROBE.len());
        ts.break_piece(p, Breaking::ONE_LINE);
        ts.shaped[&p].layout.root_span_metrics().expect("metrics").ascent
    };
    let mono = ascent(&mut ts, &parent.shape());
    let inter_32 = ascent(
        &mut ts,
        &ResolvedText {
            size: 32.0,
            ..child.clone().shape()
        },
    );
    assert!((mono - inter_32).abs() > 0.5, "the faces differ: {mono} {inter_32}");
    let base = ts.metrics("abc", &child, None).first_baseline.expect("a line");
    let inter_16 = ascent(&mut ts, &child.clone().shape());
    let (above, thickness) = line_through(inter_16, mono, 32.0);
    let want = line_rect(0.0, 1.0, base + above, thickness, 1.0).expect("a line");
    let rects = decoration_rects(Paragraph::from("abc"), &child, 1.0);
    assert_eq!(rects.len(), 1, "{rects:?}");
    assert_eq!((rects[0][1], rects[0][3]), (want.y, want.y + want.height), "{rects:?}");
}

#[test]
fn decorations_stop_at_atomic_inlines_and_out_of_flow_boxes() {
    let parent = style(16.0, 24.0, 400).underlined().struck();
    let flowing = crate::style::Style::default();
    assert!(flowing.content_in(&parent).decorations.underline());
    for stops in [
        crate::style::Style {
            inline: true,
            ..Default::default()
        },
        crate::style::Style {
            position: crate::style::Position::Absolute,
            ..Default::default()
        },
    ] {
        let inner = stops.content_in(&parent);
        assert!(inner.decorations.is_empty(), "{inner:?}");
        // What it declares itself it draws.
        let declares = crate::style::Style {
            content: ContentStyle {
                text: crate::style::TextStyle {
                    underline: Some(true),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..stops
        };
        let own = declares.content_in(&parent).decorations;
        assert!(own.underline() && !own.line_through(), "{own:?}");
    }
}

#[test]
fn a_scratch_buffer_lets_a_burst_go_and_keeps_steady_room() {
    let big = 100_000_usize;
    // A burst: one frame fills 800 KB, the next frames 10 items.
    let mut s = Scratch::<u64>::default();
    s.reuse().extend(0..u64::try_from(big).unwrap_or(0));
    s.trim();
    let mut frames = 0;
    while s.buf.capacity() >= big {
        s.reuse().extend(0..10);
        s.trim();
        frames += 1;
        assert!(
            frames <= 1 + SCRATCH_AGE,
            "it is cut within {SCRATCH_AGE} frames past the burst"
        );
    }
    assert!(frames > SCRATCH_AGE / 2, "not at once: {frames}");
    assert!(s.buf.capacity() < 64, "cut to what it held: {}", s.buf.capacity());
    // Steady: 800 KB drawn each frame keeps its room.
    let mut steady = Scratch::<u64>::default();
    for _ in 0..4 * SCRATCH_AGE {
        steady.reuse().extend(0..u64::try_from(big).unwrap_or(0));
        steady.trim();
    }
    assert!(steady.buf.capacity() >= big);
    // Small: under the large bound, never cut.
    let mut small = Scratch::<u64>::default();
    small.reuse().extend(0..1000);
    for _ in 0..4 * SCRATCH_AGE {
        small.reuse().extend(0..1);
        small.trim();
    }
    assert!(small.buf.capacity() >= 1000);
}

#[test]
fn a_small_chunk_never_holds_a_big_chunks_room() {
    // A 10,000-piece textarea recorded again (a caret blink) by turns with
    // many one-piece rows: each row's kept pieces stay a row's room, not
    // the textarea's, so the rooms add up to the chunks' own.
    let t = style(16.0, 24.0, 400);
    let big: String = (0..10_000).map(|n| format!("{n}\n")).collect();
    let mut ts = TextSystem::new();
    let mut textarea = Vec::new();
    let mut rows: Vec<Vec<u64>> = (0..64).map(|_| Vec::new()).collect();
    for round in 0..3 {
        for (r, kept) in rows.iter_mut().enumerate() {
            drawn(&mut ts, Paragraph::from(big.as_str()), &t, None);
            ts.swap_drawn(&mut textarea);
            assert_eq!(textarea.len(), 10_001);
            let label = format!("row {r} {round}");
            drawn(&mut ts, Paragraph::from(label.as_str()), &t, None);
            ts.swap_drawn(kept);
            assert_eq!(kept.len(), 1);
        }
        ts.end_frame();
    }
    for kept in &rows {
        assert!(kept.capacity() < 64, "a row holds {}", kept.capacity());
    }
    // A new chunk takes its pieces at their length too.
    drawn(&mut ts, Paragraph::from(big.as_str()), &t, None);
    ts.clear_drawn();
    drawn(&mut ts, Paragraph::from("new"), &t, None);
    assert!(ts.take_drawn().capacity() < 64);
}

#[test]
fn an_rtl_line_is_cut_by_its_text_not_the_space_that_hangs_at_its_end() {
    // "שלום עולם " unwrapped: the space at its end (its left) hangs. In a
    // box a pixel narrower than the text and the space, the text fits: no
    // ellipsis, every glyph, flush right. A pixel narrower than the text
    // alone, it overflows and ends in an ellipsis.
    let rtl = ResolvedText {
        direction: Direction::Rtl,
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        truncate: false,
        nowrap: true,
        ..rtl.clone()
    };
    let mut ts = TextSystem::new();
    let text = "שלום עולם ";
    let visible = ts.measure("שלום עולם", &plain, None).0;
    let whole = ts.measure(text, &plain, None).0;
    assert!(
        whole - visible > 1.0,
        "the space is wider than a pixel: {visible} {whole}"
    );
    let dots = ellipsis_glyph(&rtl);
    let fits = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &rtl, Some(whole - 1.0)));
    assert!(fits.iter().all(|g| g.0.glyph != dots), "{fits:?}");
    assert_eq!(fits.len(), alone(&plain, text).len(), "{fits:?}");
    let cut = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &rtl, Some(visible - 1.0)));
    assert!(cut.iter().any(|g| g.0.glyph == dots), "{cut:?}");
}

/// The glyph keys and x of `text` drawn alone in `t`, unwrapped.
fn alone(t: &ResolvedText, text: &str) -> Vec<(GlyphKey, i32, i32)> {
    glyphs_of(&drawn(&mut TextSystem::new(), Paragraph::from(text), t, None))
}

#[test]
fn a_clamped_line_and_its_ellipsis_are_aligned_as_one() {
    // css-overflow-4 block-ellipsis: `X…` fits a 120px box, so X stays,
    // wherever the box aligns its text.
    let base = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let x = alone(&base, "X")[0].0.glyph;
    let dots = ellipsis_glyph(&base);
    let mut ts = TextSystem::new();
    // Glyph origins on whole pixels (Chrome on Linux at 1x): the
    // ellipsis's too, beside the text's.
    ts.whole_origins = true;
    let wide = ts.measure("X\u{2026}", &base, None).0;
    let x_w = ts.measure("X", &base, None).0;
    for (align, expect) in [
        (TextAlign::Right, 120.0 - wide),
        (TextAlign::Center, (120.0 - wide) / 2.0),
        (TextAlign::Left, 0.0),
    ] {
        let t = ResolvedText { align, ..base.clone() };
        let glyphs = glyphs_of(&drawn(&mut ts, Paragraph::from("X\nnext"), &t, Some(120.0)));
        let ids: Vec<u32> = glyphs.iter().map(|g| g.0.glyph).collect();
        assert_eq!(ids, [x, dots], "{align:?}: {glyphs:?}");
        let left = i32_to_f32(glyphs[0].1);
        assert!((left - expect).abs() <= 1.0, "{align:?}: X at {left}, not {expect}");
        let at = i32_to_f32(glyphs[1].1);
        assert!(
            (at - (expect + x_w)).abs() <= 1.0,
            "{align:?}: the ellipsis at {at}, not {}",
            expect + x_w
        );
        assert!(
            glyphs.iter().all(|g| g.0.bin == 0),
            "{align:?}: whole pixels: {glyphs:?}"
        );
    }
    // Right to left, aligned left with room to spare: all of its glyphs
    // stay, the ellipsis at the left edge and the text after it.
    let rtl = ResolvedText {
        direction: Direction::Rtl,
        align: TextAlign::Left,
        ..base.clone()
    };
    let word = alone(
        &ResolvedText {
            line_clamp: None,
            ..rtl.clone()
        },
        "שלום",
    );
    let glyphs = glyphs_of(&drawn(&mut ts, Paragraph::from("שלום\nעוד"), &rtl, Some(120.0)));
    let dots = ellipsis_glyph(&rtl);
    assert_eq!(glyphs.len(), word.len() + 1, "{glyphs:?}");
    let first = glyphs.iter().min_by_key(|g| g.1).expect("glyphs");
    assert_eq!(first.0.glyph, dots, "{glyphs:?}");
    assert!(first.1 <= 1, "at the left: {glyphs:?}");
}

#[test]
fn a_cut_keeps_or_drops_whole_grapheme_clusters() {
    // x + U+0301 (no precomposed form): a base glyph and a mark of no
    // advance, one grapheme. In an LTR run embedded in an RTL line, and
    // in an LTR line, cut at every width: never a mark without its base.
    let base = style(16.0, 24.0, 400);
    let pair = alone(&base, "x\u{301}");
    assert_eq!(pair.len(), 2, "a base and a mark: {pair:?}");
    let (xg, mark) = (pair[0].0.glyph, pair[1].0.glyph);
    let latin = "x\u{301}".repeat(12);
    for (direction, text) in [
        (Direction::Rtl, format!("שלום {latin} עולם")),
        (Direction::Ltr, format!("ab {latin} cd")),
    ] {
        let t = ResolvedText {
            direction,
            truncate: true,
            ..base.clone()
        };
        let mut ts = TextSystem::new();
        for width in 20..160 {
            #[expect(clippy::cast_precision_loss, clippy::as_conversions, reason = "a small width")]
            let w = width as f32;
            let glyphs = glyphs_of(&drawn(&mut ts, Paragraph::from(text.as_str()), &t, Some(w)));
            let bases = glyphs.iter().filter(|g| g.0.glyph == xg).count();
            let marks = glyphs.iter().filter(|g| g.0.glyph == mark).count();
            assert_eq!(bases, marks, "{direction:?} at {width}: {glyphs:?}");
        }
    }
}

#[test]
fn an_inline_box_the_cut_or_the_clamp_hides_does_not_show() {
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let slot = |at, id| InlineSlot {
        at,
        width: 30.0,
        height: 16.0,
        baseline: None,
        id,
    };
    // "ab [1] a long tail [2]\nnext [3]" wrapped in 80px: box 1 shows,
    // box 2 and box 3 are on lines the clamp hides.
    let content = "ab \u{FFFC} a long tail that runs on \u{FFFC}\nnext \u{FFFC}";
    let two = content.find("on \u{FFFC}").map_or(0, |i| i + 3);
    let three = content.rfind('\u{FFFC}').unwrap_or(0);
    let boxes = [slot(3, 1), slot(two, 2), slot(three, 3)];
    let p = Paragraph {
        content,
        runs: &[],
        boxes: &boxes,
    };
    let mut ts = TextSystem::new();
    let placed = ts.placed_inline_boxes(p, &t, Some(80.0));
    let mut shown: Vec<(u64, bool)> = placed.iter().map(|b| (b.id, b.shown)).collect();
    shown.sort_unstable();
    assert_eq!(shown, [(1, true), (2, false), (3, false)], "{placed:?}");
    // Unwrapped, the line has no soft wrap opportunity to give the
    // ellipsis room at: as much of its start as fits stays, by whole
    // units, box 1 among them.
    let nowrap = ResolvedText {
        nowrap: true,
        ..t.clone()
    };
    let placed = ts.placed_inline_boxes(p, &nowrap, Some(80.0));
    let mut shown: Vec<(u64, bool)> = placed.iter().map(|b| (b.id, b.shown)).collect();
    shown.sort_unstable();
    assert_eq!(shown, [(1, true), (2, false), (3, false)], "{placed:?}");
    // Truncated, the ellipsis hides what is visually past it: box 1 stays.
    let truncated = ResolvedText {
        line_clamp: None,
        truncate: true,
        ..t.clone()
    };
    let placed = ts.placed_inline_boxes(p, &truncated, Some(80.0));
    let mut shown: Vec<(u64, bool)> = placed.iter().map(|b| (b.id, b.shown)).collect();
    shown.sort_unstable();
    assert_eq!(shown, [(1, true), (2, false), (3, false)], "{placed:?}");
    // Not clamped, all show.
    let free = ResolvedText { line_clamp: None, ..t };
    assert!(ts.placed_inline_boxes(p, &free, Some(800.0)).iter().all(|b| b.shown));
}

/// The glyph ids of `glyphs`, sorted (what shows, wherever).
fn ids(glyphs: &[(GlyphKey, i32, i32)]) -> Vec<u32> {
    let mut v: Vec<u32> = glyphs.iter().map(|g| g.0.glyph).collect();
    v.sort_unstable();
    v
}

#[test]
fn a_ligature_whose_continuation_is_styled_otherwise_keeps_every_glyph() {
    // Parley groups a ligature under its first character's style. "fiX"
    // with only its i recoloured, and in RTL lam-alef-beh with only the
    // alef (the continuation, visually first) recoloured: every glyph
    // after the ligature draws, cut short or not.
    let base = style(16.0, 24.0, 400);
    let serif = ResolvedText {
        family: FontFamily::new(&[], crate::style::GenericFamily::Serif),
        ..base.clone()
    };
    let arabic = ResolvedText {
        direction: Direction::Rtl,
        ..base
    };
    let cases = [
        (serif, "fi", "fiX", 1..2),
        (arabic, "\u{644}\u{627}", "\u{644}\u{627}\u{628}", 2..4),
    ];
    for (t, ligature, text, recoloured) in cases {
        let lig = alone(&t, ligature);
        if lig.len() != 1 {
            eprintln!("no {ligature:?} ligature in this system's fonts: skipped");
            continue;
        }
        let red = ResolvedText {
            color: Color::hex(0xff_00_00),
            ..t.clone()
        };
        let runs = [run(recoloured, red)];
        let want = ids(&alone(&t, text));
        let mut ts = TextSystem::new();
        let truncated = ResolvedText {
            truncate: true,
            ..t.clone()
        };
        for (what, style, width) in [("free", &t, None), ("truncated", &truncated, Some(400.0))] {
            let got = glyphs_of(&drawn(&mut ts, Paragraph::new(text, &runs), style, width));
            assert!(
                got.iter().any(|g| g.0.glyph == lig[0].0.glyph),
                "{text:?} ligates: {got:?}"
            );
            assert_eq!(ids(&got), want, "{text:?} {what}: {got:?}");
            let all = decoration_rects(Paragraph::new(text, &runs), &style.clone().underlined(), 1.0);
            assert!(!all.is_empty(), "{text:?} {what}: underlined");
        }
    }
}

#[test]
fn an_inline_box_is_cut_as_a_unit_of_its_own_width() {
    let t = ResolvedText {
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        truncate: false,
        nowrap: true,
        ..t.clone()
    };
    let slot = |at| InlineSlot {
        at,
        width: 30.0,
        height: 16.0,
        baseline: None,
        id: 7,
    };
    let mut ts = TextSystem::new();
    let x = ts.measure("X", &plain, None).0;
    let dots_w = ts.measure(ELLIPSIS, &plain, None).0;
    let dots = ellipsis_glyph(&t);
    // "X[box]" where X alone fits: the box overflows, so the line ends in
    // an ellipsis and the box goes.
    let boxes = [slot(1)];
    let p = Paragraph {
        content: "X\u{FFFC}",
        runs: &[],
        boxes: &boxes,
    };
    let g = glyphs_of(&drawn(&mut ts, p, &t, Some(x + 1.0)));
    assert!(g.iter().any(|g| g.0.glyph == dots), "{g:?}");
    let placed = ts.placed_inline_boxes(p, &t, Some(x + 1.0));
    assert!(!placed[0].shown, "{placed:?}");
    // "[box]XXXXXXXX" where the box and the ellipsis fit and no X does:
    // the box shows, the ellipsis after it.
    let boxes = [slot(0)];
    let p = Paragraph {
        content: "\u{FFFC}XXXXXXXX",
        runs: &[],
        boxes: &boxes,
    };
    let w = 30.0 + dots_w + 1.0;
    let placed = ts.placed_inline_boxes(p, &t, Some(w));
    assert!(placed[0].shown, "{placed:?}");
    let g = glyphs_of(&drawn(&mut ts, p, &t, Some(w)));
    assert_eq!(ids(&g), [dots], "{g:?}");
    assert!(g[0].1 >= 29, "after the box: {g:?}");
}

#[test]
fn a_clamped_line_gives_its_ellipsis_room_at_a_soft_wrap_opportunity() {
    // css-overflow-4 block-ellipsis: "abc אבג דהו" fits its box but not
    // with the ellipsis, so content is displaced from its end as if it
    // wrapped: "דהו" goes (the logical end, though visually it is in the
    // middle of the line), and "abc אבג" is laid out as alone, the
    // ellipsis after it.
    let base = style(16.0, 24.0, 400);
    let t = ResolvedText {
        line_clamp: Some(1),
        ..base.clone()
    };
    let plain = ResolvedText { nowrap: true, ..base };
    let mut ts = TextSystem::new();
    let line = "abc \u{5d0}\u{5d1}\u{5d2} \u{5d3}\u{5d4}\u{5d5}";
    let full = ts.measure(line, &plain, None).0;
    let dots_w = ts.measure(ELLIPSIS, &plain, None).0;
    let content = format!("{line}\nnext");
    let mut got = glyphs_of(&drawn(
        &mut ts,
        Paragraph::from(content.as_str()),
        &t,
        Some(full + dots_w - 2.0),
    ));
    got.sort_by_key(|g| g.1);
    let kept = "abc \u{5d0}\u{5d1}\u{5d2}";
    let mut want = alone(&plain, kept);
    want.sort_by_key(|g| g.1);
    let dots = got.pop().expect("glyphs");
    assert_eq!(dots.0.glyph, ellipsis_glyph(&t), "the ellipsis last: {got:?}");
    let pairs: Vec<(u32, i32)> = got.iter().map(|g| (g.0.glyph, g.1)).collect();
    let alone_pairs: Vec<(u32, i32)> = want.iter().map(|g| (g.0.glyph, g.1)).collect();
    assert_eq!(pairs, alone_pairs, "kept as laid out alone");
    let kept_w = ts.measure(kept, &plain, None).0;
    assert!(
        (i32_to_f32(dots.1) - kept_w).abs() <= 1.0,
        "the ellipsis at {}, after {kept_w}",
        dots.1
    );
}

/// The width of `text` in `t` laid out on one line, as Parley has it
/// (not rounded to layout units, as [`TextSystem::measure`] rounds it).
fn exact_width(ts: &mut TextSystem, text: &str, t: &ResolvedText) -> f32 {
    let laid = ts.laid_out(Paragraph::from(text), t, None);
    ts.shaped[&laid[0].0.hash].layout.width()
}

#[test]
fn a_line_a_fraction_of_a_pixel_too_wide_is_cut() {
    // Monospace, so a character's advance is the same whatever follows.
    let t = ResolvedText {
        truncate: true,
        family: FontFamily::GEIST_MONO,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        truncate: false,
        nowrap: true,
        ..t.clone()
    };
    let mut ts = TextSystem::new();
    let text = "abcdef";
    let full = exact_width(&mut ts, text, &plain);
    let ab = exact_width(&mut ts, "ab", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let dots = ellipsis_glyph(&t);
    let shown = |ts: &mut TextSystem, w: f32| {
        let g = glyphs_of(&drawn(ts, Paragraph::from(text), &t, Some(w)));
        let cut = g.iter().any(|g| g.0.glyph == dots);
        (cut, g.len() - usize::from(cut))
    };
    assert_eq!(shown(&mut ts, full), (false, 6), "it fits its own width");
    let (cut, kept) = shown(&mut ts, full - 0.004);
    assert!(cut && kept < 6, "0.004px too wide: {kept} kept");
    // "ab…" a fraction of a pixel wider than the box: b goes too.
    assert_eq!(shown(&mut ts, ab + dots_w - 0.004), (true, 1));
    assert_eq!(shown(&mut ts, ab + dots_w + 0.004), (true, 2));
}

#[test]
fn a_clamped_line_with_no_soft_wrap_opportunity_that_fits_keeps_what_fits() {
    // No soft wrap opportunity leaves the ellipsis room: unwrapped words,
    // or one word longer than the box (broken only where overflow-wrap
    // allows). As much of the line's start as fits stays, by whole
    // grapheme clusters, before the ellipsis, not the ellipsis alone.
    let mono = ResolvedText {
        family: FontFamily::GEIST_MONO,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        nowrap: true,
        ..mono.clone()
    };
    let mut ts = TextSystem::new();
    let a = exact_width(&mut ts, "a", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let dots = ellipsis_glyph(&mono);
    let clamped = ResolvedText {
        line_clamp: Some(1),
        ..mono
    };
    let nowrap = ResolvedText {
        nowrap: true,
        ..clamped.clone()
    };
    let width = 8.5 * a + dots_w;
    for (what, t, content) in [
        ("unwrapped words", &nowrap, "ab cd ef gh ij kl\nnext"),
        ("one long word", &clamped, "abcdefghijklmnopqrstuvwxyz\nnext"),
    ] {
        let mut got = glyphs_of(&drawn(&mut ts, Paragraph::from(content), t, Some(width)));
        got.sort_by_key(|g| g.1);
        let last = got.pop().expect("glyphs");
        assert_eq!(last.0.glyph, dots, "{what}: ends in the ellipsis: {got:?}");
        // Eight cells fit before the ellipsis (a space draws a glyph too).
        assert_eq!(got.len(), 8, "{what}: {got:?}");
        let mut want = alone(&plain, &content[..8]);
        want.sort_by_key(|g| g.1);
        let ids_x = |g: &[(GlyphKey, i32, i32)]| g.iter().map(|g| (g.0.glyph, g.1)).collect::<Vec<_>>();
        assert_eq!(ids_x(&got), ids_x(&want), "{what}: the line's start, as alone");
    }
}

#[test]
fn a_clamp_keeps_no_padding_of_a_run_it_hides() {
    // "AA" then a span "BBB" padded 8px on its left: the padding is room
    // after the last A. Clamped where AA and the padding fit but no B
    // does, the B run goes and its padding with it: the ellipsis follows
    // AA, with no gap.
    let base = ResolvedText {
        family: FontFamily::GEIST_MONO,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        nowrap: true,
        ..base.clone()
    };
    let t = ResolvedText {
        nowrap: true,
        line_clamp: Some(1),
        ..base.clone()
    };
    let mut ts = TextSystem::new();
    let aa = exact_width(&mut ts, "AA", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let runs = [Run {
        padding: Edges {
            left: 8.0,
            ..Edges::default()
        },
        background: Some(Color::hex(0x3b_82_f6)),
        ..run(2..5, base)
    }];
    let p = Paragraph::new("AABBB\nnext", &runs);
    let items = drawn(&mut ts, p, &t, Some(aa + 8.0 + dots_w + 1.0));
    let mut glyphs = glyphs_of(&items);
    glyphs.sort_by_key(|g| g.1);
    assert_eq!(glyphs.len(), 3, "A, A and the ellipsis: {glyphs:?}");
    let dots = glyphs[2];
    assert_eq!(dots.0.glyph, ellipsis_glyph(&t));
    assert!(
        (i32_to_f32(dots.1) - aa).abs() <= 1.0,
        "the ellipsis at {}, right after AA ({aa})",
        dots.1
    );
    // No background: the run that has one is hidden.
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, Item::Rect { .. } | Item::RoundedRect { .. })),
        "{items:?}"
    );
}

#[test]
fn a_line_that_starts_with_spaces_counts_them_toward_its_width() {
    // "    X" in a box wide enough for X alone: the spaces it starts with are
    // content (they do not hang), so it overflows and ends in an ellipsis;
    // in RTL its start is its right.
    for direction in [Direction::Ltr, Direction::Rtl] {
        let t = ResolvedText {
            truncate: true,
            direction,
            ..style(16.0, 24.0, 400)
        };
        let plain = ResolvedText {
            truncate: false,
            nowrap: true,
            ..t.clone()
        };
        let mut ts = TextSystem::new();
        let text = if direction == Direction::Ltr {
            "    X"
        } else {
            "    \u{5d0}"
        };
        let alone_w = exact_width(&mut ts, text.trim_start(), &plain);
        let dots = ellipsis_glyph(&t);
        let g = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &t, Some(alone_w + 1.0)));
        assert!(g.iter().any(|g| g.0.glyph == dots), "{direction:?}: {g:?}");
    }
}

#[test]
fn a_clamped_lines_kept_text_is_laid_out_as_if_it_were_the_whole_line() {
    // "א-123456" in an LTR line: before the digits the hyphen resolves
    // RTL, so it sits left of the alef. Clamped to "א-" (no soft wrap
    // opportunity: unwrapped), the hyphen ends the line and resolves LTR:
    // right of the alef, as "א-" laid out alone has it.
    let base = style(16.0, 24.0, 400);
    let plain = ResolvedText { nowrap: true, ..base };
    let t = ResolvedText {
        line_clamp: Some(1),
        ..plain.clone()
    };
    let mut ts = TextSystem::new();
    let kept = "\u{5d0}-";
    let kept_w = exact_width(&mut ts, kept, &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let mut got = glyphs_of(&drawn(
        &mut ts,
        Paragraph::from("\u{5d0}-123456"),
        &t,
        Some(kept_w + dots_w + 1.0),
    ));
    got.sort_by_key(|g| g.1);
    let dots = got.pop().expect("glyphs");
    assert_eq!(dots.0.glyph, ellipsis_glyph(&t), "{got:?}");
    let mut want = alone(&plain, kept);
    want.sort_by_key(|g| g.1);
    let order = |g: &[(GlyphKey, i32, i32)]| g.iter().map(|g| (g.0.glyph, g.1)).collect::<Vec<_>>();
    assert_eq!(order(&got), order(&want), "laid out as alone");
}

#[test]
fn inline_boxes_are_found_at_once_however_many() {
    // 2,000 inline boxes on one line, truncated: drawing it and placing its
    // boxes finds each box a few times, never by searching the others.
    let n = 2_000_usize;
    let content: String = std::iter::repeat_n('\u{FFFC}', n).collect();
    let slots: Vec<InlineSlot> = (0..n)
        .map(|i| InlineSlot {
            at: i * '\u{FFFC}'.len_utf8(),
            width: 4.0,
            height: 4.0,
            baseline: None,
            id: u64::try_from(i).unwrap_or(0),
        })
        .collect();
    let p = Paragraph {
        content: &content,
        runs: &[],
        boxes: &slots,
    };
    let t = ResolvedText {
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let mut ts = TextSystem::new();
    BOX_LOOKS.with(|c| c.set(0));
    drawn(&mut ts, p, &t, Some(4000.0));
    let placed = ts.placed_inline_boxes(p, &t, Some(4000.0));
    assert_eq!(placed.len(), n);
    let looks = BOX_LOOKS.with(std::cell::Cell::get);
    let bound = 16 * u64::try_from(n).unwrap_or(0);
    assert!(looks <= bound, "{looks} box lookups for {n} boxes (at most {bound})");
}

#[test]
fn a_long_truncated_lines_runs_find_their_units_in_one_walk() {
    // Each character of a cut line read for a screen reader finds the unit
    // it is drawn in by one pointer over the units, not a search of them
    // all per character: units looked at stay linear in the line.
    let content = "ab cd ef gh ".repeat(400);
    let t = ResolvedText {
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let mut ts = TextSystem::new();
    UNIT_LOOKS.with(|c| c.set(0));
    let runs = ts.text_runs(&content, &t, Some(3000.0));
    let read: String = runs.iter().filter_map(|r| content.get(r.range.clone())).collect();
    assert_eq!(read, content, "the whole value stays readable");
    let shown = character_boxes(&runs).iter().filter(|b| b.2 > 0.0).count();
    assert!(shown > 300, "{shown} characters shown: a long kept part");
    let looks = UNIT_LOOKS.with(std::cell::Cell::get);
    assert!(looks > 0, "the walk ran");
    let bound = 2 * u64::try_from(content.len()).unwrap_or(0);
    assert!(
        looks <= bound,
        "{looks} units looked at for {} characters (at most {bound})",
        content.len()
    );
}

/// `runs` tell all of `content`, each at most 255 entries (AccessKit's
/// limit: its end a `u8` too), each entry a whole number of code points,
/// the bytes of each run its entries' sum, and each `b` found where it is.
fn check_runs(content: &str, runs: &[TextRun], what: &str) {
    let read: String = runs.iter().filter_map(|r| content.get(r.range.clone())).collect();
    assert_eq!(read, content, "{what}: the whole value");
    for r in runs {
        assert!(r.lengths.len() <= 255, "{what}: a run of {} entries", r.lengths.len());
        assert!(
            r.positions.len() == r.lengths.len() && r.widths.len() == r.lengths.len(),
            "{what}: its entries' lists agree"
        );
        let mut at = r.range.start;
        for &len in &r.lengths {
            at = at.saturating_add(usize::from(len));
            assert!(
                content.is_char_boundary(at),
                "{what}: a piece ends inside a code point at {at}"
            );
        }
        assert_eq!(at, r.range.end, "{what}: a run's lengths sum to its bytes");
    }
    for (i, _) in content.match_indices('b') {
        let r = runs.iter().find(|r| r.range.contains(&i)).expect("a run holds b");
        assert_eq!(r.byte_of(r.character_at(i)), i, "{what}: b found where it is");
    }
}

#[test]
fn a_letter_under_many_marks_keeps_every_offset_after_it_right() {
    // One grapheme of 261 bytes (a letter and 130 acute accents) is more
    // than a character's `u8` length holds: told in pieces cut between
    // code points, every byte after it is where it is, shown or hidden.
    let grapheme = format!("a{}", "\u{301}".repeat(130));
    let content = format!("x {grapheme}b\nline {grapheme}b");
    let mut ts = TextSystem::new();
    let runs = ts.text_runs(&content, &style(16.0, 24.0, 400), Some(400.0));
    check_runs(&content, &runs, "shown");
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    check_runs(
        &content,
        &ts.text_runs(&content, &t, Some(400.0)),
        "its line hidden by a clamp",
    );
}

#[test]
fn a_grapheme_of_more_pieces_than_a_run_holds_spans_runs() {
    // A letter under 33,000 marks, hidden by a clamp or shown: 259
    // pieces, more than one run's 255 entries; its run gives way to the
    // next inside it.
    let grapheme = format!("a{}", "\u{301}".repeat(33_000));
    let content = format!("x\n{grapheme}b");
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let mut ts = TextSystem::new();
    check_runs(&content, &ts.text_runs(&content, &t, Some(400.0)), "hidden");
    // And shown, laid out.
    let content = format!("x {grapheme}b");
    check_runs(
        &content,
        &ts.text_runs(&content, &style(16.0, 24.0, 400), Some(400.0)),
        "shown",
    );
}

#[test]
fn an_rtl_grapheme_split_across_runs_reads_its_rest_at_its_logical_end() {
    // 254 alefs fill a run but one; then a bet under 130 qamats (two
    // pieces), its second piece the next run's first: zero wide at the
    // bet's logical end, its left in RTL. Alone there, or before a gimel,
    // each character's box is where it is drawn, so a point finds it; in
    // an RTL paragraph, and as an RTL run inside an LTR one.
    let grapheme = format!("\u{5d1}{}", "\u{5b8}".repeat(130));
    for (direction, before, after) in [
        (Direction::Rtl, "", ""),
        (Direction::Rtl, "", "\u{5d2}"),
        (Direction::Ltr, "ab ", ""),
        (Direction::Ltr, "ab ", "\u{5d2}"),
    ] {
        let t = ResolvedText {
            direction,
            ..style(16.0, 24.0, 400)
        };
        let content = format!("{before}{}{grapheme}{after}", "\u{5d0}".repeat(254));
        let mut ts = TextSystem::new();
        let runs = ts.text_runs(&content, &t, Some(8000.0));
        check_runs(&content, &runs, "rtl");
        let bet = before.len() + 254 * 2;
        let mut drawn = |from: usize, to: usize| {
            let b = ts.selection(content.as_str(), &t, Some(8000.0), from, to);
            (b[0].x, b[0].x + b[0].width)
        };
        let (bet_left, bet_right) = drawn(bet, bet + grapheme.len());
        let boxes = character_boxes(&runs);
        let piece = boxes
            .iter()
            .find(|b| b.0 > bet && b.0 < bet + grapheme.len())
            .unwrap_or_else(|| panic!("its second piece: {boxes:?}"));
        assert!(
            (piece.1 - bet_left).abs() < 0.5,
            "{direction:?}, after {after:?}: its rest at its logical end {bet_left}, not {} (its right {bet_right})",
            piece.1
        );
        if !after.is_empty() {
            let at = bet + grapheme.len();
            let (left, right) = drawn(at, content.len());
            let gimel = boxes.iter().find(|b| b.0 == at).expect("the gimel");
            assert!(
                (gimel.1 - left).abs() < 0.5 && (gimel.1 + gimel.2 - right).abs() < 0.5,
                "the gimel's box {gimel:?} where it is drawn ({left}..{right})"
            );
        }
    }
}

#[test]
fn a_full_runs_line_break_starts_a_run_of_its_own() {
    // A line of 255 letters, then a line break: the break is an entry
    // too, in a run of its own on the same line, past the full one.
    let content = format!("{}\nb", "a".repeat(255));
    let mut ts = TextSystem::new();
    let runs = ts.text_runs(&content, &style(16.0, 24.0, 400), Some(8000.0));
    check_runs(&content, &runs, "shown");
    let broken = runs.iter().find(|r| r.range.end == 256).expect("a run holds the break");
    assert_eq!(broken.line, runs[0].line, "the break on its line");
    assert_eq!(broken.lengths, [1], "the break alone");
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let content = format!("x\n{}\nb", "a".repeat(255));
    check_runs(&content, &ts.text_runs(&content, &t, Some(8000.0)), "hidden");
}

#[test]
fn float_noise_never_reaches_a_visible_width() {
    for w in [0.0, 1.0, 100.0, 2_048.0, 16_777_216.0, f32::MAX] {
        assert!(noise(w) <= 1.0 / 1024.0, "{w}: {}", noise(w));
    }
    assert!(noise(100.0) > 0.0);
}

#[test]
fn an_rtl_ligature_at_the_cut_is_kept_or_dropped_whole() {
    // RTL "ب لا ببببب" truncated where "ب لا" and the ellipsis just fit:
    // the lam-alef ligature stays; half a pixel narrower, it goes whole
    // (its glyph with both its clusters), and "ب" stays.
    let t = ResolvedText {
        direction: Direction::Rtl,
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        truncate: false,
        nowrap: true,
        ..t.clone()
    };
    let lig = alone(&plain, "\u{644}\u{627}");
    if lig.len() != 1 {
        eprintln!("no lam-alef ligature in this system's fonts: skipped");
        return;
    }
    let lig = lig[0].0.glyph;
    let beh = alone(&plain, "\u{628}")[0].0.glyph;
    let mut ts = TextSystem::new();
    let kept_w = exact_width(&mut ts, "\u{628} \u{644}\u{627}", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let text = "\u{628} \u{644}\u{627} \u{628}\u{628}\u{628}\u{628}\u{628}";
    let fits = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &t, Some(kept_w + dots_w + 0.5)));
    assert!(fits.iter().any(|g| g.0.glyph == lig), "kept: {fits:?}");
    let short = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &t, Some(kept_w + dots_w - 0.5)));
    assert!(!short.iter().any(|g| g.0.glyph == lig), "dropped: {short:?}");
    assert!(short.iter().any(|g| g.0.glyph == beh), "the first beh stays: {short:?}");
}

#[test]
fn a_truncated_line_keeps_no_padding_of_a_run_it_drops() {
    // LTR: "AA" then a span "BBB" padded 8px on its left, underlined;
    // truncated where AA and the padding fit but no B does. The B run
    // goes and its padding with it: the ellipsis and the underline end
    // at AA.
    let base = ResolvedText {
        family: FontFamily::GEIST_MONO,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        nowrap: true,
        ..base.clone()
    };
    let t = ResolvedText {
        truncate: true,
        ..base.clone()
    }
    .underlined();
    let mut ts = TextSystem::new();
    let aa = exact_width(&mut ts, "AA", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let padded = |range: Range<usize>, text: ResolvedText| Run {
        padding: Edges {
            left: 8.0,
            ..Edges::default()
        },
        ..run(range, text)
    };
    let runs = [padded(2..5, base.clone())];
    let items = drawn(
        &mut ts,
        Paragraph::new("AABBB", &runs),
        &t,
        Some(aa + 8.0 + dots_w + 1.0),
    );
    let mut glyphs = glyphs_of(&items);
    glyphs.sort_by_key(|g| g.1);
    assert_eq!(glyphs.len(), 3, "A, A and the ellipsis: {glyphs:?}");
    assert_eq!(glyphs[2].0.glyph, ellipsis_glyph(&t));
    assert!(
        (i32_to_f32(glyphs[2].1) - aa).abs() <= 1.0,
        "the ellipsis at {}, right after AA ({aa})",
        glyphs[2].1
    );
    let right = items
        .iter()
        .filter_map(|i| match i {
            Item::Rect { bounds, .. } => Some(bounds.x + bounds.width),
            _ => None,
        })
        .fold(0.0_f32, f32::max);
    assert!(right <= aa + 1.0, "the underline ends at {right}, past AA ({aa})");
    // RTL: "אב" then a span "גד" padded on its left (Parley puts the room
    // after ב, on its right): truncated where ב and the padding fit but
    // no ג does, א and ב sit as "אב" alone has them.
    let rtl = ResolvedText {
        direction: Direction::Rtl,
        truncate: true,
        ..base
    };
    let rtl_plain = ResolvedText {
        truncate: false,
        nowrap: true,
        ..rtl.clone()
    };
    let ab = exact_width(&mut ts, "\u{5d0}\u{5d1}", &rtl_plain);
    let runs = [padded(4..8, rtl_plain.clone())];
    let text = "\u{5d0}\u{5d1}\u{5d2}\u{5d3}";
    let mut got = glyphs_of(&drawn(
        &mut ts,
        Paragraph::new(text, &runs),
        &rtl,
        Some(ab + 8.0 + dots_w + 1.0),
    ));
    got.sort_by_key(|g| g.1);
    assert_eq!(got.len(), 3, "the ellipsis, ב and א: {got:?}");
    assert_eq!(got[0].0.glyph, ellipsis_glyph(&rtl));
    let mut want = alone(&rtl_plain, "\u{5d0}\u{5d1}");
    want.sort_by_key(|g| g.1);
    // In quarter pixels (each glyph's whole pixel and quarter past it).
    let at = |g: &(GlyphKey, i32, i32)| g.1 * 4 + i32::from(g.0.bin);
    let gap = |g: &[(GlyphKey, i32, i32)]| at(&g[g.len() - 1]) - at(&g[g.len() - 2]);
    assert!(
        (gap(&got) - gap(&want)).abs() <= 4,
        "ב to א as alone (quarter pixels): {got:?} {want:?}"
    );
}

#[test]
fn a_cut_fits_its_text_as_if_a_dropped_runs_padding_were_not_there() {
    // "AA" then a span "BBB" padded 8px on its left (room after the last
    // A). With room for AA and the ellipsis and a pixel more, not for the
    // padding: the B run goes and its padding with it, so both A stay,
    // truncated or clamped (no soft wrap opportunity: the logical prefix).
    let base = ResolvedText {
        family: FontFamily::GEIST_MONO,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        nowrap: true,
        ..base.clone()
    };
    let mut ts = TextSystem::new();
    let aa = exact_width(&mut ts, "AA", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let runs = [Run {
        padding: Edges {
            left: 8.0,
            ..Edges::default()
        },
        ..run(2..5, base.clone())
    }];
    let truncated = ResolvedText { truncate: true, ..base };
    let clamped = ResolvedText {
        line_clamp: Some(1),
        ..plain.clone()
    };
    for (what, t) in [("truncated", truncated), ("clamped", clamped)] {
        let mut g = glyphs_of(&drawn(
            &mut ts,
            Paragraph::new("AABBB", &runs),
            &t,
            Some(aa + dots_w + 1.0),
        ));
        g.sort_by_key(|g| g.1);
        assert_eq!(g.len(), 3, "{what}: A, A and the ellipsis: {g:?}");
        assert_eq!(g[2].0.glyph, ellipsis_glyph(&t), "{what}");
    }
}

#[test]
fn a_truncated_line_drops_a_dropped_runs_padding_wherever_bidi_puts_it() {
    // LTR "AאבZZZ", "ZZZ" padded 8px on its left: the padding is room after
    // ב, which bidi puts between ב and א ("A" "בא" "ZZZ" left to right).
    // Truncated to "Aאב", that room goes: A, ב and א sit as "Aאב" alone has
    // them.
    let base = style(16.0, 24.0, 400);
    let plain = ResolvedText {
        nowrap: true,
        ..base.clone()
    };
    let t = ResolvedText {
        truncate: true,
        ..base.clone()
    };
    let mut ts = TextSystem::new();
    let kept = "A\u{5d0}\u{5d1}";
    let kept_w = exact_width(&mut ts, kept, &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let runs = [Run {
        padding: Edges {
            left: 8.0,
            ..Edges::default()
        },
        ..run(5..8, base)
    }];
    let mut got = glyphs_of(&drawn(
        &mut ts,
        Paragraph::new("A\u{5d0}\u{5d1}ZZZ", &runs),
        &t,
        Some(kept_w + 8.0 + dots_w + 1.0),
    ));
    got.sort_by_key(|g| g.1);
    let dots = got.pop().expect("glyphs");
    assert_eq!(dots.0.glyph, ellipsis_glyph(&t), "{got:?}");
    let mut want = alone(&plain, kept);
    want.sort_by_key(|g| g.1);
    assert_eq!(got.len(), want.len(), "{got:?} {want:?}");
    // In quarter pixels, from the first glyph.
    let at = |g: &(GlyphKey, i32, i32)| g.1 * 4 + i32::from(g.0.bin);
    let offsets = |g: &[(GlyphKey, i32, i32)]| g.iter().map(|x| (x.0.glyph, at(x) - at(&g[0]))).collect::<Vec<_>>();
    for (a, b) in offsets(&got).iter().zip(offsets(&want)) {
        assert!(a.0 == b.0 && (a.1 - b.1).abs() <= 4, "as alone: {got:?} {want:?}");
    }
    assert!(
        at(&dots) - at(&got[got.len() - 1]) <= at(&want[want.len() - 1]) - at(&want[0]) + 4 * 16,
        "{dots:?}"
    );
}

#[test]
fn float_noise_is_four_ulps_below_its_cap() {
    for w in [1.0_f32, 1.75, 100.0, 2_048.0] {
        assert_eq!(noise(w), 4.0 * (w.next_up() - w), "{w}");
    }
    // What a cut keeps uses the same tolerance: a unit past its end by
    // more than float noise (and less than 1/1024 px) is not kept.
    let cut = Cut {
        piece: 0,
        line: 0,
        keep: Keep::Visual { from: 0.0, to: 10.0 },
        kept: 10.0,
        dx: 0.0,
        ellipsis_at: 10.0,
        ellipsis: 0.0,
        probe: None,
        prefix: None,
    };
    assert!(cut.keeps(9.0, 1.0, (0, 1)));
    assert!(!cut.keeps(9.0, 1.0 + 1.0 / 2048.0, (0, 1)));
}

#[test]
fn pieces_shaped_apart_are_bounded_by_count_as_well_as_age() {
    // 600 clamped paragraphs drawn in one frame shape a probe and a kept
    // prefix each, apart, the first ten's pinned (a retained chunk shows
    // them). The frame's end keeps all it used (the bound follows the
    // working set); the frame after, that used none, no more than
    // APART_MAX stay, the pinned ones among them. All pinned (retained
    // chunks show them all), they stay; let go, they are bounded again,
    // and shaped again when next drawn, as before.
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let mut ts = TextSystem::new();
    let apart = |ts: &TextSystem| ts.shaped.values().filter(|s| s.key.apart != Apart::No).count();
    let text = |i: usize| format!("row {i} has words enough to wrap twice over");
    let mut first = Vec::new();
    for i in 0..600 {
        let items = drawn(&mut ts, Paragraph::from(text(i).as_str()), &t, Some(120.0));
        if i == 0 {
            first = glyphs_of(&items);
        }
        if i == 9 {
            let pins = ts.take_drawn();
            ts.pin(&pins);
        }
    }
    let pinned: Vec<u64> = ts
        .pinned
        .keys()
        .copied()
        .filter(|h| ts.shaped.get(h).is_some_and(|s| s.key.apart != Apart::No))
        .collect();
    assert_eq!(pinned.len(), 20, "a probe and a prefix for each of ten");
    assert_eq!(apart(&ts), 1200, "a probe and a prefix each");
    ts.end_frame();
    assert_eq!(apart(&ts), 1200, "what the frame used stays");
    ts.end_frame();
    assert!(apart(&ts) <= APART_MAX, "{} pieces apart", apart(&ts));
    assert!(pinned.iter().all(|h| ts.shaped.contains_key(h)), "the pinned stay");
    // Every piece pinned: the working set, kept; unpinned, bounded.
    ts.unpin(&ts.pinned.keys().copied().collect::<Vec<_>>());
    ts.take_drawn();
    for i in 0..600 {
        drawn(&mut ts, Paragraph::from(text(i).as_str()), &t, Some(120.0));
    }
    let all = ts.take_drawn();
    ts.pin(&all);
    ts.end_frame();
    ts.end_frame();
    assert_eq!(apart(&ts), 1200, "what retained chunks show stays");
    ts.unpin(&all);
    ts.end_frame();
    assert!(apart(&ts) <= APART_MAX, "{} pieces apart, let go", apart(&ts));
    let again = glyphs_of(&drawn(&mut ts, Paragraph::from(text(0).as_str()), &t, Some(120.0)));
    assert_eq!(again, first, "drawn again as before");
    // Two halves drawn by turns, a frame each (rows re-recorded in two
    // chunks): the bound follows the working set, so neither half is let
    // go for the other, and nothing is shaped again.
    let mut ts = TextSystem::new();
    for _ in 0..2 {
        for half in [0..300, 300..600] {
            for i in half {
                drawn(&mut ts, Paragraph::from(text(i).as_str()), &t, Some(120.0));
            }
            ts.end_frame();
        }
    }
    let warm = ts.shapes;
    for half in [0..300, 300..600] {
        for i in half {
            drawn(&mut ts, Paragraph::from(text(i).as_str()), &t, Some(120.0));
        }
        ts.end_frame();
    }
    assert_eq!(
        ts.shapes, warm,
        "a working set of 1200 pieces apart shapes nothing again"
    );
}

#[test]
fn a_cut_lines_hit_caret_and_selection_follow_what_it_shows() {
    // "א-123456" clamped to "א-": the hyphen, drawn right of the alef, is
    // selected, hit and stood at there; the digits it hides have no box,
    // and no point on the line hits into them; below it, the text's end
    // is hit (a drag past it selects to the end, as Chrome's).
    let base = style(16.0, 24.0, 400);
    let plain = ResolvedText { nowrap: true, ..base };
    let t = ResolvedText {
        line_clamp: Some(1),
        ..plain.clone()
    };
    let mut ts = TextSystem::new();
    let text = "\u{5d0}-123456";
    let kept_w = exact_width(&mut ts, "\u{5d0}-", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let w = Some(kept_w + dots_w + 1.0);
    let alef = ts.selection(Paragraph::from(text), &t, w, 0, 2);
    let hyphen = ts.selection(Paragraph::from(text), &t, w, 2, 3);
    assert_eq!((alef.len(), hyphen.len()), (1, 1), "{alef:?} {hyphen:?}");
    let (alef, hyphen) = (alef[0], hyphen[0]);
    assert!(
        hyphen.x >= alef.x + alef.width - 0.5,
        "the hyphen right of the alef: {alef:?} {hyphen:?}"
    );
    assert!(ts.selection(Paragraph::from(text), &t, w, 3, text.len()).is_empty());
    let mid = |b: Bounds| b.x + b.width / 2.0;
    let at = ts.hit(text, &t, w, mid(hyphen), 12.0);
    assert!(at == 2 || at == 3, "the hyphen hit: {at}");
    let at = ts.hit(text, &t, w, mid(alef), 12.0);
    assert!(at <= 2, "the alef hit: {at}");
    for x in [0.0, 4.0, kept_w - 1.0] {
        let at = ts.hit(text, &t, w, x, 12.0);
        assert!(at <= 3, "({x}, 12) hits {at}, in the hidden digits");
    }
    // Over the ellipsis or past it: the end, as a screen reader finds.
    for x in [kept_w + 1.0, kept_w + dots_w - 1.0, 400.0] {
        assert_eq!(ts.hit(text, &t, w, x, 12.0), text.len(), "({x}, 12): the end");
    }
    for x in [0.0, kept_w, kept_w + dots_w, 400.0] {
        assert_eq!(ts.hit(text, &t, w, x, 40.0), text.len(), "({x}, 40): the end");
    }
    let c = ts.caret(text, &t, w, 3);
    assert!(
        (c.x - (hyphen.x + hyphen.width)).abs() <= 1.0,
        "after the hyphen: {c:?} {hyphen:?}"
    );
    let hidden = ts.caret(text, &t, w, 6);
    assert!(
        (hidden.x - c.x).abs() <= 1.0,
        "a hidden offset at the kept end: {hidden:?} {c:?}"
    );
    // Truncated: past what it keeps is its end, not the hidden text.
    let t = ResolvedText {
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        truncate: false,
        nowrap: true,
        ..t.clone()
    };
    let w = Some(exact_width(&mut ts, "abc", &plain) + dots_w + 0.5);
    assert_eq!(ts.hit("abcdefgh", &t, w, 400.0, 12.0), 8, "past the ellipsis: the end");
    assert_eq!(ts.hit("abcdefgh", &t, w, 2.0, 30.0), 8, "below the line: the end");
    assert_eq!(
        ts.hit("abcdefgh\nmore", &t, w, 2.0, 30.0),
        13,
        "on a hidden line: the end"
    );
    assert!(ts.selection(Paragraph::from("abcdefgh"), &t, w, 3, 8).is_empty());
    let end = ts.caret("abcdefgh", &t, w, 3).x;
    assert!((ts.caret("abcdefgh", &t, w, 6).x - end).abs() <= 0.5);
}

#[test]
fn a_truncation_drops_the_padding_of_every_run_it_drops() {
    // Isolates nested by turns, "a" RLI "א" LRI "b" RLI "ב" ...: each
    // Latin letter shows left of every Hebrew one ("abcde" then the Hebrew),
    // while in the text each Hebrew letter follows a Latin one. Each
    // Hebrew letter, with the isolate it opens (which draws nothing), is a
    // run padded 8px on its left: room after the Latin letter before it. Truncated to "abcde", all
    // five runs go and so does all their padding: the Latin letters sit as
    // "abcde" alone has them, the ellipsis right after.
    let base = style(16.0, 24.0, 400);
    let plain = ResolvedText {
        nowrap: true,
        ..base.clone()
    };
    let t = ResolvedText {
        truncate: true,
        ..base.clone()
    };
    let latin = ['a', 'b', 'c', 'd', 'e'];
    let hebrew = ['\u{5d0}', '\u{5d1}', '\u{5d2}', '\u{5d3}', '\u{5d4}'];
    let mut text = String::new();
    let mut runs = Vec::new();
    for (i, (l, h)) in latin.iter().zip(hebrew).enumerate() {
        if i > 0 {
            text.push('\u{2066}');
        }
        text.push(*l);
        let start = text.len();
        text.push('\u{2067}');
        text.push(h);
        runs.push(Run {
            padding: Edges {
                left: 8.0,
                ..Edges::default()
            },
            ..run(start..text.len(), base.clone())
        });
    }
    text.extend(std::iter::repeat_n('\u{2069}', 9));
    let mut ts = TextSystem::new();
    let kept_w = exact_width(&mut ts, "abcde", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let mut got = glyphs_of(&drawn(
        &mut ts,
        Paragraph::new(&text, &runs),
        &t,
        Some(kept_w + dots_w + 1.0),
    ));
    let mut want = alone(&plain, "abcde");
    want.sort_by_key(|g| g.1);
    let dots = *got
        .iter()
        .find(|g| g.0.glyph == ellipsis_glyph(&t))
        .expect("an ellipsis");
    let hebrew_ids = ids(&alone(&plain, "\u{5d0}\u{5d1}\u{5d2}\u{5d3}\u{5d4}"));
    assert!(
        !got.iter().any(|g| hebrew_ids.contains(&g.0.glyph)),
        "no Hebrew: {got:?}"
    );
    // The isolates' marks draw nothing; the letters are compared.
    let latin_ids = ids(&want);
    got.retain(|g| latin_ids.contains(&g.0.glyph));
    got.sort_by_key(|g| g.1);
    let at = |g: &(GlyphKey, i32, i32)| g.1 * 4 + i32::from(g.0.bin);
    let ids_at = |g: &[(GlyphKey, i32, i32)]| g.iter().map(|x| (x.0.glyph, at(x))).collect::<Vec<_>>();
    assert_eq!(ids_at(&got).len(), 5, "{got:?}");
    for (a, b) in ids_at(&got).iter().zip(ids_at(&want)) {
        assert!(a.0 == b.0 && (a.1 - b.1).abs() <= 4, "as alone: {got:?} {want:?}");
    }
    assert!(
        (i32_to_f32(dots.1) - kept_w).abs() <= 1.0,
        "the ellipsis at {}, right after abcde ({kept_w})",
        dots.1
    );
}

#[test]
fn a_letter_of_a_kept_ligature_is_selected_by_its_share() {
    // "fi" shown as one ligature on a truncated line: selecting the "i"
    // alone covers the ligature's right part, selecting the "f" its left.
    let t = ResolvedText {
        truncate: true,
        family: FontFamily::new(&[], crate::style::GenericFamily::Serif),
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        truncate: false,
        nowrap: true,
        ..t.clone()
    };
    if alone(&plain, "fi").len() != 1 {
        eprintln!("no fi ligature in this system's fonts: skipped");
        return;
    }
    let mut ts = TextSystem::new();
    let text = "fi and more words than fit";
    let fi = exact_width(&mut ts, "fi", &plain);
    let w = Some(exact_width(&mut ts, "fi and", &plain) + exact_width(&mut ts, ELLIPSIS, &plain) + 0.5);
    let i = ts.selection(Paragraph::from(text), &t, w, 1, 2);
    let f = ts.selection(Paragraph::from(text), &t, w, 0, 1);
    assert_eq!((i.len(), f.len()), (1, 1), "{i:?} {f:?}");
    assert!(i[0].width > 1.0 && i[0].width < fi, "{i:?} of {fi}");
    assert!(
        f[0].x < i[0].x && f[0].x + f[0].width <= i[0].x + 0.5,
        "{f:?} then {i:?}"
    );
}

#[test]
fn a_clamped_prefix_shaped_alone_still_leaves_its_ellipsis_room() {
    // Arabic letters join: in "سسسس…" each seen is medial, but the last a
    // clamp keeps ends its word, final, and so wider. However wide the box,
    // the kept letters as shaped alone and the ellipsis fit it: the
    // ellipsis, at the left of an RTL line, never starts before it.
    let t = ResolvedText {
        direction: Direction::Rtl,
        nowrap: true,
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let mut ts = TextSystem::new();
    let text = "\u{633}".repeat(24);
    let dots = ellipsis_glyph(&t);
    let mut w = 20.0;
    while w < 160.0 {
        let g = glyphs_of(&drawn(&mut ts, Paragraph::from(text.as_str()), &t, Some(w)));
        let e = g.iter().find(|g| g.0.glyph == dots).expect("an ellipsis");
        assert!(
            e.1 * 4 + i32::from(e.0.bin) >= 0,
            "at {w}px the ellipsis starts at {e:?}, before the box"
        );
        w += 0.25;
    }
}

#[test]
fn one_text_drawn_by_turns_at_two_widths_draws_as_if_alone() {
    // One text, clamped to two lines, drawn by turns at 150px left aligned
    // and 190px centred: each draw (its breaking swapped back, not broken
    // again) puts every glyph where a system that only drew it would.
    let text = "the same words in every row ".repeat(12);
    let looks = [(150.0, TextAlign::Left), (190.0, TextAlign::Center)];
    let style_of = |align: TextAlign| ResolvedText {
        line_clamp: Some(2),
        align,
        ..style(16.0, 24.0, 400)
    };
    let mut ts = TextSystem::new();
    for _ in 0..3 {
        for (w, align) in looks {
            let t = style_of(align);
            let got = glyphs_of(&drawn(&mut ts, Paragraph::from(text.as_str()), &t, Some(w)));
            let want = glyphs_of(&drawn(
                &mut TextSystem::new(),
                Paragraph::from(text.as_str()),
                &t,
                Some(w),
            ));
            assert_eq!(got, want, "at {w}px {align:?}");
        }
    }
}

/// Each character of `runs` (AccessKit's view): its first byte, and its
/// box's x, width and top.
fn character_boxes(runs: &[TextRun]) -> Vec<(usize, f32, f32, f32)> {
    let mut out = Vec::new();
    for r in runs {
        let mut at = r.range.start;
        for ((len, pos), w) in r.lengths.iter().zip(&r.positions).zip(&r.widths) {
            let x = if r.rtl {
                r.bounds.x + r.bounds.width - pos - w
            } else {
                r.bounds.x + pos
            };
            out.push((at, x, *w, r.bounds.y));
            at = at.saturating_add(usize::from(*len));
        }
    }
    out
}

#[test]
fn a_screen_reader_finds_a_cut_lines_text_where_it_shows() {
    // "א-123456" clamped to "א-": the hyphen's box is where it is drawn,
    // right of the alef; the hidden digits are still read (the runs cover
    // the whole text) but are zero wide at the ellipsis.
    let base = style(16.0, 24.0, 400);
    let plain = ResolvedText { nowrap: true, ..base };
    let t = ResolvedText {
        line_clamp: Some(1),
        ..plain.clone()
    };
    let mut ts = TextSystem::new();
    let text = "\u{5d0}-123456";
    let kept_w = exact_width(&mut ts, "\u{5d0}-", &plain);
    let dots_w = exact_width(&mut ts, ELLIPSIS, &plain);
    let w = Some(kept_w + dots_w + 1.0);
    let runs = ts.text_runs(text, &t, w);
    let read: String = runs.iter().filter_map(|r| text.get(r.range.clone())).collect();
    assert_eq!(read, text, "the whole text is read");
    let boxes = character_boxes(&runs);
    let hyphen = ts.selection(Paragraph::from(text), &t, w, 2, 3)[0];
    let h = boxes.iter().find(|b| b.0 == 2).expect("the hyphen");
    assert!(
        (h.1 - hyphen.x).abs() <= 0.5 && (h.2 - hyphen.width).abs() <= 0.5,
        "{h:?} drawn at {hyphen:?}"
    );
    let end = ts.caret(text, &t, w, 3).x;
    for b in boxes.iter().filter(|b| b.0 >= 3) {
        assert!(
            b.2 == 0.0 && (b.1 - end).abs() <= 0.5,
            "hidden {b:?}, at the ellipsis ({end})"
        );
    }
    // Wrapped and clamped to one line: the hidden lines' text sits on the
    // shown line, zero wide, not on rows that are not drawn.
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let text = "one two three four five six seven eight";
    let runs = ts.text_runs(text, &t, Some(90.0));
    let read: String = runs.iter().filter_map(|r| text.get(r.range.clone())).collect();
    assert_eq!(read, text);
    for b in character_boxes(&runs) {
        assert!(b.3 < 24.0, "{b:?} on a hidden row");
    }
    let shown = ts.selection(Paragraph::from(text), &t, Some(90.0), 0, text.len());
    let right = shown.iter().map(|b| b.x + b.width).fold(0.0_f32, f32::max);
    for b in character_boxes(&runs) {
        assert!(b.1 + b.2 <= right + 0.5 + dots_w, "{b:?} past what shows ({right})");
    }
    // Truncated: what is kept as drawn, the rest zero wide at the ellipsis.
    let t = ResolvedText {
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let w = Some(exact_width(&mut ts, "abc", &plain) + dots_w + 0.5);
    let boxes = character_boxes(&ts.text_runs("abcdefgh", &t, w));
    let end = ts.caret("abcdefgh", &t, w, 3).x;
    for b in &boxes {
        let drawn = ts.selection(Paragraph::from("abcdefgh"), &t, w, b.0, b.0 + 1);
        match drawn.first() {
            Some(d) if b.0 < 3 => assert!((b.1 - d.x).abs() <= 0.5 && (b.2 - d.width).abs() <= 0.5, "{b:?} {d:?}"),
            _ => assert!(b.0 >= 3 && b.2 == 0.0 && (b.1 - end).abs() <= 0.5, "{b:?} at {end}"),
        }
    }
}

#[test]
fn a_clamp_searches_its_prefix_in_bounded_shapes_and_keeps_only_the_one_it_shows() {
    // A line of four-letter Arabic words clamped (no soft wrap: nowrap),
    // short and very long, at every width: the prefix that fits shaped
    // alone is found shaping one word's partial prefixes and a check or
    // two of the whole, however long the line, and is the one piece
    // shaped apart that stays.
    let t = ResolvedText {
        direction: Direction::Rtl,
        nowrap: true,
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    for n in [16, 1024] {
        let text = vec!["\u{633}\u{644}\u{627}\u{645}"; n].join(" ");
        let mut w = 20.0;
        while w < 200.0 {
            let mut ts = TextSystem::new();
            drawn(&mut ts, Paragraph::from(""), &t, Some(w));
            let before = ts.shapes;
            drawn(&mut ts, Paragraph::from(text.as_str()), &t, Some(w));
            let shapes = ts.shapes - before;
            // The paragraph, the ellipsis, a word's partial prefixes (four
            // letters), and the whole checked: a few at most.
            assert!(shapes <= 2 + 4 + 3, "{n} words at {w}px: {shapes} shapes");
            let apart = ts.shaped.values().filter(|s| s.key.apart != Apart::No).count();
            assert_eq!(apart, 1, "at {w}px: the prefix shown alone");
            let again = ts.shapes;
            drawn(&mut ts, Paragraph::from(text.as_str()), &t, Some(w));
            assert_eq!(ts.shapes, again, "at {w}px drawn again: no shape");
            w += 2.5;
        }
    }
}

#[test]
fn a_clamp_tells_which_prefixes_end_a_word_in_one_walk() {
    // A very long unbreakable line clamped: whether each prefix ends a
    // word is met by one pointer over the line's word boundaries, not a
    // search of them per prefix: boundaries looked at stay linear.
    let t = ResolvedText {
        nowrap: true,
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let text = "ab cd ".repeat(2000);
    let mut ts = TextSystem::new();
    BOUND_LOOKS.with(|c| c.set(0));
    drawn(&mut ts, Paragraph::from(text.as_str()), &t, Some(100.0));
    let looks = BOUND_LOOKS.with(std::cell::Cell::get);
    assert!(looks > 0, "the clamp walked its prefixes");
    let bound = 2 * u64::try_from(text.len()).unwrap_or(0);
    assert!(
        looks <= bound,
        "{looks} boundaries looked at for {} bytes (at most {bound})",
        text.len()
    );
}

#[test]
fn a_letter_of_a_kept_rtl_ligature_is_selected_on_its_side() {
    // "لا" shown as one lam-alef ligature on a truncated RTL line: the lam
    // comes first, so on the right; selecting it alone covers the right
    // part of the ligature, the alef alone the left part.
    let t = ResolvedText {
        direction: Direction::Rtl,
        truncate: true,
        ..style(16.0, 24.0, 400)
    };
    let plain = ResolvedText {
        truncate: false,
        nowrap: true,
        ..t.clone()
    };
    if alone(&plain, "\u{644}\u{627}").len() != 1 {
        eprintln!("no lam-alef ligature in this system's fonts: skipped");
        return;
    }
    let mut ts = TextSystem::new();
    let text = "\u{644}\u{627} \u{628}\u{628}\u{628} \u{628}\u{628}\u{628}\u{628}\u{628}\u{628}";
    let lig = exact_width(&mut ts, "\u{644}\u{627}", &plain);
    let w = Some(
        exact_width(&mut ts, "\u{644}\u{627} \u{628}\u{628}\u{628}", &plain)
            + exact_width(&mut ts, ELLIPSIS, &plain)
            + 0.5,
    );
    let lam = ts.selection(Paragraph::from(text), &t, w, 0, 2);
    let alef = ts.selection(Paragraph::from(text), &t, w, 2, 4);
    let both = ts.selection(Paragraph::from(text), &t, w, 0, 4);
    assert_eq!(
        (lam.len(), alef.len(), both.len()),
        (1, 1, 1),
        "{lam:?} {alef:?} {both:?}"
    );
    let (lam, alef, both) = (lam[0], alef[0], both[0]);
    assert!(
        lam.width > 1.0 && lam.width < lig && alef.width > 1.0 && alef.width < lig,
        "{lam:?} {alef:?} of {lig}"
    );
    assert!(
        (lam.x + lam.width - (both.x + both.width)).abs() <= 0.5,
        "the lam at the right: {lam:?} in {both:?}"
    );
    assert!(
        (alef.x - both.x).abs() <= 0.5,
        "the alef at the left: {alef:?} in {both:?}"
    );
}

#[test]
fn text_being_edited_is_never_cut_and_its_caret_shows_where_it_is() {
    // A textarea clamped to two lines, being edited: no ellipsis, and the
    // caret at every offset stands at its character, on the line that
    // holds it (none at a hidden ellipsis), as the field's keys move it.
    let clamped = ResolvedText {
        line_clamp: Some(2),
        ..style(16.0, 24.0, 400)
    };
    let t = clamped.clone().editing();
    let mut ts = TextSystem::new();
    let text = "one two three four five six seven eight nine ten";
    let w = Some(90.0);
    let g = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &t, w));
    assert!(!g.iter().any(|g| g.0.glyph == ellipsis_glyph(&t)), "no ellipsis");
    for at in (0..text.len()).filter(|&i| !text[i..].starts_with(' ')) {
        let c = ts.caret(text, &t, w, at);
        let ch = ts.selection(Paragraph::from(text), &t, w, at, at + 1);
        assert_eq!(ch.len(), 1, "{at}: {ch:?}");
        assert!(
            (c.x - ch[0].x).abs() <= 0.5 && (c.y - ch[0].y).abs() <= 0.5,
            "{at}: {c:?} at {:?}",
            ch[0]
        );
        assert_eq!(ts.hit(text, &t, w, ch[0].x + 0.5, ch[0].y + 1.0), at, "{at} hit back");
    }
    // Not being edited, the clamp holds.
    let g = glyphs_of(&drawn(&mut ts, Paragraph::from(text), &clamped, w));
    assert!(g.iter().any(|g| g.0.glyph == ellipsis_glyph(&clamped)));
}

#[test]
fn a_focused_clamped_textarea_shows_the_text_its_caret_moves_through() {
    // A textarea clamped to one line, three lines tall, focused: it is not
    // cut while edited, so as Right moves the caret through what the clamp
    // would hide, the caret stays in the box after a character drawn
    // there, and Backspace takes that character.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::events::{Modifiers, NamedKey};
    use crate::styled::Styled;
    let content = "one two three four five six";
    let field = TextField::new(content);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(text_input(&f).width(120.0).multiline().height(80.0).line_clamp(1))
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    field.edit.update(|e| e.place(0, false));
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let geometry = |s: &Scenario, ts: &mut TextSystem| {
        let n = s
            .last()
            .nodes
            .iter()
            .find(|n| matches!(n.content, crate::layout::Content::Input { .. }))
            .unwrap()
            .clone();
        let crate::layout::Content::Input {
            edit,
            text: t,
            multiline,
            focused,
            ..
        } = &n.content
        else {
            unreachable!()
        };
        let it = crate::layout::input_text(
            ts,
            &edit.text,
            t,
            n.content_box,
            *multiline,
            *focused,
            n.scroll.map_or((0.0, 0.0), |s| s.applied),
        );
        let caret = ts.caret(&edit.text, &it.t, it.width, edit.caret);
        let before = edit
            .caret
            .checked_sub(1)
            .map(|b| ts.selection(&*edit.text, &it.t, it.width, b, edit.caret));
        let at = |b: Bounds| (b.x + it.origin.0, b.y + it.origin.1);
        (
            edit.caret,
            at(Bounds {
                x: caret.x,
                y: caret.y,
                width: 0.0,
                height: caret.height,
            }),
            caret.height,
            before.map(|b| b.into_iter().map(at).collect::<Vec<_>>()),
            n.content_box,
        )
    };
    for step in 0..content.len() - 1 {
        s.press(NamedKey::ArrowRight, Modifiers::default());
        s.settle().unwrap();
        let (caret, (x, y), height, before, area) = geometry(&s, &mut ts);
        assert_eq!(caret, step + 1);
        assert!(
            x >= area.x - 0.5 && x <= area.right() + 0.5 && y >= area.y - 0.5 && y + height <= area.bottom() + 0.5,
            "step {step}: the caret at ({x}, {y}) is out of {area:?}"
        );
        assert!(
            before.is_some_and(|b| !b.is_empty()),
            "step {step}: the character before it is drawn"
        );
    }
    let (caret, ..) = geometry(&s, &mut ts);
    s.press(NamedKey::Backspace, Modifiers::default());
    s.settle().unwrap();
    let mut want = content.to_owned();
    want.remove(caret - 1);
    assert_eq!(field.edit.with(|e| e.text().to_owned()), want);
}

#[test]
fn a_clamp_kept_by_one_label_fits_another_that_starts_its_line_elsewhere() {
    // Two labels clamped to two lines share their long second line, but
    // not their first lines' lengths: what the clamp found for the line in
    // one is kept for the other, but from the line's own start, so the
    // other's kept text and ellipsis fit its box as if drawn alone.
    let t = ResolvedText {
        line_clamp: Some(2),
        ..style(16.0, 24.0, 400)
    };
    let second = "the same long second line that every label shares in full";
    let long = format!("a first line\n{second}");
    let short = format!("a\n{second}");
    let w = Some(160.0);
    let mut ts = TextSystem::new();
    drawn(&mut ts, Paragraph::from(long.as_str()), &t, w);
    let got = glyphs_of(&drawn(&mut ts, Paragraph::from(short.as_str()), &t, w));
    let width = ts.measure(short.as_str(), &t, w).0;
    assert!(width <= 160.0, "{width}px in a 160px box");
    let want = glyphs_of(&drawn(&mut TextSystem::new(), Paragraph::from(short.as_str()), &t, w));
    assert_eq!(got, want, "as if drawn alone");
}

#[test]
fn a_clamp_cuts_by_its_lines_advances_and_shapes_a_cut_word_once() {
    // Prefixes 30, 60 (a word's end), 90, 120 and 150 wide in the line.
    let prefix = |end: usize, inline: f32, closes: bool| Prefix { end, inline, closes };
    let prefixes = [
        prefix(2, 30.0, false),
        prefix(4, 60.0, true),
        prefix(7, 90.0, false),
        prefix(9, 120.0, false),
        prefix(11, 150.0, true),
    ];
    let count = |room: f32, alone: &dyn Fn(usize) -> f32| {
        let mut shaped = 0;
        let k = word_fit(&prefixes, room, |k| {
            shaped += 1;
            alone(k)
        });
        (k, shaped)
    };
    // Cut at a word's end: as wide alone, nothing shaped.
    assert_eq!(count(70.0, &|_| unreachable!()), (2, 0));
    assert_eq!(count(150.0, &|_| unreachable!()), (5, 0));
    // Cut inside a word: shaped alone once, and kept when it fits so.
    assert_eq!(count(125.0, &|k| prefixes[k - 1].inline), (4, 1));
    // Wider alone by 25 (a letter kept turns final): stepped back by that
    // and shaped again, kept as it fits.
    assert_eq!(count(125.0, &|k| prefixes[k - 1].inline + 25.0), (3, 2));
    // Wider by more than the word has: its last whole word, unshaped.
    assert_eq!(count(125.0, &|k| prefixes[k - 1].inline + 80.0), (2, 1));
    // None fits in the line: its first unit shaped alone, once, and
    // kept only if that fits.
    assert_eq!(count(20.0, &|k| prefixes[k - 1].inline), (0, 1));
    assert_eq!(word_fit(&[], 100.0, |_| unreachable!()), 0);
}

#[test]
fn a_clamps_cut_inside_a_word_never_overflows_and_shapes_a_bounded_number_of_times() {
    // One word of forty units, 10 px each in the line, whose prefixes
    // shaped alone grow more the shorter they are (each step back cuts
    // where a wider form follows): the stepped-back cut still measures
    // wider, again and again. What is kept fits shaped alone, is nothing
    // only when its first unit does not, and no more than `FIT_SHAPES`
    // prefixes and that unit are shaped.
    let prefixes: Vec<Prefix> = (1..=40)
        .map(|k| Prefix {
            end: k,
            inline: 10.0 * crate::convert::usize_to_f32(k),
            closes: k == 40,
        })
        .collect();
    let room = 205.0;
    for growth in [0.0_f32, 4.0, 15.0, 30.0, 60.0, 400.0] {
        // Alone, prefix k is its width in the line plus `growth` for each
        // unit short of the twentieth.
        let width = |k: usize| {
            10.0 * crate::convert::usize_to_f32(k) + 12.0 + growth * (20.0 - crate::convert::usize_to_f32(k)).max(0.0)
        };
        let mut shaped = 0;
        let k = word_fit(&prefixes, room, |k| {
            shaped += 1;
            width(k)
        });
        assert!(shaped <= FIT_SHAPES + 1, "growth {growth}: {shaped} shapes");
        assert!(
            k > 0 || width(1) > room,
            "growth {growth}: kept nothing, yet its first unit fits"
        );
        assert!(
            k == 0 || width(k) <= room,
            "growth {growth}: kept {k}, {} wide",
            width(k)
        );
    }
    // Every prefix but its first unit far wider alone: that unit, not
    // nothing.
    let k = word_fit(&prefixes, room, |k| if k == 1 { 12.0 } else { 1000.0 });
    assert_eq!(k, 1, "its first unit, which fits alone");
    // No prefix fits by its width in the line, yet its first unit fits
    // alone: that unit.
    let wide = [Prefix {
        end: 1,
        inline: 11.0,
        closes: false,
    }];
    assert_eq!(word_fit(&wide, 10.0, |_| 9.0), 1, "its first unit, 9 px alone in 10");
    // And with a whole word before it: never short of that.
    let mut two = prefixes;
    two[4].closes = true;
    let k = word_fit(&two, room, |k| 10.0 * crate::convert::usize_to_f32(k) + 1000.0);
    assert_eq!(k, 5, "the whole word before");
}

#[test]
fn a_clamps_prefix_fits_alone_and_is_the_longest_its_line_fits_at_every_width() {
    // Clamped with no soft wrap opportunity (the logical prefix): at every
    // width, the prefix kept fits shaped alone, never overflowing, in
    // Arabic (whose last letter kept turns final), in words and in one,
    // Devanagari conjuncts, kerned Latin, and Arabic and Latin in one
    // word; it is the longest whose width in the line fits when that one
    // fits alone too, and never short of the last whole word that does.
    let texts = [
        "\u{633}".repeat(40),
        "\u{644}\u{627}\u{633}\u{645}\u{628}\u{633}".repeat(6),
        "\u{644}\u{627}\u{633}\u{645} \u{628}\u{633}\u{645} \u{647}\u{647}".repeat(5),
        "\u{915}\u{94d}\u{937}\u{924}\u{94d}\u{930}\u{93f}\u{92f}".repeat(6),
        "\u{915}\u{94d}\u{937} \u{924}\u{94d}\u{930} \u{91c}\u{94d}\u{91e} \u{936}\u{94d}\u{930}".repeat(4),
        "AVAWAYATAVo.Te".repeat(4),
        "AVA WAY TAVo. Te ".repeat(4),
        format!("{}AVATAR{}", "\u{628}\u{633}".repeat(8), "\u{633}".repeat(8)),
    ];
    for (text, rtl) in texts.iter().zip([true, true, true, false, false, false, false, false]) {
        let t = ResolvedText {
            direction: if rtl { Direction::Rtl } else { Direction::Ltr },
            nowrap: true,
            line_clamp: Some(1),
            ..style(16.0, 24.0, 400)
        };
        let p = Paragraph::from(text.as_str());
        let mut ts = TextSystem::new();
        let mut probe = TextSystem::new();
        let mut w = 16.0;
        while w < 260.0 {
            let laid = ts.laid_out(p, &t, Some(w));
            let Some(c) = ts.cut(p, &laid, &t, Some(w)) else {
                w += 0.75;
                continue;
            };
            let kept = c.prefix.expect("a clamp keeps a prefix");
            let room = w - ts.ellipsis_width(&t);
            let (piece, _) = laid[c.piece];
            let (_, _, soft) = ts.block_ellipsis(p, &t, piece, c.line, room);
            assert!(!soft, "no soft wrap opportunity");
            let s = &ts.shaped[&piece.hash];
            let l = s.layout.get(c.line).expect("the line");
            let lead = s.key.lead;
            let at = |i: usize| piece.start + i.saturating_sub(lead);
            let from = at(l.text_range().start);
            let mut units = Units::default();
            units.fill(s, &l);
            let ends: Vec<usize> = units
                .units
                .iter()
                .filter(|u| !u.space && u.range.1 > lead)
                .map(|u| at(u.range.1))
                .collect();
            let mut alone = |to: usize| {
                let h = probe.piece_as(p, &t, from, to, Apart::Kept);
                probe.break_piece(h, Breaking::ONE_LINE);
                probe.alone_extent(h).1
            };
            let fit = room + noise(room);
            assert!(
                kept.end == from || alone(kept.end) <= fit,
                "{text:?} at {w}px: kept to {} overflows",
                kept.end
            );
            // The oracle's own, not the clamp's scratch: each prefix's
            // width in the line (its clusters' advances), and whether it
            // ends a word (a space after it, or the line's end).
            let clusters: Vec<(usize, f32)> = l
                .runs()
                .flat_map(|r| {
                    r.clusters()
                        .map(|c| (c.text_range().end, c.advance()))
                        .collect::<Vec<_>>()
                })
                .filter(|&(e, _)| e > lead)
                .map(|(e, a)| (at(e), a))
                .collect();
            let inline = |to: usize| -> f32 { clusters.iter().filter(|c| c.0 <= to).map(|c| c.1).sum() };
            let line_end = at(l.text_range().end);
            let closes = |e: usize| e >= line_end || text.get(e..).is_some_and(|r| r.starts_with(' '));
            assert!(
                ends.contains(&kept.end) || kept.end == from,
                "{text:?} at {w}px: at a unit's end"
            );
            // The longest the line fits: kept when it fits alone.
            if let Some(top) = ends.iter().copied().filter(|&e| inline(e) <= fit).max()
                && alone(top) <= fit
            {
                assert_eq!(kept.end, top, "{text:?} at {w}px");
            }
            // Never short of the last whole word that fits alone.
            let words = ends
                .iter()
                .copied()
                .filter(|&e| closes(e) && alone(e) <= fit)
                .max()
                .unwrap_or(from);
            assert!(kept.end >= words, "{text:?} at {w}px: short of its last whole word");
            // A prefix that fits alone wins over keeping none.
            if ends.iter().any(|&e| alone(e) <= fit) {
                assert!(kept.end > from, "{text:?} at {w}px: kept none, yet a prefix fits");
            }
            w += 0.75;
        }
    }
}

#[test]
fn a_clamped_preview_of_a_long_document_holds_only_what_it_shows() {
    // A hundred thousand lines clamped to one: only the first is shaped,
    // drawn, and pinned by the chunk that shows it; what the clamp hides
    // is never shaped, yet measured, hit and read as before.
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let doc: String = (0..100_000).map(|n| format!("line {n} of a long document\n")).collect();
    let mut ts = TextSystem::new();
    let w = Some(120.0);
    drawn(&mut ts, Paragraph::from(doc.as_str()), &t, w);
    let pieces = ts.take_drawn();
    assert!(pieces.len() <= 4, "{} pieces drawn", pieces.len());
    ts.pin(&pieces);
    assert!(ts.pinned.len() <= 4, "{} pieces pinned", ts.pinned.len());
    assert!(ts.shaped.len() <= 6, "{} pieces shaped", ts.shaped.len());
    let (_, h) = ts.measure(doc.as_str(), &t, w);
    assert!((h - 24.0).abs() < 0.01, "one line: {h}");
    assert_eq!(
        ts.hit(&doc, &t, w, 118.0, 12.0),
        doc.len(),
        "over the ellipsis: the end"
    );
    assert_eq!(ts.hit(&doc, &t, w, 10.0, 200.0), doc.len(), "below: the end");
    assert!(ts.shaped.len() <= 6, "{} pieces shaped", ts.shaped.len());
    // A screen reader still reads all of a shorter one, hidden lines and
    // their breaks included, at the ellipsis.
    let short: String = (0..1_000).map(|n| format!("line {n} of a document\n")).collect();
    let mut ts = TextSystem::new();
    let runs = ts.text_runs(&short, &t, w);
    let read: String = runs.iter().filter_map(|r| short.get(r.range.clone())).collect();
    assert_eq!(read, short);
    assert!(runs.windows(2).all(|r| r[0].range.end == r[1].range.start));
    // What it hides of the shown hard line is on the shown line; each
    // hard line after it, a line of its own.
    for r in &runs {
        let hard = short.get(..r.range.start).map_or(0, |b| b.matches('\n').count());
        assert_eq!(r.line, hard, "{r:?}");
    }
    assert!(ts.shaped.len() <= 6, "{} pieces shaped", ts.shaped.len());
    let hidden = &runs[runs.len() - 2];
    assert!(hidden.widths.iter().all(|w| *w == 0.0) && hidden.lengths.len() == "line 999 of a document\n".len());
    let ws: Vec<u8> = hidden.word_starts.clone();
    assert_eq!(ws, vec![0, 5, 9, 12, 14], "{hidden:?}");
}

#[test]
fn what_a_cut_hides_has_one_box_over_its_ellipsis() {
    // Clamped to a line, left to right and right to left, with lines
    // laid out past the cut and lines never laid out: of all the runs it
    // hides, one alone has a box (over the ellipsis), the last; the
    // others are zero wide at the ellipsis's start.
    for direction in [Direction::Ltr, Direction::Rtl] {
        let t = ResolvedText {
            line_clamp: Some(1),
            direction,
            ..style(16.0, 24.0, 400)
        };
        let doc: String = (0..40)
            .map(|n| format!("line {n} of a document that wraps\n"))
            .collect();
        let mut ts = TextSystem::new();
        let runs = ts.text_runs(&doc, &t, Some(120.0));
        let hidden: Vec<&TextRun> = runs.iter().filter(|r| r.hidden).collect();
        assert!(hidden.len() > 40, "{direction:?}: {} hidden runs", hidden.len());
        let boxed: Vec<usize> = runs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.hidden && r.bounds.width > 0.0)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(boxed, vec![runs.len() - 1], "{direction:?}: one box, the last run's");
        let at = hidden.first().map(|r| r.bounds.x).unwrap_or_default();
        for r in &hidden[..hidden.len() - 1] {
            assert!(
                r.bounds.width == 0.0 && (r.bounds.x - at).abs() < 0.01,
                "{direction:?}: {r:?}"
            );
        }
    }
}

#[test]
fn a_clamped_long_document_read_at_several_widths_breaks_and_cuts_what_it_must_once() {
    // A hundred thousand lines clamped to three, the first wrapping into
    // thousands: read by a screen reader at width after width (a window
    // resized), only the lines shown are broken, and the lines never laid
    // out are cut into characters once, not at every width.
    let t = ResolvedText {
        line_clamp: Some(3),
        ..style(16.0, 24.0, 400)
    };
    let mut doc = "word ".repeat(20_000);
    doc.push('\n');
    for n in 1..100_000 {
        doc.push_str(&format!("line {n}\n"));
    }
    let mut ts = TextSystem::new();
    for (i, w) in [200.0, 230.0, 260.0, 290.0, 200.0].into_iter().enumerate() {
        let (broken, segmented) = (ts.lines_broken, ts.hidden_segmented);
        let runs = ts.text_runs(&doc, &t, Some(w));
        let read: usize = runs.iter().map(|r| r.range.len()).sum();
        assert_eq!(read, doc.len(), "at {w}px all of it is read");
        let broken = ts.lines_broken - broken;
        // The three lines shown, the probe for the clamped line's last
        // soft wrap opportunity, and its kept prefix: never the thousands
        // the first line wraps into.
        assert!(broken <= 8, "at {w}px: {broken} lines broken");
        let segmented = ts.hidden_segmented - segmented;
        // The hard line the clamp stopped breaking (its rest read in its
        // context), and the hundred thousand after it; once.
        let want = if i == 0 { 100_001 } else { 0 };
        assert_eq!(segmented, want, "at {w}px: hidden lines cut");
    }
    // A one-line preview of one huge paragraph (no line breaks), wrapping
    // as balanced or pretty text too: neither breaks it whole to learn it
    // is long, nor is its unbroken rest cut into characters again.
    let paragraph = "word ".repeat(100_000);
    for wrap_style in [WrapStyle::Auto, WrapStyle::Balance, WrapStyle::Pretty] {
        let t = ResolvedText {
            line_clamp: Some(1),
            wrap_style,
            ..style(16.0, 24.0, 400)
        };
        let mut ts = TextSystem::new();
        for (i, w) in [200.0, 230.0, 260.0, 200.0].into_iter().enumerate() {
            let (broken, segmented) = (ts.lines_broken, ts.hidden_segmented);
            let runs = ts.text_runs(&paragraph, &t, Some(w));
            let read: usize = runs.iter().map(|r| r.range.len()).sum();
            assert_eq!(read, paragraph.len(), "{wrap_style:?} at {w}px: all of it is read");
            let broken = ts.lines_broken - broken;
            assert!(broken <= 8, "{wrap_style:?} at {w}px: {broken} lines broken");
            let segmented = ts.hidden_segmented - segmented;
            let want = u64::from(i == 0);
            assert_eq!(segmented, want, "{wrap_style:?} at {w}px: its rest cut once");
        }
    }
}

#[test]
fn two_clamped_fields_read_by_turns_cut_their_hidden_lines_once() {
    // Two clamped documents read by a screen reader by turns, a frame
    // each, at width after width: each one's hidden lines are cut once,
    // the other's reading never making it cut them again. Many texts read
    // once each are let go, past a bound.
    let t = ResolvedText {
        line_clamp: Some(2),
        ..style(16.0, 24.0, 400)
    };
    let doc = |name: &str| -> String { (0..2_000).map(|n| format!("{name} line {n}\n")).collect() };
    let (a, b) = (doc("first"), doc("second"));
    let mut ts = TextSystem::new();
    for w in [200.0, 230.0, 260.0] {
        for d in [&a, &b] {
            let before = ts.hidden_segmented;
            ts.text_runs(d, &t, Some(w));
            ts.end_frame();
            let cut = ts.hidden_segmented - before;
            if w == 200.0 {
                assert!(cut > 1_000, "read first: cut ({cut})");
            } else {
                assert_eq!(cut, 0, "at {w}px, read again by turns: nothing cut again");
            }
        }
    }
    for n in 0..40 {
        ts.text_runs(&doc(&format!("text {n}")), &t, Some(200.0));
        ts.end_frame();
    }
    assert!(
        ts.hidden_lines.len() <= HIDDEN_MAX,
        "{} texts kept",
        ts.hidden_lines.len()
    );
}

#[test]
fn a_long_word_a_clamp_cuts_mid_word_starts_no_word_inside_it() {
    // One unspaced word, wrapping (`overflow-wrap: break-word`) into many
    // lines, clamped to one: the clamp stops breaking inside the word, and
    // what follows is read as the rest of that word, not a word of its own.
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let word = "abcdefghij".repeat(300);
    let mut ts = TextSystem::new();
    let runs = ts.text_runs(&word, &t, Some(120.0));
    let read: String = runs.iter().filter_map(|r| word.get(r.range.clone())).collect();
    assert_eq!(read, word);
    let starts: Vec<usize> = runs
        .iter()
        .flat_map(|r| r.word_starts.iter().map(|&w| r.byte_of(usize::from(w))))
        .collect();
    assert_eq!(starts, vec![0], "one word, starting once");
}

#[test]
fn measuring_clamped_rows_on_a_layout_worker_keeps_no_answers_past_its_pass() {
    // A layout pool thread measures unique clamped rows, pass after pass,
    // and never ends a frame: forgetting what it shaped at each pass
    // forgets where those clamps kept their lines to as well.
    let t = ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    };
    let mut ts = TextSystem::new();
    for n in 0..200 {
        let row = format!("row {n} says something too long for its box");
        ts.measure(row.as_str(), &t, Some(80.0));
        assert!(!ts.fits.is_empty(), "the clamp was found");
        ts.forget_shaped();
        assert!(ts.fits.len() <= 1, "pass {n}: {} answers kept", ts.fits.len());
    }
}

/// The field input `s` shows: its box, the caret's box as painted in the
/// window, and its caret offset and text.
fn field_caret(s: &crate::app::Scenario, ts: &mut TextSystem) -> (Bounds, Bounds, usize, String) {
    let n = s
        .last()
        .nodes
        .iter()
        .find(|n| matches!(n.content, crate::layout::Content::Input { .. }))
        .unwrap()
        .clone();
    let crate::layout::Content::Input {
        edit,
        text: t,
        multiline,
        focused,
        ..
    } = &n.content
    else {
        unreachable!()
    };
    let it = crate::layout::input_text(
        ts,
        &edit.text,
        t,
        n.content_box,
        *multiline,
        *focused,
        n.scroll.map_or((0.0, 0.0), |s| s.applied),
    );
    let c = ts.caret(&edit.text, &it.t, it.width, edit.caret);
    let caret = Bounds {
        x: c.x + it.origin.0,
        y: c.y + it.origin.1,
        width: 0.0,
        height: c.height,
    };
    (n.content_box, caret, edit.caret, edit.text.to_string())
}

#[test]
fn a_focused_clamped_textarea_keeps_its_height_and_scrolls_to_its_caret() {
    // An auto-height textarea clamped to one line is one line tall; focused
    // it is not cut, and keeps that height (as Chrome's box does not grow),
    // its text scrolling up so the caret on its last line stays inside.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::events::{Modifiers, NamedKey};
    use crate::styled::Styled;
    let content = "one two three four five six seven eight nine ten";
    let field = TextField::new(content);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(text_input(&f).width(120.0).multiline().line_clamp(1))
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let (area, ..) = field_caret(&s, &mut ts);
    s.act("focus", || field.focus.focus());
    field.edit.update(|e| e.place(0, false));
    s.settle().unwrap();
    for _ in 0..12 {
        s.press(NamedKey::ArrowDown, Modifiers::default());
        s.settle().unwrap();
    }
    let (focused, caret, at, _) = field_caret(&s, &mut ts);
    assert_eq!(at, content.len());
    assert!(
        (focused.height - area.height).abs() < 0.01,
        "{focused:?} grew from {area:?}"
    );
    assert!(
        caret.y >= focused.y - 0.5 && caret.y + caret.height <= focused.bottom() + 0.5,
        "the caret {caret:?} is out of {focused:?}"
    );
    // At its start again, the text scrolls back down.
    for _ in 0..12 {
        s.press(NamedKey::ArrowUp, Modifiers::default());
        s.settle().unwrap();
    }
    let (focused, caret, ..) = field_caret(&s, &mut ts);
    assert!((caret.y - focused.y).abs() < 0.5, "{caret:?} at the top of {focused:?}");
}

#[test]
fn a_press_on_an_unfocused_clamped_textarea_hits_what_it_shows() {
    // A textarea clamped to one line, not focused: a press over its
    // ellipsis (what is painted: the cut text) puts the caret at the
    // text's end, not where the expanded text's first line ends.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::styled::Styled;
    let content = "one two three four five six seven eight nine ten";
    let field = TextField::new(content);
    field.edit.update(|e| e.place(0, false));
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(text_input(&f).width(120.0).multiline().height(80.0).line_clamp(1))
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let (area, ..) = field_caret(&s, &mut ts);
    // The caret at the end of a cut text stands by its ellipsis: a press
    // just right of there is over it.
    let n = s
        .last()
        .nodes
        .iter()
        .find(|n| matches!(n.content, crate::layout::Content::Input { .. }))
        .unwrap()
        .clone();
    let crate::layout::Content::Input { text: t, .. } = &n.content else {
        unreachable!()
    };
    let shown = crate::layout::input_text(&mut ts, content, t, area, true, false, (0.0, 0.0));
    let dots = ts.caret(content, &shown.t, shown.width, content.len());
    s.click(
        shown.origin.0 + dots.x + 2.0,
        shown.origin.1 + dots.y + dots.height / 2.0,
    );
    s.settle().unwrap();
    assert_eq!(field.edit.with(|e| e.caret), content.len(), "at the text's end");
}

#[test]
fn a_focused_clamped_textarea_scrolled_to_its_end_paints_nothing_outside_its_box() {
    // An auto-height textarea clamped to one line, focused with its caret
    // on its last line: its text scrolls up in its one-line box, and what
    // scrolls out is clipped to the box, as Chrome clips a textarea, not
    // painted over what is above it.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::events::{Modifiers, NamedKey};
    use crate::styled::Styled;
    let content = "one two three four five six seven eight nine ten";
    let field = TextField::new(content);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .background(Color::WHITE)
                .padding_top(60.0)
                .padding_left(20.0)
                .child(text_input(&f).width(120.0).multiline().line_clamp(1))
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    field.edit.update(|e| e.place(0, false));
    s.settle().unwrap();
    for _ in 0..12 {
        s.press(NamedKey::ArrowDown, Modifiers::default());
        s.settle().unwrap();
    }
    let mut ts = TextSystem::new();
    let (area, caret, at, _) = field_caret(&s, &mut ts);
    assert_eq!(at, content.len());
    assert!(
        caret.y >= area.y - 0.5 && caret.y + caret.height <= area.bottom() + 0.5,
        "scrolled to the caret: {caret:?} in {area:?}"
    );
    let shot = s.shot(false).unwrap();
    let (w, h) = (shot.width(), shot.height());
    let mut inside = 0;
    for y in 0..h {
        for x in 0..w {
            let ink = shot.pixel(x, y).is_some_and(|p| p[0] < 160);
            let (fx, fy) = (crate::convert::u32_to_f32(x) + 0.5, crate::convert::u32_to_f32(y) + 0.5);
            let within =
                fx >= area.x - 1.0 && fx <= area.right() + 1.0 && fy >= area.y - 1.0 && fy <= area.bottom() + 1.0;
            if ink && within {
                inside += 1;
            }
            assert!(!ink || within, "ink at ({x}, {y}), outside {area:?}");
        }
    }
    assert!(inside > 0, "its last line shows in its box");
}

/// The scroll offset of the first text input `s` shows, and of the scroll
/// container around it (none: zero).
fn field_scrolls(s: &crate::app::Scenario) -> ((f32, f32), (f32, f32), (f32, f32)) {
    let nodes = &s.last().nodes;
    let i = nodes
        .iter()
        .position(|n| matches!(n.content, crate::layout::Content::Input { .. }))
        .unwrap();
    let field = nodes[i].scroll.expect("a field scrolls");
    let mut up = nodes[i].parent;
    let mut outer = (0.0, 0.0);
    while let Some(p) = up {
        if let Some(sc) = nodes[p].scroll {
            outer = sc.offset;
            break;
        }
        up = nodes[p].parent;
    }
    (field.offset, field.max, outer)
}

/// A focused multiline field of `content`, `height` tall and 120 wide, at
/// (20, 20) in a white page; the caret at its start.
fn focused_textarea(content: &'static str, height: f32) -> (crate::app::Scenario, crate::edit::TextField) {
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::styled::Styled;
    let field = TextField::new(content);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .background(Color::WHITE)
                .padding_top(20.0)
                .padding_left(20.0)
                .child(text_input(&f).width(120.0).multiline().height(height))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    field.edit.update(|e| e.place(0, false));
    s.settle().unwrap();
    (s, field)
}

const LONG: &str = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen \
                    fifteen sixteen seventeen eighteen nineteen twenty";

#[test]
fn text_an_input_method_commits_past_a_fields_end_scrolls_to_show_its_caret() {
    // An input method commits more than a one-line field shows, as typing
    // does: the field scrolls so the caret after it is inside its box.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::styled::Styled;
    let field = TextField::new("");
    let f = field;
    let mut s = Scenario::measuring(
        move || div().child(text_input(&f).width(120.0)).into_element(),
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    s.settle().unwrap();
    s.commit("a long run of words an input method typed at once");
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let (area, caret, at, text) = field_caret(&s, &mut ts);
    assert_eq!(at, text.len(), "the caret after what it typed");
    assert!(field_scrolls(&s).0.0 > 0.0, "the field scrolled");
    assert!(
        caret.x >= area.x - 0.5 && caret.x <= area.right() + 0.5,
        "the caret {caret:?} is out of {area:?}"
    );
}

#[test]
fn a_caret_moving_within_what_a_field_shows_does_not_scroll_it() {
    // A field three lines tall holding more: the caret stepping down to
    // its last line scrolls it just enough to show it there; stepping up
    // a line, still shown, scrolls nothing (as Chrome scrolls a caret into
    // view only as far as it must); past the top row, a line's worth.
    use crate::events::{Modifiers, NamedKey};
    let (mut s, _field) = focused_textarea(LONG, 72.0);
    let key = |s: &mut crate::app::Scenario, k: NamedKey| {
        s.press(k, Modifiers::default());
        s.settle().unwrap();
    };
    let mut ts = TextSystem::new();
    for _ in 0..20 {
        key(&mut s, NamedKey::ArrowDown);
    }
    let (bottom, max, _) = field_scrolls(&s);
    let (area, caret, at, text) = field_caret(&s, &mut ts);
    assert_eq!(at, text.len());
    assert!(
        bottom.1 > 0.0 && (bottom.1 - max.1).abs() < 0.5,
        "scrolled to its end: {bottom:?} of {max:?}"
    );
    assert!(caret.y + caret.height <= area.bottom() + 0.5 && caret.y >= area.y - 0.5);
    key(&mut s, NamedKey::ArrowUp);
    let (up, ..) = field_scrolls(&s);
    assert_eq!(up, bottom, "a line up, still shown: no scroll");
    key(&mut s, NamedKey::ArrowUp);
    assert_eq!(field_scrolls(&s).0, bottom, "the top row, still shown: no scroll");
    key(&mut s, NamedKey::ArrowUp);
    let (past, ..) = field_scrolls(&s);
    let (area, caret, ..) = field_caret(&s, &mut ts);
    assert!(
        (bottom.1 - past.1 - 24.0).abs() < 0.5,
        "past the top: a line's worth, {bottom:?} to {past:?}"
    );
    assert!((caret.y - area.y).abs() < 0.5, "on the top row: {caret:?} in {area:?}");
}

#[test]
fn the_wheel_scrolls_a_field_then_the_container_around_it() {
    // A field in a scroll container, under the pointer: the wheel's
    // gesture scrolls the field, latched to it to its end; the next
    // gesture there, the field at its end, scrolls the container.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::style::Overflow;
    use crate::styled::Styled;
    let field = TextField::new(LONG);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(200.0)
                .height(120.0)
                .overflow_y(Overflow::Scroll)
                .child(text_input(&f).width(120.0).multiline().height(48.0))
                .child(div().height(400.0))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    s.settle().unwrap();
    s.hover(40.0, 20.0);
    s.settle().unwrap();
    for _ in 0..30 {
        s.wheel(20.0);
        s.settle().unwrap();
    }
    let (inner, max, outer) = field_scrolls(&s);
    assert!(
        max.1 > 0.0 && (inner.1 - max.1).abs() < 0.5,
        "the field to its end: {inner:?} of {max:?}"
    );
    assert_eq!(
        outer,
        (0.0, 0.0),
        "the container stays, the gesture latched to the field"
    );
    s.wait(std::time::Duration::from_secs(2));
    s.settle().unwrap();
    s.wheel(20.0);
    s.settle().unwrap();
    let (_, _, outer) = field_scrolls(&s);
    assert!(outer.1 > 0.0, "the next gesture scrolls the container: {outer:?}");
}

#[test]
fn a_wheel_scrolling_a_long_textarea_resolves_nothing_and_copies_no_text() {
    // Its scroll is read where it is placed, not where its value is: the
    // wheel moves its text and resolves no node again; and neither the
    // wheel nor keys that move its caret copy its text anywhere on the
    // edit path (what it shows is shared, keys read the edit in place).
    use crate::app::Scenario;
    use crate::edit::{TEXT_COPIES, TextField};
    use crate::element::{IntoElement, div, text_input};
    use crate::events::{Modifiers, NamedKey};
    use crate::styled::Styled;
    let field = TextField::new(LONG.repeat(40));
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(text_input(&f).width(160.0).multiline().height(200.0))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    // Focused: its scroll is its handle's.
    s.act("focus", || field.focus.focus());
    s.settle().unwrap();
    s.hover(60.0, 60.0);
    s.settle().unwrap();
    for _ in 0..3 {
        s.wheel(20.0);
        s.settle().unwrap();
    }
    let before = field_scrolls(&s).0;
    TEXT_COPIES.with(|c| c.set(0));
    let mut resolved = 0;
    for step in 0..10 {
        s.wheel(20.0);
        resolved += s.frame(&format!("wheel {step}")).unwrap().visits.resolved;
        resolved += s.frame(&format!("after wheel {step}")).unwrap().visits.resolved;
        // As a window asks each frame: where the input method's candidates go.
        assert!(
            s.pipeline_mut().ime_target().is_some(),
            "the field takes an input method"
        );
    }
    let after = field_scrolls(&s).0;
    assert!(after.1 > before.1, "it scrolled: {before:?} to {after:?}");
    assert_eq!(field.offset(), after, "its handle's scroll");
    assert_eq!(resolved, 0, "no node resolved again");
    assert_eq!(TEXT_COPIES.with(std::cell::Cell::get), 0, "the wheel copied its text");
    let caret = field.edit.with(|e| e.caret);
    for _ in 0..6 {
        s.press(NamedKey::ArrowUp, Modifiers::default());
        s.settle().unwrap();
        s.press(NamedKey::ArrowLeft, Modifiers::default());
        s.settle().unwrap();
        assert!(
            s.pipeline_mut().ime_target().is_some(),
            "the field takes an input method"
        );
    }
    assert_ne!(field.edit.with(|e| e.caret), caret, "the caret moved");
    assert_eq!(
        TEXT_COPIES.with(std::cell::Cell::get),
        0,
        "moving the caret copied its text"
    );
    // An input method moving its caret inside what it composes, the
    // composition's text the same: applied in place, shown as it was.
    s.compose("かな");
    s.settle().unwrap();
    TEXT_COPIES.with(|c| c.set(0));
    for at in [0, 3, 6, 3] {
        s.send(crate::window::Input::Ime(crate::events::ImeEvent::Preedit(
            crate::edit::Preedit {
                text: "かな".to_owned(),
                cursor: Some((at, at)),
            },
        )));
        s.settle().unwrap();
        assert!(
            s.pipeline_mut().ime_target().is_some(),
            "the field takes an input method"
        );
    }
    let shown = s.last().nodes.iter().find_map(|n| match &n.content {
        crate::layout::Content::Input { edit, .. } => Some(edit.caret),
        _ => None,
    });
    assert_eq!(
        shown,
        Some(field.edit.with(|e| e.caret) + 3),
        "its caret inside the composition"
    );
    assert_eq!(
        TEXT_COPIES.with(std::cell::Cell::get),
        0,
        "moving the input method's caret copied its text"
    );
}

#[test]
fn holding_an_arrow_key_in_a_field_settles_with_no_new_room() {
    // Each key moves the caret and its field reveals it, said by placing
    // and taken by settling, pass after pass: the list that carries it
    // goes back and forth, grown once, never again.
    use crate::edit::{FIELD_GROWS, TextField};
    use crate::events::{Modifiers, NamedKey};
    use crate::reactive::Signal;
    let (a, b) = (TextField::new(LONG), TextField::new(LONG));
    let mut s = one_of_two_textareas(a, b, Signal::new(true), Signal::new(true));
    s.act("focus", || a.focus.focus());
    s.settle().unwrap();
    let press = |s: &mut crate::app::Scenario, n: usize| {
        for k in 0..n {
            let key = if (k / 6) % 2 == 0 {
                NamedKey::ArrowDown
            } else {
                NamedKey::ArrowUp
            };
            s.press(key, Modifiers::default());
            s.settle().unwrap();
        }
    };
    press(&mut s, 12);
    FIELD_GROWS.with(|c| c.set(0));
    press(&mut s, 24);
    assert_eq!(
        FIELD_GROWS.with(std::cell::Cell::get),
        0,
        "settling grew its list again"
    );
}

#[test]
fn a_drag_held_past_a_fields_edge_scrolls_it_on() {
    // A selection dragged from a field's first line to below it and held
    // there: the field scrolls on, frame after frame with the pointer
    // still, and the selection grows with what comes under the pointer.
    use crate::window::Input;
    let (mut s, field) = focused_textarea(LONG, 48.0);
    s.send(Input::PointerMoved { x: 25.0, y: 30.0 });
    s.send(Input::MouseDown { x: 25.0, y: 30.0 });
    s.send(Input::PointerMoved { x: 60.0, y: 100.0 });
    s.settle().unwrap();
    let mut last = (field_scrolls(&s).0, field.edit.with(|e| e.caret));
    let anchor = field.edit.with(|e| e.anchor);
    assert!(anchor <= 2, "pressed on its first word: {anchor}");
    for step in 0..4 {
        s.wait(std::time::Duration::from_millis(100));
        s.settle().unwrap();
        let now = (field_scrolls(&s).0, field.edit.with(|e| e.caret));
        assert!(now.0.1 > last.0.1, "step {step}: it scrolls on: {last:?} to {now:?}");
        assert!(now.1 >= last.1, "step {step}: the selection grows: {last:?} to {now:?}");
        last = now;
    }
    assert!(
        last.1 > anchor && field.edit.with(|e| e.anchor) == anchor,
        "selected from where it went down"
    );
    s.send(Input::MouseUp { x: 60.0, y: 100.0 });
    s.settle().unwrap();
    let stopped = field_scrolls(&s).0;
    s.wait(std::time::Duration::from_millis(200));
    s.settle().unwrap();
    assert_eq!(field_scrolls(&s).0, stopped, "released: it stops");
}

#[test]
fn a_drag_held_past_a_fields_side_does_not_scroll_it_up_or_down() {
    // Held right of a textarea (which does not scroll sideways), level
    // with its first line: it is past no edge it scrolls along, so it
    // does not scroll at all.
    use crate::window::Input;
    let (mut s, _) = focused_textarea(LONG, 48.0);
    assert!(field_scrolls(&s).1.1 > 0.0, "it scrolls up and down");
    s.send(Input::PointerMoved { x: 25.0, y: 30.0 });
    s.send(Input::MouseDown { x: 25.0, y: 30.0 });
    s.send(Input::PointerMoved { x: 250.0, y: 30.0 });
    s.settle().unwrap();
    for _ in 0..4 {
        s.wait(std::time::Duration::from_millis(100));
        s.settle().unwrap();
    }
    assert_eq!(field_scrolls(&s).0, (0.0, 0.0), "held to its side: no scroll");
    s.send(Input::MouseUp { x: 250.0, y: 30.0 });
}

#[test]
fn a_drag_held_past_a_fields_edge_wakes_no_frame_at_its_limit_or_once_released() {
    // Held below a textarea, it scrolls to its end, and there, still held,
    // has no wakeup queued; released mid-scroll, its queued one goes too.
    use crate::window::Input;
    let (mut s, _) = focused_textarea(LONG, 48.0);
    s.send(Input::PointerMoved { x: 25.0, y: 30.0 });
    s.send(Input::MouseDown { x: 25.0, y: 30.0 });
    s.send(Input::PointerMoved { x: 60.0, y: 100.0 });
    s.settle().unwrap();
    s.wait(std::time::Duration::from_millis(50));
    s.settle().unwrap();
    assert!(s.pipeline().input_state().autoscroll_queued(), "scrolling: woken again");
    for _ in 0..40 {
        s.wait(std::time::Duration::from_millis(100));
        s.settle().unwrap();
    }
    let (offset, max, _) = field_scrolls(&s);
    assert_eq!(offset.1, max.1, "at its end");
    assert!(
        !s.pipeline().input_state().autoscroll_queued(),
        "at its limit, still held: nothing queued"
    );
    // Back up a little, held above it: it scrolls again, and the release
    // cancels the step it queued.
    s.send(Input::PointerMoved { x: 60.0, y: 5.0 });
    s.settle().unwrap();
    s.wait(std::time::Duration::from_millis(50));
    s.settle().unwrap();
    assert!(
        s.pipeline().input_state().autoscroll_queued(),
        "scrolling up: woken again"
    );
    s.send(Input::MouseUp { x: 60.0, y: 5.0 });
    assert!(
        !s.pipeline().input_state().autoscroll_queued(),
        "released: nothing queued"
    );
}

/// A page of one textarea, 120 by 48 at (20, 20): field `a` or `b` (as
/// `which` says) at one path, or none (`shown` false).
fn one_of_two_textareas(
    a: crate::edit::TextField,
    b: crate::edit::TextField,
    which: crate::reactive::Signal<bool>,
    shown: crate::reactive::Signal<bool>,
) -> crate::app::Scenario {
    use crate::element::{IntoElement, div, text_input};
    use crate::styled::Styled;
    let mut s = crate::app::Scenario::measuring(
        move || {
            let field = if which.get() { a } else { b };
            div()
                .padding_top(20.0)
                .padding_left(20.0)
                .children(
                    shown
                        .get()
                        .then(|| text_input(&field).width(120.0).multiline().height(48.0)),
                )
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s
}

/// Focus `field` and take its caret to its end by keys: its text scrolls up.
fn scroll_to_end(s: &mut crate::app::Scenario, field: crate::edit::TextField) {
    use crate::events::{Modifiers, NamedKey};
    s.act("focus", || field.focus.focus());
    field.edit.update(|e| e.place(0, false));
    s.settle().unwrap();
    for _ in 0..12 {
        s.press(NamedKey::ArrowDown, Modifiers::default());
        s.settle().unwrap();
    }
    assert!(field_scrolls(s).0.1 > 0.0, "scrolled to its end");
}

#[test]
fn another_field_at_a_scrolled_fields_path_starts_unscrolled() {
    use crate::edit::TextField;
    use crate::reactive::Signal;
    let (a, b) = (TextField::new(LONG), TextField::new(LONG));
    let (which, shown) = (Signal::new(true), Signal::new(true));
    let mut s = one_of_two_textareas(a, b, which, shown);
    scroll_to_end(&mut s, a);
    s.act("swap", || which.set(false));
    s.settle().unwrap();
    assert_eq!(field_scrolls(&s).0, (0.0, 0.0), "the other field: its own scroll");
}

#[test]
fn a_field_unmounted_and_mounted_again_keeps_the_scroll_its_handle_holds() {
    // Its scroll is its handle's (as Compose's `TextFieldState` holds its
    // scroll): a field shown again, its handle still held, is where it was.
    use crate::edit::TextField;
    use crate::reactive::Signal;
    let (a, b) = (TextField::new(LONG), TextField::new(LONG));
    let (which, shown) = (Signal::new(true), Signal::new(true));
    let mut s = one_of_two_textareas(a, b, which, shown);
    scroll_to_end(&mut s, a);
    let far = field_scrolls(&s).0;
    s.act("hide", || shown.set(false));
    s.settle().unwrap();
    s.act("show", || shown.set(true));
    s.settle().unwrap();
    assert_eq!(field_scrolls(&s).0, far, "mounted again: its handle's scroll");
}

#[test]
fn fields_patched_into_one_node_by_turns_each_show_their_own_scroll() {
    // One node shows field `a`, then `b`, then `a` again: each its own
    // scroll, whatever the node held before.
    use crate::edit::TextField;
    use crate::reactive::Signal;
    let (a, b) = (TextField::new(LONG), TextField::new(LONG));
    let (which, shown) = (Signal::new(true), Signal::new(true));
    let mut s = one_of_two_textareas(a, b, which, shown);
    scroll_to_end(&mut s, a);
    let far = field_scrolls(&s).0;
    s.act("to b", || which.set(false));
    s.settle().unwrap();
    assert_eq!(field_scrolls(&s).0, (0.0, 0.0), "b: its own, none");
    s.act("to a", || which.set(true));
    s.settle().unwrap();
    assert_eq!(field_scrolls(&s).0, far, "a again: its own");
}

#[test]
fn a_fields_scroll_lives_as_long_as_its_handle() {
    // Made in a scope, the field's view lives until the scope goes.
    use crate::edit::{FieldView, TextField};
    use crate::reactive::Scope;
    let before = FieldView::live();
    let scope = Scope::new();
    let field = scope.run(|| TextField::new(LONG));
    field.scroll_to((0.0, 12.0));
    assert_eq!(FieldView::live(), before + 1);
    scope.dispose();
    assert_eq!(FieldView::live(), before, "the handle gone: its scroll freed");
}

/// The scroll offsets of the inputs of `s`'s last frame, in tree order.
fn input_offsets(s: &crate::app::Scenario) -> Vec<(f32, f32)> {
    s.last()
        .nodes
        .iter()
        .filter(|n| matches!(n.content, crate::layout::Content::Input { .. }))
        .map(|n| n.scroll.map_or((0.0, 0.0), |sc| sc.offset))
        .collect()
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "a TextField is shown by two fields")]
fn one_text_field_shown_by_two_fields_panics() {
    // Its focus, its input method and its caret are one node's: a second
    // field showing it while the first is mounted is a bug.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    let field = TextField::new(LONG);
    let f = field;
    let mut s = Scenario::measuring(
        move || div().child(text_input(&f)).child(text_input(&f)).into_element(),
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
}

/// `field` kept in step with `model` both ways: two fields bound to one
/// model, each its own handle.
fn bind_to(field: crate::edit::TextField, model: crate::reactive::Signal<String>) {
    use crate::reactive::watch;
    watch(move || field.text(), move |new, _| model.set(new.clone()));
    watch(
        move || model.get(),
        move |new, _| {
            if field.edit.with_untracked(|e| e.text() != *new) {
                field.edit.update(|e| e.set_text(new.as_str()));
            }
        },
    );
}

#[test]
fn two_fields_bound_to_one_model_each_reveal_their_own_caret() {
    // A short textarea and a tall one that holds all of the text, each
    // its own `TextField` bound to one model: the short one's caret taken
    // to the end scrolls it to show it, the tall one stays at the top; and
    // what is typed in one is the other's text.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::events::{Modifiers, NamedKey};
    use crate::reactive::Signal;
    use crate::styled::Styled;
    let model = Signal::new(LONG.to_owned());
    let (short, tall) = (TextField::new(LONG), TextField::new(LONG));
    bind_to(short, model);
    bind_to(tall, model);
    let mut s = Scenario::measuring(
        move || {
            div()
                .padding_top(20.0)
                .padding_left(20.0)
                .child(text_input(&short).width(120.0).multiline().height(48.0))
                .child(text_input(&tall).width(120.0).multiline().height(600.0))
                .into_element()
        },
        300,
        900,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || short.focus.focus());
    short.edit.update(|e| e.place(0, false));
    s.settle().unwrap();
    for _ in 0..12 {
        s.press(NamedKey::ArrowDown, Modifiers::default());
        s.settle().unwrap();
    }
    assert_eq!(short.edit.with(|e| e.caret), LONG.len(), "the caret at its end");
    let offsets = input_offsets(&s);
    assert!(offsets[0].1 > 0.0, "the short one shows its caret: {offsets:?}");
    assert_eq!(offsets[1], (0.0, 0.0), "the tall one stays at the top");
    s.type_text("!");
    s.settle().unwrap();
    let typed = format!("{LONG}!");
    assert_eq!(model.get(), typed, "the model has it");
    assert_eq!(tall.text(), typed, "the other field shows it");
    assert_eq!(input_offsets(&s)[1], (0.0, 0.0), "and stays at the top");
}

#[test]
fn a_field_put_where_a_wheel_scrolled_unfocused_one_was_opens_at_the_top() {
    // Textarea `a`, never focused, scrolled by the wheel; `b` (its own
    // handle) shown at its path in its place opens at the top, and `a`
    // keeps where it was.
    use crate::edit::TextField;
    use crate::reactive::Signal;
    let (a, b) = (TextField::new(LONG), TextField::new(LONG));
    let (which, shown) = (Signal::new(true), Signal::new(true));
    let mut s = one_of_two_textareas(a, b, which, shown);
    s.hover(60.0, 40.0);
    s.settle().unwrap();
    for _ in 0..3 {
        s.wheel(20.0);
        s.settle().unwrap();
    }
    let far = field_scrolls(&s).0;
    assert!(far.1 > 0.0, "a scrolled: {far:?}");
    s.act("swap", || which.set(false));
    s.settle().unwrap();
    assert_eq!(field_scrolls(&s).0, (0.0, 0.0), "b opens at the top");
    assert_eq!(a.offset(), far, "a keeps its own");
}

#[test]
fn typing_for_ten_thousand_frames_keeps_no_growing_bookkeeping() {
    // Typed into and deleted from, frame after frame: what layout and
    // input keep for fields stays as it was after the first few.
    use crate::edit::TextField;
    use crate::events::{Modifiers, NamedKey};
    use crate::reactive::Signal;
    let (a, b) = (TextField::new(LONG), TextField::new(LONG));
    let mut s = one_of_two_textareas(a, b, Signal::new(true), Signal::new(true));
    s.act("focus", || a.focus.focus());
    s.settle().unwrap();
    let frame = |s: &mut crate::app::Scenario, k: usize| {
        if k.is_multiple_of(2) {
            s.type_text("x");
        } else {
            s.press(NamedKey::Backspace, Modifiers::default());
        }
        s.settle().unwrap();
    };
    for k in 0..20 {
        frame(&mut s, k);
    }
    let kept = s.pipeline().field_bookkeeping();
    for k in 0..10_000 {
        frame(&mut s, k);
    }
    assert_eq!(a.text(), LONG, "typed and deleted");
    assert_eq!(s.pipeline().field_bookkeeping(), kept, "its bookkeeping grew");
}

#[test]
fn a_field_moved_from_a_closing_layer_to_the_page_in_one_frame_keeps_its_focus() {
    // A focused field in a layer; in one frame the layer closes and the
    // same handle shows in the page. Fields claim their handles once the
    // whole frame is reconciled, the leaving one gone: no second field,
    // and focus stays on it.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, Positioning, deferred, div, text_input};
    use crate::reactive::Signal;
    use crate::styled::Styled;
    let field = TextField::new("hello");
    let in_layer = Signal::new(true);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            let input = text_input(&f).width(120.0);
            if in_layer.get() {
                div()
                    .child(deferred(Positioning::At { x: 20.0, y: 100.0 }, input))
                    .into_element()
            } else {
                div().child(input).into_element()
            }
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    s.settle().unwrap();
    let layer_of = |s: &Scenario| {
        s.last()
            .nodes
            .iter()
            .find(|n| matches!(n.content, crate::layout::Content::Input { focused: true, .. }))
            .map(|n| n.layer)
    };
    assert_eq!(layer_of(&s), Some(1), "focused in its layer");
    s.act("move", || in_layer.set(false));
    s.settle().unwrap();
    assert!(field.focus.is_focused(), "focus stays on it");
    assert_eq!(layer_of(&s), Some(0), "focused in the page");
    assert_eq!(s.pipeline().field_claims(), 1);
}

#[test]
fn one_field_node_shown_with_a_thousand_handles_by_turns_holds_one_claim() {
    // One node patched from handle to handle: each patch moves its
    // claim, so the claims are one, the one it shows.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::reactive::Signal;
    use crate::styled::Styled;
    let shown = Signal::new(TextField::new("0"));
    let mut s = Scenario::measuring(
        move || div().child(text_input(&shown.get()).width(120.0)).into_element(),
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let path = |s: &Scenario| {
        s.last()
            .nodes
            .iter()
            .find(|n| matches!(n.content, crate::layout::Content::Input { .. }))
            .map(|n| n.path.clone())
    };
    let first = path(&s);
    for k in 1..=1000 {
        let next = TextField::new(k.to_string());
        s.act("next", || shown.set(next));
        s.settle().unwrap();
    }
    assert_eq!(path(&s), first, "the one node");
    assert_eq!(s.pipeline().field_claims(), 1, "a claim per handle it showed");
    assert_eq!(s.pipeline().focus_entries(), 1, "a focus entry per handle it showed");
}

#[test]
fn a_composition_left_in_a_field_whose_scope_is_gone_is_dropped_untouched() {
    // Composing in a field; the field closes and its scope is disposed:
    // where the input method types is asked again, and the handle it
    // composed in, gone, is dropped, never written.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::reactive::{Scope, Signal};
    use crate::styled::Styled;
    let scope = Scope::new();
    let field = scope.run(|| TextField::new("hello"));
    let shown = Signal::new(true);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .children(shown.get().then(|| text_input(&f).width(120.0)))
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    s.settle().unwrap();
    s.compose("ka");
    s.settle().unwrap();
    assert!(s.pipeline_mut().ime_target().is_some(), "composing");
    s.act("close", || shown.set(false));
    s.settle().unwrap();
    scope.dispose();
    assert!(!field.is_alive());
    assert_eq!(s.pipeline_mut().ime_target(), None, "nothing to type into");
    s.settle().unwrap();
}

#[test]
fn a_caret_moved_in_a_multi_megabyte_value_compares_and_copies_none_of_it() {
    // What a field shows is told the same by its value's revision: a caret
    // moved resolves the field and reads none of its text.
    use crate::app::Scenario;
    use crate::edit::{TEXT_COMPARES, TEXT_COPIES, TextField};
    use crate::element::{IntoElement, div, text_input};
    use crate::events::{Modifiers, NamedKey};
    use crate::styled::Styled;
    let field = TextField::new("word ".repeat(420_000));
    let f = field;
    let mut s = Scenario::measuring(
        move || div().child(text_input(&f).width(160.0)).into_element(),
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    field.edit.update(|e| e.place(0, false));
    s.settle().unwrap();
    TEXT_COMPARES.with(|c| c.set(0));
    TEXT_COPIES.with(|c| c.set(0));
    for _ in 0..4 {
        s.press(NamedKey::ArrowRight, Modifiers::default());
        s.settle().unwrap();
    }
    assert_eq!(field.edit.with(|e| e.caret), 4, "the caret moved");
    assert_eq!(TEXT_COMPARES.with(std::cell::Cell::get), 0, "its text compared");
    assert_eq!(TEXT_COPIES.with(std::cell::Cell::get), 0, "its text copied");
}

#[test]
fn a_composition_cancelled_leaves_the_value_and_its_selection_as_they_were() {
    // "world" selected in "hello world"; an input method composes over it,
    // shown in its place, then cancels: the value and the selection are
    // as they were, and a model bound to the value was never written.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::reactive::Signal;
    use crate::styled::Styled;
    let field = TextField::new("hello world");
    let model = Signal::labeled("model", "hello world".to_owned());
    bind_to(field, model);
    let f = field;
    let mut s = Scenario::measuring(
        move || div().child(text_input(&f).width(200.0)).into_element(),
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    field.edit.update(|e| {
        e.place(6, false);
        e.place(11, true);
    });
    s.settle().unwrap();
    let shown = |s: &Scenario| {
        s.last().nodes.iter().find_map(|n| match &n.content {
            crate::layout::Content::Input { edit, .. } => Some(edit.text.to_string()),
            _ => None,
        })
    };
    // Each frame's report takes what the runtime wrote since the last.
    crate::reactive::start_trace();
    let mut writes = Vec::new();
    let mut frames = |s: &mut Scenario, what: &str| {
        for k in 0..3 {
            writes.extend(s.frame(&format!("{what} {k}")).unwrap().reactive.writes);
        }
    };
    s.compose("wo");
    frames(&mut s, "compose");
    assert_eq!(
        shown(&s).as_deref(),
        Some("hello wo"),
        "composed in the selection's place"
    );
    assert_eq!(field.text(), "hello world", "the value untouched while composing");
    s.compose("");
    frames(&mut s, "cancel");
    crate::reactive::stop_trace();
    assert_eq!(field.text(), "hello world", "cancelled: the value as it was");
    assert_eq!(field.edit.with(|e| (e.anchor, e.caret)), (6, 11), "and the selection");
    assert_eq!(shown(&s).as_deref(), Some("hello world"));
    assert!(
        !writes.iter().any(|w| w.label == Some("model")),
        "the model was written: {writes:?}"
    );
}

#[test]
fn a_value_set_twice_the_same_tells_once_and_copies_nothing_the_second_time() {
    use crate::edit::{TEXT_COPIES, TextField};
    let field = TextField::new("x");
    let big = "word ".repeat(200_000);
    let revisions = std::rc::Rc::new(std::cell::Cell::new(0));
    let r = std::rc::Rc::clone(&revisions);
    crate::reactive::watch(move || field.revision(), move |_, _| r.set(r.get() + 1));
    crate::reactive::start_trace();
    field.set_text(big.clone());
    let second = big.clone();
    TEXT_COPIES.with(|c| c.set(0));
    field.set_text(second);
    let copies = TEXT_COPIES.with(std::cell::Cell::get);
    let writes = crate::reactive::take_trace().writes;
    crate::reactive::stop_trace();
    assert_eq!(writes.len(), 1, "one publish: {writes:?}");
    assert_eq!(revisions.get(), 1, "one new value");
    assert_eq!(copies, 0, "the second set copied its text");
    assert_eq!(field.text(), big);
}

#[test]
fn a_value_set_after_layout_past_a_queued_reveal_has_its_caret_revealed() {
    // A focused textarea asked to show its caret (a value set); in the
    // same frame, after that layout, a hook sets a longer value. The ask
    // the first layout took does not take the newer one: the next pass
    // shows the new caret.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::reactive::Signal;
    use crate::styled::Styled;
    let field = TextField::new("short");
    let f = field;
    let tick = Signal::new(0.0_f32);
    let armed = std::rc::Rc::new(std::cell::Cell::new(false));
    let arm = std::rc::Rc::clone(&armed);
    let mut s = Scenario::measuring(
        move || {
            let arm = std::rc::Rc::clone(&arm);
            div()
                .padding_top(20.0)
                .padding_left(20.0)
                .child(text_input(&f).width(120.0).multiline().height(48.0))
                .child(div().width(10.0 + tick.get()).height(4.0).on_measured(move |_| {
                    if arm.replace(false) {
                        f.set_text(LONG);
                    }
                }))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    s.settle().unwrap();
    s.act("set, and set again after layout", || {
        armed.set(true);
        tick.set(1.0);
        field.set_text("short too");
    });
    s.settle().unwrap();
    assert!(!armed.get(), "the hook ran");
    let mut ts = TextSystem::new();
    let (area, caret, at, text) = field_caret(&s, &mut ts);
    assert_eq!((at, text.as_str()), (LONG.len(), LONG));
    assert!(
        caret.y >= area.y - 0.5 && caret.y + caret.height <= area.bottom() + 0.5,
        "the caret {caret:?} is out of {area:?}"
    );
}

#[test]
fn a_fields_scroll_its_text_shrank_under_does_not_come_back_as_it_regrows() {
    // Scrolled to its end, its text cut to a line more than it shows
    // (the offset clamped to what is left), then grown back: it stays
    // where the clamp put it, not where it was.
    use crate::edit::TextField;
    use crate::reactive::Signal;
    let (a, b) = (TextField::new(LONG), TextField::new(LONG));
    let (which, shown) = (Signal::new(true), Signal::new(true));
    let mut s = one_of_two_textareas(a, b, which, shown);
    scroll_to_end(&mut s, a);
    let far = field_scrolls(&s).0.1;
    let short = "one two three four five six seven eight nine ten eleven";
    s.act("shrink", || a.edit.update(|e| e.set_text(short)));
    s.settle().unwrap();
    let clamped = field_scrolls(&s).0.1;
    assert!(clamped < far, "clamped: {clamped} from {far}");
    s.act("regrow", || a.edit.update(|e| e.set_text(LONG)));
    s.settle().unwrap();
    assert_eq!(field_scrolls(&s).0.1, clamped, "regrown: where the clamp left it");
}

#[test]
fn a_value_set_on_a_focused_field_scrolls_to_show_its_caret() {
    // `set_text` puts the caret at the end of a value longer than the
    // field shows: the field scrolls to it, as typing would.
    let (mut s, field) = focused_textarea("short", 48.0);
    s.act("set", || field.set_text(LONG));
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let (area, caret, at, text) = field_caret(&s, &mut ts);
    assert_eq!(at, text.len());
    assert!(
        caret.y >= area.y - 0.5 && caret.y + caret.height <= area.bottom() + 0.5,
        "the caret {caret:?} is out of {area:?}"
    );
}

#[test]
fn a_held_press_on_a_clamps_ellipsis_keeps_the_caret_at_the_end_through_focus() {
    // A press on the ellipsis of a textarea not focused, held while its
    // focus frame expands its text under the still pointer: the caret
    // stays at the text's end (Chrome extends a selection only as the
    // pointer moves), and nothing is selected.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::styled::Styled;
    use crate::window::Input;
    let content = "one two three four five six seven eight nine ten";
    let field = TextField::new(content);
    field.edit.update(|e| e.place(0, false));
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(text_input(&f).width(120.0).multiline().height(80.0).line_clamp(1))
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let (area, ..) = field_caret(&s, &mut ts);
    let n = s
        .last()
        .nodes
        .iter()
        .find(|n| matches!(n.content, crate::layout::Content::Input { .. }))
        .unwrap()
        .clone();
    let crate::layout::Content::Input { text: t, .. } = &n.content else {
        unreachable!()
    };
    let shown = crate::layout::input_text(&mut ts, content, t, area, true, false, (0.0, 0.0));
    let dots = ts.caret(content, &shown.t, shown.width, content.len());
    let (x, y) = (
        shown.origin.0 + dots.x + 2.0,
        shown.origin.1 + dots.y + dots.height / 2.0,
    );
    s.send(Input::PointerMoved { x, y });
    s.send(Input::MouseDown { x, y });
    for _ in 0..3 {
        s.settle().unwrap();
    }
    assert_eq!(
        field.edit.with(|e| (e.anchor, e.caret)),
        (content.len(), content.len()),
        "held through the focus frame: at the end"
    );
    s.send(Input::MouseUp { x, y });
}

#[test]
fn a_field_across_a_scroll_containers_edge_draws_no_text_past_it() {
    // A textarea half below a scroll container's bottom edge: on the GPU
    // its glyphs are cut by both its own clip and the container's (the
    // chain the shaders read as one rectangle), so no ink shows below the
    // container, where the field's own box goes on.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::style::Overflow;
    use crate::styled::Styled;
    let field = TextField::new(LONG);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .background(Color::WHITE)
                .width(300.0)
                .height(300.0)
                .child(
                    div()
                        .width(200.0)
                        .height(60.0)
                        .overflow_y(Overflow::Scroll)
                        .child(div().height(30.0))
                        .child(text_input(&f).width(120.0).multiline().height(64.0))
                        .child(div().height(200.0)),
                )
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let shot = s.shot(false).unwrap();
    let ink = |y0: u32, y1: u32| {
        (y0..y1)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|&(x, y)| shot.pixel(x, y).is_some_and(|p| p[0] < 160))
            .count()
    };
    assert!(ink(30, 60) > 0, "its first lines show inside the container");
    assert_eq!(ink(61, 100), 0, "nothing of it shows below the container");
    // The render thread scrolls the container alone (the field's clip
    // moves with its content, its entry written again as clipped): the
    // lines that come up show, and still nothing below the edge.
    let before = ink(30, 60);
    // Two steps: the second drawn from the first's plan, its clips'
    // entries written again where they changed.
    assert!(
        s.compositor_wheel((160.0, 30.0), (0.0, 10.0), 1.0),
        "the container scrolls"
    );
    assert!(s.compositor_wheel((160.0, 30.0), (0.0, 10.0), 3.0), "and scrolls on");
    let shot = s.shot(false).unwrap();
    let ink = |y0: u32, y1: u32| {
        (y0..y1)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|&(x, y)| shot.pixel(x, y).is_some_and(|p| p[0] < 160))
            .count()
    };
    assert!(ink(10, 60) > before, "more of it shows: {} from {before}", ink(10, 60));
    assert_eq!(ink(61, 100), 0, "scrolled: nothing below the container");
}

#[test]
fn a_fields_text_scrolled_into_its_container_shows_as_it_comes_in() {
    // A field half below the fold of a scroll container: its text is
    // clipped to its own box (which moves with it) within the container's
    // (which stays). As the compositor scrolls the container (its content
    // transform moves, the field's clip with it, before the UI lays out
    // again), what comes into the container shows: the container's edge,
    // not where it cut the field before, crops it.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::style::Overflow;
    use crate::styled::Styled;
    let field = TextField::new(LONG);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(
                    div()
                        .width(200.0)
                        .height(60.0)
                        .overflow_y(Overflow::Scroll)
                        .child(div().height(40.0))
                        .child(text_input(&f).width(120.0).multiline().height(48.0))
                        .child(div().height(200.0)),
                )
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let scene = crate::paint::Painter::default().cold(s.last(), &mut ts, (300, 300), 1.0, Color::WHITE);
    let mut props = scene.properties;
    // The field's own clip: the one within another.
    let id = (1..props.clips.len())
        .find(|&i| (props.clips[i].clip.bounds.y - 40.0).abs() < 0.5)
        .expect("the field's clip");
    let id32 = u32::try_from(id).unwrap();
    let shown = props.clip(id32).unwrap().clip.bounds;
    assert!(
        (shown.y - 40.0).abs() < 0.5 && (shown.bottom() - 60.0).abs() < 0.5,
        "cut at the container's bottom: {shown:?}"
    );
    // The compositor scrolls the container 20px: the field's clip moves
    // with its transform (as `displaced_clip` moves it), the container's
    // does not.
    props.clips[id].clip.bounds = props.clips[id].clip.bounds.translated(0.0, -20.0);
    let now = props.clip(id32).unwrap().clip.bounds;
    assert!(
        (now.y - 20.0).abs() < 0.5 && (now.bottom() - 60.0).abs() < 0.5,
        "20px more of it shows, to the container's bottom: {now:?}"
    );
}

/// The first text input `s` shows: its content box, text, style, whether
/// it is multiline and focused, and its scroll's offset and reach.
type InputNode = (Bounds, String, ResolvedText, bool, bool, (f32, f32), (f32, f32));

fn input_node(s: &crate::app::Scenario) -> InputNode {
    let n = s
        .last()
        .nodes
        .iter()
        .find(|n| matches!(n.content, crate::layout::Content::Input { .. }))
        .unwrap()
        .clone();
    let crate::layout::Content::Input {
        edit,
        text: t,
        multiline,
        focused,
        ..
    } = &n.content
    else {
        unreachable!()
    };
    let sc = n.scroll.expect("a field scrolls");
    (
        n.content_box,
        edit.text.to_string(),
        t.clone(),
        *multiline,
        *focused,
        sc.offset,
        sc.max,
    )
}

#[test]
fn an_unfocused_truncating_input_ends_in_an_ellipsis_and_focused_scrolls_its_whole_text() {
    // `truncate(true)` on a one-line field: not focused, its value is cut
    // at its box with an ellipsis, as Chrome's `text-overflow: ellipsis`
    // cuts an input; being edited, it is never cut, and scrolls instead.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::styled::Styled;
    let field = TextField::new(LONG);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .padding_top(20.0)
                .padding_left(20.0)
                .child(text_input(&f).width(120.0).truncate(true))
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let shown = |s: &Scenario, ts: &mut TextSystem| {
        let (area, text, t, multiline, focused, _, max) = input_node(s);
        let it = crate::layout::input_text(ts, &text, &t, area, multiline, focused, (0.0, 0.0));
        let glyphs = glyphs_of(&drawn(ts, Paragraph::from(text.as_str()), &it.t, it.width));
        (glyphs.last().map(|g| g.0.glyph), glyphs.len(), focused, max)
    };
    let dots = ellipsis_glyph(&ResolvedText::root(Color::BLACK));
    let (last, cut, focused, max) = shown(&s, &mut ts);
    assert!(!focused);
    assert_eq!(last, Some(dots), "not focused: cut with an ellipsis");
    assert_eq!(max, (0.0, 0.0), "cut to its box: nothing to scroll");
    s.act("focus", || field.focus.focus());
    s.settle().unwrap();
    let (last, whole, focused, max) = shown(&s, &mut ts);
    assert!(focused);
    assert_ne!(last, Some(dots), "focused: not cut");
    assert!(whole > cut, "focused: all of it, {whole} glyphs, from {cut}");
    assert!(max.0 > 0.0, "focused: it scrolls across, {max:?}");
}

#[test]
fn a_nowrap_textarea_scrolls_across_to_show_its_caret() {
    // A 120 px `nowrap` textarea, its caret at the end of a line far past
    // its box: a caret move scrolls it across (one reveal for both axes),
    // as a single line's does.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, text_input};
    use crate::events::{Modifiers, NamedKey};
    use crate::styled::Styled;
    let field = TextField::new(LONG);
    let f = field;
    let mut s = Scenario::measuring(
        move || {
            div()
                .padding_top(20.0)
                .padding_left(20.0)
                .child(text_input(&f).width(120.0).multiline().height(48.0).nowrap(true))
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    field.edit.update(|e| e.place(LONG.len(), false));
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let (area, text, t, multiline, focused, ..) = input_node(&s);
    let it = crate::layout::input_text(&mut ts, &text, &t, area, multiline, focused, (0.0, 0.0));
    let end = ts.caret(&text, &it.t, it.width, LONG.len());
    assert!(it.origin.0 + end.x > area.right() + 600.0, "its end, far past: {end:?}");
    s.press(NamedKey::ArrowLeft, Modifiers::default());
    s.settle().unwrap();
    let (area, caret, at, _) = field_caret(&s, &mut ts);
    assert_eq!(at, LONG.len() - 1);
    assert!(
        caret.x >= area.x - 0.5 && caret.x <= area.right() + 0.5,
        "the caret {caret:?} is out of {area:?}"
    );
    let (.., offset, max) = input_node(&s);
    assert!(offset.0 > 0.0 && max.0 >= offset.0, "{offset:?} within {max:?}");
}

#[test]
fn a_focused_field_re_keyed_as_its_value_is_set_shows_its_caret_and_keeps_its_scroll() {
    // A focused clamped textarea re-keyed (a new node) in the render that
    // sets its value: the reveal the setter asked for is its session's,
    // taken by the node holding it now, laid out with the new value. And
    // scrolled, then re-keyed alone, its scroll goes with it.
    use crate::app::Scenario;
    use crate::edit::TextField;
    use crate::element::{IntoElement, div, key_of, text_input};
    use crate::reactive::Signal;
    use crate::styled::Styled;
    let field = TextField::new("short");
    let f = field;
    let key = Signal::new(0_u32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .padding_top(20.0)
                .padding_left(20.0)
                .child(
                    text_input(&f)
                        .key(key_of(&format!("k{}", key.get())))
                        .width(120.0)
                        .multiline()
                        .height(48.0)
                        .line_clamp(2),
                )
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.act("focus", || field.focus.focus());
    s.settle().unwrap();
    s.act("re-key and set", || {
        key.set(1);
        field.set_text(LONG);
    });
    s.settle().unwrap();
    s.settle().unwrap();
    let mut ts = TextSystem::new();
    let (area, caret, at, text) = field_caret(&s, &mut ts);
    assert_eq!(at, text.len());
    assert!(
        caret.y >= area.y - 0.5 && caret.y + caret.height <= area.bottom() + 0.5,
        "the caret {caret:?} is out of {area:?}"
    );
    let scrolled = input_node(&s).5;
    assert!(scrolled.1 > 0.0, "scrolled to its end: {scrolled:?}");
    s.act("re-key", || key.set(2));
    s.settle().unwrap();
    s.settle().unwrap();
    assert_eq!(input_node(&s).5, scrolled, "re-keyed: its scroll kept");
}

#[test]
fn a_drag_held_past_a_fields_edge_at_its_limit_queues_no_first_step() {
    // Held above a textarea not scrolled, it cannot scroll that way: not
    // even its first tick is queued.
    use crate::window::Input;
    let (mut s, _) = focused_textarea(LONG, 48.0);
    s.send(Input::PointerMoved { x: 25.0, y: 30.0 });
    s.send(Input::MouseDown { x: 25.0, y: 30.0 });
    s.send(Input::PointerMoved { x: 60.0, y: 2.0 });
    s.settle().unwrap();
    assert_eq!(field_scrolls(&s).0, (0.0, 0.0));
    assert!(
        !s.pipeline().input_state().autoscroll_queued(),
        "at its limit that way: nothing queued"
    );
    s.send(Input::MouseUp { x: 60.0, y: 2.0 });
}

#[test]
fn a_clamp_cuts_a_long_token_in_as_many_shapes_however_long() {
    // A URL of 200, 400 and 800 characters and a katakana run as long,
    // truncated, clamped to two lines and clamped unwrapped in 220 px: where the clamp cuts
    // is found walking the shaped line's advances once, so the shapes it
    // takes do not grow with the token.
    // Percent-encoded, as a URL is in another's query: no soft wrap
    // opportunity in it (UAX #14), so the clamp's walk finds the cut.
    let url = |n: usize| {
        let head = "https%3A%2F%2Fexample.com%2Fsearch%3Fq%3D";
        let tail: String = "a1b2c3d4e5f6g7h8i9j0".chars().cycle().take(n - head.len()).collect();
        format!("{head}{tail}")
    };
    let kana = |n: usize| "カタカナテキスト".chars().cycle().take(n).collect::<String>();
    let styles = [
        ResolvedText {
            truncate: true,
            ..style(16.0, 24.0, 400)
        },
        ResolvedText {
            line_clamp: Some(2),
            ..style(16.0, 24.0, 400)
        },
        ResolvedText {
            nowrap: true,
            line_clamp: Some(1),
            ..style(16.0, 24.0, 400)
        },
    ];
    for t in &styles {
        let makes: [&dyn Fn(usize) -> String; 2] = [&url, &kana];
        for make in makes {
            let counts: Vec<u64> = [200, 400, 800]
                .iter()
                .map(|&n| {
                    let text = make(n);
                    let mut ts = TextSystem::new();
                    drawn(&mut ts, Paragraph::from(""), t, Some(220.0));
                    let before = ts.shapes;
                    drawn(&mut ts, Paragraph::from(text.as_str()), t, Some(220.0));
                    ts.shapes - before
                })
                .collect();
            assert!(
                counts.windows(2).all(|c| c[0] == c[1]) && counts[0] <= 5,
                "{:?} clamp {:?}: {counts:?} shapes",
                t.truncate,
                t.line_clamp
            );
        }
    }
}

#[test]
fn a_thai_textareas_words_start_alike_shown_or_hidden_by_its_clamp() {
    // Thai has no spaces between words: its word boundaries come from a
    // dictionary. Its second hard line read shown, and read hidden by a
    // clamp to one line: the same words start, both from Parley's analysis.
    let content = "บรรทัดแรก\nสวัสดีครับวันนี้อากาศดีมากเราไปเที่ยวทะเลกัน";
    let second = content.find('\n').unwrap() + 1;
    let starts = |t: &ResolvedText| {
        let mut ts = TextSystem::new();
        let runs = ts.text_runs(content, t, Some(600.0));
        let mut out = Vec::new();
        for r in runs.iter().filter(|r| r.range.start >= second) {
            let mut at = r.range.start;
            for (i, len) in r.lengths.iter().enumerate() {
                if r.word_starts.contains(&u8::try_from(i).unwrap()) {
                    out.push(at);
                }
                at += usize::from(*len);
            }
        }
        (out, runs.iter().any(|r| r.hidden))
    };
    let (shown, any_hidden) = starts(&style(16.0, 24.0, 400));
    assert!(!any_hidden);
    let (hidden, any_hidden) = starts(&ResolvedText {
        line_clamp: Some(1),
        ..style(16.0, 24.0, 400)
    });
    assert!(any_hidden, "the clamp hides the second line");
    assert!(shown.len() > 3, "a dictionary's words: {shown:?}");
    assert_eq!(hidden, shown);
}

#[test]
fn a_rounded_scroll_container_in_a_rounded_card_leaves_the_cards_corners_untouched() {
    // A card rounded 16 that clips, holding a scroll container rounded 8
    // that fills it, its content black throughout: what the container
    // holds is cut by both clips (one tree, each clip its own corners), so
    // the card's corners stay the page's white though the container's own,
    // smaller corners would let it show there.
    use crate::app::Scenario;
    use crate::element::{IntoElement, div};
    use crate::style::Corners;
    use crate::style::Overflow;
    use crate::styled::Styled;
    let mut s = Scenario::measuring(
        move || {
            div()
                .background(Color::WHITE)
                .width(200.0)
                .height(200.0)
                .padding_top(20.0)
                .padding_left(20.0)
                .child(
                    div()
                        .width(100.0)
                        .height(100.0)
                        .radius(Corners::all(16.0))
                        .overflow(Overflow::Hidden)
                        .child(
                            div()
                                .width(100.0)
                                .height(100.0)
                                .radius(Corners::all(8.0))
                                .overflow_y(Overflow::Scroll)
                                .child(div().width(100.0).height(300.0).background(Color::BLACK)),
                        ),
                )
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    let shot = s.shot(false).unwrap();
    let px = |x: u32, y: u32| shot.pixel(x, y).unwrap();
    // Outside the card's corner (radius 16 about (36, 36)), inside the
    // container's (radius 8 about (28, 28)).
    for (x, y) in [(23, 23), (116, 23), (23, 116), (116, 116)] {
        assert_eq!(px(x, y), [255, 255, 255, 255], "the card's corner at ({x}, {y})");
    }
    assert_eq!(px(70, 70), [0, 0, 0, 255], "inside: the content");
}
