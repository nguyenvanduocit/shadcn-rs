//! Taffy's own layout fixtures (`references/taffy/tests/xml`, 1 689 cases
//! in four variants each), laid out on the retained tree (ADR-0014) and on
//! `TaffyTree`, which must agree: every box (place, size, scrollable
//! overflow), bit for bit, cold, laid out
//! again after a change, in parallel and with jobs in reverse order.
//!
//! The fixtures live in the reference checkout, which CI does not have: the
//! test passes vacuously without it. Values are read by the small parser
//! below (Taffy's own needs `cssparser`, whose licence the workspace does
//! not take); both trees get the same styles, so a value it reads loosely
//! still tests that the two agree.

#![expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::expect_used,
    reason = "a test harness: fixture numbers are small, and a fixture that does not parse is a failed test"
)]

use std::path::{Path, PathBuf};

use gui::layout::engine::{Measure, MeasureReads, Measured, Node, Schedule};
use taffy::prelude::*;
use taffy::{
    AlignContent, AlignItems, BoxSizing, Clear, Float, GridTemplateComponent, GridTemplateRepetition, Layout,
    LayoutInput, LayoutOutput, MinMax, Overflow, Point, RepetitionCount, ResolveOrZero, TaffyTree,
};

type Style = taffy::Style<String>;

// ── Measuring ────────────────────────────────────────────────────────────────

/// Ahem text as Taffy's test helpers measure it: 10 px squares, lines
/// broken only at zero-width spaces.
#[derive(Clone, Debug)]
struct Ahem {
    text: String,
    vertical: bool,
}

impl Ahem {
    fn size(&self, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Size<f32> {
        if let Size {
            width: Some(width),
            height: Some(height),
        } = known
        {
            return Size { width, height };
        }
        let lines: Vec<&str> = self.text.split('\u{200B}').collect();
        let min_line = lines.iter().map(|l| l.len()).max().unwrap_or(0) as f32 * 10.0;
        let max_line = lines.iter().map(|l| l.len()).sum::<usize>() as f32 * 10.0;
        let (known_inline, space, known_block) = if self.vertical {
            (known.height, available.height, known.width)
        } else {
            (known.width, available.width, known.height)
        };
        let inline = known_inline
            .unwrap_or(match space {
                AvailableSpace::MinContent => min_line,
                AvailableSpace::MaxContent => max_line,
                AvailableSpace::Definite(v) => v.min(max_line),
            })
            .max(min_line);
        let block = known_block.unwrap_or_else(|| {
            let fits = (inline / 10.0).floor() as usize;
            let (mut count, mut current) = (1, 0);
            for line in &lines {
                if current + line.len() > fits {
                    if current > 0 {
                        count += 1;
                    }
                    current = line.len();
                } else {
                    current += line.len();
                }
            }
            count as f32 * 10.0
        });
        if self.vertical {
            Size {
                width: block,
                height: inline,
            }
        } else {
            Size {
                width: inline,
                height: block,
            }
        }
    }

    /// Its baselines below its content box's top, `height` tall.
    fn baselines(&self, height: f32) -> taffy::Baselines {
        if self.vertical {
            return taffy::Baselines::NONE;
        }
        // Ahem at 10px, line-height 1: ascent 8, descent 2.
        let first = 8.0;
        let last = height - 2.0;
        taffy::Baselines {
            first: Some(first),
            last: Some(last.max(first)),
        }
    }
}

impl Measure for Ahem {
    type Cx = ();

    // Horizontal text breaks lines in the width alone; vertical text in the height.
    fn reads(&self) -> MeasureReads {
        if self.vertical {
            MeasureReads::ALL
        } else {
            MeasureReads::WIDTH_ONLY
        }
    }

    fn measure(&self, (): &mut (), known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Measured {
        let s = self.size(known, available);
        let size = Size {
            width: known.width.unwrap_or(s.width),
            height: known.height.unwrap_or(s.height),
        };
        Measured {
            size,
            baselines: self.baselines(size.height),
        }
    }
}

/// Taffy's measure function for the same leaves.
fn taffy_measure(input: LayoutInput, _: NodeId, ahem: Option<&mut Ahem>, style: &Style) -> LayoutOutput {
    let ahem = ahem.map(|a| a.clone());
    let mut baselines = taffy::Baselines::NONE;
    let mut out = taffy::compute_leaf_layout(
        input,
        style,
        |_, _| 0.0,
        |known, available| match &ahem {
            Some(a) => {
                let m = a.measure(&mut (), known, available);
                baselines = m.baselines;
                m.size
            }
            None => known.map(|d| d.unwrap_or(0.0)),
        },
    );
    let basis = input.parent_size.width;
    let top = style
        .padding
        .top
        .resolve_or_zero(Some(basis.unwrap_or(0.0)), |_, _| 0.0)
        + style.border.top.resolve_or_zero(Some(basis.unwrap_or(0.0)), |_, _| 0.0);
    out.baselines = taffy::Baselines {
        first: baselines.first.map(|b| b + top),
        last: baselines.last.map(|b| b + top),
    };
    out
}

// ── Values ───────────────────────────────────────────────────────────────────

fn number(v: &str) -> Option<f32> {
    v.trim().parse().ok()
}

/// `10px`, `10`, `50%`, `auto` and the sizing keywords, as a size.
fn dimension(v: &str) -> Option<Dimension> {
    let v = v.trim();
    Some(match v {
        "auto" => Dimension::auto(),
        "min-content" => Dimension::min_content(),
        "max-content" => Dimension::max_content(),
        "fit-content" => Dimension::fit_content(),
        _ => {
            if let Some(p) = v.strip_suffix('%') {
                Dimension::percent(number(p)? / 100.0)
            } else if let Some(inner) = v.strip_prefix("fit-content(").and_then(|r| r.strip_suffix(')')) {
                match length_percentage(inner)?.into_raw() {
                    raw if raw.tag() == taffy::CompactLength::PERCENT_TAG => {
                        Dimension::fit_content_percent(raw.value())
                    }
                    raw => Dimension::fit_content_px(raw.value()),
                }
            } else {
                Dimension::length(number(v.strip_suffix("px").unwrap_or(v))?)
            }
        }
    })
}

fn length_percentage(v: &str) -> Option<LengthPercentage> {
    let v = v.trim();
    Some(match v.strip_suffix('%') {
        Some(p) => LengthPercentage::percent(number(p)? / 100.0),
        None => LengthPercentage::length(number(v.strip_suffix("px").unwrap_or(v))?),
    })
}

fn length_percentage_auto(v: &str) -> Option<LengthPercentageAuto> {
    if v.trim() == "auto" {
        return Some(LengthPercentageAuto::auto());
    }
    Some(length_percentage(v)?.into())
}

fn available(v: Option<&str>) -> AvailableSpace {
    match v.map(str::trim) {
        None | Some("max-content") => AvailableSpace::MaxContent,
        Some("min-content") => AvailableSpace::MinContent,
        Some(v) => {
            number(v.strip_suffix("px").unwrap_or(v)).map_or(AvailableSpace::MaxContent, AvailableSpace::Definite)
        }
    }
}

fn align(v: &str) -> Option<AlignItems> {
    Some(match v.trim() {
        "start" => AlignItems::START,
        "end" => AlignItems::END,
        "flex-start" => AlignItems::FLEX_START,
        "self-start" => AlignItems::SELF_START,
        "flex-end" => AlignItems::FLEX_END,
        "self-end" => AlignItems::SELF_END,
        "center" => AlignItems::CENTER,
        "baseline" => AlignItems::BASELINE,
        "stretch" => AlignItems::STRETCH,
        "normal" => AlignItems::NORMAL,
        _ => return None,
    })
}

fn align_content(v: &str) -> Option<AlignContent> {
    Some(match v.trim() {
        "start" => AlignContent::START,
        "end" => AlignContent::END,
        "flex-start" => AlignContent::FLEX_START,
        "flex-end" => AlignContent::FLEX_END,
        "center" => AlignContent::CENTER,
        "stretch" => AlignContent::STRETCH,
        "space-between" => AlignContent::SPACE_BETWEEN,
        "space-evenly" => AlignContent::SPACE_EVENLY,
        "space-around" => AlignContent::SPACE_AROUND,
        "normal" => AlignContent::NORMAL,
        _ => return None,
    })
}

/// One track sizing function: a length, `fr`, a keyword, `minmax()` or `fit-content()`.
fn track(v: &str) -> Option<TrackSizingFunction> {
    let v = v.trim();
    if let Some(inner) = v.strip_prefix("minmax(").and_then(|r| r.strip_suffix(')')) {
        let (min, max) = inner.split_once(',')?;
        let min = match min.trim() {
            "auto" => MinTrackSizingFunction::auto(),
            "min-content" => MinTrackSizingFunction::min_content(),
            "max-content" => MinTrackSizingFunction::max_content(),
            m => MinTrackSizingFunction::from(length_percentage(m)?),
        };
        return Some(MinMax {
            min,
            max: max_track(max)?,
        });
    }
    if let Some(inner) = v.strip_prefix("fit-content(").and_then(|r| r.strip_suffix(')')) {
        let lp = length_percentage(inner)?;
        return Some(MinMax {
            min: MinTrackSizingFunction::auto(),
            max: MaxTrackSizingFunction::fit_content(lp),
        });
    }
    let max = max_track(v)?;
    let min = match v {
        _ if v.ends_with("fr") => MinTrackSizingFunction::auto(),
        "auto" => MinTrackSizingFunction::auto(),
        "min-content" => MinTrackSizingFunction::min_content(),
        "max-content" => MinTrackSizingFunction::max_content(),
        _ => MinTrackSizingFunction::from(length_percentage(v)?),
    };
    Some(MinMax { min, max })
}

fn max_track(v: &str) -> Option<MaxTrackSizingFunction> {
    let v = v.trim();
    Some(match v {
        "auto" => MaxTrackSizingFunction::auto(),
        "min-content" => MaxTrackSizingFunction::min_content(),
        "max-content" => MaxTrackSizingFunction::max_content(),
        _ => match v.strip_suffix("fr") {
            Some(f) => MaxTrackSizingFunction::fr(number(f)?),
            None => MaxTrackSizingFunction::from(length_percentage(v)?),
        },
    })
}

/// Split at top-level whitespace, keeping `(...)` and `[...]` whole.
fn tokens(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0_i32;
    for c in v.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
        if c.is_whitespace() && depth == 0 {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn names(t: &str) -> Option<Vec<String>> {
    let inner = t.strip_prefix('[')?.strip_suffix(']')?;
    Some(inner.split_whitespace().map(str::to_owned).collect())
}

/// A track list with its line names: `[a] 10px repeat(2, [b] 1fr) [c]`.
/// A grid template's tracks, and the line names around them.
type Template = (Vec<GridTemplateComponent<String>>, Vec<Vec<String>>);

fn template(v: &str) -> Option<Template> {
    if v.trim() == "none" {
        return Some((Vec::new(), Vec::new()));
    }
    let mut tracks = Vec::new();
    let mut line_names: Vec<Vec<String>> = vec![Vec::new()];
    for t in tokens(v) {
        if let Some(n) = names(&t) {
            line_names.last_mut()?.extend(n);
        } else if let Some(inner) = t.strip_prefix("repeat(").and_then(|r| r.strip_suffix(')')) {
            let (count, body) = inner.split_once(',')?;
            let count = match count.trim() {
                "auto-fill" => RepetitionCount::AutoFill,
                "auto-fit" => RepetitionCount::AutoFit,
                n => RepetitionCount::Count(n.parse().ok()?),
            };
            let mut inner_tracks = Vec::new();
            let mut inner_names: Vec<Vec<String>> = vec![Vec::new()];
            for t in tokens(body) {
                if let Some(n) = names(&t) {
                    inner_names.last_mut()?.extend(n);
                } else {
                    inner_tracks.push(track(&t)?);
                    inner_names.push(Vec::new());
                }
            }
            let named = inner_names.iter().any(|n| !n.is_empty());
            tracks.push(GridTemplateComponent::Repeat(GridTemplateRepetition {
                count,
                tracks: inner_tracks,
                line_names: if named { inner_names } else { Vec::new() },
            }));
            line_names.push(Vec::new());
        } else {
            tracks.push(GridTemplateComponent::Single(track(&t)?));
            line_names.push(Vec::new());
        }
    }
    let named = line_names.iter().any(|n| !n.is_empty());
    Some((tracks, if named { line_names } else { Vec::new() }))
}

fn placement(v: &str) -> Option<GridPlacement<String>> {
    let parts: Vec<&str> = v.split_whitespace().collect();
    Some(match parts.as_slice() {
        ["auto"] | [] => GridPlacement::Auto,
        ["span", n] => match n.parse::<u16>() {
            Ok(n) => GridPlacement::Span(n),
            Err(_) => GridPlacement::NamedSpan((*n).to_owned(), 1),
        },
        ["span", n, name] | ["span", name, n] if n.parse::<u16>().is_ok() => {
            GridPlacement::NamedSpan((*name).to_owned(), n.parse().ok()?)
        }
        [n] => match n.parse::<i16>() {
            Ok(n) => GridPlacement::from_line_index(n),
            Err(_) => GridPlacement::NamedLine((*n).to_owned(), 0),
        },
        [n, name] | [name, n] if n.parse::<i16>().is_ok() => {
            GridPlacement::NamedLine((*name).to_owned(), n.parse().ok()?)
        }
        _ => return None,
    })
}

/// A fixture node's style, as the XML gives it.
fn style(x: roxmltree::Node<'_, '_>) -> Style {
    let a = |name: &str| x.attribute(name);
    let lpa = |name: &str, d: LengthPercentageAuto| a(name).and_then(length_percentage_auto).unwrap_or(d);
    let lp = |name: &str| a(name).and_then(length_percentage).unwrap_or(LengthPercentage::ZERO);
    let dim = |name: &str| a(name).and_then(dimension).unwrap_or(Dimension::auto());
    let (columns, column_names) = a("grid-template-columns").and_then(template).unwrap_or_default();
    let (rows, row_names) = a("grid-template-rows").and_then(template).unwrap_or_default();
    let auto_tracks = |name: &str| -> Vec<TrackSizingFunction> {
        a(name)
            .map(|v| tokens(v).iter().filter_map(|t| track(t)).collect())
            .unwrap_or_default()
    };
    let overflow = |name: &str| match a(name) {
        Some("hidden") => Overflow::Hidden,
        Some("scroll") => Overflow::Scroll,
        Some("clip") => Overflow::Clip,
        _ => Overflow::Visible,
    };
    Style {
        display: match a("display") {
            Some("none") => Display::None,
            Some("grid") => Display::Grid,
            Some("block") => Display::Block,
            Some("flow-root") => Display::FlowRoot,
            _ => Display::Flex,
        },
        direction: match a("direction") {
            Some("rtl") => taffy::Direction::Rtl,
            _ => taffy::Direction::Ltr,
        },
        box_sizing: match a("box-sizing") {
            Some("content-box") => BoxSizing::ContentBox,
            _ => BoxSizing::BorderBox,
        },
        overflow: Point {
            x: overflow("overflow-x"),
            y: overflow("overflow-y"),
        },
        scrollbar_width: a("scrollbar-width").and_then(number).unwrap_or(0.0),
        float: match a("float") {
            Some("left") => Float::Left,
            Some("right") => Float::Right,
            _ => Float::None,
        },
        clear: match a("clear") {
            Some("left") => Clear::Left,
            Some("right") => Clear::Right,
            Some("both") => Clear::Both,
            _ => Clear::None,
        },
        position: match a("position") {
            Some("absolute") => Position::Absolute,
            _ => Position::Relative,
        },
        size: Size {
            width: dim("width"),
            height: dim("height"),
        },
        min_size: Size {
            width: lpa("min-width", LengthPercentageAuto::auto()),
            height: lpa("min-height", LengthPercentageAuto::auto()),
        },
        max_size: Size {
            width: lpa("max-width", LengthPercentageAuto::auto()),
            height: lpa("max-height", LengthPercentageAuto::auto()),
        },
        inset: Rect {
            top: lpa("top", LengthPercentageAuto::auto()),
            left: lpa("left", LengthPercentageAuto::auto()),
            bottom: lpa("bottom", LengthPercentageAuto::auto()),
            right: lpa("right", LengthPercentageAuto::auto()),
        },
        margin: Rect {
            top: lpa("margin-top", LengthPercentageAuto::ZERO),
            left: lpa("margin-left", LengthPercentageAuto::ZERO),
            bottom: lpa("margin-bottom", LengthPercentageAuto::ZERO),
            right: lpa("margin-right", LengthPercentageAuto::ZERO),
        },
        padding: Rect {
            top: lp("padding-top"),
            left: lp("padding-left"),
            bottom: lp("padding-bottom"),
            right: lp("padding-right"),
        },
        border: Rect {
            top: lp("border-top"),
            left: lp("border-left"),
            bottom: lp("border-bottom"),
            right: lp("border-right"),
        },
        gap: Size {
            width: lp("column-gap"),
            height: lp("row-gap"),
        },
        aspect_ratio: a("aspect-ratio").and_then(|v| match v.split_once('/') {
            Some((w, h)) => Some(number(w)? / number(h)?),
            None => number(v),
        }),
        align_items: a("align-items").and_then(align).unwrap_or(AlignItems::NORMAL),
        align_self: a("align-self").and_then(align),
        justify_items: a("justify-items").and_then(align).unwrap_or(AlignItems::NORMAL),
        justify_self: a("justify-self").and_then(align),
        align_content: a("align-content")
            .and_then(align_content)
            .unwrap_or(AlignContent::NORMAL),
        justify_content: a("justify-content")
            .and_then(align_content)
            .unwrap_or(AlignContent::NORMAL),
        flex_direction: match a("flex-direction") {
            Some("column") => FlexDirection::Column,
            Some("row-reverse") => FlexDirection::RowReverse,
            Some("column-reverse") => FlexDirection::ColumnReverse,
            _ => FlexDirection::Row,
        },
        flex_wrap: match a("flex-wrap") {
            Some("wrap") => FlexWrap::Wrap,
            Some("wrap-reverse") => FlexWrap::WrapReverse,
            Some("balance") => FlexWrap::Balance,
            Some("balance-reverse") => FlexWrap::BalanceReverse,
            _ => FlexWrap::NoWrap,
        },
        flex_line_count: a("flex-line-count").and_then(|v| v.trim().parse().ok()).unwrap_or(1),
        flex_grow: a("flex-grow").and_then(number).unwrap_or(0.0),
        flex_shrink: a("flex-shrink").and_then(number).unwrap_or(1.0),
        flex_basis: dim("flex-basis"),
        grid_auto_flow: match a("grid-auto-flow").map(str::trim) {
            Some("column") => GridAutoFlow::Column,
            Some("row dense" | "dense") => GridAutoFlow::RowDense,
            Some("column dense") => GridAutoFlow::ColumnDense,
            _ => GridAutoFlow::Row,
        },
        grid_template_rows: rows,
        grid_template_row_names: row_names,
        grid_template_columns: columns,
        grid_template_column_names: column_names,
        grid_auto_rows: auto_tracks("grid-auto-rows"),
        grid_auto_columns: auto_tracks("grid-auto-columns"),
        grid_row: Line {
            start: a("grid-row-start").and_then(placement).unwrap_or_default(),
            end: a("grid-row-end").and_then(placement).unwrap_or_default(),
        },
        grid_column: Line {
            start: a("grid-column-start").and_then(placement).unwrap_or_default(),
            end: a("grid-column-end").and_then(placement).unwrap_or_default(),
        },
        ..Style::default()
    }
}

// ── Trees ────────────────────────────────────────────────────────────────────

type Ours = Node<Style, Ahem>;

fn leaf_of(x: roxmltree::Node<'_, '_>) -> Option<Ahem> {
    let text = x.text().map(str::trim)?;
    Some(Ahem {
        text: text.to_owned(),
        vertical: x.attribute("writing-mode").is_some_and(|w| w.contains("vertical")),
    })
}

fn build_ours(x: roxmltree::Node<'_, '_>) -> Ours {
    let kids: Vec<_> = x.children().filter(roxmltree::Node::is_element).collect();
    if kids.is_empty() {
        return Node::new(style(x), leaf_of(x));
    }
    let mut n = Node::new(style(x), None);
    n.children = kids.into_iter().map(build_ours).collect();
    n
}

fn build_taffy(x: roxmltree::Node<'_, '_>, tree: &mut TaffyTree<Ahem>) -> NodeId {
    let kids: Vec<_> = x.children().filter(roxmltree::Node::is_element).collect();
    if kids.is_empty() {
        let id = tree.new_leaf(style(x)).expect("leaf");
        if let Some(a) = leaf_of(x) {
            tree.set_node_context(id, Some(a)).expect("context");
        }
        return id;
    }
    let ids: Vec<NodeId> = kids.into_iter().map(|k| build_taffy(k, tree)).collect();
    tree.new_with_children(style(x), &ids).expect("node")
}

fn same(a: &Layout, b: &Layout) -> bool {
    a.location == b.location && a.size == b.size && a.scrollable_overflow_rect == b.scrollable_overflow_rect
}

/// Where `ours` and Taffy's tree first differ, if they do.
fn compare(ours: &Ours, tree: &TaffyTree<Ahem>, id: NodeId, at: &str) -> Option<String> {
    let theirs = tree.layout(id).ok()?;
    if !same(ours.layout(), theirs) {
        return Some(format!(
            "{at}: ours {:?} {:?}, Taffy's {:?} {:?}",
            ours.layout().location,
            ours.layout().size,
            theirs.location,
            theirs.size
        ));
    }
    let kids = tree.children(id).ok()?;
    if kids.len() != ours.children.len() {
        return Some(format!("{at}: child count"));
    }
    ours.children
        .iter()
        .zip(kids)
        .enumerate()
        .find_map(|(i, (c, k))| compare(c, tree, k, &format!("{at}/{i}")))
}

/// Every node of `n`'s subtree, depth first, with `f`.
fn each(n: &mut Ours, f: &mut impl FnMut(&mut Ours)) {
    f(n);
    for c in &mut n.children {
        each(c, f);
    }
}

fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../references/taffy/tests/xml");
    let mut out = Vec::new();
    let Ok(groups) = std::fs::read_dir(&root) else {
        return out;
    };
    for g in groups.flatten() {
        if let Ok(files) = std::fs::read_dir(g.path()) {
            out.extend(
                files
                    .flatten()
                    .map(|f| f.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "xml")),
            );
        }
    }
    out.sort();
    out
}

/// Lay one fixture out every way; a description of the first mismatch.
fn run(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let doc = roxmltree::Document::parse(&raw).ok()?;
    let root = doc.root_element();
    let viewport = root.children().find(|n| n.has_tag_name("viewport"))?;
    let input = root
        .children()
        .find(|n| n.has_tag_name("input"))?
        .first_element_child()?;
    let space = Size {
        width: available(viewport.attribute("width")),
        height: available(viewport.attribute("height")),
    };

    let mut tree = TaffyTree::<Ahem>::new();
    tree.disable_rounding();
    let id = build_taffy(input, &mut tree);
    tree.compute_layout_with_measure(id, space, taffy_measure).ok()?;

    let schedules = [
        Schedule::default(),
        Schedule {
            parallel: true,
            reverse: false,
        },
        Schedule {
            parallel: false,
            reverse: true,
        },
    ];
    for schedule in schedules {
        let mut ours = build_ours(input);
        ours.recount();
        ours.compute_root(space, &mut (), schedule);
        if let Some(d) = compare(&ours, &tree, id, "") {
            return Some(format!("cold {schedule:?}{d}"));
        }
        // Nothing changed: one call, answered from the cache.
        let stats = ours.compute_root(space, &mut (), schedule);
        if stats.misses != 0 {
            return Some(format!(
                "{schedule:?}: an unchanged tree ran {} algorithms",
                stats.misses
            ));
        }
        // Each node in turn changed (its cache dropped, as a style change
        // does), and settled as the tree's owner settles it: the layout
        // equals the cold one.
        let mut count = 0_usize;
        each(&mut ours, &mut |_| count += 1);
        for target in 0..count {
            let mut i = 0_usize;
            settle(&mut ours, target, &mut i);
            ours.compute_root(space, &mut (), schedule);
            if let Some(d) = compare(&ours, &tree, id, "") {
                return Some(format!("after node {target} changed {schedule:?}{d}"));
            }
        }
    }
    None
}

/// Settle the tree with node `target` (in depth-first order) changed in
/// content; how far its change reaches.
fn settle(n: &mut Ours, target: usize, i: &mut usize) -> gui::layout::engine::Change {
    let me = *i;
    *i += 1;
    let mut reach = gui::layout::engine::Change::None;
    for c in &mut n.children {
        reach = reach.max(settle(c, target, i));
    }
    n.settle(false, me == target, reach)
}

#[test]
fn taffy_fixtures_lay_out_on_the_retained_tree_as_on_taffy_tree() {
    // Every flex column keeps its items' answers, so each change below
    // goes through what is kept.
    gui::layout::engine::set_memo_min(0);
    let all = fixtures();
    if all.is_empty() {
        eprintln!("no fixtures (references/taffy is not checked out): skipped");
        return;
    }
    let failed: Vec<String> = all
        .iter()
        .filter_map(|p| {
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            run(p).map(|d| format!("{name}: {d}"))
        })
        .collect();
    eprintln!("{} fixtures, {} differ", all.len(), failed.len());
    assert!(
        failed.is_empty(),
        "{} of {} differ:\n{}",
        failed.len(),
        all.len(),
        failed.iter().take(400).cloned().collect::<Vec<_>>().join("\n")
    );
}
