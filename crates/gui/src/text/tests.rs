use super::*;

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
}

#[test]
fn balance_evens_the_lines_and_pretty_leaves_no_orphan() {
    let mut ts = TextSystem::new();
    let text = "Balanced lines are about as wide as each other";
    let auto = ResolvedText::root(Color::BLACK);
    let balanced = ResolvedText {
        wrap_style: WrapStyle::Balance,
        ..auto
    };
    let (wa, ha) = ts.measure(text, &auto, Some(300.0));
    let (wb, hb) = ts.measure(text, &balanced, Some(300.0));
    assert!((ha - hb).abs() < 0.01 && wb < wa, "{wa} {wb}");
    let pretty = ResolvedText {
        wrap_style: WrapStyle::Pretty,
        ..auto
    };
    let orphan = "Words wrap so that the last line holds one word alone";
    let w = ts.measure(orphan, &auto, None).0 - 20.0;
    let p = ts.pieces(Paragraph::from(orphan), &pretty);
    ts.break_piece(p[0].hash, Breaking::of(&pretty, Some(w)));
    let s = &ts.shaped[&p[0].hash];
    assert!(!orphan_line(s), "pretty leaves no word alone");
}

fn orphan_line(s: &Shaped) -> bool {
    orphan(&s.layout, &s.content)
}

#[test]
fn baselines_round_as_blink_places_them() {
    // Geist Mono at 14px on a 22.75px line: its 18px content box 2px below
    // each line's top (ADR-0005).
    let mut ts = TextSystem::new();
    let t = ResolvedText {
        family: FontFamily::Mono,
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
        text: t,
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
        id: 7,
    }];
    let p = Paragraph {
        content,
        runs: &[],
        boxes: &boxes,
    };
    let placed = ts.inline_boxes(p, &t, None);
    assert_eq!(placed.len(), 1);
    let (id, b) = placed[0];
    let hello = ts.measure("hello ", &t, None).0;
    assert_eq!(id, 7);
    assert!(
        (b.x - hello).abs() < 1.0 && (b.width - 20.0).abs() < 0.01,
        "{b:?} after {hello}"
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
