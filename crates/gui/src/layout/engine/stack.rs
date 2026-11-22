//! A flex column of stretched items, laid out by Taffy's flexbox algorithm
//! step for step and remembered per item (ADR-0014).
//!
//! Taffy lays a flex container out by asking each item the same questions
//! again on every run: its style, its flex base size, its min-content size,
//! its final layout. In a column of 5 000 rows one row growing reruns all of
//! that, and the parent's run is the whole cost of the change. This is that
//! algorithm (`compute_flexbox_layout`, the column case of every step it
//! takes) with what each item answered kept beside the container, keyed by
//! the container's input: an item whose cache was not cleared since (its
//! [`super::cache::NodeCache::generation`]) is not asked again, and only the
//! sums, the clamps and the positions run over all items, in Taffy's own
//! order of float operations, so the result is bit-identical to Taffy's.
//!
//! It takes the case where every step is the column one: `flex-direction:
//! column`, `nowrap`, left to right, the container's width known, and every
//! item in flow, stretched across it (`width: auto`, `align-self: stretch`),
//! with no auto margin, no aspect ratio and no sizing keyword. Anything else
//! is Taffy's own algorithm, before any item is laid out.

use taffy::{
    AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignSelf, AvailableSpace, Baselines,
    BoxGenerationMode, BoxSizing, Contain, CoreStyle, Direction, FlexDirection, FlexWrap, FlexboxContainerStyle,
    FlexboxItemStyle, JustifyContent, Layout, LayoutInput, LayoutOutput, LengthPercentageAuto, Line, MaybeMath,
    MaybeResolve, OofCandidates, OofPositioningArea, Overflow, Point, Position, Rect, RequestedAxis, ResolveOrZero,
    RunMode, Size, SizingMode,
};

use super::{Measure, NodeStyle, View};
use crate::convert::usize_to_f32;

/// Fewest items a column keeps its items' answers for; a shorter one runs
/// the same algorithm with nothing kept (asking a few items again costs
/// less than keeping their answers).
static MEMO_MIN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(32);
/// How many of the container's inputs it keeps answers for (a parent asks
/// a flex base size, a min-content size and a final layout).
const ENTRIES: usize = 4;

/// Keep the answers of columns of at least `items` items (tests set it to 0,
/// so every column they lay out goes through what is kept).
#[doc(hidden)]
pub fn set_memo_min(items: usize) {
    MEMO_MIN.store(items, std::sync::atomic::Ordering::Relaxed);
}

/// What a column keeps of its items' answers, per input it was given.
#[derive(Debug, Default)]
pub struct Memo {
    entries: Vec<Entry>,
    clock: u32,
}

#[derive(Debug)]
struct Entry {
    input: LayoutInput,
    used: u32,
    items: Vec<Item>,
}

/// One item as the column's run for an input saw it: what its style
/// resolved to and what it answered.
#[derive(Clone, Debug, Default)]
struct Item {
    /// The item's cache generation these answers came from; `None`: not filled.
    generation: Option<u32>,
    margin: Rect<f32>,
    padding: Rect<f32>,
    border: Rect<f32>,
    /// Its resolved height (`size.main`).
    height: Option<f32>,
    min: Size<Option<f32>>,
    max: Size<Option<f32>>,
    inset: Size<f32>,
    overflow: Point<Overflow>,
    contain: Contain,
    scrollbar_width: f32,
    grow: f32,
    shrink: f32,
    reads: Size<bool>,
    baseline: bool,
    flex_basis: f32,
    basis_is_definite: bool,
    resolved_min: f32,
    /// Its content contribution under a min- or max-content constraint.
    contribution: Option<f32>,
    /// It was asked a size probe for these answers: reusing them uses
    /// more of it than a final layout at a fixed size (not a boundary).
    probed: bool,
    last: Option<Final>,
}

/// An item's final layout as it answered it.
#[derive(Clone, Copy, Debug)]
struct Final {
    input: LayoutInput,
    /// The item's cache serial once it answered: it is still placed for it.
    serial: u32,
    size: Size<f32>,
    overflow: Rect<f32>,
    first_baseline: Option<f32>,
}

/// What the run works out per item.
#[derive(Clone, Copy, Debug, Default)]
struct Flex {
    hypothetical_inner: f32,
    hypothetical_outer: f32,
    inner_flex_basis: f32,
    content_flex_fraction: f32,
    target: Size<f32>,
    outer_target: Size<f32>,
    frozen: bool,
    violation: f32,
    offset_main: f32,
    baseline: f32,
}

/// The container's constants (Taffy's `AlgoConstants`, the column case).
struct Constants {
    min_size: Size<Option<f32>>,
    max_size: Size<Option<f32>>,
    border: Rect<f32>,
    content_box_inset: Rect<f32>,
    scrollbar_gutter: Point<f32>,
    is_scroll_container: bool,
    gap: Size<f32>,
    align_items: AlignItems,
    align_content: AlignContent,
    justify_content: JustifyContent,
    node_outer_size: Size<Option<f32>>,
    node_inner_size: Size<Option<f32>>,
    known_main_size_is_definite: bool,
    has_definite_main_size: bool,
    container_size: Size<f32>,
    inner_container_size: Size<f32>,
}

/// `a + b` as Taffy's `Rect` adds, side by side.
fn rect_add(a: Rect<f32>, b: Rect<f32>) -> Rect<f32> {
    Rect {
        left: a.left + b.left,
        right: a.right + b.right,
        top: a.top + b.top,
        bottom: a.bottom + b.bottom,
    }
}

/// `a + b` as Taffy's `Size` adds.
fn size_add(a: Size<f32>, b: Size<f32>) -> Size<f32> {
    Size {
        width: a.width + b.width,
        height: a.height + b.height,
    }
}

/// `a - b` as Taffy's `Size` subtracts.
fn size_sub(a: Size<f32>, b: Size<f32>) -> Size<f32> {
    Size {
        width: a.width - b.width,
        height: a.height - b.height,
    }
}

fn resolve_normal_alignment(alignment: AlignItems) -> AlignItems {
    match alignment.keyword {
        AlignItemsKeyword::Normal => AlignItems::STRETCH,
        _ => alignment,
    }
}

fn sum_axis_gaps(gap: f32, items: usize) -> f32 {
    if items <= 1 {
        0.0
    } else {
        gap * usize_to_f32(items.saturating_sub(1))
    }
}

/// Taffy's `apply_alignment_fallback`.
fn alignment_fallback(free_space: f32, items: usize, mode: AlignContent) -> AlignContentKeyword {
    let mut keyword = match mode.keyword {
        AlignContentKeyword::Normal => AlignContentKeyword::Stretch,
        keyword => keyword,
    };
    let mut is_safe = mode.is_safe();
    if items <= 1 || free_space <= 0.0 {
        (keyword, is_safe) = match keyword {
            AlignContentKeyword::Stretch | AlignContentKeyword::SpaceBetween => (AlignContentKeyword::FlexStart, true),
            AlignContentKeyword::SpaceAround | AlignContentKeyword::SpaceEvenly => (AlignContentKeyword::Center, true),
            other => (other, is_safe),
        };
    }
    if free_space <= 0.0 && is_safe {
        keyword = AlignContentKeyword::Start;
    }
    keyword
}

/// Taffy's `compute_alignment_offset`.
fn alignment_offset(free_space: f32, items: usize, gap: f32, mode: AlignContentKeyword, first: bool) -> f32 {
    let n = usize_to_f32(items);
    if first {
        match mode {
            AlignContentKeyword::Start
            | AlignContentKeyword::FlexStart
            | AlignContentKeyword::Normal
            | AlignContentKeyword::Stretch
            | AlignContentKeyword::SpaceBetween => 0.0,
            AlignContentKeyword::End | AlignContentKeyword::FlexEnd => free_space,
            AlignContentKeyword::Center => free_space / 2.0,
            AlignContentKeyword::SpaceAround => {
                if free_space >= 0.0 {
                    (free_space / n) / 2.0
                } else {
                    free_space / 2.0
                }
            }
            AlignContentKeyword::SpaceEvenly => {
                if free_space >= 0.0 {
                    free_space / usize_to_f32(items.saturating_add(1))
                } else {
                    free_space / 2.0
                }
            }
        }
    } else {
        let free_space = free_space.max(0.0);
        gap + match mode {
            AlignContentKeyword::Start
            | AlignContentKeyword::FlexStart
            | AlignContentKeyword::End
            | AlignContentKeyword::FlexEnd
            | AlignContentKeyword::Center
            | AlignContentKeyword::Normal
            | AlignContentKeyword::Stretch => 0.0,
            AlignContentKeyword::SpaceBetween => free_space / usize_to_f32(items.saturating_sub(1)),
            AlignContentKeyword::SpaceAround => free_space / n,
            AlignContentKeyword::SpaceEvenly => free_space / usize_to_f32(items.saturating_add(1)),
        }
    }
}

/// Taffy's `child_first_baseline`.
fn child_first_baseline(first: Option<f32>, height: f32, overflow_y: Overflow) -> Option<f32> {
    if overflow_y.is_scroll_container() {
        Some(first.unwrap_or(height).min(height).max(0.0))
    } else {
        first
    }
}

/// Taffy's `compute_scrollable_overflow_contribution`.
fn overflow_contribution(
    location: Point<f32>,
    size: Size<f32>,
    rect: Rect<f32>,
    overflow: Point<Overflow>,
    contain: Contain,
    parent_is_scroll_container: bool,
) -> Rect<f32> {
    let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();
    let contained = contain.contains_scrollable_overflow();
    let propagates = Point {
        x: !is_scroll_container && !contained && overflow.x == Overflow::Visible,
        y: !is_scroll_container && !contained && overflow.y == Overflow::Visible,
    };
    let end = Size {
        width: if propagates.x {
            size.width.max(rect.right)
        } else {
            size.width
        },
        height: if propagates.y {
            size.height.max(rect.bottom)
        } else {
            size.height
        },
    };
    if end.width <= 0.0 || end.height <= 0.0 {
        return Rect::ZERO;
    }
    let start = Point {
        x: if propagates.x { 0.0_f32.min(rect.left) } else { 0.0 },
        y: if propagates.y { 0.0_f32.min(rect.top) } else { 0.0 },
    };
    let contribution = Rect {
        left: location.x + start.x,
        right: location.x + end.width,
        top: location.y + start.y,
        bottom: location.y + end.height,
    };
    let unreachable = contribution.right <= 0.0 || contribution.bottom <= 0.0;
    if parent_is_scroll_container && unreachable {
        Rect::ZERO
    } else {
        contribution
    }
}

/// Whether a final layout cached for `old` answers `new`, as the item's cache would.
fn answers(old: &LayoutInput, new: &LayoutInput, reads: Size<bool>) -> bool {
    let bits = |v: Option<f32>| v.map(f32::to_bits);
    bits(old.known_dimensions.width) == bits(new.known_dimensions.width)
        && bits(old.known_dimensions.height) == bits(new.known_dimensions.height)
        && old.known_dimensions_are_definite == new.known_dimensions_are_definite
        && (!reads.width || bits(old.parent_size.width) == bits(new.parent_size.width))
        && (!reads.height || bits(old.parent_size.height) == bits(new.parent_size.height))
}

/// Lay `view`'s node out as a flex column for `inputs`, as
/// `taffy::compute_flexbox_layout` would; `None` when it is not a column
/// this takes (before anything was laid out: Taffy's own algorithm runs).
pub(super) fn compute<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    inputs: LayoutInput,
) -> Option<LayoutOutput> {
    let LayoutInput {
        known_dimensions,
        parent_size,
        run_mode,
        ..
    } = inputs;
    let calc = S::calc;
    let (styled_known, contain) = {
        let style = view.node.style.view();
        if style.flex_direction() != FlexDirection::Column
            || style.flex_wrap() != FlexWrap::NoWrap
            || style.direction() != Direction::Ltr
        {
            return None;
        }
        let contain = style.contain();
        let aspect_ratio = style.aspect_ratio();
        let padding = style.padding().resolve_or_zero(parent_size.width, calc);
        let border = style.border().resolve_or_zero(parent_size.width, calc);
        let padding_border_sum = size_add(padding.sum_axes(), border.sum_axes());
        let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
            padding_border_sum
        } else {
            Size::ZERO
        };
        let min_size = style
            .min_size()
            .maybe_resolve(parent_size, calc)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment);
        let max_size = style
            .max_size()
            .maybe_resolve(parent_size, calc)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment);
        let clamped_style_size = if inputs.sizing_mode == SizingMode::InherentSize {
            style
                .size()
                .maybe_resolve(parent_size, calc)
                .maybe_apply_aspect_ratio(aspect_ratio)
                .maybe_add(box_sizing_adjustment)
                .maybe_clamp(min_size, max_size)
        } else {
            Size::NONE
        };
        let min_max_definite_size = min_size.zip_map(max_size, |min, max| match (min, max) {
            (Some(min), Some(max)) if max <= min => Some(min),
            _ => None,
        });
        let styled = known_dimensions.or(min_max_definite_size
            .or(clamped_style_size)
            .maybe_max(padding_border_sum));
        (styled, contain)
    };
    if run_mode == RunMode::ComputeSize {
        if let Size {
            width: Some(width),
            height: Some(height),
        } = styled_known
        {
            return Some(LayoutOutput::from_outer_size(Size { width, height }));
        }
        if inputs.axis == RequestedAxis::Horizontal
            && let Some(width) = styled_known.width
        {
            return Some(LayoutOutput::from_outer_size(Size { width, height: 0.0 }));
        }
    }
    // The column case needs the width: the items stretch across it.
    styled_known.width?;
    let definite = inputs
        .known_dimensions_are_definite
        .zip_map(known_dimensions, |d, k| d || k.is_none());
    let mut output = preliminary(
        view,
        LayoutInput {
            known_dimensions: styled_known,
            known_dimensions_are_definite: definite,
            ..inputs
        },
    )?;
    if contain.suppresses_baseline() {
        output.baselines = Baselines::NONE;
    }
    Some(output)
}

/// Taffy's `compute_constants`, the column case.
fn constants<S: NodeStyle>(
    style: &S::Ref<'_>,
    known_dimensions: Size<Option<f32>>,
    definite: Size<bool>,
    parent_size: Size<Option<f32>>,
) -> Constants {
    let calc = S::calc;
    let aspect_ratio = style.aspect_ratio();
    let padding = style.padding().resolve_or_zero(parent_size.width, calc);
    let border = style.border().resolve_or_zero(parent_size.width, calc);
    let padding_border_sum = size_add(padding.sum_axes(), border.sum_axes());
    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
        padding_border_sum
    } else {
        Size::ZERO
    };
    let align_items = resolve_normal_alignment(style.align_items());
    let align_content = match style.align_content() {
        AlignContent {
            keyword: AlignContentKeyword::Normal,
            ..
        } => AlignContent::STRETCH,
        a => a,
    };
    let justify_content = match style.justify_content() {
        JustifyContent {
            keyword: AlignContentKeyword::Normal,
            ..
        } => JustifyContent::FLEX_START,
        j => j,
    };
    let overflow = style.overflow();
    let scrollbar_width = style.scrollbar_width();
    let scrollbar_gutter = Point {
        x: if overflow.y == Overflow::Scroll {
            scrollbar_width
        } else {
            0.0
        },
        y: if overflow.x == Overflow::Scroll {
            scrollbar_width
        } else {
            0.0
        },
    };
    let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();
    let mut content_box_inset = rect_add(padding, border);
    content_box_inset.bottom += scrollbar_gutter.y;
    content_box_inset.right += scrollbar_gutter.x;
    let node_outer_size = known_dimensions;
    let node_inner_size = node_outer_size.maybe_sub(content_box_inset.sum_axes());
    let known_main_size_is_definite = definite.height;
    let gap = style.gap().resolve_or_zero(node_inner_size.or(Size::zero()), calc);
    Constants {
        min_size: style
            .min_size()
            .maybe_resolve(parent_size, calc)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment),
        max_size: style
            .max_size()
            .maybe_resolve(parent_size, calc)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment),
        border,
        content_box_inset,
        scrollbar_gutter,
        is_scroll_container,
        gap,
        align_items,
        align_content,
        justify_content,
        node_outer_size,
        node_inner_size,
        known_main_size_is_definite,
        has_definite_main_size: known_main_size_is_definite && known_dimensions.height.is_some(),
        container_size: Size::zero(),
        inner_container_size: Size::zero(),
    }
}

/// An item's style as Taffy's `generate_anonymous_flex_items` resolves it,
/// or `None` when it is not an item this takes.
fn item_style<S: NodeStyle>(style: &S::Ref<'_>, c: &Constants, reads: Size<bool>, baseline: bool) -> Option<Item> {
    let calc = S::calc;
    if style.position().is_out_of_flow()
        || style.box_generation_mode() == BoxGenerationMode::None
        || style.aspect_ratio().is_some()
    {
        return None;
    }
    let margin_style = style.margin();
    if [
        margin_style.left,
        margin_style.right,
        margin_style.top,
        margin_style.bottom,
    ]
    .iter()
    .any(|m| LengthPercentageAuto::is_auto(*m))
    {
        return None;
    }
    let size_style = style.size();
    let basis_style = style.flex_basis();
    if size_style.height.is_sizing_keyword() || basis_style.is_sizing_keyword() {
        return None;
    }
    let align_self = resolve_normal_alignment(style.align_self().unwrap_or(c.align_items)).resolve_self_relative(
        style.direction(),
        Direction::Ltr,
        true,
    );
    // Stretched across the column, measured at its width.
    if align_self != AlignSelf::STRETCH || !size_style.width.is_auto() {
        return None;
    }
    let percent = if c.known_main_size_is_definite {
        c.node_inner_size
    } else {
        Size {
            width: c.node_inner_size.width,
            height: None,
        }
    };
    let padding = style.padding().resolve_or_zero(c.node_inner_size.width, calc);
    let border = style.border().resolve_or_zero(c.node_inner_size.width, calc);
    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
        rect_add(padding, border).sum_axes()
    } else {
        Size::ZERO
    };
    let size = style
        .size()
        .maybe_resolve(percent, calc)
        .maybe_apply_aspect_ratio(None)
        .maybe_add(box_sizing_adjustment);
    let inset = if style.position() == Position::Relative {
        let i = style.inset();
        let top = i.top.maybe_resolve(c.node_inner_size.height, calc);
        let bottom = i.bottom.maybe_resolve(c.node_inner_size.height, calc);
        let left = i.left.maybe_resolve(c.node_inner_size.width, calc);
        let right = i.right.maybe_resolve(c.node_inner_size.width, calc);
        Size {
            width: left.or(right.map(|p| -p)).unwrap_or(0.0),
            height: top.or(bottom.map(|p| -p)).unwrap_or(0.0),
        }
    } else {
        Size::ZERO
    };
    // The flex basis: definite, or measured (case E of the spec).
    let percent_main = if c.known_main_size_is_definite {
        c.node_inner_size.height
    } else {
        None
    };
    let flex_basis = basis_style
        .maybe_resolve(percent_main, calc)
        .maybe_add(box_sizing_adjustment.height);
    let definite_basis = if basis_style.is_content() {
        None
    } else {
        flex_basis.or(size.height.filter(|_| basis_style.is_auto()))
    };
    Some(Item {
        generation: None,
        margin: margin_style.resolve_or_zero(c.node_inner_size.width, calc),
        padding,
        border,
        height: size.height,
        min: style
            .min_size()
            .maybe_resolve(percent, calc)
            .maybe_add(box_sizing_adjustment),
        max: style
            .max_size()
            .maybe_resolve(percent, calc)
            .maybe_add(box_sizing_adjustment),
        inset,
        overflow: style.overflow(),
        contain: style.contain(),
        scrollbar_width: style.scrollbar_width(),
        grow: style.flex_grow(),
        shrink: style.flex_shrink(),
        reads,
        baseline,
        flex_basis: definite_basis.unwrap_or(0.0),
        basis_is_definite: definite_basis.is_some(),
        resolved_min: 0.0,
        contribution: None,
        probed: false,
        last: None,
    })
}

/// The width an item is measured at (`cross_axis_available_space`).
fn cross_space(c: &Constants, available: Size<AvailableSpace>, item: &Item) -> AvailableSpace {
    match available.width {
        AvailableSpace::Definite(val) => AvailableSpace::Definite(
            (c.node_inner_size.width.unwrap_or(val) - item.margin.horizontal_axis_sum())
                .max(0.0)
                .maybe_clamp(item.min.width, item.max.width),
        ),
        AvailableSpace::MinContent => item
            .min
            .width
            .map_or(AvailableSpace::MinContent, AvailableSpace::Definite),
        AvailableSpace::MaxContent => item
            .max
            .width
            .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
    }
}

/// A size probe of item `i` (Taffy's `measure_child_size`, the vertical axis).
fn probe<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    i: usize,
    known: Size<Option<f32>>,
    parent: Size<Option<f32>>,
    available: Size<AvailableSpace>,
) -> f32 {
    view.child_layout(
        taffy::NodeId::new(u64::try_from(i).unwrap_or(u64::MAX)),
        LayoutInput {
            known_dimensions: known,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: parent,
            available_space: available,
            sizing_mode: SizingMode::ContentSize,
            axis: RequestedAxis::Vertical,
            run_mode: RunMode::ComputeSize,
            vertical_margins_are_collapsible: Line::FALSE,
        },
        None,
    )
    .size
    .height
}

/// Fill `item` (resolved from its style) with its flex base size and its
/// min-content size: Taffy's `determine_flex_base_size` for one item.
fn base_size<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    i: usize,
    item: &mut Item,
    c: &Constants,
    available: Size<AvailableSpace>,
) {
    let cross = cross_space(c, available, item);
    let parent = Size {
        width: c.node_inner_size.width,
        height: None,
    };
    let known = Size {
        width: cross.into_option(),
        height: None,
    };
    if !item.basis_is_definite {
        let space = Size {
            width: cross,
            height: if available.height == AvailableSpace::MinContent {
                AvailableSpace::MinContent
            } else {
                AvailableSpace::MaxContent
            },
        };
        item.flex_basis = probe(view, i, known, parent, space);
        item.probed = true;
    }
    let pb_main = item.padding.vertical_axis_sum() + item.border.vertical_axis_sum();
    item.flex_basis = item.flex_basis.max(pb_main);
    let automatic = if item.overflow.y.is_scroll_container() {
        Some(0.0)
    } else {
        None
    };
    item.resolved_min = match item.min.height.or(automatic) {
        Some(min) => min,
        None => {
            let space = Size {
                width: cross,
                height: AvailableSpace::MinContent,
            };
            let min_content = probe(view, i, known, parent, space);
            item.probed = true;
            let pb = rect_add(item.padding, item.border).sum_axes();
            min_content
                .maybe_min(item.height)
                .maybe_min(item.max.height)
                .maybe_max(Some(pb.height))
        }
    };
}

/// The item's content contribution under a min- or max-content constraint
/// (Taffy's `determine_container_main_size`, the `MinContent | MaxContent` arm).
fn contribution<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    i: usize,
    item: &Item,
    c: &Constants,
    available: Size<AvailableSpace>,
) -> (f32, bool) {
    let style_min = item.min.height;
    let margin_sum = item.margin.vertical_axis_sum();
    let style_preferred = item.height;
    let style_max = item.max.height;
    let clamping_basis = Some(item.flex_basis);
    let flex_basis_min = clamping_basis.filter(|_| item.shrink == 0.0);
    let flex_basis_max = clamping_basis.filter(|_| item.grow == 0.0);
    let min_main_size = style_min
        .maybe_max(flex_basis_min)
        .or(flex_basis_min)
        .unwrap_or(item.resolved_min)
        .max(item.resolved_min);
    let max_main_size = style_max
        .maybe_min(flex_basis_max)
        .or(flex_basis_max)
        .unwrap_or(f32::INFINITY);
    let scroll = item.overflow.x.is_scroll_container() || item.overflow.y.is_scroll_container();
    match (min_main_size, style_preferred, max_main_size) {
        (min, Some(pref), max) if max <= min || max <= pref => (pref.min(max).max(min) + margin_sum, false),
        (min, _, max) if max <= min => (min + margin_sum, false),
        _ if scroll => (item.flex_basis + margin_sum, false),
        (_, Some(pref), _) => {
            let pb = item.padding.vertical_axis_sum() + item.border.vertical_axis_sum();
            (
                (pref.max(pb).max(item.flex_basis) + margin_sum).maybe_clamp(style_min, style_max),
                false,
            )
        }
        _ => {
            let cross = available
                .width
                .map_definite_value(|val| {
                    (c.node_inner_size.width.unwrap_or(val) - item.margin.horizontal_axis_sum()).max(0.0)
                })
                .maybe_clamp(item.min.width, item.max.width);
            let space = Size {
                width: cross,
                height: available.height,
            };
            let known = Size {
                width: cross.into_option(),
                height: None,
            };
            let measured = probe(view, i, known, c.node_inner_size, space);
            (
                (measured.max(item.flex_basis) + margin_sum).maybe_clamp(style_min, style_max),
                true,
            )
        }
    }
}

/// Taffy's `compute_preliminary` for a column.
#[expect(clippy::too_many_lines, reason = "Taffy's algorithm, step by step, in its order")]
fn preliminary<S: NodeStyle, L: Measure>(view: &mut View<'_, S, L>, inputs: LayoutInput) -> Option<LayoutOutput> {
    let LayoutInput {
        known_dimensions,
        parent_size,
        available_space,
        run_mode,
        ..
    } = inputs;
    let calc = S::calc;
    let mut c = {
        let style = view.node.style.view();
        constants::<S>(
            &style,
            known_dimensions,
            inputs.known_dimensions_are_definite,
            parent_size,
        )
    };
    let count = view.node.children.len();
    if count == 0 {
        return None;
    }
    let available = Size {
        width: match known_dimensions.width {
            Some(w) => AvailableSpace::Definite((w - c.content_box_inset.horizontal_axis_sum()).max(0.0)),
            None => available_space
                .width
                .maybe_sub(c.content_box_inset.horizontal_axis_sum())
                .maybe_max(0.0),
        },
        height: match known_dimensions.height {
            Some(h) => AvailableSpace::Definite((h - c.content_box_inset.vertical_axis_sum()).max(0.0)),
            None => available_space
                .height
                .maybe_sub(c.content_box_inset.vertical_axis_sum())
                .maybe_max(0.0),
        },
    };

    // The items' answers for this input: kept beside the node for a long
    // column, else for this run only.
    let keep = count >= MEMO_MIN.load(std::sync::atomic::Ordering::Relaxed);
    let mut memo: Box<Memo> = view.node.stack.take().unwrap_or_default();
    let mut entry = memo.take(&inputs, count);
    let filled = fill(view, &mut entry, &c, available);
    if !filled {
        // Not a column this takes: what was found stays for the next run.
        if keep {
            memo.put(entry);
            view.node.stack = Some(memo);
        }
        return None;
    }
    let items = &mut entry.items;
    let mut flex: Vec<Flex> = items
        .iter()
        .map(|item| {
            let pb_main = item.padding.vertical_axis_sum() + item.border.vertical_axis_sum();
            let pb = rect_add(item.padding, item.border).sum_axes();
            let hypothetical_min = item.resolved_min.maybe_max(item.min.height).maybe_max(Some(pb.height));
            let hypothetical_inner = item.flex_basis.maybe_clamp(Some(hypothetical_min), item.max.height);
            Flex {
                hypothetical_inner,
                hypothetical_outer: hypothetical_inner + item.margin.vertical_axis_sum(),
                inner_flex_basis: item.flex_basis - pb_main,
                ..Flex::default()
            }
        })
        .collect();

    // The container's height.
    if let Some(inner_main) = c.node_inner_size.height {
        let outer_main = inner_main + c.content_box_inset.vertical_axis_sum();
        c.inner_container_size.height = inner_main;
        c.container_size.height = outer_main;
    } else {
        let inset = c.content_box_inset.vertical_axis_sum();
        let outer: f32 = match c.node_outer_size.height {
            Some(h) => h,
            None => match available.height {
                AvailableSpace::Definite(_) => {
                    let total = items
                        .iter()
                        .map(|item| {
                            let pb = rect_add(item.padding, item.border).vertical_axis_sum();
                            (item.flex_basis.maybe_max(item.min.height) + item.margin.vertical_axis_sum()).max(pb)
                        })
                        .sum::<f32>();
                    total + sum_axis_gaps(c.gap.height, count) + inset
                }
                AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                    for (i, item) in items.iter_mut().enumerate() {
                        let value = match item.contribution {
                            Some(v) => v,
                            None => {
                                let (v, probed) = contribution(view, i, item, &c, available);
                                item.contribution = Some(v);
                                item.probed |= probed;
                                v
                            }
                        };
                        if let Some(f) = flex.get_mut(i) {
                            let margin_sum = item.margin.vertical_axis_sum();
                            let diff = value - margin_sum - item.flex_basis;
                            f.content_flex_fraction = if diff > 0.0 {
                                diff / 1.0_f32.max(item.grow)
                            } else if diff < 0.0 {
                                diff / (1.0_f32.max(item.shrink) * f.inner_flex_basis)
                            } else {
                                0.0
                            };
                        }
                    }
                    let sum = items
                        .iter()
                        .zip(&flex)
                        .map(|(item, f)| {
                            let fraction = f.content_flex_fraction;
                            let flex_contribution = if fraction > 0.0 {
                                1.0_f32.max(item.grow) * fraction
                            } else if fraction < 0.0 {
                                let scaled = 1.0_f32.max(item.shrink) * f.inner_flex_basis;
                                if scaled == 0.0 { 0.0 } else { scaled * fraction }
                            } else {
                                0.0
                            };
                            item.flex_basis + flex_contribution + item.margin.vertical_axis_sum()
                        })
                        .sum::<f32>();
                    let gaps = sum_axis_gaps(c.gap.height, count);
                    0.0_f32.max(0.0_f32.max(sum + gaps)) + inset
                }
            },
        };
        let outer = outer
            .maybe_clamp(c.min_size.height, c.max_size.height)
            .max(inset - c.scrollbar_gutter.y);
        let inner = (outer - inset).max(0.0);
        c.container_size.height = outer;
        c.inner_container_size.height = inner;
        c.node_inner_size.height = Some(inner);
        c.node_outer_size.height = Some(outer);
        let style = view.node.style.view();
        c.gap.height = style.gap().height.maybe_resolve(inner, calc).unwrap_or(0.0);
    }
    if run_mode == RunMode::ComputeSize && inputs.axis == RequestedAxis::Vertical {
        let height = c.container_size.height;
        memo.put(entry);
        if keep {
            view.node.stack = Some(memo);
        }
        return Some(LayoutOutput::from_outer_size(Size { width: 0.0, height }));
    }

    resolve_flexible_lengths(items, &mut flex, &c);

    // The line's cross size: the container's width (single-line, definite).
    let cross_pb = c.content_box_inset.horizontal_axis_sum();
    let mut line_cross = known_dimensions
        .width
        .maybe_clamp(c.min_size.width, c.max_size.width)
        .maybe_sub(cross_pb)
        .maybe_max(0.0)
        .unwrap_or(0.0);
    if c.align_content == AlignContent::STRETCH {
        let container_min_inner_cross = known_dimensions
            .width
            .or(c.min_size.width)
            .maybe_clamp(c.min_size.width, c.max_size.width)
            .maybe_sub(cross_pb)
            .maybe_max(0.0)
            .unwrap_or(0.0);
        let lines_total: f32 = [line_cross].iter().sum::<f32>() + sum_axis_gaps(c.gap.width, 1);
        if lines_total < container_min_inner_cross {
            line_cross += (container_min_inner_cross - lines_total) / 1.0;
        }
    }
    for (item, f) in items.iter().zip(flex.iter_mut()) {
        let margin_cross = item.margin.horizontal_axis_sum();
        f.target.width = (line_cross - margin_cross)
            .max(0.0)
            .maybe_clamp(item.min.width, item.max.width);
        f.outer_target.width = f.target.width + margin_cross;
    }

    // Main-axis alignment.
    let gaps = sum_axis_gaps(c.gap.height, count);
    let used: f32 = gaps + flex.iter().map(|f| f.outer_target.height).sum::<f32>();
    let free_space = c.inner_container_size.height - used;
    let justify = alignment_fallback(free_space, count, c.justify_content);
    for (i, f) in flex.iter_mut().enumerate() {
        f.offset_main = alignment_offset(free_space, count, c.gap.height, justify, i == 0);
    }

    // The container's width.
    let total_line_cross: f32 = [line_cross].iter().sum::<f32>();
    let outer_cross = known_dimensions
        .width
        .unwrap_or(total_line_cross + sum_axis_gaps(c.gap.width, 1) + cross_pb)
        .maybe_clamp(c.min_size.width, c.max_size.width)
        .max(cross_pb - c.scrollbar_gutter.x);
    c.container_size.width = outer_cross;
    c.inner_container_size.width = (outer_cross - cross_pb).max(0.0);
    if run_mode == RunMode::ComputeSize {
        let size = c.container_size;
        memo.put(entry);
        if keep {
            view.node.stack = Some(memo);
        }
        return Some(LayoutOutput::from_outer_size(size));
    }
    let free_cross = c.inner_container_size.width - total_line_cross - sum_axis_gaps(c.gap.width, 1);
    let align = alignment_fallback(free_cross, 1, c.align_content);
    let line_offset_cross = alignment_offset(free_cross, 1, c.gap.width, align, true);

    // The final layout pass.
    let mut overflow_rect = Rect::ZERO;
    let mut candidates = OofCandidates::new();
    let total_offset_cross = c.content_box_inset.left;
    let mut total_offset_main = c.content_box_inset.top;
    let container_baseline = view.node.style.baseline();
    let run = view.node.run;
    let has_definite_main_size = c.has_definite_main_size;
    for (i, (item, f)) in items.iter_mut().zip(flex.iter_mut()).enumerate() {
        let input = LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::ContentSize,
            axis: RequestedAxis::Both,
            known_dimensions: Size {
                width: Some(f.target.width),
                height: Some(f.target.height),
            },
            known_dimensions_are_definite: Size {
                width: true,
                height: has_definite_main_size || item.basis_is_definite,
            },
            parent_size: c.node_inner_size,
            available_space: size_sub(c.container_size, item.margin.sum_axes())
                .map(|s| AvailableSpace::Definite(s.max(0.0))),
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let serial = view.node.children.get(i).map(|n| n.cache.serial());
        let kept = item
            .last
            .filter(|last| Some(last.serial) == serial && answers(&last.input, &input, item.reads));
        let (size, scrollable, first_baseline, mut oof) = match kept {
            Some(last) => {
                // What the item's cache would hand back: its side effects, without the asking.
                if let Some(child) = view.node.children.get_mut(i) {
                    let fixed = !container_baseline && !item.baseline;
                    child.tight &= fixed;
                    child.input = Some(input);
                    child.placed_in = run;
                    child.in_flow = false;
                }
                (last.size, last.overflow, last.first_baseline, OofCandidates::NONE)
            }
            None => {
                let mut out = view.child_layout(taffy::NodeId::new(u64::try_from(i).unwrap_or(u64::MAX)), input, None);
                let oof = out.oof_candidates.take();
                let serial = view.node.children.get(i).map_or(0, |n| n.cache.serial());
                item.last = oof.is_empty().then_some(Final {
                    input,
                    serial,
                    size: out.size,
                    overflow: out.scrollable_overflow_rect,
                    first_baseline: out.baselines.first,
                });
                (out.size, out.scrollable_overflow_rect, out.baselines.first, oof)
            }
        };
        let offset_main = total_offset_main + f.offset_main + item.margin.top + item.inset.height;
        let offset_cross = total_offset_cross + 0.0 + line_offset_cross + item.margin.left + item.inset.width;
        let inner_baseline = child_first_baseline(first_baseline, size.height, item.overflow.y).unwrap_or(size.height);
        f.baseline = (total_offset_main + f.offset_main + item.margin.top) + inner_baseline;
        let location = Point {
            x: offset_cross,
            y: offset_main,
        };
        let layout = Layout {
            order: u32::try_from(i).unwrap_or(u32::MAX),
            size,
            scrollable_overflow_rect: scrollable,
            scrollbar_size: Size {
                width: if item.overflow.y == Overflow::Scroll {
                    item.scrollbar_width
                } else {
                    0.0
                },
                height: if item.overflow.x == Overflow::Scroll {
                    item.scrollbar_width
                } else {
                    0.0
                },
            },
            location,
            padding: item.padding,
            border: item.border,
            margin: item.margin,
        };
        if let Some(child) = view.node.children.get_mut(i)
            && child.layout != layout
        {
            child.layout = layout;
            child.moved = true;
        }
        if !oof.is_empty() {
            oof.translate(location);
            candidates.append(&mut oof);
        }
        total_offset_main += f.offset_main + item.margin.vertical_axis_sum() + size.height;
        let at = Point {
            x: location.x - c.border.left,
            y: location.y - c.border.top,
        };
        overflow_rect = overflow_rect.union(overflow_contribution(
            at,
            size,
            scrollable,
            item.overflow,
            item.contain,
            c.is_scroll_container,
        ));
    }
    if c.is_scroll_container {
        overflow_rect.right += c.content_box_inset.right - c.border.right - c.scrollbar_gutter.x;
        overflow_rect.bottom += c.content_box_inset.bottom - c.border.bottom - c.scrollbar_gutter.y;
    }
    let inset = rect_add(
        c.border,
        Rect {
            left: 0.0,
            right: c.scrollbar_gutter.x,
            top: 0.0,
            bottom: c.scrollbar_gutter.y,
        },
    );
    let first = flex.first().map(|f| f.baseline);
    let mut output =
        LayoutOutput::from_sizes_and_baselines(c.container_size, overflow_rect, Baselines::from_first(first));
    output.oof_candidates = candidates;
    output.oof_positioning_area = Some(OofPositioningArea {
        size: size_sub(c.container_size, inset.sum_axes()),
        offset: Point {
            x: inset.left,
            y: inset.top,
        },
    });
    memo.put(entry);
    if keep {
        view.node.stack = Some(memo);
    }
    Some(output)
}

/// Resolve each item's style and ask it what it was not asked since its
/// cache was last cleared; whether every child is an item this takes.
fn fill<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    entry: &mut Entry,
    c: &Constants,
    available: Size<AvailableSpace>,
) -> bool {
    for i in 0..entry.items.len() {
        let Some(child) = view.node.children.get(i) else {
            return false;
        };
        let generation = child.cache.generation();
        if let Some(it) = entry.items.get(i).filter(|it| it.generation == Some(generation)) {
            // Its answers are used as if it were asked again.
            if it.probed
                && let Some(child) = view.node.children.get_mut(i)
            {
                child.tight = false;
            }
            continue;
        }
        let reads = child.style.reads_parent();
        let baseline = child.style.baseline();
        let style = child.style.view();
        let Some(mut item) = item_style::<S>(&style, c, reads, baseline) else {
            return false;
        };
        drop(style);
        base_size(view, i, &mut item, c, available);
        // Read after asking: an answer under another sizing mode clears the cache.
        item.generation = view.node.children.get(i).map(|n| n.cache.generation());
        if let Some(slot) = entry.items.get_mut(i) {
            *slot = item;
        }
    }
    true
}

/// Taffy's `resolve_flexible_lengths` for the one line.
fn resolve_flexible_lengths(items: &[Item], flex: &mut [Flex], c: &Constants) {
    let total_gap = sum_axis_gaps(c.gap.height, items.len());
    let total_hypothetical = flex.iter().map(|f| f.hypothetical_outer).sum::<f32>();
    let used_flex_factor = total_gap + total_hypothetical;
    let inner_main = c.node_inner_size.height;
    let growing = used_flex_factor < inner_main.unwrap_or(0.0);
    let shrinking = used_flex_factor > inner_main.unwrap_or(0.0);
    let exactly_sized = !growing & !shrinking;
    for (item, f) in items.iter().zip(flex.iter_mut()) {
        f.target.height = f.hypothetical_inner;
        if exactly_sized
            || (item.grow == 0.0 && item.shrink == 0.0)
            || (growing && item.flex_basis > f.hypothetical_inner)
            || (shrinking && item.flex_basis < f.hypothetical_inner)
        {
            f.frozen = true;
            f.outer_target.height = f.target.height + item.margin.vertical_axis_sum();
        }
    }
    if exactly_sized {
        return;
    }
    let used_space: f32 = total_gap
        + items
            .iter()
            .zip(flex.iter())
            .map(|(item, f)| {
                if f.frozen {
                    f.outer_target.height
                } else {
                    item.flex_basis + item.margin.vertical_axis_sum()
                }
            })
            .sum::<f32>();
    let initial_free_space = inner_main.maybe_sub(used_space).unwrap_or(0.0);
    loop {
        let mut all_frozen = true;
        let mut items_space: f32 = 0.0;
        let mut sum_grow: f32 = 0.0;
        let mut sum_shrink: f32 = 0.0;
        for (item, f) in items.iter().zip(flex.iter()) {
            if f.frozen {
                items_space += f.outer_target.height;
            } else {
                all_frozen = false;
                items_space += item.flex_basis + item.margin.vertical_axis_sum();
                sum_grow += item.grow;
                sum_shrink += item.shrink;
            }
        }
        if all_frozen {
            break;
        }
        let used_space: f32 = total_gap + items_space;
        let free_space = if growing && sum_grow < 1.0 {
            (initial_free_space * sum_grow - total_gap).maybe_min(inner_main.maybe_sub(used_space))
        } else if shrinking && sum_shrink < 1.0 {
            (initial_free_space * sum_shrink - total_gap).maybe_max(inner_main.maybe_sub(used_space))
        } else {
            inner_main
                .maybe_sub(used_space)
                .unwrap_or(used_flex_factor - used_space)
        };
        if free_space.is_normal() {
            if growing && sum_grow > 0.0 {
                for (item, f) in items.iter().zip(flex.iter_mut()).filter(|(_, f)| !f.frozen) {
                    f.target.height = item.flex_basis + free_space * (item.grow / sum_grow);
                }
            } else if shrinking && sum_shrink > 0.0 {
                let sum_scaled: f32 = items
                    .iter()
                    .zip(flex.iter())
                    .filter(|(_, f)| !f.frozen)
                    .map(|(item, f)| f.inner_flex_basis * item.shrink)
                    .sum();
                if sum_scaled > 0.0 {
                    for (item, f) in items.iter().zip(flex.iter_mut()).filter(|(_, f)| !f.frozen) {
                        let scaled = f.inner_flex_basis * item.shrink;
                        f.target.height = item.flex_basis + free_space * (scaled / sum_scaled);
                    }
                }
            }
        }
        let total_violation =
            items
                .iter()
                .zip(flex.iter_mut())
                .filter(|(_, f)| !f.frozen)
                .fold(0.0, |acc, (item, f)| -> f32 {
                    let clamped = f
                        .target
                        .height
                        .maybe_clamp(Some(item.resolved_min), item.max.height)
                        .max(0.0);
                    f.violation = clamped - f.target.height;
                    f.target.height = clamped;
                    f.outer_target.height = f.target.height + item.margin.vertical_axis_sum();
                    acc + f.violation
                });
        for f in flex.iter_mut().filter(|f| !f.frozen) {
            f.frozen = match total_violation {
                v if v > 0.0 => f.violation > 0.0,
                v if v < 0.0 => f.violation < 0.0,
                _ => true,
            };
        }
    }
}

impl Memo {
    /// The entry for `input` (taken out; [`Memo::put`] returns it), or a new one.
    fn take(&mut self, input: &LayoutInput, count: usize) -> Entry {
        let found = self.entries.iter().position(|e| e.input == *input);
        let mut entry = match found {
            Some(at) => self.entries.swap_remove(at),
            None => Entry {
                input: *input,
                used: 0,
                items: Vec::new(),
            },
        };
        if entry.items.len() != count {
            entry.items.clear();
            entry.items.resize(count, Item::default());
        }
        entry
    }

    /// Keep `entry`, dropping the least recently used beyond [`ENTRIES`].
    fn put(&mut self, mut entry: Entry) {
        self.clock = self.clock.wrapping_add(1);
        entry.used = self.clock;
        if self.entries.len() >= ENTRIES
            && let Some(oldest) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.used)
                .map(|(i, _)| i)
        {
            self.entries.swap_remove(oldest);
        }
        self.entries.push(entry);
    }
}
