//! Flexbox: Taffy 0.15.1's `compute_flexbox_layout` (both directions and
//! their reverses, `wrap`, `wrap-reverse`, `balance`, `flex-line-count`,
//! baselines, `align-content`, auto margins, sizing keywords, the static
//! positions of out-of-flow children) ported step for step on the
//! retained tree, in Taffy's order of float operations, so the result is
//! bit-identical to Taffy's where nothing below says otherwise. `row.rs`
//! and `stack.rs` take the common single-line cases first, memoised.
//!
//! `aspect-ratio` differs (`sizing.rs`):
//!
//! - The container resolves its own ratio ([`with_ratio`]), at least as
//!   large as its lines need in that axis.
//! - An item's size is its style's (Taffy carries one axis to the other
//!   through the ratio, as definite): the item resolves its own ratio, so
//!   its content can make it larger, as in Chrome. Case B of the flex base
//!   size (a definite cross size through the ratio) is kept, in the box
//!   `box-sizing` names.
//! - An item's min and max carry through its ratio only into an `auto`
//!   axis ([`transfer`]): `width: 60px; aspect-ratio: 2; max-height: 20px`
//!   is 60x20 (Taffy 40x20; Chrome 60x20).
//! - A probe of an item with a ratio does not give it the cross size it
//!   would be stretched to as known unless that size is definite: its
//!   ratio would make the probe's main size from a size it does not have.

use taffy::style_helpers::{TaffyMaxContent, TaffyMinContent};
use taffy::{
    AbsoluteAxis, AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignSelf, AlignmentSafety,
    AvailableSpace, AxisStaticAlign, AxisStaticEdge, AxisStaticPosition, Baselines, BoxGenerationMode, BoxSizing,
    Contain, CoreStyle, Dimension, Direction, FlexDirection, FlexboxContainerStyle, FlexboxItemStyle, JustifyContent,
    Layout, LayoutInput, LayoutOutput, LayoutPartialTree, LengthPercentageAuto, Line, MaybeMath, MaybeResolve, NodeId,
    OofCandidate, OofCandidates, OofPositioningArea, Overflow, Point, Position, Rect, RequestedAxis, ResolveOrZero,
    RunMode, Size, SizingMode,
};

use super::axes::{
    Axes, Sides, cross_axis, cross_sum, is_balance, is_column, is_multi_line, is_reverse, is_row, is_wrap_reverse,
    main_axis, main_sum, point_cross, point_main, rect_add, size_add, size_sub, zip_size,
};
use super::balance::balanced_line_item_counts;
use super::grid::{alignment_fallback, child_first_baseline, overflow_contribution};
use super::row::{alignment_offset, resolve_normal_alignment, sum_axis_gaps};
use super::sizing::{Keyword, Laid, Ratio, Reports, keyword, ratio_of, transfer, with_ratio};
use super::{Measure, NodeStyle, View};
use crate::convert::usize_to_f32;

/// One flex item as the algorithm sees it (Taffy's `FlexItem`).
struct Item {
    child: usize,
    order: u32,
    /// Its size as styled (no ratio applied).
    size: Size<Option<f32>>,
    size_style: Size<Dimension>,
    min_size: Size<Option<f32>>,
    max_size: Size<Option<f32>>,
    /// Its min and max with what its ratio carries into an `auto` axis.
    transferred_min: Size<Option<f32>>,
    transferred_max: Size<Option<f32>>,
    aspect_ratio: Option<f32>,
    /// Its `box-sizing` adjustment (padding and border for `content-box`).
    box_sizing: Size<f32>,
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
    fn is_scroll_container(&self) -> bool {
        self.overflow.x.is_scroll_container() | self.overflow.y.is_scroll_container()
    }

    fn is_stretched(&self, dir: FlexDirection) -> bool {
        let cross_size = self.size_style.cross(dir);
        !self.margin_is_auto.cross_start(dir)
            && !self.margin_is_auto.cross_end(dir)
            && (cross_size.is_stretch() || (self.align_self == AlignSelf::STRETCH && cross_size.is_auto()))
    }

    fn participates_in_baseline_alignment(&self, dir: FlexDirection) -> bool {
        self.align_self == AlignSelf::BASELINE
            && !self.margin_is_auto.cross_start(dir)
            && !self.margin_is_auto.cross_end(dir)
    }

    /// Its main size through its ratio from `cross`, in the box
    /// `box-sizing` names.
    fn ratio_main(&self, ratio: f32, cross: f32, dir: FlexDirection) -> f32 {
        let adj = self.box_sizing;
        if is_row(dir) {
            (cross - adj.height) * ratio + adj.width
        } else {
            (cross - adj.width) / ratio + adj.height
        }
    }
}

/// A line: a run of items (Taffy's `FlexLine`), as a range of the items.
#[derive(Clone, Copy)]
struct FlexLine {
    start: usize,
    end: usize,
    cross_size: f32,
    offset_cross: f32,
}

impl FlexLine {
    const fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            cross_size: 0.0,
            offset_cross: 0.0,
        }
    }

    fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

fn line_items<'a>(items: &'a [Item], line: &FlexLine) -> &'a [Item] {
    items.get(line.start..line.end).unwrap_or(&[])
}

fn line_items_mut<'a>(items: &'a mut [Item], line: &FlexLine) -> &'a mut [Item] {
    items.get_mut(line.start..line.end).unwrap_or(&mut [])
}

/// Taffy's `AlgoConstants`.
struct Constants {
    dir: FlexDirection,
    layout_direction: Direction,
    is_row: bool,
    is_column: bool,
    is_wrap: bool,
    is_wrap_reverse: bool,
    is_balance: bool,
    line_count: Option<u16>,
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
    has_definite_cross_size: bool,
    cross_axis_available_space_is_definite: bool,
    container_size: Size<f32>,
    inner_container_size: Size<f32>,
}

impl Constants {
    fn divided_cross_space(&self, cross_available_space: f32) -> f32 {
        if let Some(line_count) = self.line_count
            && line_count > 1
        {
            let line_count = f32::from(line_count);
            return (cross_available_space - (line_count - 1.0) * self.gap.cross(self.dir)) / line_count;
        }
        cross_available_space
    }
}

fn id(child: usize) -> NodeId {
    NodeId::new(u64::try_from(child).unwrap_or(u64::MAX))
}

fn order_of(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// Taffy's `FlexDirection::main_axis_is_requested_axis`.
const fn main_axis_is_requested_axis(dir: FlexDirection, axis: RequestedAxis) -> bool {
    match axis {
        RequestedAxis::Horizontal => is_row(dir),
        RequestedAxis::Vertical => is_column(dir),
        RequestedAxis::Both => false,
    }
}

/// Taffy's `Overflow::maybe_into_automatic_min_size`.
fn automatic_min_size(o: Overflow) -> Option<f32> {
    o.is_scroll_container().then_some(0.0)
}

/// Taffy's `resolve_self_alignment_safety`.
fn self_alignment_safety(alignment: AlignItems, overflows: bool) -> AlignItemsKeyword {
    if matches!(alignment.safety, AlignmentSafety::Safe) && overflows {
        AlignItemsKeyword::Start
    } else {
        alignment.keyword
    }
}

/// Lay `view`'s node out as a flex container for `inputs` (Taffy's
/// `compute_flexbox_layout`).
pub(super) fn compute<S: NodeStyle, L: Measure>(view: &mut View<'_, S, L>, inputs: LayoutInput) -> LayoutOutput {
    let LayoutInput {
        known_dimensions,
        parent_size,
        run_mode,
        ..
    } = inputs;
    let calc = S::calc;
    let (styled, contain, ratio) = {
        let style = view.node.style.view();
        let contain = style.contain();
        let aspect_ratio = ratio_of(&style);
        let padding = style.padding().resolve_or_zero(parent_size.width, calc);
        let border = style.border().resolve_or_zero(parent_size.width, calc);
        let padding_border_sum = size_add(padding.sum_axes(), border.sum_axes());
        let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
            padding_border_sum
        } else {
            Size::ZERO
        };
        let size_style = style.size();
        let raw_min = style.min_size().maybe_resolve(parent_size, calc);
        let raw_max = style.max_size().maybe_resolve(parent_size, calc);
        let min_size = transfer(raw_min, aspect_ratio, size_style).maybe_add(box_sizing_adjustment);
        let max_size = transfer(raw_max, aspect_ratio, size_style).maybe_add(box_sizing_adjustment);
        let clamped_style_size = if inputs.sizing_mode == SizingMode::InherentSize {
            size_style
                .maybe_resolve(parent_size, calc)
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
        let overflow = style.overflow();
        let ratio = Ratio::find(
            aspect_ratio,
            styled,
            box_sizing_adjustment,
            raw_min.maybe_add(box_sizing_adjustment),
            raw_max.maybe_add(box_sizing_adjustment),
            padding_border_sum,
            overflow.x.is_scroll_container() || overflow.y.is_scroll_container(),
        );
        (styled, contain, ratio)
    };

    if run_mode == RunMode::ComputeSize {
        if let Size {
            width: Some(width),
            height: Some(height),
        } = styled
        {
            return LayoutOutput::from_outer_size(Size { width, height });
        }
        if inputs.axis == RequestedAxis::Horizontal
            && let Some(width) = styled.width
        {
            return LayoutOutput::from_outer_size(Size { width, height: 0.0 });
        }
    }

    // The definiteness flags apply only to sizes the parent gave; those
    // from the node's own style are definite.
    let known_dimensions_are_definite = inputs
        .known_dimensions_are_definite
        .zip_map(known_dimensions, |is_definite, known| is_definite || known.is_none());
    let inputs = LayoutInput {
        known_dimensions: styled,
        known_dimensions_are_definite,
        ..inputs
    };

    let mut output = match ratio {
        None => preliminary(view, inputs, false).0,
        Some(ratio) => with_ratio(
            view,
            inputs,
            ratio,
            Reports::Content { rerun_last: false },
            |view, inputs, _| {
                let (output, content) = preliminary(view, inputs, ratio.axis == AbsoluteAxis::Vertical);
                Laid { output, content }
            },
        ),
    };
    if contain.suppresses_baseline() {
        output.baselines = Baselines::NONE;
    }
    output
}

/// Taffy's `compute_preliminary`, and, when `report` is set, the height
/// its content takes (the outer height its lines need).
#[expect(clippy::too_many_lines, reason = "Taffy's compute_preliminary, ported as one step")]
fn preliminary<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    inputs: LayoutInput,
    report: bool,
) -> (LayoutOutput, Option<f32>) {
    let LayoutInput {
        known_dimensions,
        parent_size,
        available_space,
        run_mode,
        ..
    } = inputs;

    let mut constants = constants::<S>(
        &view.node.style.view(),
        known_dimensions,
        inputs.known_dimensions_are_definite,
        parent_size,
        available_space,
    );
    let dir = constants.dir;

    if view.node.children.is_empty() {
        return (empty_container_layout(inputs, constants), None);
    }

    // 9.1. Initial Setup
    let mut items = generate_anonymous_flex_items(view, &constants);

    // 9.2. Line Length Determination
    let available_space = determine_available_space(known_dimensions, available_space, &constants);
    determine_flex_base_size(view, &constants, available_space, &mut items);

    // 9.3. Main Size Determination
    let mut lines = if constants.is_balance {
        collect_balanced_flex_lines(&constants, available_space, &items)
    } else {
        collect_flex_lines(&constants, available_space, &items)
    };

    // What a column's content takes: its longest line.
    let mut content = (report && constants.is_column).then(|| {
        let longest = lines
            .iter()
            .map(|line| {
                let items = line_items(&items, line);
                items
                    .iter()
                    .map(|item| item.hypothetical_outer_size.main(dir))
                    .sum::<f32>()
                    + sum_axis_gaps(constants.gap.main(dir), items.len())
            })
            .fold(0.0_f32, f32::max);
        longest + main_sum(constants.content_box_inset, dir)
    });

    if let Some(inner_main_size) = constants.node_inner_size.main(dir) {
        let outer_main_size = inner_main_size + main_sum(constants.content_box_inset, dir);
        constants.inner_container_size.set_main(dir, inner_main_size);
        constants.container_size.set_main(dir, outer_main_size);
    } else {
        determine_container_main_size(view, available_space, &lines, &mut items, &mut constants);
        constants
            .node_inner_size
            .set_main(dir, Some(constants.inner_container_size.main(dir)));
        constants
            .node_outer_size
            .set_main(dir, Some(constants.container_size.main(dir)));

        // Re-resolve percentage gaps
        let inner_container_size = constants.inner_container_size.main(dir);
        let new_gap = view
            .node
            .style
            .view()
            .gap()
            .main(dir)
            .maybe_resolve(inner_container_size, S::calc)
            .unwrap_or(0.0);
        constants.gap.set_main(dir, new_gap);
    }

    if run_mode == RunMode::ComputeSize && main_axis_is_requested_axis(dir, inputs.axis) {
        let size = Size::ZERO.with_main(dir, constants.container_size.main(dir));
        return (LayoutOutput::from_outer_size(size), content);
    }

    // 6. Resolve the flexible lengths
    for line in &lines {
        resolve_flexible_lengths(line_items_mut(&mut items, line), &constants);
    }

    // 9.4. Cross Size Determination
    for line in &lines {
        determine_hypothetical_cross_size(
            view,
            line_items_mut(&mut items, line),
            &constants,
            known_dimensions,
            available_space,
        );
    }
    calculate_children_base_lines(view, known_dimensions, available_space, &lines, &mut items, &constants);
    calculate_cross_size(&mut lines, &items, known_dimensions, &constants);

    // What a row's content takes: its lines, or a single line's items.
    if report && constants.is_row {
        let lines_cross = if constants.is_wrap {
            lines.iter().map(|line| line.cross_size).sum::<f32>() + sum_axis_gaps(constants.gap.cross(dir), lines.len())
        } else {
            items
                .iter()
                .map(|item| item.hypothetical_outer_size.cross(dir))
                .fold(0.0_f32, f32::max)
        };
        content = Some(lines_cross + cross_sum(constants.content_box_inset, dir));
    }

    handle_align_content_stretch(&mut lines, known_dimensions, &constants);
    determine_used_cross_size(view, &lines, &mut items, &constants);

    // 9.5. Main-Axis Alignment
    distribute_remaining_free_space(&lines, &mut items, &constants);

    // 9.6. Cross-Axis Alignment
    resolve_cross_axis_auto_margins(&lines, &mut items, &constants);
    let total_line_cross_size = determine_container_cross_size(&lines, known_dimensions, &mut constants);

    if run_mode == RunMode::ComputeSize {
        return (LayoutOutput::from_outer_size(constants.container_size), content);
    }

    align_flex_lines_per_align_content(&mut lines, &constants, total_line_cross_size);
    let inflow_overflow_rect = final_layout_pass(view, &lines, &mut items, &constants);

    let mut candidates = OofCandidates::new();
    collect_oof_candidates(view, &constants, &mut items, &mut candidates);

    let absolute_position_inset = rect_add(
        constants.border,
        Rect {
            left: if constants.layout_direction.is_rtl() {
                constants.scrollbar_gutter.x
            } else {
                0.0
            },
            right: if constants.layout_direction.is_rtl() {
                0.0
            } else {
                constants.scrollbar_gutter.x
            },
            top: 0.0,
            bottom: constants.scrollbar_gutter.y,
        },
    );
    let absolute_position_area = size_sub(constants.container_size, absolute_position_inset.sum_axes());
    let absolute_position_offset = Point {
        x: absolute_position_inset.left,
        y: absolute_position_inset.top,
    };

    for order in 0..view.node.children.len() {
        let hidden = view
            .node
            .children
            .get(order)
            .is_some_and(|c| c.style.view().box_generation_mode() == BoxGenerationMode::None);
        if hidden {
            view.set_unrounded_layout(id(order), &Layout::with_order(order_of(order)));
            view.perform_child_layout(
                id(order),
                Size::NONE,
                Size::NONE,
                Size::MAX_CONTENT,
                SizingMode::ContentSize,
                Line::FALSE,
            );
        }
    }

    // 8.5. Flex Container Baselines: from the startmost line.
    let first_line = if constants.is_wrap_reverse {
        lines.last()
    } else {
        lines.first()
    };
    let first_vertical_baseline = first_line.and_then(|line| {
        let line = line_items(&items, line);
        if constants.is_column {
            let item = if is_reverse(dir) { line.last() } else { line.first() };
            item.map(|child| child.baseline)
        } else {
            line.iter()
                .find(|item| item.participates_in_baseline_alignment(dir))
                .or_else(|| line.first())
                .map(|child| child.baseline)
        }
    });

    let mut output = LayoutOutput::from_sizes_and_baselines(
        constants.container_size,
        inflow_overflow_rect,
        Baselines::from_first(first_vertical_baseline),
    );
    output.oof_candidates = candidates;
    output.oof_positioning_area = Some(OofPositioningArea {
        size: absolute_position_area,
        offset: absolute_position_offset,
    });
    (output, content)
}

/// Taffy's `compute_empty_container_layout`.
fn empty_container_layout(inputs: LayoutInput, mut constants: Constants) -> LayoutOutput {
    let LayoutInput {
        known_dimensions,
        run_mode,
        ..
    } = inputs;
    let dir = constants.dir;
    if let Some(inner_main_size) = constants.node_inner_size.main(dir) {
        let outer_main_size = inner_main_size + main_sum(constants.content_box_inset, dir);
        constants.inner_container_size.set_main(dir, inner_main_size);
        constants.container_size.set_main(dir, outer_main_size);
    } else {
        empty_container_main_size(&mut constants);
    }
    if run_mode == RunMode::ComputeSize && main_axis_is_requested_axis(dir, inputs.axis) {
        let size = Size::ZERO.with_main(dir, constants.container_size.main(dir));
        return LayoutOutput::from_outer_size(size);
    }
    let _total: f32 = determine_container_cross_size(&[], known_dimensions, &mut constants);
    let mut output = LayoutOutput::from_outer_size(constants.container_size);
    output.oof_positioning_area = Some(OofPositioningArea {
        size: Size::ZERO,
        offset: Point::ZERO,
    });
    output
}

/// Taffy's `determine_container_main_size` with no lines.
fn empty_container_main_size(constants: &mut Constants) {
    let dir = constants.dir;
    let main_content_box_inset = main_sum(constants.content_box_inset, dir);
    // Every case of Taffy's sums no line: no length, then the inset.
    let outer_main_size = constants
        .node_outer_size
        .main(dir)
        .unwrap_or(0.0 + main_content_box_inset);
    set_main_size(outer_main_size, main_content_box_inset, constants);
}

/// The end of Taffy's `determine_container_main_size`.
fn set_main_size(outer_main_size: f32, main_content_box_inset: f32, constants: &mut Constants) {
    let dir = constants.dir;
    let outer_main_size = outer_main_size
        .maybe_clamp(constants.min_size.main(dir), constants.max_size.main(dir))
        .max(main_content_box_inset - point_main(constants.scrollbar_gutter, dir));
    let inner_main_size = (outer_main_size - main_content_box_inset).max(0.0);
    constants.container_size.set_main(dir, outer_main_size);
    constants.inner_container_size.set_main(dir, inner_main_size);
    constants.node_inner_size.set_main(dir, Some(inner_main_size));
}

/// Taffy's `compute_constants`.
fn constants<S: NodeStyle>(
    style: &S::Ref<'_>,
    known_dimensions: Size<Option<f32>>,
    known_dimensions_are_definite: Size<bool>,
    parent_size: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
) -> Constants {
    let calc = S::calc;
    let dir = style.flex_direction();
    let flex_wrap = style.flex_wrap();
    let is_wrap = is_multi_line(flex_wrap);
    let line_count = is_wrap.then(|| style.flex_line_count().max(1));

    let aspect_ratio = ratio_of(style);
    let padding = style.padding().resolve_or_zero(parent_size.width, calc);
    let border = style.border().resolve_or_zero(parent_size.width, calc);
    let padding_border_sum = size_add(padding.sum_axes(), border.sum_axes());
    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
        padding_border_sum
    } else {
        Size::ZERO
    };

    let align_items = resolve_normal_alignment(style.align_items());
    let align_content = match FlexboxContainerStyle::align_content(style) {
        AlignContent {
            keyword: AlignContentKeyword::Normal,
            ..
        } => AlignContent::STRETCH,
        align_content => align_content,
    };
    let justify_content = match style.justify_content() {
        JustifyContent {
            keyword: AlignContentKeyword::Normal,
            ..
        } => JustifyContent::FLEX_START,
        justify_content => justify_content,
    };
    let layout_direction = style.direction();

    let scrollbar_gutter = style.overflow().transpose().map(|overflow| match overflow {
        Overflow::Scroll => style.scrollbar_width(),
        _ => 0.0,
    });
    let is_scroll_container = {
        let overflow = style.overflow();
        overflow.x.is_scroll_container() || overflow.y.is_scroll_container()
    };
    let mut content_box_inset = rect_add(padding, border);
    content_box_inset.bottom += scrollbar_gutter.y;
    match layout_direction {
        Direction::Ltr => content_box_inset.right += scrollbar_gutter.x,
        Direction::Rtl => content_box_inset.left += scrollbar_gutter.x,
    }

    let node_outer_size = known_dimensions;
    let node_inner_size = node_outer_size.maybe_sub(content_box_inset.sum_axes());
    let known_main_size_is_definite = known_dimensions_are_definite.main(dir);
    let has_definite_main_size = known_main_size_is_definite && known_dimensions.main(dir).is_some();
    let has_definite_cross_size = known_dimensions_are_definite.cross(dir) && known_dimensions.cross(dir).is_some();
    let cross_axis_available_space_is_definite =
        has_definite_cross_size || matches!(available_space.cross(dir), AvailableSpace::Definite(_));
    let gap = style.gap().resolve_or_zero(node_inner_size.or(Size::zero()), calc);
    let size_style = style.size();

    Constants {
        dir,
        layout_direction,
        is_row: is_row(dir),
        is_column: is_column(dir),
        is_wrap,
        is_wrap_reverse: is_wrap_reverse(flex_wrap),
        is_balance: is_balance(flex_wrap),
        line_count,
        min_size: transfer(
            style.min_size().maybe_resolve(parent_size, calc),
            aspect_ratio,
            size_style,
        )
        .maybe_add(box_sizing_adjustment),
        max_size: transfer(
            style.max_size().maybe_resolve(parent_size, calc),
            aspect_ratio,
            size_style,
        )
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
        has_definite_main_size,
        has_definite_cross_size,
        cross_axis_available_space_is_definite,
        container_size: Size::zero(),
        inner_container_size: Size::zero(),
    }
}

/// Taffy's `resolve_relative_inset`.
fn resolve_relative_inset(inset: Rect<Option<f32>>, constants: &Constants) -> Size<f32> {
    let direction = constants.dir;
    let is_rtl_row = is_row(direction) && constants.layout_direction.is_rtl();
    let is_rtl_column = is_column(direction) && constants.layout_direction.is_rtl();
    let main = if is_rtl_row {
        inset
            .main_end(direction)
            .or(inset.main_start(direction).map(|pos| -pos))
            .unwrap_or(0.0)
    } else {
        inset
            .main_start(direction)
            .or(inset.main_end(direction).map(|pos| -pos))
            .unwrap_or(0.0)
    };
    let cross = if is_rtl_column {
        inset
            .cross_end(direction)
            .map(|pos| -pos)
            .or(inset.cross_start(direction))
            .unwrap_or(0.0)
    } else {
        inset
            .cross_start(direction)
            .or(inset.cross_end(direction).map(|pos| -pos))
            .unwrap_or(0.0)
    };
    Size::ZERO.with_main(direction, main).with_cross(direction, cross)
}

/// Taffy's `generate_anonymous_flex_items`.
fn generate_anonymous_flex_items<S: NodeStyle, L: Measure>(view: &View<'_, S, L>, constants: &Constants) -> Vec<Item> {
    let calc = S::calc;
    let percent_resolution_size = if constants.known_main_size_is_definite {
        constants.node_inner_size
    } else {
        constants.node_inner_size.with_main(constants.dir, None)
    };
    let mut items = Vec::with_capacity(view.node.children.len());
    for (index, node) in view.node.children.iter().enumerate() {
        let child_style = node.style.view();
        if child_style.position().is_out_of_flow() || child_style.box_generation_mode() == BoxGenerationMode::None {
            continue;
        }
        let aspect_ratio = ratio_of(&child_style);
        let padding = child_style
            .padding()
            .resolve_or_zero(constants.node_inner_size.width, calc);
        let border = child_style
            .border()
            .resolve_or_zero(constants.node_inner_size.width, calc);
        let box_sizing = if child_style.box_sizing() == BoxSizing::ContentBox {
            rect_add(padding, border).sum_axes()
        } else {
            Size::ZERO
        };
        let size_style = child_style.size();
        let raw_min = child_style.min_size().maybe_resolve(percent_resolution_size, calc);
        let raw_max = child_style.max_size().maybe_resolve(percent_resolution_size, calc);
        items.push(Item {
            child: index,
            order: order_of(index),
            size: size_style
                .maybe_resolve(percent_resolution_size, calc)
                .maybe_add(box_sizing),
            size_style,
            min_size: raw_min.maybe_add(box_sizing),
            max_size: raw_max.maybe_add(box_sizing),
            transferred_min: transfer(raw_min, aspect_ratio, size_style).maybe_add(box_sizing),
            transferred_max: transfer(raw_max, aspect_ratio, size_style).maybe_add(box_sizing),
            aspect_ratio,
            box_sizing,
            relative_inset: if child_style.position() == Position::Relative {
                let inset = zip_size(child_style.inset(), constants.node_inner_size, |p, s| {
                    p.maybe_resolve(s, calc)
                });
                resolve_relative_inset(inset, constants)
            } else {
                Size::ZERO
            },
            margin: child_style
                .margin()
                .resolve_or_zero(constants.node_inner_size.width, calc),
            margin_is_auto: child_style.margin().map(LengthPercentageAuto::is_auto),
            padding,
            border,
            align_self: resolve_normal_alignment(
                FlexboxItemStyle::align_self(&child_style).unwrap_or(constants.align_items),
            )
            .resolve_self_relative(
                child_style.direction(),
                constants.layout_direction,
                constants.is_column,
            ),
            overflow: child_style.overflow(),
            contain: child_style.contain(),
            scrollbar_width: child_style.scrollbar_width(),
            flex_grow: child_style.flex_grow(),
            flex_shrink: child_style.flex_shrink(),
            flex_basis_is_definite: false,
            flex_basis: 0.0,
            inner_flex_basis: 0.0,
            violation: 0.0,
            frozen: false,
            resolved_minimum_main_size: 0.0,
            hypothetical_inner_size: Size::zero(),
            hypothetical_outer_size: Size::zero(),
            target_size: Size::zero(),
            outer_target_size: Size::zero(),
            content_flex_fraction: 0.0,
            baseline: 0.0,
            offset_main: 0.0,
            offset_cross: 0.0,
            oof_candidates: OofCandidates::NONE,
        });
    }
    items
}

/// Taffy's `determine_available_space`.
fn determine_available_space(
    known_dimensions: Size<Option<f32>>,
    outer_available_space: Size<AvailableSpace>,
    constants: &Constants,
) -> Size<AvailableSpace> {
    let width = match known_dimensions.width {
        Some(node_width) => {
            AvailableSpace::Definite((node_width - constants.content_box_inset.horizontal_axis_sum()).max(0.0))
        }
        None => outer_available_space
            .width
            .maybe_sub(constants.content_box_inset.horizontal_axis_sum())
            .maybe_max(0.0),
    };
    let height = match known_dimensions.height {
        Some(node_height) => {
            AvailableSpace::Definite((node_height - constants.content_box_inset.vertical_axis_sum()).max(0.0))
        }
        None => outer_available_space
            .height
            .maybe_sub(constants.content_box_inset.vertical_axis_sum())
            .maybe_max(0.0),
    };
    Size { width, height }
}

/// Taffy's `determine_flex_base_size`.
#[expect(
    clippy::too_many_lines,
    reason = "Taffy's determine_flex_base_size, ported as one step"
)]
fn determine_flex_base_size<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    constants: &Constants,
    available_space: Size<AvailableSpace>,
    items: &mut [Item],
) {
    let dir = constants.dir;
    let calc = S::calc;

    for child in items.iter_mut() {
        let cross_axis_parent_size = constants.node_inner_size.cross(dir);
        let child_parent_size = Size::NONE.with_cross(dir, cross_axis_parent_size);

        let transferred_min_size = child.transferred_min;
        let transferred_max_size = child.transferred_max;
        let child_min_cross = transferred_min_size.cross(dir);
        let child_max_cross = transferred_max_size.cross(dir);

        let cross_axis_available_space = match available_space.cross(dir) {
            AvailableSpace::Definite(val) => AvailableSpace::Definite(
                (constants.divided_cross_space(cross_axis_parent_size.unwrap_or(val)) - cross_sum(child.margin, dir))
                    .max(0.0)
                    .maybe_clamp(child_min_cross, child_max_cross),
            ),
            AvailableSpace::MinContent => match child_min_cross {
                Some(min) => AvailableSpace::Definite(min),
                None => AvailableSpace::MinContent,
            },
            AvailableSpace::MaxContent => match child_max_cross {
                Some(max) => AvailableSpace::Definite(max),
                None => AvailableSpace::MaxContent,
            },
        };

        let mut child_cross_size_is_definite = child.size.cross(dir).is_some();
        let child_known_dimensions = {
            let mut ckd = child.size.with_main(dir, None);
            ckd.set_cross(
                dir,
                ckd.cross(dir)
                    .maybe_clamp(transferred_min_size.cross(dir), transferred_max_size.cross(dir)),
            );
            if child.align_self == AlignSelf::STRETCH
                && !child.margin_is_auto.cross_start(dir)
                && !child.margin_is_auto.cross_end(dir)
                && ckd.cross(dir).is_none()
            {
                let definite =
                    !constants.is_wrap && constants.has_definite_cross_size && cross_axis_parent_size.is_some();
                // An item with a ratio is not given a stretched size its
                // line may not have: its ratio would turn it into its main
                // size.
                if child.aspect_ratio.is_none() || definite {
                    ckd.set_cross(dir, cross_axis_available_space.into_option());
                    child_cross_size_is_definite = definite;
                }
            }
            ckd
        };

        let box_sizing_adjustment = child.box_sizing.main(dir);
        let percent_resolution_main_size = if constants.known_main_size_is_definite {
            constants.node_inner_size.main(dir)
        } else {
            None
        };
        let flex_basis_style = view
            .node
            .children
            .get(child.child)
            .map_or(Dimension::auto(), |n| n.style.view().flex_basis());
        let flex_basis = flex_basis_style
            .maybe_resolve(percent_resolution_main_size, calc)
            .maybe_add(box_sizing_adjustment);

        child.flex_basis = 'flex_basis: {
            let main_size = child.size.main(dir);
            let main_stretch_size = percent_resolution_main_size
                .maybe_sub(main_sum(child.margin, dir))
                .maybe_max(0.0);

            let keyword_main_available_space = if flex_basis_style.is_content() {
                None
            } else if flex_basis_style.is_sizing_keyword() {
                match keyword(flex_basis_style, main_stretch_size, percent_resolution_main_size, calc) {
                    Some(Keyword::Exact(size)) => {
                        child.flex_basis_is_definite = true;
                        break 'flex_basis size;
                    }
                    Some(Keyword::Measure(available)) => Some(available),
                    None => None,
                }
            } else {
                if let Some(flex_basis) = flex_basis.or(main_size.filter(|_| flex_basis_style.is_auto())) {
                    child.flex_basis_is_definite = true;
                    break 'flex_basis flex_basis;
                }
                if flex_basis_style.is_auto() {
                    match keyword(
                        child.size_style.main(dir),
                        main_stretch_size,
                        percent_resolution_main_size,
                        calc,
                    ) {
                        Some(Keyword::Exact(size)) => {
                            child.flex_basis_is_definite = true;
                            break 'flex_basis size;
                        }
                        Some(Keyword::Measure(available)) => Some(available),
                        None => None,
                    }
                } else {
                    None
                }
            };

            // B. A ratio and a definite cross size.
            if child_cross_size_is_definite
                && let (Some(ratio), Some(cross)) = (child.aspect_ratio, child_known_dimensions.cross(dir))
            {
                child.flex_basis_is_definite = true;
                break 'flex_basis child.ratio_main(ratio, cross, dir);
            }

            // E. Measure it.
            let child_available_space = Size::MAX_CONTENT
                .with_main(
                    dir,
                    keyword_main_available_space.unwrap_or(
                        if available_space.main(dir) == AvailableSpace::MinContent {
                            AvailableSpace::MinContent
                        } else {
                            AvailableSpace::MaxContent
                        },
                    ),
                )
                .with_cross(dir, cross_axis_available_space);

            break 'flex_basis view.measure_child_size(
                id(child.child),
                child_known_dimensions,
                child_parent_size,
                child_available_space,
                SizingMode::ContentSize,
                main_axis(dir),
                Line::FALSE,
            );
        };

        let padding_border_sum = main_sum(child.padding, dir) + main_sum(child.border, dir);
        child.flex_basis = child.flex_basis.max(padding_border_sum);
        child.inner_flex_basis = child.flex_basis - padding_border_sum;

        let padding_border_axes_sums = rect_add(child.padding, child.border).sum_axes().map(Some);

        let style_min_main_size = child
            .min_size
            .or(child.overflow.map(automatic_min_size).into())
            .main(dir);

        child.resolved_minimum_main_size = style_min_main_size.unwrap_or_else(|| {
            let min_content_main_size = {
                let child_available_space = Size::MIN_CONTENT.with_cross(dir, cross_axis_available_space);
                view.measure_child_size(
                    id(child.child),
                    child_known_dimensions,
                    child_parent_size,
                    child_available_space,
                    SizingMode::ContentSize,
                    main_axis(dir),
                    Line::FALSE,
                )
            };
            let clamped_min_content_size = min_content_main_size
                .maybe_min(child.size.main(dir))
                .maybe_min(transferred_max_size.main(dir));
            clamped_min_content_size.maybe_max(padding_border_axes_sums.main(dir))
        });

        let hypothetical_inner_min_main = child
            .resolved_minimum_main_size
            .maybe_max(transferred_min_size.main(dir))
            .maybe_max(padding_border_axes_sums.main(dir));
        let hypothetical_inner_size = child
            .flex_basis
            .maybe_clamp(Some(hypothetical_inner_min_main), transferred_max_size.main(dir));
        let hypothetical_outer_size = hypothetical_inner_size + main_sum(child.margin, dir);

        child.hypothetical_inner_size.set_main(dir, hypothetical_inner_size);
        child.hypothetical_outer_size.set_main(dir, hypothetical_outer_size);
    }
}

/// The main space a multi-line container's items wrap against (Taffy's,
/// shared by greedy and balanced collection).
fn wrap_space(constants: &Constants, available_space: Size<AvailableSpace>) -> AvailableSpace {
    let dir = constants.dir;
    match constants.max_size.main(dir) {
        Some(max_size) => AvailableSpace::Definite({
            let available = available_space.main(dir).into_option().unwrap_or(max_size);
            let available = if constants.has_definite_main_size {
                available
            } else {
                available.min(max_size)
            };
            available.maybe_max(constants.min_size.main(dir))
        }),
        None if !is_row(dir) && !constants.has_definite_main_size && available_space.main(dir).is_definite() => {
            AvailableSpace::MaxContent
        }
        None => available_space.main(dir),
    }
}

/// One line per item.
fn single_item_lines(items: &[Item]) -> Vec<FlexLine> {
    (0..items.len())
        .map(|i| FlexLine::new(i, i.saturating_add(1)))
        .collect()
}

/// Taffy's `collect_flex_lines`.
fn collect_flex_lines(constants: &Constants, available_space: Size<AvailableSpace>, items: &[Item]) -> Vec<FlexLine> {
    if !constants.is_wrap || !constants.known_main_size_is_definite {
        return vec![FlexLine::new(0, items.len())];
    }
    match wrap_space(constants, available_space) {
        AvailableSpace::MaxContent => vec![FlexLine::new(0, items.len())],
        AvailableSpace::MinContent => single_item_lines(items),
        AvailableSpace::Definite(main_axis_available_space) => {
            let mut lines = Vec::with_capacity(1);
            let main_axis_gap = constants.gap.main(constants.dir);
            let mut start = 0;
            while start < items.len() {
                let rest = items.get(start..).unwrap_or(&[]);
                let mut line_length = 0.0;
                let index = rest
                    .iter()
                    .enumerate()
                    .find(|&(idx, child)| {
                        let gap_contribution = if idx == 0 { 0.0 } else { main_axis_gap };
                        line_length += child.hypothetical_outer_size.main(constants.dir) + gap_contribution;
                        line_length > main_axis_available_space && idx != 0
                    })
                    .map_or(rest.len(), |(idx, _)| idx);
                let end = start.saturating_add(index);
                lines.push(FlexLine::new(start, end));
                start = end;
            }
            lines
        }
    }
}

/// Taffy's `collect_balanced_flex_lines`.
fn collect_balanced_flex_lines(
    constants: &Constants,
    available_space: Size<AvailableSpace>,
    items: &[Item],
) -> Vec<FlexLine> {
    if items.is_empty() {
        return Vec::new();
    }
    let main_axis_available_space = if constants.known_main_size_is_definite {
        wrap_space(constants, available_space)
    } else {
        AvailableSpace::MaxContent
    };
    if main_axis_available_space == AvailableSpace::MinContent {
        return single_item_lines(items);
    }
    let line_break_size = main_axis_available_space.into_option().unwrap_or(f32::INFINITY);
    let min_line_count = usize::from(constants.line_count.unwrap_or(1));
    let item_counts = balanced_line_item_counts(
        items
            .iter()
            .map(|item| item.hypothetical_outer_size.main(constants.dir)),
        line_break_size,
        constants.gap.main(constants.dir),
        min_line_count,
    );
    let mut lines = Vec::with_capacity(item_counts.len());
    let mut start = 0_usize;
    for count in item_counts {
        let end = start.saturating_add(count);
        lines.push(FlexLine::new(start, end));
        start = end;
    }
    lines
}

/// Taffy's `item_known_dimension_definiteness`.
fn item_known_dimension_definiteness(constants: &Constants, item: &Item) -> Size<bool> {
    let dir = constants.dir;
    let main_is_definite = constants.has_definite_main_size || item.flex_basis_is_definite;
    let has_cross_auto_margins = item.margin_is_auto.cross_start(dir) || item.margin_is_auto.cross_end(dir);
    let cross_size = item.size_style.cross(dir);
    let is_stretched = !has_cross_auto_margins
        && (cross_size.is_stretch() || (item.align_self == AlignSelf::STRETCH && cross_size.is_auto()));
    let cross_is_definite = is_stretched
        || item.size.cross(dir).is_some()
        || (!is_row(dir) && constants.cross_axis_available_space_is_definite);
    Size {
        width: true,
        height: true,
    }
    .with_main(dir, main_is_definite)
    .with_cross(dir, cross_is_definite)
}

/// An item's main length in a line, for the container's main size.
fn item_main_length(child: &Item, dir: FlexDirection) -> f32 {
    let padding_border_sum = main_sum(rect_add(child.padding, child.border), dir);
    (child.flex_basis.maybe_max(child.min_size.main(dir)) + main_sum(child.margin, dir)).max(padding_border_sum)
}

/// Taffy's `determine_container_main_size`.
#[expect(
    clippy::too_many_lines,
    reason = "Taffy's determine_container_main_size, ported as one step"
)]
fn determine_container_main_size<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    available_space: Size<AvailableSpace>,
    lines: &[FlexLine],
    items: &mut [Item],
    constants: &mut Constants,
) {
    let dir = constants.dir;
    let main_content_box_inset = main_sum(constants.content_box_inset, dir);

    let outer_main_size: f32 = match constants.node_outer_size.main(dir) {
        Some(size) => size,
        None => match available_space.main(dir) {
            AvailableSpace::Definite(main_axis_available_space) => {
                let main_axis_gap = constants.gap.main(dir);
                let longest_line_length: f32 = lines
                    .iter()
                    .map(|line| {
                        let line_items = line_items(items, line);
                        let line_main_axis_gap = sum_axis_gaps(main_axis_gap, line_items.len());
                        let total_target_size = line_items.iter().map(|c| item_main_length(c, dir)).sum::<f32>();
                        total_target_size + line_main_axis_gap
                    })
                    .max_by(f32::total_cmp)
                    .unwrap_or(0.0);
                let size = longest_line_length + main_content_box_inset;

                if constants.is_balance {
                    let min_line_count = constants.line_count.unwrap_or(1);
                    let item_count = lines.iter().map(FlexLine::len).sum::<usize>();
                    if item_count == 0 {
                        set_main_size(size, main_content_box_inset, constants);
                        return;
                    }
                    let mut item_lengths: Vec<f32> = Vec::with_capacity(item_count);
                    for line in lines.iter() {
                        for child in line_items(items, line) {
                            item_lengths.push(item_main_length(child, dir));
                        }
                    }
                    let item_counts = balanced_line_item_counts(
                        item_lengths.iter().copied(),
                        f32::INFINITY,
                        main_axis_gap,
                        usize::from(min_line_count.max(1)),
                    );
                    let mut widest_line_length: f32 = 0.0;
                    let mut index = 0_usize;
                    for count in item_counts {
                        let end = index.saturating_add(count);
                        let line_length: f32 = item_lengths.get(index..end).unwrap_or(&[]).iter().sum::<f32>()
                            + sum_axis_gaps(main_axis_gap, count);
                        widest_line_length = widest_line_length.max(line_length);
                        index = end;
                    }
                    let max_content_size = widest_line_length + main_content_box_inset;
                    size.max(max_content_size.min(main_axis_available_space))
                } else if lines.len() > 1 {
                    size.max(main_axis_available_space)
                } else {
                    size
                }
            }
            AvailableSpace::MinContent if constants.is_wrap => {
                let longest_line_length: f32 = lines
                    .iter()
                    .map(|line| {
                        let line_items = line_items(items, line);
                        let line_main_axis_gap = sum_axis_gaps(constants.gap.main(dir), line_items.len());
                        let total_target_size = line_items.iter().map(|c| item_main_length(c, dir)).sum::<f32>();
                        total_target_size + line_main_axis_gap
                    })
                    .max_by(f32::total_cmp)
                    .unwrap_or(0.0);
                longest_line_length + main_content_box_inset
            }
            AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                let mut main_size = 0.0;
                for line in lines.iter() {
                    let line_len = line.len();
                    for item in line_items_mut(items, line) {
                        let content_contribution = content_contribution(view, item, available_space, constants);
                        let margin_sum = main_sum(item.margin, dir);
                        item.content_flex_fraction = {
                            let diff = content_contribution - margin_sum - item.flex_basis;
                            if diff > 0.0 {
                                diff / 1.0_f32.max(item.flex_grow)
                            } else if diff < 0.0 {
                                let scaled_shrink_factor = 1.0_f32.max(item.flex_shrink) * item.inner_flex_basis;
                                diff / scaled_shrink_factor
                            } else {
                                0.0
                            }
                        };
                    }

                    let floor_by_largest_item = constants.is_wrap && constants.is_row;
                    let mut largest_item_size = 0.0_f32;
                    let item_main_size_sum = line_items(items, line)
                        .iter()
                        .map(|item| {
                            let flex_fraction = item.content_flex_fraction;
                            let flex_contribution = if item.content_flex_fraction > 0.0 {
                                1.0_f32.max(item.flex_grow) * flex_fraction
                            } else if item.content_flex_fraction < 0.0 {
                                let scaled_shrink_factor = 1.0_f32.max(item.flex_shrink) * item.inner_flex_basis;
                                if scaled_shrink_factor == 0.0 {
                                    0.0
                                } else {
                                    scaled_shrink_factor * flex_fraction
                                }
                            } else {
                                0.0
                            };
                            let size = item.flex_basis + flex_contribution + main_sum(item.margin, dir);
                            if floor_by_largest_item {
                                largest_item_size = largest_item_size.max(size);
                            }
                            size
                        })
                        .sum::<f32>();

                    let gap_sum = sum_axis_gaps(constants.gap.main(dir), line_len);
                    main_size = f32::max(main_size, largest_item_size.max(item_main_size_sum + gap_sum));
                }
                main_size + main_content_box_inset
            }
        },
    };
    set_main_size(outer_main_size, main_content_box_inset, constants);
}

/// An item's min- or max-content contribution (in Taffy's
/// `determine_container_main_size`).
fn content_contribution<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    item: &Item,
    available_space: Size<AvailableSpace>,
    constants: &Constants,
) -> f32 {
    let dir = constants.dir;
    let style_min = item.min_size.main(dir);
    let margin_sum = main_sum(item.margin, dir);
    let style_preferred = item.size.main(dir);
    let style_max = item.max_size.main(dir);

    let clamping_basis = if constants.is_row {
        Some(item.flex_basis).maybe_max(style_preferred)
    } else {
        Some(item.flex_basis)
    };
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
            let item_pb_main = main_sum(item.padding, dir) + main_sum(item.border, dir);
            let inner_main_size = pref.max(item_pb_main);
            if constants.is_row {
                (inner_main_size + margin_sum).maybe_clamp(style_min, style_max)
            } else {
                (inner_main_size.max(item.flex_basis) + margin_sum).maybe_clamp(style_min, style_max)
            }
        }
        _ => {
            let cross_axis_parent_size = constants.node_inner_size.cross(dir);
            let child_min_cross = item.min_size.cross(dir);
            let child_max_cross = item.max_size.cross(dir);
            let cross_axis_available_space = available_space
                .cross(dir)
                .map_definite_value(|val| {
                    (constants.divided_cross_space(cross_axis_parent_size.unwrap_or(val)) - cross_sum(item.margin, dir))
                        .max(0.0)
                })
                .maybe_clamp(child_min_cross, child_max_cross);
            let child_available_space = available_space.with_cross(dir, cross_axis_available_space);

            let child_known_dimensions = {
                let mut ckd = item.size.with_main(dir, None);
                ckd.set_cross(
                    dir,
                    ckd.cross(dir)
                        .maybe_clamp(item.transferred_min.cross(dir), item.transferred_max.cross(dir)),
                );
                if item.align_self == AlignSelf::STRETCH && ckd.cross(dir).is_none() {
                    let definite =
                        !constants.is_wrap && constants.has_definite_cross_size && cross_axis_parent_size.is_some();
                    if item.aspect_ratio.is_none() || definite {
                        ckd.set_cross(dir, cross_axis_available_space.into_option());
                    }
                }
                ckd
            };

            let measured_main_size = view.measure_child_size(
                id(item.child),
                child_known_dimensions,
                constants.node_inner_size,
                child_available_space,
                SizingMode::ContentSize,
                main_axis(dir),
                Line::FALSE,
            );
            let transferred_main_size = item
                .aspect_ratio
                .zip(child_known_dimensions.cross(dir))
                .map(|(ratio, cross)| item.ratio_main(ratio, cross, dir));
            let inner_main_size = measured_main_size.maybe_max(transferred_main_size);
            if constants.is_row {
                (inner_main_size + margin_sum).maybe_clamp(style_min, style_max)
            } else {
                (inner_main_size.max(item.flex_basis) + margin_sum).maybe_clamp(style_min, style_max)
            }
        }
    }
}

/// Taffy's `resolve_flexible_lengths`, for one line's items.
fn resolve_flexible_lengths(items: &mut [Item], constants: &Constants) {
    let dir = constants.dir;
    let total_main_axis_gap = sum_axis_gaps(constants.gap.main(dir), items.len());
    let inner_main = constants.node_inner_size.main(dir);

    let total_hypothetical_outer_main_size = items
        .iter()
        .map(|child| child.hypothetical_outer_size.main(dir))
        .sum::<f32>();
    let used_flex_factor: f32 = total_main_axis_gap + total_hypothetical_outer_main_size;
    let growing = used_flex_factor < inner_main.unwrap_or(0.0);
    let shrinking = used_flex_factor > inner_main.unwrap_or(0.0);
    let exactly_sized = !growing & !shrinking;

    for child in items.iter_mut() {
        let inner_target_size = child.hypothetical_inner_size.main(dir);
        child.target_size.set_main(dir, inner_target_size);
        if exactly_sized
            || (child.flex_grow == 0.0 && child.flex_shrink == 0.0)
            || (growing && child.flex_basis > child.hypothetical_inner_size.main(dir))
            || (shrinking && child.flex_basis < child.hypothetical_inner_size.main(dir))
        {
            child.frozen = true;
            let outer_target_size = inner_target_size + main_sum(child.margin, dir);
            child.outer_target_size.set_main(dir, outer_target_size);
        }
    }
    if exactly_sized {
        return;
    }

    let used_space: f32 = total_main_axis_gap
        + items
            .iter()
            .map(|child| {
                if child.frozen {
                    child.outer_target_size.main(dir)
                } else {
                    child.flex_basis + main_sum(child.margin, dir)
                }
            })
            .sum::<f32>();
    let initial_free_space = inner_main.maybe_sub(used_space).unwrap_or(0.0);

    loop {
        let mut all_frozen = true;
        let mut items_space: f32 = 0.0;
        let mut sum_flex_grow: f32 = 0.0;
        let mut sum_flex_shrink: f32 = 0.0;
        for child in items.iter() {
            if child.frozen {
                items_space += child.outer_target_size.main(dir);
            } else {
                all_frozen = false;
                items_space += child.flex_basis + main_sum(child.margin, dir);
                sum_flex_grow += child.flex_grow;
                sum_flex_shrink += child.flex_shrink;
            }
        }
        if all_frozen {
            break;
        }

        let used_space: f32 = total_main_axis_gap + items_space;
        let free_space = if growing && sum_flex_grow < 1.0 {
            (initial_free_space * sum_flex_grow - total_main_axis_gap).maybe_min(inner_main.maybe_sub(used_space))
        } else if shrinking && sum_flex_shrink < 1.0 {
            (initial_free_space * sum_flex_shrink - total_main_axis_gap).maybe_max(inner_main.maybe_sub(used_space))
        } else {
            inner_main
                .maybe_sub(used_space)
                .unwrap_or(used_flex_factor - used_space)
        };

        if free_space.is_normal() {
            if growing && sum_flex_grow > 0.0 {
                for child in items.iter_mut().filter(|child| !child.frozen) {
                    child
                        .target_size
                        .set_main(dir, child.flex_basis + free_space * (child.flex_grow / sum_flex_grow));
                }
            } else if shrinking && sum_flex_shrink > 0.0 {
                let sum_scaled_shrink_factor: f32 = items
                    .iter()
                    .filter(|child| !child.frozen)
                    .map(|child| child.inner_flex_basis * child.flex_shrink)
                    .sum();
                if sum_scaled_shrink_factor > 0.0 {
                    for child in items.iter_mut().filter(|child| !child.frozen) {
                        let scaled_shrink_factor = child.inner_flex_basis * child.flex_shrink;
                        child.target_size.set_main(
                            dir,
                            child.flex_basis + free_space * (scaled_shrink_factor / sum_scaled_shrink_factor),
                        );
                    }
                }
            }
        }

        let total_violation = items
            .iter_mut()
            .filter(|child| !child.frozen)
            .fold(0.0, |acc, child| -> f32 {
                let resolved_min_main: Option<f32> = child.resolved_minimum_main_size.into();
                let max_main = child.max_size.main(dir);
                let clamped = child
                    .target_size
                    .main(dir)
                    .maybe_clamp(resolved_min_main, max_main)
                    .max(0.0);
                child.violation = clamped - child.target_size.main(dir);
                child.target_size.set_main(dir, clamped);
                child
                    .outer_target_size
                    .set_main(dir, child.target_size.main(dir) + main_sum(child.margin, dir));
                acc + child.violation
            });

        for child in items.iter_mut().filter(|child| !child.frozen) {
            match total_violation {
                v if v > 0.0 => child.frozen = child.violation > 0.0,
                v if v < 0.0 => child.frozen = child.violation < 0.0,
                _ => child.frozen = true,
            }
        }
    }
}

/// Taffy's `determine_hypothetical_cross_size`, for one line's items.
fn determine_hypothetical_cross_size<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    items: &mut [Item],
    constants: &Constants,
    node_size: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
) {
    let dir = constants.dir;
    let line_cross_size_is_known = !constants.is_wrap && node_size.cross(dir).is_some();

    for child in items.iter_mut() {
        let padding_border_sum = cross_sum(rect_add(child.padding, child.border), dir);
        if line_cross_size_is_known && child.is_stretched(dir) && !child.participates_in_baseline_alignment(dir) {
            continue;
        }
        let child_available_main: AvailableSpace = (constants.container_size.main(dir) - main_sum(child.margin, dir))
            .max(0.0)
            .into();

        let transferred_min_cross = child.transferred_min.cross(dir);
        let transferred_max_cross = child.transferred_max.cross(dir);

        let child_cross = child
            .size
            .cross(dir)
            .maybe_clamp(transferred_min_cross, transferred_max_cross)
            .maybe_max(padding_border_sum);

        let child_available_cross = available_space
            .cross(dir)
            .map_definite_value(|val| (constants.divided_cross_space(val) - cross_sum(child.margin, dir)).max(0.0))
            .maybe_clamp(transferred_min_cross, transferred_max_cross)
            .maybe_max(padding_border_sum);

        let cross_stretch_size = constants
            .node_inner_size
            .cross(dir)
            .map(|val| constants.divided_cross_space(val))
            .maybe_sub(cross_sum(child.margin, dir))
            .maybe_max(0.0);
        let child_available_cross = match keyword(
            child.size_style.cross(dir),
            cross_stretch_size,
            constants.node_inner_size.cross(dir),
            S::calc,
        ) {
            Some(Keyword::Measure(available)) => available,
            _ => child_available_cross,
        };

        let child_inner_cross = child_cross.unwrap_or_else(|| {
            view.compute_child_layout(
                id(child.child),
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    sizing_mode: SizingMode::ContentSize,
                    axis: cross_axis(dir).into(),
                    known_dimensions: Size {
                        width: if constants.is_row {
                            child.target_size.width.into()
                        } else {
                            child_cross
                        },
                        height: if constants.is_row {
                            child_cross
                        } else {
                            child.target_size.height.into()
                        },
                    },
                    known_dimensions_are_definite: item_known_dimension_definiteness(constants, child),
                    parent_size: constants.node_inner_size,
                    available_space: Size {
                        width: if constants.is_row {
                            child_available_main
                        } else {
                            child_available_cross
                        },
                        height: if constants.is_row {
                            child_available_cross
                        } else {
                            child_available_main
                        },
                    },
                    vertical_margins_are_collapsible: Line::FALSE,
                },
            )
            .size
            .get_abs(cross_axis(dir))
            .maybe_clamp(transferred_min_cross, transferred_max_cross)
            .max(padding_border_sum)
        });
        let child_outer_cross = child_inner_cross + cross_sum(child.margin, dir);
        child.hypothetical_inner_size.set_cross(dir, child_inner_cross);
        child.hypothetical_outer_size.set_cross(dir, child_outer_cross);
    }
}

/// Taffy's `calculate_children_base_lines`.
fn calculate_children_base_lines<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    node_size: Size<Option<f32>>,
    available_space: Size<AvailableSpace>,
    lines: &[FlexLine],
    items: &mut [Item],
    constants: &Constants,
) {
    let dir = constants.dir;
    if !constants.is_row {
        return;
    }
    for line in lines {
        let line = line_items_mut(items, line);
        let line_baseline_child_count = line
            .iter()
            .filter(|child| child.participates_in_baseline_alignment(dir))
            .count();
        if line_baseline_child_count <= 1 {
            continue;
        }
        for child in line.iter_mut() {
            if !child.participates_in_baseline_alignment(dir) {
                continue;
            }
            let measured_size_and_baselines = view.compute_child_layout(
                id(child.child),
                LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    sizing_mode: SizingMode::ContentSize,
                    axis: RequestedAxis::Both,
                    known_dimensions: Size {
                        width: if constants.is_row {
                            child.target_size.width.into()
                        } else {
                            child.hypothetical_inner_size.width.into()
                        },
                        height: if constants.is_row {
                            child.hypothetical_inner_size.height.into()
                        } else {
                            child.target_size.height.into()
                        },
                    },
                    known_dimensions_are_definite: item_known_dimension_definiteness(constants, child),
                    parent_size: constants.node_inner_size,
                    available_space: Size {
                        width: if constants.is_row {
                            constants.container_size.width.into()
                        } else {
                            available_space.width.maybe_set(node_size.width)
                        },
                        height: if constants.is_row {
                            available_space.height.maybe_set(node_size.height)
                        } else {
                            constants.container_size.height.into()
                        },
                    }
                    .maybe_sub(child.margin.sum_axes())
                    .maybe_max(Size::ZERO),
                    vertical_margins_are_collapsible: Line::FALSE,
                },
            );
            let height = measured_size_and_baselines.size.height;
            let baseline = child_first_baseline(measured_size_and_baselines.baselines.first, height, child.overflow.y)
                .unwrap_or(height);
            child.baseline = baseline + child.margin.top;
        }
    }
}

/// Taffy's `calculate_cross_size`.
fn calculate_cross_size(lines: &mut [FlexLine], items: &[Item], node_size: Size<Option<f32>>, constants: &Constants) {
    let dir = constants.dir;
    if !constants.is_wrap && node_size.cross(dir).is_some() {
        let cross_axis_padding_border = cross_sum(constants.content_box_inset, dir);
        let cross_min_size = constants.min_size.cross(dir);
        let cross_max_size = constants.max_size.cross(dir);
        if let Some(first) = lines.first_mut() {
            first.cross_size = node_size
                .cross(dir)
                .maybe_clamp(cross_min_size, cross_max_size)
                .maybe_sub(cross_axis_padding_border)
                .maybe_max(0.0)
                .unwrap_or(0.0);
        }
    } else {
        for line in lines.iter_mut() {
            let line_items = line_items(items, line);
            let max_baseline: f32 = line_items.iter().map(|child| child.baseline).fold(0.0, f32::max);
            line.cross_size = line_items
                .iter()
                .map(|child| {
                    if child.participates_in_baseline_alignment(dir) {
                        max_baseline - child.baseline + child.hypothetical_outer_size.cross(dir)
                    } else {
                        child.hypothetical_outer_size.cross(dir)
                    }
                })
                .fold(0.0, f32::max);
        }
        if !constants.is_wrap {
            let cross_axis_padding_border = cross_sum(constants.content_box_inset, dir);
            let cross_min_size = constants.min_size.cross(dir);
            let cross_max_size = constants.max_size.cross(dir);
            if let Some(first) = lines.first_mut() {
                first.cross_size = first.cross_size.maybe_clamp(
                    cross_min_size.maybe_sub(cross_axis_padding_border),
                    cross_max_size.maybe_sub(cross_axis_padding_border),
                );
            }
        }
    }
}

/// Taffy's `handle_align_content_stretch`.
fn handle_align_content_stretch(lines: &mut [FlexLine], node_size: Size<Option<f32>>, constants: &Constants) {
    let dir = constants.dir;
    if constants.align_content == AlignContent::STRETCH {
        let cross_axis_padding_border = cross_sum(constants.content_box_inset, dir);
        let cross_min_size = constants.min_size.cross(dir);
        let cross_max_size = constants.max_size.cross(dir);
        let container_min_inner_cross = node_size
            .cross(dir)
            .or(cross_min_size)
            .maybe_clamp(cross_min_size, cross_max_size)
            .maybe_sub(cross_axis_padding_border)
            .maybe_max(0.0)
            .unwrap_or(0.0);
        let total_cross_axis_gap = sum_axis_gaps(constants.gap.cross(dir), lines.len());
        let lines_total_cross: f32 = lines.iter().map(|line| line.cross_size).sum::<f32>() + total_cross_axis_gap;
        if lines_total_cross < container_min_inner_cross {
            let remaining = container_min_inner_cross - lines_total_cross;
            let addition = remaining / usize_to_f32(lines.len());
            for line in lines.iter_mut() {
                line.cross_size += addition;
            }
        }
    }
}

/// Taffy's `determine_used_cross_size`.
fn determine_used_cross_size<S: NodeStyle, L: Measure>(
    view: &View<'_, S, L>,
    lines: &[FlexLine],
    items: &mut [Item],
    constants: &Constants,
) {
    let dir = constants.dir;
    for line in lines {
        let line_cross_size = line.cross_size;
        for child in line_items_mut(items, line) {
            let cross = if child.is_stretched(dir) {
                // The max ignores the ratio here (Chrome and Firefox agree).
                let max_size_ignoring_aspect_ratio = view.node.children.get(child.child).map_or(Size::NONE, |n| {
                    n.style
                        .view()
                        .max_size()
                        .maybe_resolve(constants.node_inner_size, S::calc)
                        .maybe_add(child.box_sizing)
                });
                (line_cross_size - cross_sum(child.margin, dir))
                    .max(0.0)
                    .maybe_clamp(child.min_size.cross(dir), max_size_ignoring_aspect_ratio.cross(dir))
            } else {
                child.hypothetical_inner_size.cross(dir)
            };
            child.target_size.set_cross(dir, cross);
            child
                .outer_target_size
                .set_cross(dir, child.target_size.cross(dir) + cross_sum(child.margin, dir));
        }
    }
}

/// Taffy's `distribute_remaining_free_space`.
fn distribute_remaining_free_space(lines: &[FlexLine], items: &mut [Item], constants: &Constants) {
    let dir = constants.dir;
    for line in lines {
        let line = line_items_mut(items, line);
        let total_main_axis_gap = sum_axis_gaps(constants.gap.main(dir), line.len());
        let used_space: f32 =
            total_main_axis_gap + line.iter().map(|child| child.outer_target_size.main(dir)).sum::<f32>();
        let mut free_space = constants.inner_container_size.main(dir) - used_space;
        let mut num_auto_margins = 0_usize;
        for child in line.iter() {
            if child.margin_is_auto.main_start(dir) {
                num_auto_margins = num_auto_margins.saturating_add(1);
            }
            if child.margin_is_auto.main_end(dir) {
                num_auto_margins = num_auto_margins.saturating_add(1);
            }
        }
        if free_space > 0.0 && num_auto_margins > 0 {
            let margin = free_space / usize_to_f32(num_auto_margins);
            for child in line.iter_mut() {
                if child.margin_is_auto.main_start(dir) {
                    if constants.is_row {
                        child.margin.left = margin;
                    } else {
                        child.margin.top = margin;
                    }
                }
                if child.margin_is_auto.main_end(dir) {
                    if constants.is_row {
                        child.margin.right = margin;
                    } else {
                        child.margin.bottom = margin;
                    }
                }
            }
            free_space = 0.0;
        }

        let num_items = line.len();
        let layout_reverse = is_reverse(dir);
        let gap = constants.gap.main(dir);
        let justify_content_mode = alignment_fallback(free_space, num_items, constants.justify_content);
        let justify_item = |(i, child): (usize, &mut Item)| {
            child.offset_main =
                alignment_offset(free_space, num_items, gap, justify_content_mode, layout_reverse, i == 0);
        };
        if layout_reverse {
            line.iter_mut().rev().enumerate().for_each(justify_item);
        } else {
            line.iter_mut().enumerate().for_each(justify_item);
        }
    }
}

/// Taffy's `resolve_cross_axis_auto_margins`.
fn resolve_cross_axis_auto_margins(lines: &[FlexLine], items: &mut [Item], constants: &Constants) {
    let dir = constants.dir;
    for line in lines {
        let line_cross_size = line.cross_size;
        let line = line_items_mut(items, line);
        let max_baseline: f32 = line.iter().map(|child| child.baseline).fold(0.0, f32::max);
        let max_baseline_to_bottom_distance: f32 = line
            .iter()
            .filter(|child| child.participates_in_baseline_alignment(dir))
            .map(|child| child.outer_target_size.cross(dir) - child.baseline)
            .fold(0.0, f32::max);

        for child in line.iter_mut() {
            let free_space = line_cross_size - child.outer_target_size.cross(dir);
            if child.margin_is_auto.cross_start(dir) && child.margin_is_auto.cross_end(dir) {
                if constants.is_row {
                    child.margin.top = free_space / 2.0;
                    child.margin.bottom = free_space / 2.0;
                } else {
                    child.margin.left = free_space / 2.0;
                    child.margin.right = free_space / 2.0;
                }
            } else if child.margin_is_auto.cross_start(dir) {
                if constants.is_row {
                    child.margin.top = free_space;
                } else {
                    child.margin.left = free_space;
                }
            } else if child.margin_is_auto.cross_end(dir) {
                if constants.is_row {
                    child.margin.bottom = free_space;
                } else {
                    child.margin.right = free_space;
                }
            } else {
                child.offset_cross = align_flex_items_along_cross_axis(
                    child,
                    free_space,
                    max_baseline,
                    max_baseline_to_bottom_distance,
                    constants,
                );
            }
        }
    }
}

/// Taffy's `align_flex_items_along_cross_axis`.
fn align_flex_items_along_cross_axis(
    child: &Item,
    free_space: f32,
    max_baseline: f32,
    max_baseline_to_bottom_distance: f32,
    constants: &Constants,
) -> f32 {
    let cross_axis_should_reverse = constants.is_column && matches!(constants.layout_direction, Direction::Rtl);
    let align_keyword = if child.align_self.is_safe() && free_space < 0.0 {
        AlignItemsKeyword::Start
    } else {
        child.align_self.keyword
    };
    match align_keyword {
        // `normal` is resolved to `stretch`, and `self-start`/`self-end`
        // to `start`/`end` when the items are generated.
        AlignItemsKeyword::Start | AlignItemsKeyword::SelfStart => {
            if cross_axis_should_reverse {
                free_space
            } else {
                0.0
            }
        }
        AlignItemsKeyword::FlexStart | AlignItemsKeyword::Stretch | AlignItemsKeyword::Normal => {
            if constants.is_wrap_reverse ^ cross_axis_should_reverse {
                free_space
            } else {
                0.0
            }
        }
        AlignItemsKeyword::End | AlignItemsKeyword::SelfEnd => {
            if cross_axis_should_reverse {
                0.0
            } else {
                free_space
            }
        }
        AlignItemsKeyword::FlexEnd => {
            if constants.is_wrap_reverse ^ cross_axis_should_reverse {
                0.0
            } else {
                free_space
            }
        }
        AlignItemsKeyword::Center => free_space / 2.0,
        AlignItemsKeyword::Baseline => {
            if constants.is_row {
                if constants.is_wrap_reverse {
                    let line_cross_size = free_space + child.outer_target_size.cross(constants.dir);
                    line_cross_size - max_baseline_to_bottom_distance - child.baseline
                } else {
                    max_baseline - child.baseline
                }
            } else {
                let baseline_column_should_reverse = cross_axis_should_reverse && !constants.is_wrap;
                if constants.is_wrap_reverse ^ baseline_column_should_reverse {
                    free_space
                } else {
                    0.0
                }
            }
        }
    }
}

/// Taffy's `determine_container_cross_size`.
fn determine_container_cross_size(lines: &[FlexLine], node_size: Size<Option<f32>>, constants: &mut Constants) -> f32 {
    let dir = constants.dir;
    let total_cross_axis_gap = sum_axis_gaps(constants.gap.cross(dir), lines.len());
    let total_line_cross_size: f32 = lines.iter().map(|line| line.cross_size).sum::<f32>();

    let padding_border_sum = cross_sum(constants.content_box_inset, dir);
    let cross_scrollbar_gutter = point_cross(constants.scrollbar_gutter, dir);
    let min_cross_size = constants.min_size.cross(dir);
    let max_cross_size = constants.max_size.cross(dir);
    let outer_container_size = node_size
        .cross(dir)
        .unwrap_or(total_line_cross_size + total_cross_axis_gap + padding_border_sum)
        .maybe_clamp(min_cross_size, max_cross_size)
        .max(padding_border_sum - cross_scrollbar_gutter);
    let inner_container_size = (outer_container_size - padding_border_sum).max(0.0);

    constants.container_size.set_cross(dir, outer_container_size);
    constants.inner_container_size.set_cross(dir, inner_container_size);
    total_line_cross_size
}

/// Taffy's `align_flex_lines_per_align_content`.
fn align_flex_lines_per_align_content(lines: &mut [FlexLine], constants: &Constants, total_cross_size: f32) {
    let num_lines = lines.len();
    let gap = constants.gap.cross(constants.dir);
    let total_cross_axis_gap = sum_axis_gaps(gap, num_lines);
    let free_space = constants.inner_container_size.cross(constants.dir) - total_cross_size - total_cross_axis_gap;
    let align_content_mode = alignment_fallback(free_space, num_lines, constants.align_content);
    let align_line = |(i, line): (usize, &mut FlexLine)| {
        line.offset_cross = alignment_offset(
            free_space,
            num_lines,
            gap,
            align_content_mode,
            constants.is_wrap_reverse,
            i == 0,
        );
    };
    if constants.is_wrap_reverse {
        lines.iter_mut().rev().enumerate().for_each(align_line);
    } else {
        lines.iter_mut().enumerate().for_each(align_line);
    }
}

/// Taffy's `calculate_flex_item`.
fn calculate_flex_item<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    item: &mut Item,
    total_offset_main: &mut f32,
    total_offset_cross: f32,
    line_offset_cross: f32,
    total_overflow_rect: &mut Rect<f32>,
    constants: &Constants,
) {
    let container_size = constants.container_size;
    let border = constants.border;
    let direction = constants.dir;
    let layout_direction = constants.layout_direction;
    let mut layout_output = view.compute_child_layout(
        id(item.child),
        LayoutInput {
            run_mode: RunMode::PerformLayout,
            sizing_mode: SizingMode::ContentSize,
            axis: RequestedAxis::Both,
            known_dimensions: item.target_size.map(Some),
            known_dimensions_are_definite: item_known_dimension_definiteness(constants, item),
            parent_size: constants.node_inner_size,
            available_space: size_sub(container_size, item.margin.sum_axes()).map(|s| s.max(0.0).into()),
            vertical_margins_are_collapsible: Line::FALSE,
        },
    );
    let size = layout_output.size;
    let scrollable_overflow_rect = layout_output.scrollable_overflow_rect;

    let is_rtl_row = is_row(direction) && layout_direction.is_rtl();
    let is_rtl_column = is_column(direction) && layout_direction.is_rtl();
    let main_relative_inset = item.relative_inset.main(direction);
    let cross_relative_inset = item.relative_inset.cross(direction);
    let effective_line_offset_cross = if is_rtl_column { 0.0 } else { line_offset_cross };

    let offset_main = if is_rtl_row {
        *total_offset_main - item.offset_main - item.margin.main_end(direction) - main_relative_inset - size.width
    } else {
        *total_offset_main + item.offset_main + item.margin.main_start(direction) + main_relative_inset
    };
    let offset_cross = total_offset_cross
        + item.offset_cross
        + effective_line_offset_cross
        + item.margin.cross_start(direction)
        + cross_relative_inset;

    let inner_baseline =
        child_first_baseline(layout_output.baselines.first, size.height, item.overflow.y).unwrap_or(size.height);
    if is_row(direction) {
        let baseline_offset_cross =
            total_offset_cross + item.offset_cross + effective_line_offset_cross + item.margin.cross_start(direction);
        item.baseline = baseline_offset_cross + inner_baseline;
    } else {
        let baseline_offset_main = *total_offset_main + item.offset_main + item.margin.main_start(direction);
        item.baseline = baseline_offset_main + inner_baseline;
    }

    let location = if is_row(direction) {
        Point {
            x: offset_main,
            y: offset_cross,
        }
    } else {
        Point {
            x: offset_cross,
            y: offset_main,
        }
    };
    let scrollbar_size = Size {
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
    };

    view.set_unrounded_layout(
        id(item.child),
        &Layout {
            order: item.order,
            size,
            scrollable_overflow_rect,
            scrollbar_size,
            location,
            padding: item.padding,
            border: item.border,
            margin: item.margin,
        },
    );

    item.oof_candidates = layout_output.oof_candidates.take();
    if !item.oof_candidates.is_empty() {
        item.oof_candidates.translate(location);
    }

    if is_rtl_row {
        *total_offset_main -= item.offset_main + main_sum(item.margin, direction) + size.main(direction);
    } else {
        *total_offset_main += item.offset_main + main_sum(item.margin, direction) + size.main(direction);
    }

    let contribution_location = if layout_direction.is_rtl() {
        Point {
            x: container_size.width - (location.x + size.width) - border.right,
            y: location.y - border.top,
        }
    } else {
        Point {
            x: location.x - border.left,
            y: location.y - border.top,
        }
    };
    *total_overflow_rect = total_overflow_rect.union(overflow_contribution(
        contribution_location,
        size,
        scrollable_overflow_rect,
        item.overflow,
        item.contain,
        constants.is_scroll_container,
    ));
}

/// Taffy's `calculate_layout_line`.
fn calculate_layout_line<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    line: &FlexLine,
    items: &mut [Item],
    total_offset_cross: &mut f32,
    overflow_rect: &mut Rect<f32>,
    constants: &Constants,
) {
    let container_size = constants.container_size;
    let padding_border = constants.content_box_inset;
    let direction = constants.dir;
    let layout_direction = constants.layout_direction;
    let mut total_offset_main = if layout_direction.is_rtl() && is_row(direction) {
        container_size.width - padding_border.main_end(direction)
    } else {
        padding_border.main_start(direction)
    };
    let line_offset_cross = line.offset_cross;

    let is_rtl_column = layout_direction.is_rtl() && is_column(direction);
    if is_rtl_column {
        *total_offset_cross -= line_offset_cross + line.cross_size;
    }

    let line_items = line_items_mut(items, line);
    if is_reverse(direction) {
        for item in line_items.iter_mut().rev() {
            calculate_flex_item(
                view,
                item,
                &mut total_offset_main,
                *total_offset_cross,
                line_offset_cross,
                overflow_rect,
                constants,
            );
        }
    } else {
        for item in line_items.iter_mut() {
            calculate_flex_item(
                view,
                item,
                &mut total_offset_main,
                *total_offset_cross,
                line_offset_cross,
                overflow_rect,
                constants,
            );
        }
    }

    if !is_rtl_column {
        *total_offset_cross += line_offset_cross + line.cross_size;
    }
}

/// Taffy's `final_layout_pass`.
fn final_layout_pass<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    lines: &[FlexLine],
    items: &mut [Item],
    constants: &Constants,
) -> Rect<f32> {
    let mut total_offset_cross = if constants.is_column && constants.layout_direction.is_rtl() {
        constants.container_size.width - constants.content_box_inset.cross_end(constants.dir)
    } else {
        constants.content_box_inset.cross_start(constants.dir)
    };
    let mut overflow_rect = Rect::ZERO;
    if constants.is_wrap_reverse {
        for line in lines.iter().rev() {
            calculate_layout_line(
                view,
                line,
                items,
                &mut total_offset_cross,
                &mut overflow_rect,
                constants,
            );
        }
    } else {
        for line in lines {
            calculate_layout_line(
                view,
                line,
                items,
                &mut total_offset_cross,
                &mut overflow_rect,
                constants,
            );
        }
    }
    if constants.is_scroll_container {
        overflow_rect.right += if constants.layout_direction.is_rtl() {
            constants.content_box_inset.left - constants.border.left - constants.scrollbar_gutter.x
        } else {
            constants.content_box_inset.right - constants.border.right - constants.scrollbar_gutter.x
        };
        overflow_rect.bottom +=
            constants.content_box_inset.bottom - constants.border.bottom - constants.scrollbar_gutter.y;
    }
    overflow_rect
}

/// The static edge a cross-axis alignment puts an out-of-flow child at.
fn cross_edge_for(keyword: AlignItemsKeyword, cross_is_rtl: bool, flex_start_reversed: bool) -> AxisStaticEdge {
    let start_position = match keyword {
        AlignItemsKeyword::Start | AlignItemsKeyword::Baseline | AlignItemsKeyword::SelfStart => !cross_is_rtl,
        AlignItemsKeyword::End | AlignItemsKeyword::SelfEnd => cross_is_rtl,
        _ => true,
    };
    match (keyword, flex_start_reversed) {
        (
            AlignItemsKeyword::Start
            | AlignItemsKeyword::End
            | AlignItemsKeyword::Baseline
            | AlignItemsKeyword::SelfStart
            | AlignItemsKeyword::SelfEnd,
            _,
        ) => {
            if start_position {
                AxisStaticEdge::Start
            } else {
                AxisStaticEdge::End
            }
        }
        (AlignItemsKeyword::Stretch | AlignItemsKeyword::FlexStart | AlignItemsKeyword::Normal, false)
        | (AlignItemsKeyword::FlexEnd, true) => AxisStaticEdge::Start,
        (AlignItemsKeyword::Stretch | AlignItemsKeyword::FlexStart | AlignItemsKeyword::Normal, true)
        | (AlignItemsKeyword::FlexEnd, false) => AxisStaticEdge::End,
        (AlignItemsKeyword::Center, _) => AxisStaticEdge::Center,
    }
}

/// Taffy's `collect_oof_candidates`.
fn collect_oof_candidates<S: NodeStyle, L: Measure>(
    view: &View<'_, S, L>,
    constants: &Constants,
    items: &mut [Item],
    candidates: &mut OofCandidates,
) {
    let mut items = items
        .iter_mut()
        .filter(|item| !item.oof_candidates.is_empty())
        .peekable();

    let dir = constants.dir;
    let container_size = constants.container_size;
    let content_box_inset = constants.content_box_inset;
    let main_axis_is_horizontal = constants.is_row;
    let cross_axis_is_horizontal = !constants.is_row;
    let main_is_rtl = main_axis_is_horizontal && constants.layout_direction.is_rtl();
    let cross_is_rtl = cross_axis_is_horizontal && constants.layout_direction.is_rtl();
    let main_axis_flex_start_reversed = is_reverse(dir) ^ main_is_rtl;
    let cross_axis_flex_start_reversed = constants.is_wrap_reverse ^ cross_is_rtl;

    let main_area = Line {
        start: content_box_inset.main_start(dir),
        end: container_size.main(dir) - content_box_inset.main_end(dir),
    };
    let cross_area = Line {
        start: content_box_inset.cross_start(dir),
        end: container_size.cross(dir) - content_box_inset.cross_end(dir),
    };

    for (order, node) in view.node.children.iter().enumerate() {
        let child_style = node.style.view();
        let position = child_style.position();
        if child_style.box_generation_mode() == BoxGenerationMode::None {
            continue;
        }
        if !position.is_out_of_flow() {
            if let Some(item) = items.next_if(|item| item.order == order_of(order)) {
                candidates.append(&mut item.oof_candidates);
            }
            continue;
        }

        let align_self =
            resolve_normal_alignment(FlexboxItemStyle::align_self(&child_style).unwrap_or(constants.align_items))
                .resolve_self_relative(child_style.direction(), constants.layout_direction, constants.is_column);

        let main_keyword = constants.justify_content.keyword;
        let main_start_position = match main_keyword {
            AlignContentKeyword::Start => !main_is_rtl,
            AlignContentKeyword::End => main_is_rtl,
            _ => true,
        };
        let main_edge = match (main_keyword, main_axis_flex_start_reversed) {
            (
                AlignContentKeyword::SpaceBetween
                | AlignContentKeyword::Normal
                | AlignContentKeyword::Stretch
                | AlignContentKeyword::FlexStart,
                false,
            )
            | (AlignContentKeyword::FlexEnd, true) => AxisStaticEdge::Start,
            (AlignContentKeyword::Start | AlignContentKeyword::End, _) => {
                if main_start_position {
                    AxisStaticEdge::Start
                } else {
                    AxisStaticEdge::End
                }
            }
            (AlignContentKeyword::FlexEnd, false)
            | (
                AlignContentKeyword::FlexStart
                | AlignContentKeyword::Normal
                | AlignContentKeyword::Stretch
                | AlignContentKeyword::SpaceBetween,
                true,
            ) => AxisStaticEdge::End,
            (AlignContentKeyword::SpaceEvenly | AlignContentKeyword::SpaceAround | AlignContentKeyword::Center, _) => {
                AxisStaticEdge::Center
            }
        };

        let cross = AxisStaticPosition {
            area: cross_area,
            align: AxisStaticAlign {
                keyword: cross_edge_for(align_self.keyword, cross_is_rtl, cross_axis_flex_start_reversed),
                safety: align_self.safety,
                fallback: cross_edge_for(
                    self_alignment_safety(align_self, true),
                    cross_is_rtl,
                    cross_axis_flex_start_reversed,
                ),
            },
        };
        let main = AxisStaticPosition {
            area: main_area,
            align: AxisStaticAlign::from_keyword(main_edge),
        };
        let static_position = if constants.is_row {
            Point { x: main, y: cross }
        } else {
            Point { x: cross, y: main }
        };
        candidates.push(OofCandidate {
            node: id(order),
            order: order_of(order),
            position,
            static_position,
        });
    }
}
