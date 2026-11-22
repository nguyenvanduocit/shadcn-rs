//! Layout checked against Chrome (M9 I4): each fixture in `chrome_layout/`
//! is a small tree of `<div>` boxes and `<p>` paragraphs styled inline,
//! built here through gui's element API (`div()`, `text()`, the
//! [`Styled`] style) and laid out in an 800×600 window as a frame is; every
//! box must equal the border box Chrome gave the same markup, which
//! `crates/shadcn/verdict/layout.mjs` wrote next to it (`<name>.chrome`).
//! The expectations are committed, so the test needs no browser.
//!
//! Both sides lay out text in gui's own Inter (`assets/Inter.ttf`, its
//! `opsz` following the size) at gui's root style, 16 px on 24 px, so text
//! sizes match too. The page gives every element gui's defaults: border-box
//! sizing, a position (every gui box contains its absolute children), no
//! margin or padding of its own, scrollbars that take no room.
//!
//! Chrome lays out in LayoutUnits (1/64 px) and gui in `f32`, so a box may
//! differ by rounding: [`TOLERANCE`] is one LayoutUnit and a little, for
//! every box, text or not (both round text widths up to a LayoutUnit).
//!
//! A difference is either on purpose, gui being more correct than Chrome
//! ([`BETTER`]: the boxes, and why gui's are better), or a bug
//! ([`KNOWN_BUGS`]: the fixture and the bug). Either must still differ: an
//! entry that matches Chrome again fails the test until it is taken out.
//!
//! Regenerate the expectations after changing a fixture:
//! `node crates/shadcn/verdict/layout.mjs [name ...]`.

#![expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "a test harness: fixture numbers are small, and a fixture that does not parse is a failed test"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use gui::layout::{FrameNode, UiState, layout};
use gui::style::{
    Align, Display, Edges, FlexDirection, GridSpan, Justify, Length, LineHeight, Overflow, Position, Style, Track,
};
use gui::text::TextSystem;
use gui::{Element, IntoElement, Motion, Styled, div, text};

/// The window both lay a fixture out in (`layout.mjs` uses the same).
const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

/// How far a gui box may be from Chrome's, per edge and size: one
/// LayoutUnit (1/64 px) for Chrome's rounding, and a little for `f32`'s.
const TOLERANCE: f32 = 1.0 / 64.0 + 0.001;

/// Boxes where gui differs from Chrome on purpose: the fixture, the boxes
/// (indexes in document order, as a failure prints them), and why gui's
/// are better. The rest of the fixture must match.
const BETTER: &[(&str, &[usize], &str)] = &[(
    "text_single_line",
    &[8],
    "gui's advances are the font's own (hmtx and HVAR at opsz 16, wght 400, read by skrifa): `i` is \
     488/2048 em, so the ten `i`s before this box are 38.125 px; Chrome on macOS shapes with the \
     advances Skia reads through CoreText, 38.109 px for them, so this box starts 1/64 px and its \
     ten `W`s end 2/64 px from gui's (160.406 px wide, Chrome 160.375)",
)];

/// Fixtures gui lays out wrongly: the bug, until it is fixed.
const KNOWN_BUGS: &[(&str, &str)] = &[];

// ── Reading a fixture ────────────────────────────────────────────────────────

fn number(v: &str) -> f32 {
    v.trim().parse().unwrap_or_else(|_| panic!("{v:?} is not a number"))
}

fn pixels(v: &str) -> f32 {
    let v = v.trim();
    if v == "0" {
        return 0.0;
    }
    number(v.strip_suffix("px").unwrap_or_else(|| panic!("{v:?} is not in px")))
}

/// `calc(a% ± bpx)`, in either order.
fn calc(inner: &str) -> Length {
    let (mut fraction, mut px) = (0.0, 0.0);
    let mut sign = 1.0;
    for token in inner.split_whitespace() {
        match token {
            "+" => sign = 1.0,
            "-" => sign = -1.0,
            t => {
                if let Some(p) = t.strip_suffix('%') {
                    fraction += sign * number(p) / 100.0;
                } else {
                    px += sign * pixels(t);
                }
            }
        }
    }
    Length::Calc { fraction, px }
}

fn length(v: &str) -> Length {
    let v = v.trim();
    match v {
        "auto" => Length::Auto,
        "fit-content" => Length::FitContent,
        _ => {
            if let Some(inner) = v.strip_prefix("calc(").and_then(|r| r.strip_suffix(')')) {
                calc(inner)
            } else if let Some(p) = v.strip_suffix('%') {
                Length::Fraction(number(p) / 100.0)
            } else {
                Length::Px(pixels(v))
            }
        }
    }
}

/// A CSS box shorthand of one to four values: top, right, bottom, left.
fn edges<T: Copy>(v: &str, f: impl Fn(&str) -> T) -> Edges<T> {
    let parts: Vec<T> = v.split_whitespace().map(f).collect();
    let (top, right, bottom, left) = match parts.as_slice() {
        [a] => (*a, *a, *a, *a),
        [a, b] => (*a, *b, *a, *b),
        [a, b, c] => (*a, *b, *c, *b),
        [a, b, c, d] => (*a, *b, *c, *d),
        _ => panic!("{v:?} is not one to four values"),
    };
    Edges {
        top,
        right,
        bottom,
        left,
    }
}

fn align(v: &str) -> Align {
    match v.trim() {
        "flex-start" | "start" | "self-start" => Align::Start,
        "flex-end" | "end" | "self-end" => Align::End,
        "center" => Align::Center,
        "stretch" => Align::Stretch,
        "baseline" => Align::Baseline,
        other => panic!("no alignment {other:?}"),
    }
}

fn justify(v: &str) -> Justify {
    match v.trim() {
        "flex-start" | "start" => Justify::Start,
        "flex-end" | "end" => Justify::End,
        "center" => Justify::Center,
        "space-between" => Justify::Between,
        "space-around" => Justify::Around,
        "space-evenly" => Justify::Evenly,
        "stretch" => Justify::Stretch,
        other => panic!("no distribution {other:?}"),
    }
}

fn overflow(v: &str) -> Overflow {
    match v.trim() {
        "visible" => Overflow::Visible,
        "hidden" => Overflow::Hidden,
        "scroll" | "auto" => Overflow::Scroll,
        other => panic!("no overflow {other:?}"),
    }
}

fn track(v: &str) -> Track {
    match v {
        "auto" => Track::Auto,
        "min-content" => Track::MinContent,
        "max-content" => Track::MaxContent,
        _ => match v.strip_suffix("fr") {
            Some(fr) => Track::Fr(number(fr)),
            None => Track::Px(pixels(v)),
        },
    }
}

fn tracks(v: &str) -> Vec<Track> {
    v.split_whitespace().map(track).collect()
}

/// `2`, `span 2`, `1 / span 2`, `2 / 4`.
fn grid_span(v: &str) -> GridSpan {
    let span_of = |s: &str| s.trim().strip_prefix("span ").map(|n| number(n) as u16);
    match v.split_once('/') {
        None => match span_of(v) {
            Some(span) => GridSpan {
                start: None,
                span: Some(span),
            },
            None => GridSpan {
                start: Some(number(v) as i16),
                span: None,
            },
        },
        Some((start, end)) => {
            let start = number(start) as i16;
            let span = span_of(end).unwrap_or_else(|| (number(end) as i16 - start) as u16);
            GridSpan {
                start: Some(start),
                span: Some(span),
            }
        }
    }
}

/// `flex: none | auto | <grow> [<shrink>] [<basis>] | <basis>`.
fn flex(s: &mut Style, v: &str) {
    let parts: Vec<&str> = v.split_whitespace().collect();
    let is_number = |p: &str| p.parse::<f32>().is_ok();
    let (grow, shrink, basis) = match parts.as_slice() {
        ["none"] => (0.0, 0.0, Length::Auto),
        ["auto"] => (1.0, 1.0, Length::Auto),
        [g] if is_number(g) => (number(g), 1.0, Length::Fraction(0.0)),
        [b] => (1.0, 1.0, length(b)),
        [g, sh] if is_number(sh) => (number(g), number(sh), Length::Fraction(0.0)),
        [g, b] => (number(g), 1.0, length(b)),
        [g, sh, b] => (number(g), number(sh), length(b)),
        _ => panic!("no flex {v:?}"),
    };
    s.flex_grow = grow;
    s.flex_shrink = shrink;
    s.flex_basis = basis;
}

/// Set one CSS property on a gui style.
fn declare(s: &mut Style, property: &str, v: &str) {
    match property {
        "display" => {
            s.display = match v {
                "block" => Display::Block,
                "flex" => Display::Flex,
                "grid" => Display::Grid,
                "none" => Display::None,
                other => panic!("no display {other:?}"),
            }
        }
        "position" => {
            s.position = match v {
                "relative" => Position::Relative,
                "absolute" => Position::Absolute,
                other => panic!("no position {other:?}"),
            }
        }
        "top" => s.inset.top = length(v),
        "right" => s.inset.right = length(v),
        "bottom" => s.inset.bottom = length(v),
        "left" => s.inset.left = length(v),
        "width" => s.width = length(v),
        "height" => s.height = length(v),
        "min-width" => s.min_width = length(v),
        "min-height" => s.min_height = length(v),
        "max-width" => s.max_width = length(v),
        "max-height" => s.max_height = length(v),
        "aspect-ratio" => {
            s.aspect_ratio = Some(match v.split_once('/') {
                Some((w, h)) => number(w) / number(h),
                None => number(v),
            });
        }
        "margin" => s.margin = edges(v, length),
        "margin-top" => s.margin.top = length(v),
        "margin-right" => s.margin.right = length(v),
        "margin-bottom" => s.margin.bottom = length(v),
        "margin-left" => s.margin.left = length(v),
        "padding" => s.padding = edges(v, pixels),
        "padding-top" => s.padding.top = pixels(v),
        "padding-right" => s.padding.right = pixels(v),
        "padding-bottom" => s.padding.bottom = pixels(v),
        "padding-left" => s.padding.left = pixels(v),
        "border-width" => s.border_widths = edges(v, pixels),
        "flex-direction" => {
            s.flex_direction = match v {
                "row" => FlexDirection::Row,
                "column" => FlexDirection::Column,
                "row-reverse" => FlexDirection::RowReverse,
                "column-reverse" => FlexDirection::ColumnReverse,
                other => panic!("no flex-direction {other:?}"),
            }
        }
        "flex-wrap" => s.flex_wrap = v == "wrap",
        "flex-grow" => s.flex_grow = number(v),
        "flex-shrink" => s.flex_shrink = number(v),
        "flex-basis" => s.flex_basis = length(v),
        "flex" => flex(s, v),
        "align-items" => s.align_items = Some(align(v)),
        "align-self" => s.align_self = Some(align(v)),
        "justify-items" => s.justify_items = Some(align(v)),
        "justify-self" => s.justify_self = Some(align(v)),
        "justify-content" => s.justify_content = Some(justify(v)),
        "align-content" => s.align_content = Some(justify(v)),
        "gap" => {
            let parts: Vec<f32> = v.split_whitespace().map(pixels).collect();
            s.row_gap = parts[0];
            s.column_gap = *parts.get(1).unwrap_or(&parts[0]);
        }
        "row-gap" => s.row_gap = pixels(v),
        "column-gap" => s.column_gap = pixels(v),
        "grid-template-columns" => s.grid_columns = tracks(v),
        "grid-template-rows" => s.grid_rows = tracks(v),
        "grid-auto-rows" => s.grid_auto_rows = tracks(v),
        "grid-auto-flow" => s.grid_flow_column = v == "column",
        "grid-column" => s.grid_column = grid_span(v),
        "grid-row" => s.grid_row = grid_span(v),
        "overflow" => {
            s.overflow_x = overflow(v);
            s.overflow_y = overflow(v);
        }
        "overflow-x" => s.overflow_x = overflow(v),
        "overflow-y" => s.overflow_y = overflow(v),
        "font-size" => s.text.size = Some(pixels(v)),
        "line-height" => {
            s.text.line_height = Some(if v.ends_with("px") {
                LineHeight::Px(pixels(v))
            } else {
                LineHeight::Relative(number(v))
            });
        }
        "font-weight" => s.text.weight = Some(number(v) as u16),
        "white-space" => s.text.nowrap = Some(v == "nowrap"),
        "letter-spacing" => {
            s.text.tracking = Some(number(v.strip_suffix("em").expect("letter-spacing in em")));
        }
        other => panic!("the fixtures do not use {other:?}"),
    }
}

fn styled<T: Styled>(mut node: T, style: Option<&str>) -> T {
    for declaration in style.unwrap_or("").split(';') {
        if declaration.trim().is_empty() {
            continue;
        }
        let (property, value) = declaration.split_once(':').expect("a property: value");
        declare(node.style_mut(), property.trim(), value.trim());
    }
    node
}

/// A fixture element as gui's: `<div>` a box, `<p>` a paragraph of the
/// text in it, white space collapsed as CSS collapses it.
fn build(e: roxmltree::Node<'_, '_>) -> Element {
    let style = e.attribute("style");
    match e.tag_name().name() {
        "div" => {
            let mut d = styled(div(), style);
            for c in e.children() {
                if c.is_element() {
                    d = d.child(build(c));
                } else {
                    assert!(
                        c.text().is_none_or(|t| t.trim().is_empty()),
                        "text in a <div> goes in a <p>"
                    );
                }
            }
            d.into_element()
        }
        "p" => {
            let content = e.text().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
            styled(text(content), style).into_element()
        }
        other => panic!("no element <{other}>"),
    }
}

// ── Chrome's boxes ───────────────────────────────────────────────────────────

/// A box of the fixture's tree, in document order.
#[derive(Clone, Debug)]
struct Placed {
    depth: usize,
    tag: String,
    rect: [f32; 4],
}

fn chrome(path: &Path) -> Vec<Placed> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}; run crates/shadcn/verdict/layout.mjs", path.display()));
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let depth = (l.len() - l.trim_start().len()) / 2;
            let words: Vec<&str> = l.split_whitespace().collect();
            let [tag, x, y, w, h] = words.as_slice() else {
                panic!("{}: {l:?} is not `tag x y w h`", path.display());
            };
            Placed {
                depth,
                tag: (*tag).to_owned(),
                rect: [number(x), number(y), number(w), number(h)],
            }
        })
        .collect()
}

/// gui's boxes for the fixture: the frame's nodes below the window's root.
fn ours(nodes: &[FrameNode]) -> Vec<Placed> {
    let mut depth = vec![0_usize; nodes.len()];
    let mut out = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let Some(parent) = n.parent else { continue };
        depth[i] = depth[parent] + 1;
        let tag = match n.content {
            gui::layout::Content::Text { .. } => "p",
            _ => "div",
        };
        out.push(Placed {
            depth: depth[i] - 1,
            tag: tag.to_owned(),
            rect: [n.bounds.x, n.bounds.y, n.bounds.width, n.bounds.height],
        });
    }
    out
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/chrome_layout")
}

fn fixtures() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir())
        .expect("the fixtures")
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".html").map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// Fixture `name` laid out in the window, as a frame's nodes.
fn laid_out(name: &str, ts: &mut TextSystem) -> Vec<FrameNode> {
    let source = std::fs::read_to_string(dir().join(format!("{name}.html"))).expect("a fixture");
    let doc = roxmltree::Document::parse(&source).unwrap_or_else(|e| panic!("{name}.html: {e}"));
    let root = div().child(build(doc.root_element())).into_element();
    layout(&root, WIDTH, HEIGHT, &UiState::default(), &mut Motion::default(), ts).nodes
}

/// Lay fixture `name` out; where it differs from Chrome, node by node:
/// the boxes that differ, and the report of them.
fn compare(name: &str, ts: &mut TextSystem) -> Result<(), (Vec<usize>, String)> {
    let ours = ours(&laid_out(name, ts));
    let theirs = chrome(&dir().join(format!("{name}.chrome")));
    if ours.len() != theirs.len() {
        return Err((Vec::new(), format!("{} boxes, Chrome has {}", ours.len(), theirs.len())));
    }
    let mut out = String::new();
    let mut differ = Vec::new();
    for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        assert!(
            a.depth == b.depth && a.tag == b.tag,
            "{name}: box {i} is a {} at depth {}, Chrome's a {} at depth {}",
            a.tag,
            a.depth,
            b.tag,
            b.depth
        );
        if a.rect.iter().zip(&b.rect).any(|(x, y)| (x - y).abs() > TOLERANCE) {
            differ.push(i);
            writeln!(
                out,
                "  box {i} ({}{}): gui {:?}, Chrome {:?}",
                "  ".repeat(a.depth),
                a.tag,
                a.rect,
                b.rect
            )
            .expect("a String takes text");
        }
    }
    if out.is_empty() { Ok(()) } else { Err((differ, out)) }
}

#[test]
fn layout_matches_chrome() {
    let mut ts = TextSystem::new();
    let names = fixtures();
    assert!(names.len() >= 30, "the fixtures are there: {}", names.len());
    for listed in BETTER.iter().map(|b| b.0).chain(KNOWN_BUGS.iter().map(|b| b.0)) {
        assert!(names.iter().any(|n| n == listed), "no fixture {listed}");
    }
    let mut failures = String::new();
    let (mut matched, mut better, mut bugs) = (0, 0, 0);
    for name in &names {
        let result = compare(name, &mut ts);
        let bug = KNOWN_BUGS.iter().find(|b| b.0 == name);
        let on_purpose = BETTER.iter().find(|b| b.0 == name);
        match (result, bug, on_purpose) {
            (Ok(()), None, None) => matched += 1,
            (Ok(()), _, _) => {
                writeln!(failures, "{name} now matches Chrome: take it off its list").expect("a String takes text");
            }
            (Err(_), Some((_, why)), _) => {
                bugs += 1;
                eprintln!("{name}: a known bug: {why}");
            }
            (Err((differ, _)), None, Some((_, boxes, why))) if differ == *boxes => {
                better += 1;
                eprintln!("{name}: boxes {boxes:?} differ on purpose: {why}");
            }
            (Err((_, diff)), None, listed) => {
                let listed = listed.map_or(String::new(), |b| format!(" (listed as better: boxes {:?})", b.1));
                writeln!(failures, "{name} differs from Chrome{listed}:\n{diff}").expect("a String takes text");
            }
        }
    }
    eprintln!(
        "{} fixtures: {matched} match Chrome, {better} better on purpose, {bugs} known bugs",
        names.len()
    );
    assert!(failures.is_empty(), "{failures}");
}

/// The layout dump (`gui::inspect::dump`) of fixtures with text, grids and
/// absolute boxes, as snapshots (`tests/snapshots/`): what gui lays out, in
/// whole pixels, whatever Chrome does.
#[test]
fn layout_dumps() {
    let mut ts = TextSystem::new();
    for name in [
        "absolute",
        "flex_baseline",
        "flex_text",
        "grid_span",
        "grid_text",
        "text_wrap",
    ] {
        insta::assert_snapshot!(name, gui::inspect::dump(&laid_out(name, &mut ts)));
    }
}
