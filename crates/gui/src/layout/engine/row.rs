//! A single-line flex row laid out by Taffy's flexbox algorithm step for
//! step (ADR-0014): table cells, badges, toolbars, the rows of a form.
//!
//! Taffy's `compute_flexbox_layout` is general: wrapping, balanced lines,
//! columns, baselines, aspect ratios, sizing keywords, absolutely positioned
//! children. A cell of a table uses none of it, and pays for it on every run
//! its parent asks for (a width probe, a height probe, a final layout): a
//! vector of items built anew, a vector of lines, the style read again at
//! each step. This is that algorithm for one case, every step it takes there
//! kept in Taffy's order of float operations, so the result is
//! bit-identical; what it allocates is kept between runs.
//!
//! It takes a `flex-direction: row` or `row-reverse` container that does not
//! wrap, left to right or right to left, whose children are each either
//! `display: none` or in flow with no aspect ratio, no sizing keyword (in
//! its size, its minimum and maximum sizes or its flex basis) and no
//! baseline alignment. Everything else is ported: a known or unknown width
//! (the min- and max-content main size), growing and shrinking, `auto`
//! margins, cross alignment, `justify-content`, gaps, minimum and maximum
//! sizes, padding and border, `box-sizing`, scrollbar gutters, `contain`,
//! relative offsets, and the out-of-flow boxes inside the items, passed up.
//! Anything else is Taffy's own algorithm, before any child was asked.
//!
//! What it reads of the available height is what Taffy's reads
//! ([`super::Node::heights_read`] says which).

use std::cell::RefCell;

use taffy::{
    AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignSelf, AvailableSpace, Baselines,
    BoxGenerationMode, BoxSizing, Contain, CoreStyle, Direction, FlexDirection, FlexWrap, FlexboxContainerStyle,
    FlexboxItemStyle, JustifyContent, Layout, LayoutInput, LayoutOutput, LengthPercentageAuto, Line, MaybeMath,
    MaybeResolve, NodeId, OofCandidates, OofPositioningArea, Overflow, Point, Position, Rect, RequestedAxis,
    ResolveOrZero, RunMode, Size, SizingMode,
};

use super::grid::{alignment_fallback, child_first_baseline, overflow_contribution, rect_add, size_sub};
use super::{Measure, NodeStyle, View};
use crate::convert::usize_to_f32;

/// One flex item (Taffy's `FlexItem`, the row case).
struct Item {
    /// Its index among the container's children (its `order`).
    child: usize,
    size: Size<Option<f32>>,
    /// Its cross size as styled is `auto` (else a length or percentage).
    cross_auto: bool,
    min_size: Size<Option<f32>>,
    max_size: Size<Option<f32>>,
    /// Its maximum height as styled, ignoring its ratio, as the used cross
    /// size reads it.
    max_cross_style: Option<f32>,
    align_self: AlignSelf,
    overflow: Point<Overflow>,
    contain: Contain,
    scrollbar_width: f32,
    flex_shrink: f32,
    flex_grow: f32,
    flex_basis_is_definite: bool,
    resolved_minimum_main_size: f32,
    relative_inset: Size<f32>,
    margin: Rect<f32>,
    margin_is_auto: Rect<bool>,
    padding: Rect<f32>,
    border: Rect<f32>,
    /// Its flex basis as styled, resolved (`None`: `auto` or `content`, or a
    /// percentage of an indefinite width).
    styled_basis: Option<f32>,
    basis_is_auto: bool,
    basis_is_content: bool,
    flex_basis: f32,
    inner_flex_basis: f32,
    violation: f32,
    frozen: bool,
    content_flex_fraction: f32,
    hypothetical_inner_size: Size<f32>,
    hypothetical_outer_size: Size<f32>,
    target_size: Size<f32>,
    outer_target_size: Size<f32>,
    baseline: f32,
    offset_main: f32,
    offset_cross: f32,
    oof_candidates: OofCandidates,
}

impl Item {
    /// Whether its used height is its line's: `auto` with `align-self:
    /// stretch` and no `auto` cross margin.
    fn is_stretched(&self) -> bool {
        !self.margin_is_auto.top
            && !self.margin_is_auto.bottom
            && self.align_self == AlignSelf::STRETCH
            && self.cross_auto
    }

    fn is_scroll_container(&self) -> bool {
        self.overflow.x.is_scroll_container() | self.overflow.y.is_scroll_container()
    }
}

/// Buffers kept between runs (on a pool thread, until its job group ends),
/// and the most items one may hold to be kept.
const SPARE: usize = 8;
const SPARE_ITEMS: usize = 256;

thread_local! {
    /// The items a run works on, kept between runs (one per nesting level and thread).
    static SCRATCH: RefCell<Vec<Vec<Item>>> = const { RefCell::new(Vec::new()) };
}

fn take_scratch() -> Vec<Item> {
    SCRATCH
        .with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.pop()))
        .unwrap_or_default()
}

/// Drop this thread's kept buffers: a pool thread lets go of them when its
/// job group ends, so what is held after a pass does not depend on which
/// threads the pool happened to run it on.
pub(super) fn release_scratch() {
    SCRATCH.with(|p| {
        if let Ok(mut p) = p.try_borrow_mut() {
            *p = Vec::new();
        }
    });
}

fn give_scratch(mut items: Vec<Item>) {
    items.clear();
    SCRATCH.with(|p| {
        // A few levels of nesting and a run of common size are kept: a
        // deep chain or a huge container takes fresh buffers rather than
        // leaving every pool thread holding its largest.
        if let Ok(mut p) = p.try_borrow_mut()
            && p.len() < SPARE
            && items.capacity() <= SPARE_ITEMS
        {
            p.push(items);
        }
    });
}

/// Taffy's `AlgoConstants`, the single-line row case.
struct Constants {
    reverse: bool,
    rtl: bool,
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

fn size_add(a: Size<f32>, b: Size<f32>) -> Size<f32> {
    Size {
        width: a.width + b.width,
        height: a.height + b.height,
    }
}

pub(super) fn resolve_normal_alignment(alignment: AlignItems) -> AlignItems {
    match alignment.keyword {
        AlignItemsKeyword::Normal => AlignItems::STRETCH,
        _ => alignment,
    }
}

pub(super) fn sum_axis_gaps(gap: f32, items: usize) -> f32 {
    if items <= 1 {
        0.0
    } else {
        gap * usize_to_f32(items.saturating_sub(1))
    }
}

/// Taffy's `compute_alignment_offset`.
pub(super) fn alignment_offset(
    free_space: f32,
    items: usize,
    gap: f32,
    mode: AlignContentKeyword,
    reversed: bool,
    first: bool,
) -> f32 {
    if first {
        match mode {
            AlignContentKeyword::Start
            | AlignContentKeyword::Normal
            | AlignContentKeyword::Stretch
            | AlignContentKeyword::SpaceBetween => 0.0,
            AlignContentKeyword::FlexStart => {
                if reversed {
                    free_space
                } else {
                    0.0
                }
            }
            AlignContentKeyword::End => free_space,
            AlignContentKeyword::FlexEnd => {
                if reversed {
                    0.0
                } else {
                    free_space
                }
            }
            AlignContentKeyword::Center => free_space / 2.0,
            AlignContentKeyword::SpaceAround => {
                if free_space >= 0.0 {
                    (free_space / usize_to_f32(items)) / 2.0
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
            AlignContentKeyword::SpaceAround => free_space / usize_to_f32(items),
            AlignContentKeyword::SpaceEvenly => free_space / usize_to_f32(items.saturating_add(1)),
        }
    }
}

fn id(child: usize) -> NodeId {
    NodeId::new(u64::try_from(child).unwrap_or(u64::MAX))
}

/// Taffy's `measure_child_size`: a size probe of `child`.
fn measure<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    child: usize,
    known: Size<Option<f32>>,
    parent: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    axis: RequestedAxis,
) -> Size<f32> {
    view.child_layout(
        id(child),
        LayoutInput {
            known_dimensions: known,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: parent,
            available_space: available,
            sizing_mode: SizingMode::ContentSize,
            axis,
            run_mode: RunMode::ComputeSize,
            vertical_margins_are_collapsible: Line::FALSE,
        },
        None,
    )
    .size
}

/// Lay `view`'s node out as a flex row for `inputs`, as
/// `taffy::compute_flexbox_layout` would; `None` when it is not a row this
/// takes (before any child was asked: Taffy's own algorithm runs).
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
        if !matches!(style.flex_direction(), FlexDirection::Row | FlexDirection::RowReverse)
            || style.flex_wrap() != FlexWrap::NoWrap
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
    let definite = inputs
        .known_dimensions_are_definite
        .zip_map(known_dimensions, |d, k| d || k.is_none());
    let mut items = take_scratch();
    let out = preliminary(
        view,
        LayoutInput {
            known_dimensions: styled_known,
            known_dimensions_are_definite: definite,
            ..inputs
        },
        &mut items,
    );
    give_scratch(items);
    let mut output = out?;
    if contain.suppresses_baseline() {
        output.baselines = Baselines::NONE;
    }
    Some(output)
}

/// Taffy's `compute_constants`, the single-line row case.
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
    let align_items = resolve_normal_alignment(FlexboxContainerStyle::align_items(style));
    let align_content = match FlexboxContainerStyle::align_content(style) {
        AlignContent {
            keyword: AlignContentKeyword::Normal,
            ..
        } => AlignContent::STRETCH,
        a => a,
    };
    let justify_content = match FlexboxContainerStyle::justify_content(style) {
        JustifyContent {
            keyword: AlignContentKeyword::Normal,
            ..
        } => JustifyContent::FLEX_START,
        j => j,
    };
    let rtl = style.direction() == Direction::Rtl;
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
    if rtl {
        content_box_inset.left += scrollbar_gutter.x;
    } else {
        content_box_inset.right += scrollbar_gutter.x;
    }
    let node_outer_size = known_dimensions;
    let node_inner_size = node_outer_size.maybe_sub(content_box_inset.sum_axes());
    let known_main_size_is_definite = definite.width;
    let gap = FlexboxContainerStyle::gap(style).resolve_or_zero(node_inner_size.or(Size::zero()), calc);
    Constants {
        reverse: style.flex_direction() == FlexDirection::RowReverse,
        rtl,
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
        has_definite_main_size: known_main_size_is_definite && known_dimensions.width.is_some(),
        container_size: Size::zero(),
        inner_container_size: Size::zero(),
    }
}

/// Taffy's `generate_anonymous_flex_items` into `items`; whether every
/// child is one this takes (nothing is asked of any child).
fn generate<S: NodeStyle, L: Measure>(view: &View<'_, S, L>, c: &Constants, items: &mut Vec<Item>) -> bool {
    let calc = S::calc;
    let percent = if c.known_main_size_is_definite {
        c.node_inner_size
    } else {
        Size {
            width: None,
            height: c.node_inner_size.height,
        }
    };
    let percent_main = if c.known_main_size_is_definite {
        c.node_inner_size.width
    } else {
        None
    };
    for (index, child) in view.node.children.iter().enumerate() {
        let style = child.style.view();
        if style.position().is_out_of_flow() {
            return false;
        }
        if style.box_generation_mode() == BoxGenerationMode::None {
            continue;
        }
        let size_style = style.size();
        let min_style = style.min_size();
        let max_style = style.max_size();
        let basis_style = style.flex_basis();
        if style.aspect_ratio().is_some()
            || [size_style.width, size_style.height, basis_style]
                .iter()
                .any(|d| d.is_sizing_keyword() || d.is_stretch())
        {
            return false;
        }
        let align_self = resolve_normal_alignment(FlexboxItemStyle::align_self(&style).unwrap_or(c.align_items))
            .resolve_self_relative(
                style.direction(),
                if c.rtl { Direction::Rtl } else { Direction::Ltr },
                false,
            );
        if align_self.keyword == AlignItemsKeyword::Baseline {
            return false;
        }
        let padding = style.padding().resolve_or_zero(c.node_inner_size.width, calc);
        let border = style.border().resolve_or_zero(c.node_inner_size.width, calc);
        let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
            rect_add(padding, border).sum_axes()
        } else {
            Size::ZERO
        };
        let relative_inset = if style.position() == Position::Relative {
            let i = style.inset();
            let left = i.left.maybe_resolve(c.node_inner_size.width, calc);
            let right = i.right.maybe_resolve(c.node_inner_size.width, calc);
            let top = i.top.maybe_resolve(c.node_inner_size.height, calc);
            let bottom = i.bottom.maybe_resolve(c.node_inner_size.height, calc);
            let main = if c.rtl {
                right.or(left.map(|p| -p)).unwrap_or(0.0)
            } else {
                left.or(right.map(|p| -p)).unwrap_or(0.0)
            };
            Size {
                width: main,
                height: top.or(bottom.map(|p| -p)).unwrap_or(0.0),
            }
        } else {
            Size::ZERO
        };
        let margin_style = style.margin();
        items.push(Item {
            child: index,
            size: size_style
                .maybe_resolve(percent, calc)
                .maybe_apply_aspect_ratio(None)
                .maybe_add(box_sizing_adjustment),
            cross_auto: size_style.height.is_auto(),
            min_size: min_style.maybe_resolve(percent, calc).maybe_add(box_sizing_adjustment),
            max_size: max_style.maybe_resolve(percent, calc).maybe_add(box_sizing_adjustment),
            max_cross_style: max_style
                .height
                .maybe_resolve(c.node_inner_size.height, calc)
                .maybe_add(box_sizing_adjustment.height),
            align_self,
            overflow: style.overflow(),
            contain: style.contain(),
            scrollbar_width: style.scrollbar_width(),
            flex_shrink: style.flex_shrink(),
            flex_grow: style.flex_grow(),
            flex_basis_is_definite: false,
            resolved_minimum_main_size: 0.0,
            relative_inset,
            margin: margin_style.resolve_or_zero(c.node_inner_size.width, calc),
            margin_is_auto: margin_style.map(LengthPercentageAuto::is_auto),
            padding,
            border,
            styled_basis: basis_style
                .maybe_resolve(percent_main, calc)
                .maybe_add(box_sizing_adjustment.width),
            basis_is_auto: basis_style.is_auto(),
            basis_is_content: basis_style.is_content(),
            flex_basis: 0.0,
            inner_flex_basis: 0.0,
            violation: 0.0,
            frozen: false,
            content_flex_fraction: 0.0,
            hypothetical_inner_size: Size::zero(),
            hypothetical_outer_size: Size::zero(),
            target_size: Size::zero(),
            outer_target_size: Size::zero(),
            baseline: 0.0,
            offset_main: 0.0,
            offset_cross: 0.0,
            oof_candidates: OofCandidates::NONE,
        });
    }
    true
}

/// Taffy's `determine_flex_base_size` for one item.
fn flex_base_size<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    c: &Constants,
    available: Size<AvailableSpace>,
    item: &mut Item,
) {
    let cross_parent = c.node_inner_size.height;
    let child_parent_size = Size {
        width: None,
        height: cross_parent,
    };
    let min_cross = item.min_size.height;
    let max_cross = item.max_size.height;
    let cross_available: AvailableSpace = match available.height {
        AvailableSpace::Definite(val) => AvailableSpace::Definite(
            (cross_parent.unwrap_or(val) - item.margin.vertical_axis_sum())
                .max(0.0)
                .maybe_clamp(min_cross, max_cross),
        ),
        AvailableSpace::MinContent => match min_cross {
            Some(min) => AvailableSpace::Definite(min),
            None => AvailableSpace::MinContent,
        },
        AvailableSpace::MaxContent => match max_cross {
            Some(max) => AvailableSpace::Definite(max),
            None => AvailableSpace::MaxContent,
        },
    };
    let known = {
        let mut ckd = Size {
            width: None,
            height: item.size.height,
        };
        ckd.height = ckd.height.maybe_clamp(item.min_size.height, item.max_size.height);
        if item.align_self == AlignSelf::STRETCH
            && !item.margin_is_auto.top
            && !item.margin_is_auto.bottom
            && ckd.height.is_none()
        {
            ckd.height = cross_available.into_option();
        }
        ckd
    };
    item.flex_basis = 'basis: {
        if !item.basis_is_content
            && let Some(basis) = item.styled_basis.or(item.size.width.filter(|_| item.basis_is_auto))
        {
            item.flex_basis_is_definite = true;
            break 'basis basis;
        }
        let space = Size {
            width: if available.width == AvailableSpace::MinContent {
                AvailableSpace::MinContent
            } else {
                AvailableSpace::MaxContent
            },
            height: cross_available,
        };
        measure(
            view,
            item.child,
            known,
            child_parent_size,
            space,
            RequestedAxis::Horizontal,
        )
        .width
    };
    let padding_border_sum = item.padding.horizontal_axis_sum() + item.border.horizontal_axis_sum();
    item.flex_basis = item.flex_basis.max(padding_border_sum);
    item.inner_flex_basis = item.flex_basis - padding_border_sum;
    let padding_border_axes_sums = rect_add(item.padding, item.border).sum_axes().map(Some);
    let automatic = if item.overflow.x.is_scroll_container() {
        Some(0.0)
    } else {
        None
    };
    item.resolved_minimum_main_size = match item.min_size.width.or(automatic) {
        Some(min) => min,
        None => {
            let space = Size {
                width: AvailableSpace::MinContent,
                height: cross_available,
            };
            let min_content = measure(
                view,
                item.child,
                known,
                child_parent_size,
                space,
                RequestedAxis::Horizontal,
            )
            .width;
            min_content
                .maybe_min(item.size.width)
                .maybe_min(item.max_size.width)
                .maybe_max(padding_border_axes_sums.width)
        }
    };
    let hypothetical_inner_min = item
        .resolved_minimum_main_size
        .maybe_max(item.min_size.width)
        .maybe_max(padding_border_axes_sums.width);
    let hypothetical_inner = item
        .flex_basis
        .maybe_clamp(Some(hypothetical_inner_min), item.max_size.width);
    item.hypothetical_inner_size.width = hypothetical_inner;
    item.hypothetical_outer_size.width = hypothetical_inner + item.margin.horizontal_axis_sum();
}

/// An item's content contribution under a min- or max-content constraint
/// (Taffy's `determine_container_main_size`, the `MinContent | MaxContent`
/// arm, for a row).
fn content_contribution<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    c: &Constants,
    available: Size<AvailableSpace>,
    item: &Item,
) -> f32 {
    let style_min = item.min_size.width;
    let margin_sum = item.margin.horizontal_axis_sum();
    let style_preferred = item.size.width;
    let style_max = item.max_size.width;
    let clamping_basis = Some(item.flex_basis).maybe_max(style_preferred);
    let flex_basis_min = clamping_basis.filter(|_| item.flex_shrink == 0.0);
    let flex_basis_max = clamping_basis.filter(|_| item.flex_grow == 0.0);
    let min_main_size = style_min
        .maybe_max(flex_basis_min)
        .or(flex_basis_min)
        .unwrap_or(item.resolved_minimum_main_size)
        .max(item.resolved_minimum_main_size);
    let max_main_size = style_max
        .maybe_min(flex_basis_max)
        .or(flex_basis_max)
        .unwrap_or(f32::INFINITY);
    match (min_main_size, style_preferred, max_main_size) {
        (min, Some(pref), max) if max <= min || max <= pref => pref.min(max).max(min) + margin_sum,
        (min, _, max) if max <= min => min + margin_sum,
        _ if item.is_scroll_container() => item.flex_basis + margin_sum,
        (_, Some(pref), _) => {
            let pb = item.padding.horizontal_axis_sum() + item.border.horizontal_axis_sum();
            (pref.max(pb) + margin_sum).maybe_clamp(style_min, style_max)
        }
        _ => {
            let cross_parent = c.node_inner_size.height;
            let cross = available
                .height
                .map_definite_value(|val| (cross_parent.unwrap_or(val) - item.margin.vertical_axis_sum()).max(0.0))
                .maybe_clamp(item.min_size.height, item.max_size.height);
            let space = Size {
                width: available.width,
                height: cross,
            };
            let mut known = Size {
                width: None,
                height: item.size.height,
            };
            known.height = known.height.maybe_clamp(item.min_size.height, item.max_size.height);
            if item.align_self == AlignSelf::STRETCH && known.height.is_none() {
                known.height = cross.into_option();
            }
            let measured = measure(
                view,
                item.child,
                known,
                c.node_inner_size,
                space,
                RequestedAxis::Horizontal,
            )
            .width;
            (measured + margin_sum).maybe_clamp(style_min, style_max)
        }
    }
}

/// Taffy's `determine_container_main_size` when the width is not known.
fn container_main_size<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    c: &mut Constants,
    available: Size<AvailableSpace>,
    items: &mut [Item],
) {
    let inset = c.content_box_inset.horizontal_axis_sum();
    let outer: f32 = match c.node_outer_size.width {
        Some(width) => width,
        None => match available.width {
            AvailableSpace::Definite(_) => {
                let gap = c.gap.width;
                let line = {
                    let line_gap = sum_axis_gaps(gap, items.len());
                    let total = items
                        .iter()
                        .map(|item| {
                            let padding_border_sum = rect_add(item.padding, item.border).horizontal_axis_sum();
                            (item.flex_basis.maybe_max(item.min_size.width) + item.margin.horizontal_axis_sum())
                                .max(padding_border_sum)
                        })
                        .sum::<f32>();
                    total + line_gap
                };
                let longest = [line].into_iter().max_by(f32::total_cmp).unwrap_or(0.0);
                longest + inset
            }
            AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                let mut main_size = 0.0;
                for i in 0..items.len() {
                    let Some(item) = items.get(i) else { continue };
                    let contribution = content_contribution(view, c, available, item);
                    let Some(item) = items.get_mut(i) else { continue };
                    let margin_sum = item.margin.horizontal_axis_sum();
                    item.content_flex_fraction = {
                        let diff = contribution - margin_sum - item.flex_basis;
                        if diff > 0.0 {
                            diff / f32::max(1.0, item.flex_grow)
                        } else if diff < 0.0 {
                            diff / (f32::max(1.0, item.flex_shrink) * item.inner_flex_basis)
                        } else {
                            0.0
                        }
                    };
                }
                let largest_item_size = 0.0_f32;
                let sum = items
                    .iter()
                    .map(|item| {
                        let fraction = item.content_flex_fraction;
                        let flex_contribution = if fraction > 0.0 {
                            f32::max(1.0, item.flex_grow) * fraction
                        } else if fraction < 0.0 {
                            let scaled = f32::max(1.0, item.flex_shrink) * item.inner_flex_basis;
                            if scaled == 0.0 { 0.0 } else { scaled * fraction }
                        } else {
                            0.0
                        };
                        item.flex_basis + flex_contribution + item.margin.horizontal_axis_sum()
                    })
                    .sum::<f32>();
                let gap_sum = sum_axis_gaps(c.gap.width, items.len());
                main_size = f32::max(main_size, f32::max(largest_item_size, sum + gap_sum));
                main_size + inset
            }
        },
    };
    let outer = outer
        .maybe_clamp(c.min_size.width, c.max_size.width)
        .max(inset - c.scrollbar_gutter.x);
    let inner = f32::max(outer - inset, 0.0);
    c.container_size.width = outer;
    c.inner_container_size.width = inner;
    c.node_inner_size.width = Some(inner);
}

/// Taffy's `resolve_flexible_lengths` for the one line.
fn resolve_flexible_lengths(items: &mut [Item], c: &Constants) {
    let total_gap = sum_axis_gaps(c.gap.width, items.len());
    let total_hypothetical = items.iter().map(|i| i.hypothetical_outer_size.width).sum::<f32>();
    let used_flex_factor: f32 = total_gap + total_hypothetical;
    let inner_main = c.node_inner_size.width;
    let growing = used_flex_factor < inner_main.unwrap_or(0.0);
    let shrinking = used_flex_factor > inner_main.unwrap_or(0.0);
    let exactly_sized = !growing & !shrinking;
    for item in items.iter_mut() {
        let inner_target = item.hypothetical_inner_size.width;
        item.target_size.width = inner_target;
        if exactly_sized
            || (item.flex_grow == 0.0 && item.flex_shrink == 0.0)
            || (growing && item.flex_basis > item.hypothetical_inner_size.width)
            || (shrinking && item.flex_basis < item.hypothetical_inner_size.width)
        {
            item.frozen = true;
            item.outer_target_size.width = inner_target + item.margin.horizontal_axis_sum();
        }
    }
    if exactly_sized {
        return;
    }
    let used_space: f32 = total_gap
        + items
            .iter()
            .map(|item| {
                if item.frozen {
                    item.outer_target_size.width
                } else {
                    item.flex_basis + item.margin.horizontal_axis_sum()
                }
            })
            .sum::<f32>();
    let initial_free_space = inner_main.maybe_sub(used_space).unwrap_or(0.0);
    loop {
        let mut all_frozen = true;
        let mut items_space: f32 = 0.0;
        let mut sum_grow: f32 = 0.0;
        let mut sum_shrink: f32 = 0.0;
        for item in items.iter() {
            if item.frozen {
                items_space += item.outer_target_size.width;
            } else {
                all_frozen = false;
                items_space += item.flex_basis + item.margin.horizontal_axis_sum();
                sum_grow += item.flex_grow;
                sum_shrink += item.flex_shrink;
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
                for item in items.iter_mut().filter(|i| !i.frozen) {
                    item.target_size.width = item.flex_basis + free_space * (item.flex_grow / sum_grow);
                }
            } else if shrinking && sum_shrink > 0.0 {
                let sum_scaled: f32 = items
                    .iter()
                    .filter(|i| !i.frozen)
                    .map(|i| i.inner_flex_basis * i.flex_shrink)
                    .sum();
                if sum_scaled > 0.0 {
                    for item in items.iter_mut().filter(|i| !i.frozen) {
                        let scaled = item.inner_flex_basis * item.flex_shrink;
                        item.target_size.width = item.flex_basis + free_space * (scaled / sum_scaled);
                    }
                }
            }
        }
        let total_violation = items.iter_mut().filter(|i| !i.frozen).fold(0.0, |acc, item| -> f32 {
            let clamped = item
                .target_size
                .width
                .maybe_clamp(Some(item.resolved_minimum_main_size), item.max_size.width)
                .max(0.0);
            item.violation = clamped - item.target_size.width;
            item.target_size.width = clamped;
            item.outer_target_size.width = item.target_size.width + item.margin.horizontal_axis_sum();
            acc + item.violation
        });
        for item in items.iter_mut().filter(|i| !i.frozen) {
            item.frozen = match total_violation {
                v if v > 0.0 => item.violation > 0.0,
                v if v < 0.0 => item.violation < 0.0,
                _ => true,
            };
        }
    }
}

/// Taffy's `item_known_dimension_definiteness`, for a row.
fn definiteness(c: &Constants, item: &Item) -> Size<bool> {
    Size {
        width: c.has_definite_main_size || item.flex_basis_is_definite,
        height: item.is_stretched() || item.size.height.is_some(),
    }
}

/// Taffy's `determine_hypothetical_cross_size` for one item.
fn hypothetical_cross_size<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    c: &Constants,
    node_size: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    item: &mut Item,
) {
    let line_cross_size_is_known = node_size.height.is_some();
    let padding_border_sum = rect_add(item.padding, item.border).vertical_axis_sum();
    if line_cross_size_is_known && item.is_stretched() {
        return;
    }
    let child_available_main: AvailableSpace = (c.container_size.width - item.margin.horizontal_axis_sum())
        .max(0.0)
        .into();
    let min_cross = item.min_size.height;
    let max_cross = item.max_size.height;
    let child_cross = item
        .size
        .height
        .maybe_clamp(min_cross, max_cross)
        .maybe_max(padding_border_sum);
    let child_available_cross = available
        .height
        .map_definite_value(|val| (val - item.margin.vertical_axis_sum()).max(0.0))
        .maybe_clamp(min_cross, max_cross)
        .maybe_max(padding_border_sum);
    let inner_cross = match child_cross {
        Some(cross) => cross,
        None => view
            .child_layout(
                id(item.child),
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    sizing_mode: SizingMode::ContentSize,
                    axis: RequestedAxis::Vertical,
                    known_dimensions: Size {
                        width: Some(item.target_size.width),
                        height: child_cross,
                    },
                    known_dimensions_are_definite: definiteness(c, item),
                    parent_size: c.node_inner_size,
                    available_space: Size {
                        width: child_available_main,
                        height: child_available_cross,
                    },
                    vertical_margins_are_collapsible: Line::FALSE,
                },
                None,
            )
            .size
            .height
            .maybe_clamp(min_cross, max_cross)
            .max(padding_border_sum),
    };
    item.hypothetical_inner_size.height = inner_cross;
    item.hypothetical_outer_size.height = inner_cross + item.margin.vertical_axis_sum();
}

/// Taffy's `align_flex_items_along_cross_axis`, for a single-line row.
fn cross_alignment_offset(item: &Item, free_space: f32) -> f32 {
    let keyword = if item.align_self.is_safe() && free_space < 0.0 {
        AlignItemsKeyword::Start
    } else {
        item.align_self.keyword
    };
    match keyword {
        AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd => free_space,
        AlignItemsKeyword::Center => free_space / 2.0,
        _ => 0.0,
    }
}

/// Taffy's `compute_preliminary`, the single-line row case.
#[expect(clippy::too_many_lines, reason = "Taffy's algorithm, step by step, in its order")]
fn preliminary<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    inputs: LayoutInput,
    items: &mut Vec<Item>,
) -> Option<LayoutOutput> {
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
    if view.node.children.is_empty() {
        return None;
    }

    // 9.2. Line length determination.
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
    if !generate(view, &c, items) {
        return None;
    }
    for item in items.iter_mut() {
        flex_base_size(view, &c, available, item);
    }

    // 9.3. Main size determination.
    if let Some(inner_main) = c.node_inner_size.width {
        let outer_main = inner_main + c.content_box_inset.horizontal_axis_sum();
        c.inner_container_size.width = inner_main;
        c.container_size.width = outer_main;
    } else {
        container_main_size(view, &mut c, available, items);
        c.node_inner_size.width = Some(c.inner_container_size.width);
        c.node_outer_size.width = Some(c.container_size.width);
        let style = view.node.style.view();
        c.gap.width = FlexboxContainerStyle::gap(&style)
            .width
            .maybe_resolve(c.inner_container_size.width, calc)
            .unwrap_or(0.0);
    }
    if run_mode == RunMode::ComputeSize && inputs.axis == RequestedAxis::Horizontal {
        return Some(LayoutOutput::from_outer_size(Size {
            width: c.container_size.width,
            height: 0.0,
        }));
    }
    resolve_flexible_lengths(items, &c);

    // 9.4. Cross size determination.
    for item in items.iter_mut() {
        hypothetical_cross_size(view, &c, known_dimensions, available, item);
    }
    let cross_inset = c.content_box_inset.vertical_axis_sum();
    let mut line_cross = if let Some(height) = known_dimensions.height {
        Some(height)
            .maybe_clamp(c.min_size.height, c.max_size.height)
            .maybe_sub(cross_inset)
            .maybe_max(0.0)
            .unwrap_or(0.0)
    } else {
        let cross = items
            .iter()
            .map(|item| item.hypothetical_outer_size.height)
            .fold(0.0, |acc: f32, x| acc.max(x));
        cross.maybe_clamp(
            c.min_size.height.maybe_sub(cross_inset),
            c.max_size.height.maybe_sub(cross_inset),
        )
    };
    if c.align_content == AlignContent::STRETCH {
        let container_min_inner_cross = known_dimensions
            .height
            .or(c.min_size.height)
            .maybe_clamp(c.min_size.height, c.max_size.height)
            .maybe_sub(cross_inset)
            .maybe_max(0.0)
            .unwrap_or(0.0);
        let total_gap = sum_axis_gaps(c.gap.height, 1);
        let lines_total: f32 = [line_cross].iter().sum::<f32>() + total_gap;
        if lines_total < container_min_inner_cross {
            let remaining = container_min_inner_cross - lines_total;
            line_cross += remaining / 1.0;
        }
    }
    for item in items.iter_mut() {
        item.target_size.height = if item.is_stretched() {
            (line_cross - item.margin.vertical_axis_sum())
                .max(0.0)
                .maybe_clamp(item.min_size.height, item.max_cross_style)
        } else {
            item.hypothetical_inner_size.height
        };
        item.outer_target_size.height = item.target_size.height + item.margin.vertical_axis_sum();
    }

    // 9.5. Main-axis alignment.
    {
        let total_gap = sum_axis_gaps(c.gap.width, items.len());
        let used: f32 = total_gap + items.iter().map(|i| i.outer_target_size.width).sum::<f32>();
        let mut free_space = c.inner_container_size.width - used;
        let mut auto_margins = 0_usize;
        for item in items.iter() {
            if item.margin_is_auto.left {
                auto_margins = auto_margins.saturating_add(1);
            }
            if item.margin_is_auto.right {
                auto_margins = auto_margins.saturating_add(1);
            }
        }
        if free_space > 0.0 && auto_margins > 0 {
            let margin = free_space / usize_to_f32(auto_margins);
            for item in items.iter_mut() {
                if item.margin_is_auto.left {
                    item.margin.left = margin;
                }
                if item.margin_is_auto.right {
                    item.margin.right = margin;
                }
            }
            free_space = 0.0;
        }
        let count = items.len();
        let gap = c.gap.width;
        let mode = alignment_fallback(free_space, count, c.justify_content);
        let mut justify = |(i, item): (usize, &mut Item)| {
            item.offset_main = alignment_offset(free_space, count, gap, mode, c.reverse, i == 0);
        };
        if c.reverse {
            items.iter_mut().rev().enumerate().for_each(&mut justify);
        } else {
            items.iter_mut().enumerate().for_each(&mut justify);
        }
    }

    // 9.6. Cross-axis alignment.
    for item in items.iter_mut() {
        let free_space = line_cross - item.outer_target_size.height;
        match (item.margin_is_auto.top, item.margin_is_auto.bottom) {
            (true, true) => {
                item.margin.top = free_space / 2.0;
                item.margin.bottom = free_space / 2.0;
            }
            (true, false) => item.margin.top = free_space,
            (false, true) => item.margin.bottom = free_space,
            (false, false) => item.offset_cross = cross_alignment_offset(item, free_space),
        }
    }
    let total_line_cross: f32 = [line_cross].iter().sum::<f32>();
    let outer_cross = known_dimensions
        .height
        .unwrap_or(total_line_cross + sum_axis_gaps(c.gap.height, 1) + cross_inset)
        .maybe_clamp(c.min_size.height, c.max_size.height)
        .max(cross_inset - c.scrollbar_gutter.y);
    c.container_size.height = outer_cross;
    c.inner_container_size.height = f32::max(outer_cross - cross_inset, 0.0);
    if run_mode == RunMode::ComputeSize {
        return Some(LayoutOutput::from_outer_size(c.container_size));
    }
    let line_offset_cross = {
        let gap = c.gap.height;
        let free_space = c.inner_container_size.height - total_line_cross - sum_axis_gaps(gap, 1);
        let mode = alignment_fallback(free_space, 1, c.align_content);
        alignment_offset(free_space, 1, gap, mode, false, true)
    };

    // The final layout pass.
    let mut overflow_rect = Rect::ZERO;
    let total_offset_cross = c.content_box_inset.top;
    let mut total_offset_main = if c.rtl {
        c.container_size.width - c.content_box_inset.right
    } else {
        c.content_box_inset.left
    };
    let count = items.len();
    for k in 0..count {
        let at = if c.reverse {
            count.saturating_sub(1).saturating_sub(k)
        } else {
            k
        };
        let Some(item) = items.get_mut(at) else { continue };
        let input = LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::ContentSize,
            axis: RequestedAxis::Both,
            known_dimensions: item.target_size.map(Some),
            known_dimensions_are_definite: definiteness(&c, item),
            parent_size: c.node_inner_size,
            available_space: size_sub(c.container_size, item.margin.sum_axes())
                .map(|s| AvailableSpace::Definite(s.max(0.0))),
            vertical_margins_are_collapsible: Line::FALSE,
        };
        let mut out = view.child_layout(id(item.child), input, None);
        let size = out.size;
        let offset_main = if c.rtl {
            total_offset_main - item.offset_main - item.margin.right - item.relative_inset.width - size.width
        } else {
            total_offset_main + item.offset_main + item.margin.left + item.relative_inset.width
        };
        let offset_cross =
            total_offset_cross + item.offset_cross + line_offset_cross + item.margin.top + item.relative_inset.height;
        let inner_baseline =
            child_first_baseline(out.baselines.first, size.height, item.overflow.y).unwrap_or(size.height);
        item.baseline = (total_offset_cross + item.offset_cross + line_offset_cross + item.margin.top) + inner_baseline;
        let location = Point {
            x: offset_main,
            y: offset_cross,
        };
        let layout = Layout {
            order: u32::try_from(item.child).unwrap_or(u32::MAX),
            size,
            scrollable_overflow_rect: out.scrollable_overflow_rect,
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
        if let Some(child) = view.node.children.get_mut(item.child)
            && child.layout != layout
        {
            child.layout = layout;
            child.moved = true;
        }
        item.oof_candidates = out.oof_candidates.take();
        if !item.oof_candidates.is_empty() {
            item.oof_candidates.translate(location);
        }
        if c.rtl {
            total_offset_main -= item.offset_main + item.margin.horizontal_axis_sum() + size.width;
        } else {
            total_offset_main += item.offset_main + item.margin.horizontal_axis_sum() + size.width;
        }
        let contribution_location = if c.rtl {
            Point {
                x: c.container_size.width - (location.x + size.width) - c.border.right,
                y: location.y - c.border.top,
            }
        } else {
            Point {
                x: location.x - c.border.left,
                y: location.y - c.border.top,
            }
        };
        overflow_rect = overflow_rect.union(overflow_contribution(
            contribution_location,
            size,
            out.scrollable_overflow_rect,
            item.overflow,
            item.contain,
            c.is_scroll_container,
        ));
    }
    if c.is_scroll_container {
        overflow_rect.right += if c.rtl {
            c.content_box_inset.left - c.border.left - c.scrollbar_gutter.x
        } else {
            c.content_box_inset.right - c.border.right - c.scrollbar_gutter.x
        };
        overflow_rect.bottom += c.content_box_inset.bottom - c.border.bottom - c.scrollbar_gutter.y;
    }

    // The out-of-flow boxes inside the items, in document order.
    let mut candidates = OofCandidates::new();
    for item in items.iter_mut() {
        candidates.append(&mut item.oof_candidates);
    }
    let inset = rect_add(
        c.border,
        Rect {
            left: if c.rtl { c.scrollbar_gutter.x } else { 0.0 },
            right: if c.rtl { 0.0 } else { c.scrollbar_gutter.x },
            top: 0.0,
            bottom: c.scrollbar_gutter.y,
        },
    );
    let area = size_sub(c.container_size, inset.sum_axes());

    // Hidden children.
    for order in 0..view.node.children.len() {
        let hidden = view
            .node
            .children
            .get(order)
            .is_some_and(|n| n.style.view().box_generation_mode() == BoxGenerationMode::None);
        if !hidden {
            continue;
        }
        let layout = Layout::with_order(u32::try_from(order).unwrap_or(u32::MAX));
        if let Some(child) = view.node.children.get_mut(order)
            && child.layout != layout
        {
            child.layout = layout;
            child.moved = true;
        }
        view.child_layout(
            id(order),
            LayoutInput {
                known_dimensions: Size::NONE,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size: Size::NONE,
                available_space: Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                sizing_mode: SizingMode::ContentSize,
                axis: RequestedAxis::Both,
                run_mode: RunMode::PerformLayout,
                vertical_margins_are_collapsible: Line::FALSE,
            },
            None,
        );
    }

    let first = items.first().map(|item| item.baseline);
    let mut output =
        LayoutOutput::from_sizes_and_baselines(c.container_size, overflow_rect, Baselines::from_first(first));
    output.oof_candidates = candidates;
    output.oof_positioning_area = Some(OofPositioningArea {
        size: area,
        offset: Point {
            x: inset.left,
            y: inset.top,
        },
    });
    Some(output)
}
