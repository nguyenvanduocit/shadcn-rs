//! Block layout: Taffy 0.15.1's `compute_block_layout` (block-level
//! children, margin collapsing, floats, `justify-self`, `align-content`,
//! the static positions of out-of-flow children) ported step for step on
//! the retained tree, in Taffy's order of float operations, so the result
//! is bit-identical to Taffy's where nothing below says otherwise.
//!
//! The block formatting context is ported with it ([`BlockContext`]): its
//! float bookkeeping is Taffy's own `FloatContext`, but the margin-collapse
//! strut state Taffy keeps beside it is private to Taffy.
//!
//! `aspect-ratio` differs (`sizing.rs`): the box resolves its own ratio,
//! at least as tall (or wide) as its content needs, and a child's ratio is
//! the child's to resolve: an item's size is passed down as its style has
//! it, and its min and max carry through its ratio only into an `auto`
//! axis.

use taffy::style_helpers::TaffyMaxContent;
use taffy::{
    AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignSelf, AlignmentSafety, AvailableSpace,
    AxisStaticPosition, Baselines, BfcSlot, BlockContainerStyle, BlockItemStyle, BoxGenerationMode, BoxSizing, Clear,
    CollapsibleMarginSet, Contain, ContentSlot, CoreStyle, Dimension, Direction, Float, FloatContext, FloatDirection,
    FloatIntrinsicWidthCalculator, Layout, LayoutInput, LayoutOutput, LayoutPartialTree, LengthPercentageAuto, Line,
    MaybeMath, MaybeResolve, NodeId, OofCandidate, OofCandidates, OofPositioningArea, Overflow, Point, Position, Rect,
    RequestedAxis, ResolveOrZero, RunMode, Size, SizingMode, TextAlign,
};

use super::axes::{rect_add, size_sub, zip_size};
use super::grid::{alignment_fallback, child_first_baseline, overflow_contribution};
use super::row::alignment_offset;
use super::sizing::{Keyword, Laid, Ratio, Reports, keyword, ratio_of, transfer, with_ratio};
use super::{Measure, NodeStyle, View};
use crate::convert::usize_to_f32;

/// Taffy's `FIT_TOLERANCE`: how far a box may overhang a float's slot and still fit.
const FIT_TOLERANCE: f32 = 0.001;

/// Taffy's `compute_block_align_content_offset`.
fn align_content_offset(align_content: AlignContent, free_space: f32) -> f32 {
    let keyword = alignment_fallback(free_space, 1, align_content);
    alignment_offset(free_space, 1, 0.0, keyword, false, true)
}

/// A block formatting context: where its floats are.
#[derive(Default)]
pub struct BlockFormattingContext {
    floats: FloatContext,
}

impl BlockFormattingContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The context of its root block.
    pub fn root_block_context(&mut self) -> BlockContext<'_> {
        BlockContext {
            bfc: self,
            y_offset: 0.0,
            insets: [0.0, 0.0],
            content_box_insets: [0.0, 0.0],
            float_content_contribution: f32::NEG_INFINITY,
            is_root: true,
            adjoining_floats: [false, false],
            top_adjoining_floats: None,
        }
    }
}

/// One block within a block formatting context (Taffy's `BlockContext`).
pub struct BlockContext<'bfc> {
    bfc: &'bfc mut BlockFormattingContext,
    /// Its border-top below the root's.
    y_offset: f32,
    /// Its border box's insets from the root's, each side.
    insets: [f32; 2],
    /// Its content box's insets.
    content_box_insets: [f32; 2],
    /// How far down its floats reach (`-inf`: none).
    float_content_contribution: f32,
    is_root: bool,
    /// A float was placed on each side while its strut was unresolved.
    adjoining_floats: [bool; 2],
    /// `adjoining_floats` when its first in-flow content was committed.
    top_adjoining_floats: Option<[bool; 2]>,
}

impl BlockContext<'_> {
    /// The context of a child block.
    pub fn sub_context(&mut self, additional_y_offset: f32, insets: [f32; 2]) -> BlockContext<'_> {
        let insets = [self.insets[0] + insets[0], self.insets[1] + insets[1]];
        BlockContext {
            bfc: self.bfc,
            y_offset: self.y_offset + additional_y_offset,
            insets,
            content_box_insets: insets,
            float_content_contribution: f32::NEG_INFINITY,
            is_root: false,
            adjoining_floats: self.adjoining_floats,
            top_adjoining_floats: None,
        }
    }

    #[must_use]
    pub fn is_bfc_root(&self) -> bool {
        self.is_root
    }

    fn set_width(&mut self, available_width: f32) {
        self.bfc.floats.set_width(available_width);
    }

    fn apply_content_box_inset(&mut self, content_box_x_insets: [f32; 2]) {
        self.content_box_insets[0] = self.insets[0] + content_box_x_insets[0];
        self.content_box_insets[1] = self.insets[1] + content_box_x_insets[1];
    }

    /// Whether its context holds any float.
    #[must_use]
    pub fn has_floats(&self) -> bool {
        self.bfc.floats.has_floats()
    }

    fn has_active_floats(&self, min_y: f32) -> bool {
        self.bfc.floats.has_active_floats(min_y + self.y_offset)
    }

    fn place_floated_box(
        &mut self,
        floated_box: Size<f32>,
        min_y: f32,
        direction: FloatDirection,
        clear: Clear,
        adjoins_unresolved_strut: bool,
    ) -> Point<f32> {
        if adjoins_unresolved_strut {
            match direction {
                FloatDirection::Left => self.adjoining_floats[0] = true,
                FloatDirection::Right => self.adjoining_floats[1] = true,
            }
        }
        let mut pos = self.bfc.floats.place_floated_box(
            floated_box,
            min_y + self.y_offset,
            self.content_box_insets,
            direction,
            clear,
        );
        pos.y -= self.y_offset;
        pos.x -= self.insets[0];
        self.float_content_contribution = self.float_content_contribution.max(pos.y + floated_box.height);
        pos
    }

    #[expect(
        dead_code,
        reason = "Taffy's block layout keeps it for inline content, which gui lays out in Parley"
    )]
    fn find_content_slot(&self, min_y: f32, clear: Clear, after: Option<usize>) -> ContentSlot {
        let mut slot = self
            .bfc
            .floats
            .find_content_slot(min_y + self.y_offset, self.content_box_insets, clear, after);
        slot.y -= self.y_offset;
        slot.x -= self.insets[0];
        slot
    }

    fn find_bfc_slot(
        &self,
        min_y: f32,
        margins: [f32; 2],
        direction: Direction,
        clear: Clear,
        after: Option<usize>,
    ) -> BfcSlot {
        let mut slot = self.bfc.floats.find_bfc_slot(
            min_y + self.y_offset,
            self.content_box_insets,
            margins,
            direction,
            clear,
            after,
        );
        slot.y -= self.y_offset;
        slot.x -= self.insets[0];
        slot
    }

    fn cleared_threshold(&self, clear: Clear) -> Option<f32> {
        self.bfc
            .floats
            .cleared_threshold(clear)
            .map(|threshold| threshold - self.y_offset)
    }

    fn has_adjoining_float(&self, clear: Clear) -> bool {
        match clear {
            Clear::Left => self.adjoining_floats[0],
            Clear::Right => self.adjoining_floats[1],
            Clear::Both => self.adjoining_floats[0] || self.adjoining_floats[1],
            Clear::None => false,
        }
    }

    fn merge_adjoining_floats(&mut self, flags: [bool; 2]) {
        self.adjoining_floats[0] |= flags[0];
        self.adjoining_floats[1] |= flags[1];
    }

    fn commit_strut(&mut self) {
        if self.top_adjoining_floats.is_none() {
            self.top_adjoining_floats = Some(self.adjoining_floats);
        }
        self.adjoining_floats = [false, false];
    }

    fn top_adjoining_floats(&self) -> [bool; 2] {
        self.top_adjoining_floats.unwrap_or(self.adjoining_floats)
    }

    fn add_child_floated_content_height_contribution(&mut self, child_contribution: f32) {
        self.float_content_contribution = self.float_content_contribution.max(child_contribution);
    }

    fn floated_content_height_contribution(&self) -> f32 {
        self.float_content_contribution
    }
}

/// One child as the algorithm sees it (Taffy's `BlockItem`).
struct Item {
    child: usize,
    order: u32,
    is_table: bool,
    is_compressible_replaced: bool,
    is_in_same_bfc: bool,
    float: Float,
    clear: Clear,
    size_style: Size<Dimension>,
    /// Its size as styled (no ratio applied: the child resolves its own).
    size: Size<Option<f32>>,
    min_size: Size<Option<f32>>,
    max_size: Size<Option<f32>>,
    /// Its ratio gives its width from its height: the child resolves it.
    width_from_ratio: bool,
    overflow: Point<Overflow>,
    contain: Contain,
    scrollbar_width: f32,
    position: Position,
    direction: Direction,
    align_self: Option<AlignSelf>,
    justify_self: Option<AlignSelf>,
    inset: Rect<LengthPercentageAuto>,
    margin: Rect<LengthPercentageAuto>,
    padding: Rect<f32>,
    border: Rect<f32>,
    padding_border_sum: Size<f32>,
    computed_size: Size<f32>,
    static_position: Point<f32>,
    can_be_collapsed_through: bool,
    final_layout: Option<Layout>,
    oof_candidates: OofCandidates,
}

fn id(child: usize) -> NodeId {
    NodeId::new(u64::try_from(child).unwrap_or(u64::MAX))
}

fn order_of(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// Lay `view`'s node out as a block container for `inputs` (Taffy's
/// `compute_block_layout`), in `block_ctx` when it joins its parent's
/// block formatting context.
pub(super) fn compute<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    inputs: LayoutInput,
    block_ctx: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    let LayoutInput {
        known_dimensions,
        parent_size,
        run_mode,
        ..
    } = inputs;
    let calc = S::calc;
    let (styled, establishes_new_bfc, contain, ratio) = {
        let style = view.node.style.view();
        let overflow = style.overflow();
        let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();
        let contain = style.contain();
        let establishes_new_bfc = is_scroll_container
            || BlockContainerStyle::align_content(&style).keyword != AlignContentKeyword::Normal
            || contain.establishes_independent_formatting_context();
        let aspect_ratio = ratio_of(&style);
        let padding = style.padding().resolve_or_zero(parent_size.width, calc);
        let border = style.border().resolve_or_zero(parent_size.width, calc);
        let padding_border_size = rect_add(padding, border).sum_axes();
        let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
            padding_border_size
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
        let styled = known_dimensions
            .or(min_max_definite_size)
            .or(clamped_style_size)
            .maybe_max(padding_border_size);
        let ratio = Ratio::find(
            aspect_ratio,
            styled,
            box_sizing_adjustment,
            raw_min.maybe_add(box_sizing_adjustment),
            raw_max.maybe_add(box_sizing_adjustment),
            padding_border_size,
            is_scroll_container,
        );
        (styled, establishes_new_bfc, contain, ratio)
    };

    // Short-circuit layout if the container's size is fully determined by
    // the container's size and the run mode is ComputeSize. (A ratio leaves
    // one axis unknown here: it decides it below, with the content.)
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

    let mut output = match ratio {
        None => {
            in_context(
                view,
                LayoutInput {
                    known_dimensions: styled,
                    ..inputs
                },
                establishes_new_bfc,
                block_ctx,
                Ask::LAYOUT,
            )
            .0
        }
        Some(ratio) => {
            // Only the last run joins the parent's formatting context: the
            // runs before it must place no float there.
            let joins = block_ctx.is_some() && !establishes_new_bfc;
            let mut block_ctx = block_ctx;
            with_ratio(
                view,
                LayoutInput {
                    known_dimensions: styled,
                    ..inputs
                },
                ratio,
                Reports::Content { rerun_last: joins },
                |view, inputs, last| {
                    let (output, content) = in_context(
                        view,
                        inputs,
                        establishes_new_bfc,
                        if last { block_ctx.as_deref_mut() } else { None },
                        Ask {
                            content: true,
                            percent_height: percent_height(ratio),
                        },
                    );
                    Laid { output, content }
                },
            )
        }
    };
    if contain.suppresses_baseline() {
        output.baselines = Baselines::NONE;
    }
    output
}

/// What a run of the block algorithm is asked besides its layout.
#[derive(Clone, Copy)]
struct Ask {
    /// What its content takes in height (laid out even at a known height).
    content: bool,
    /// The height its children's percentages resolve against, when not its
    /// own: a ratio's height, which stays their basis when the content
    /// makes the box taller (Chrome: a `height: 150%` child of a box the
    /// ratio makes 80 tall is 120, and so is the box).
    percent_height: Option<f32>,
}

impl Ask {
    const LAYOUT: Self = Self {
        content: false,
        percent_height: None,
    };
}

/// The basis a ratio gives its box's children's percentage heights.
fn percent_height(ratio: Ratio) -> Option<f32> {
    (ratio.axis == taffy::AbsoluteAxis::Vertical).then_some(ratio.size)
}

/// Lay the container out in its parent's formatting context, or in a new
/// one; with [`Ask::content`], also what its content takes in height
/// ([`compute_inner`]).
fn in_context<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    inputs: LayoutInput,
    establishes_new_bfc: bool,
    block_ctx: Option<&mut BlockContext<'_>>,
    ask: Ask,
) -> (LayoutOutput, Option<f32>) {
    match block_ctx {
        Some(inherited) if !establishes_new_bfc => compute_inner(view, inputs, inherited, ask),
        _ => {
            let mut root_bfc = BlockFormattingContext::new();
            let mut root_ctx = root_bfc.root_block_context();
            compute_inner(view, inputs, &mut root_ctx, ask)
        }
    }
}

/// Taffy's `compute_inner` (its own ratio already resolved into `inputs`).
#[expect(clippy::too_many_lines, reason = "Taffy's compute_inner, ported as one step")]
fn compute_inner<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    inputs: LayoutInput,
    block_ctx: &mut BlockContext<'_>,
    ask: Ask,
) -> (LayoutOutput, Option<f32>) {
    let content = ask.content;
    let LayoutInput {
        known_dimensions,
        parent_size,
        available_space,
        run_mode,
        vertical_margins_are_collapsible,
        ..
    } = inputs;
    let calc = S::calc;

    let style = view.node.style.view();
    let raw_padding = style.padding();
    let raw_border = style.border();
    let raw_margin = style.margin();
    let aspect_ratio = ratio_of(&style);
    let padding = raw_padding.resolve_or_zero(parent_size.width, calc);
    let border = raw_border.resolve_or_zero(parent_size.width, calc);
    let direction = style.direction();

    let scrollbar_gutter = {
        let offsets = style.overflow().transpose().map(|overflow| match overflow {
            Overflow::Scroll => style.scrollbar_width(),
            _ => 0.0,
        });
        match direction {
            Direction::Ltr => Rect {
                top: 0.0,
                left: 0.0,
                right: offsets.x,
                bottom: offsets.y,
            },
            Direction::Rtl => Rect {
                top: 0.0,
                left: offsets.x,
                right: 0.0,
                bottom: offsets.y,
            },
        }
    };
    let padding_border = rect_add(padding, border);
    let padding_border_size = padding_border.sum_axes();
    let content_box_inset = rect_add(padding_border, scrollbar_gutter);

    block_ctx.apply_content_box_inset([content_box_inset.left, content_box_inset.right]);

    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
        padding_border_size
    } else {
        Size::ZERO
    };
    let size_style = style.size();
    let size = size_style
        .maybe_resolve(parent_size, calc)
        .maybe_add(box_sizing_adjustment);
    let min_size = transfer(
        style.min_size().maybe_resolve(parent_size, calc),
        aspect_ratio,
        size_style,
    )
    .maybe_add(box_sizing_adjustment);
    let max_size = transfer(
        style.max_size().maybe_resolve(parent_size, calc),
        aspect_ratio,
        size_style,
    )
    .maybe_add(box_sizing_adjustment);
    // Whether its height is set (by its style, or its ratio from a set width).
    let height_is_set = size.height.is_some() || (aspect_ratio.is_some() && size.width.is_some());

    let percentage_basis_dimensions = Size {
        width: known_dimensions.width,
        height: ask.percent_height.or(known_dimensions
            .height
            .filter(|_| inputs.known_dimensions_are_definite.height)),
    };
    let container_content_box_size = percentage_basis_dimensions.maybe_sub(content_box_inset.sum_axes());

    let overflow = style.overflow();
    let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();
    let establishes_new_bfc = is_scroll_container
        || BlockContainerStyle::align_content(&style).keyword != AlignContentKeyword::Normal
        || style.contain().establishes_independent_formatting_context();

    let children_margins_adjoin_own_edges = Line {
        start: vertical_margins_are_collapsible.start
            && !establishes_new_bfc
            && !style.position().is_out_of_flow()
            && padding.top == 0.0
            && border.top == 0.0,
        end: vertical_margins_are_collapsible.end
            && !establishes_new_bfc
            && !style.position().is_out_of_flow()
            && padding.bottom == 0.0
            && border.bottom == 0.0,
    };
    let own_margins_collapse_with_children = Line {
        start: children_margins_adjoin_own_edges.start,
        end: children_margins_adjoin_own_edges.end && !height_is_set,
    };
    let has_styles_preventing_being_collapsed_through = !style.is_block()
        || block_ctx.is_bfc_root()
        || establishes_new_bfc
        || style.position().is_out_of_flow()
        || padding.top > 0.0
        || padding.bottom > 0.0
        || border.top > 0.0
        || border.bottom > 0.0;

    let text_align = style.text_align();
    let align_content = BlockContainerStyle::align_content(&style);
    let justify_items = style.justify_items();
    drop(style);

    // 1. Generate items
    let mut items = generate_item_list(view, container_content_box_size);

    // 2. Compute container width
    let container_outer_width = known_dimensions.width.unwrap_or_else(|| {
        let available_width = available_space.width.maybe_sub(content_box_inset.horizontal_axis_sum());
        let intrinsic_width = determine_content_based_container_width(view, &items, available_width)
            + content_box_inset.horizontal_axis_sum();
        intrinsic_width
            .maybe_clamp(min_size.width, max_size.width)
            .maybe_max(Some(padding_border_size.width))
    });

    // (Asked what its content takes, it lays the content out even at a
    // known height.)
    if let (RunMode::ComputeSize, Some(container_outer_height), false) = (run_mode, known_dimensions.height, content) {
        return (
            LayoutOutput::from_outer_size(Size {
                width: container_outer_width,
                height: container_outer_height,
            }),
            None,
        );
    }
    if run_mode == RunMode::ComputeSize && inputs.axis == RequestedAxis::Horizontal {
        return (
            LayoutOutput::from_outer_size(Size {
                width: container_outer_width,
                height: 0.0,
            }),
            None,
        );
    }

    let container_percentage_resolution_height =
        percentage_basis_dimensions
            .height
            .or(if inputs.sizing_mode == SizingMode::InherentSize {
                size.height.maybe_max(min_size.height)
            } else {
                None
            });

    // 3. Perform final item layout and return content height
    let percentage_resolution_width = parent_size.width.unwrap_or(container_outer_width);
    let resolved_padding = raw_padding.resolve_or_zero(Some(percentage_resolution_width), calc);
    let resolved_border = raw_border.resolve_or_zero(Some(percentage_resolution_width), calc);
    let resolved_content_box_inset = rect_add(rect_add(resolved_padding, resolved_border), scrollbar_gutter);
    let flow = Flow {
        run_mode,
        container_outer_width,
        container_percentage_resolution_height,
        content_box_inset,
        resolved_content_box_inset,
        resolved_border,
        text_align,
        justify_items,
        direction,
        own_margins_collapse_with_children,
        children_margins_adjoin_own_edges,
        is_scroll_container,
    };
    let (
        mut inflow_overflow_rect,
        mut intrinsic_outer_height,
        first_child_top_margin_set,
        last_child_bottom_margin_set,
        mut first_baseline,
    ) = perform_final_layout_on_in_flow_children(view, &mut items, &flow, block_ctx);

    // Root BFCs contain floats
    if block_ctx.is_bfc_root() || establishes_new_bfc {
        intrinsic_outer_height = intrinsic_outer_height.max(block_ctx.floated_content_height_contribution());
    }

    let container_outer_height = known_dimensions
        .height
        .unwrap_or(intrinsic_outer_height.maybe_clamp(min_size.height, max_size.height))
        .maybe_max(Some(padding_border_size.height));
    let final_outer_size = Size {
        width: container_outer_width,
        height: container_outer_height,
    };

    let height_constrained_by_min_height = matches!(min_size.height, Some(h) if h > 0.0 && h >= container_outer_height);
    let own_bottom_margin_collapses_with_children =
        own_margins_collapse_with_children.end && !height_constrained_by_min_height;

    if align_content.keyword != AlignContentKeyword::Normal {
        let container_inner_height = container_outer_height - resolved_content_box_inset.vertical_axis_sum();
        let inflow_content_height = intrinsic_outer_height - resolved_content_box_inset.vertical_axis_sum();
        let free_space = container_inner_height - inflow_content_height;
        let group_offset = align_content_offset(align_content, free_space);
        first_baseline = first_baseline.map(|baseline| baseline + group_offset);
        for item in &mut items {
            if let Some(layout) = item.final_layout.as_mut() {
                layout.location.y += group_offset;
            }
            if item.position.is_out_of_flow() {
                item.static_position.y += group_offset;
            }
        }
        let any_in_flow = items.iter().any(|item| item.final_layout.is_some());
        if any_in_flow {
            inflow_overflow_rect = Rect::ZERO;
            for item in &items {
                if let Some(layout) = item.final_layout.as_ref() {
                    let contribution_location = if direction.is_rtl() {
                        Point {
                            x: container_outer_width - (layout.location.x + layout.size.width) - resolved_border.right,
                            y: layout.location.y - resolved_border.top,
                        }
                    } else {
                        Point {
                            x: layout.location.x - resolved_border.left,
                            y: layout.location.y - resolved_border.top,
                        }
                    };
                    inflow_overflow_rect = inflow_overflow_rect.union(overflow_contribution(
                        contribution_location,
                        layout.size,
                        layout.scrollable_overflow_rect,
                        item.overflow,
                        item.contain,
                        is_scroll_container,
                    ));
                }
            }
        }
    }

    let all_in_flow_children_can_be_collapsed_through = items.iter().all(|item| {
        if item.float.is_floated() {
            return true;
        }
        item.position.is_out_of_flow() || item.can_be_collapsed_through
    });
    let can_be_collapsed_through = !has_styles_preventing_being_collapsed_through
        && container_outer_height == 0.0
        && all_in_flow_children_can_be_collapsed_through;

    let mut output = LayoutOutput {
        size: final_outer_size,
        scrollable_overflow_rect: Rect::ZERO,
        baselines: Baselines::from_first(first_baseline),
        top_margin: if own_margins_collapse_with_children.start {
            first_child_top_margin_set
        } else {
            let margin_top = raw_margin.top.resolve_or_zero(parent_size.width, calc);
            CollapsibleMarginSet::from_margin(margin_top)
        },
        bottom_margin: if own_bottom_margin_collapses_with_children {
            last_child_bottom_margin_set
        } else {
            let margin_bottom = raw_margin.bottom.resolve_or_zero(parent_size.width, calc);
            CollapsibleMarginSet::from_margin(margin_bottom)
        },
        margins_can_collapse_through: can_be_collapsed_through,
        oof_candidates: OofCandidates::NONE,
        oof_positioning_area: None,
    };

    if run_mode == RunMode::ComputeSize {
        return (output, content.then_some(intrinsic_outer_height));
    }

    // Commit deferred child layouts to the tree.
    for item in &items {
        if let Some(layout) = item.final_layout.as_ref() {
            view.set_unrounded_layout(id(item.child), layout);
        }
    }

    // 4. Collect out-of-flow candidates in document order.
    let mut candidates = OofCandidates::new();
    for item in &mut items {
        if item.position.is_out_of_flow() {
            let inline_area = Line {
                start: resolved_content_box_inset.left,
                end: container_outer_width - resolved_content_box_inset.right,
            };
            let block_area = Line {
                start: item.static_position.y,
                end: item.static_position.y,
            };
            let justify_self =
                item.justify_self
                    .unwrap_or(justify_items)
                    .resolve_self_relative(item.direction, direction, true);
            let align_self =
                item.align_self
                    .unwrap_or(AlignItems::NORMAL)
                    .resolve_self_relative(item.direction, direction, false);
            candidates.push(OofCandidate {
                node: id(item.child),
                order: item.order,
                position: item.position,
                static_position: Point {
                    x: AxisStaticPosition::from_alignment(justify_self, inline_area, direction.is_rtl()),
                    y: AxisStaticPosition::from_alignment(align_self, block_area, false),
                },
            });
        } else if !item.oof_candidates.is_empty() {
            if let Some(layout) = item.final_layout.as_ref() {
                item.oof_candidates.translate(layout.location);
            }
            candidates.append(&mut item.oof_candidates);
        }
    }

    let absolute_position_inset = rect_add(resolved_border, scrollbar_gutter);
    let absolute_position_area = size_sub(final_outer_size, absolute_position_inset.sum_axes());
    let absolute_position_offset = Point {
        x: absolute_position_inset.left,
        y: absolute_position_inset.top,
    };
    output.oof_candidates = candidates;
    output.oof_positioning_area = Some(OofPositioningArea {
        size: absolute_position_area,
        offset: absolute_position_offset,
    });

    if is_scroll_container {
        inflow_overflow_rect.right += if direction.is_rtl() {
            resolved_padding.left
        } else {
            resolved_padding.right
        };
        inflow_overflow_rect.bottom += resolved_padding.bottom;
    }
    output.scrollable_overflow_rect = inflow_overflow_rect;

    // 5. Perform hidden layout on hidden children
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
                SizingMode::InherentSize,
                Line::FALSE,
            );
        }
    }

    (output, content.then_some(intrinsic_outer_height))
}

/// Taffy's `generate_item_list`.
fn generate_item_list<S: NodeStyle, L: Measure>(
    view: &View<'_, S, L>,
    node_inner_size: Size<Option<f32>>,
) -> Vec<Item> {
    let calc = S::calc;
    view.node
        .children
        .iter()
        .enumerate()
        .filter(|(_, c)| c.style.view().box_generation_mode() != BoxGenerationMode::None)
        .enumerate()
        .map(|(order, (child, node))| {
            let child_style = node.style.view();
            let aspect_ratio = ratio_of(&child_style);
            let padding = child_style.padding().resolve_or_zero(node_inner_size.width, calc);
            let border = child_style.border().resolve_or_zero(node_inner_size.width, calc);
            let pb_sum = rect_add(padding, border).sum_axes();
            let box_sizing_adjustment = if child_style.box_sizing() == BoxSizing::ContentBox {
                pb_sum
            } else {
                Size::ZERO
            };
            let position = child_style.position();
            let overflow = child_style.overflow();
            let float = child_style.float();
            let is_not_floated = float == Float::None;
            let is_block = child_style.is_block();
            let is_table = child_style.is_table();
            let is_compressible_replaced = child_style.is_compressible_replaced();
            let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();
            let contain = child_style.contain();
            let is_in_same_bfc = is_block
                && !is_table
                && !position.is_out_of_flow()
                && is_not_floated
                && !is_scroll_container
                && !contain.establishes_independent_formatting_context()
                && BlockItemStyle::align_content(&child_style).keyword == AlignContentKeyword::Normal;
            let size_style = child_style.size();
            let size = size_style
                .maybe_resolve(node_inner_size, calc)
                .maybe_add(box_sizing_adjustment);
            let min_size = transfer(
                child_style.min_size().maybe_resolve(node_inner_size, calc),
                aspect_ratio,
                size_style,
            )
            .maybe_add(box_sizing_adjustment);
            let max_size = transfer(
                child_style.max_size().maybe_resolve(node_inner_size, calc),
                aspect_ratio,
                size_style,
            )
            .maybe_add(box_sizing_adjustment);
            Item {
                child,
                order: order_of(order),
                is_table,
                is_compressible_replaced,
                is_in_same_bfc,
                float,
                clear: child_style.clear(),
                size_style,
                size,
                min_size,
                max_size,
                width_from_ratio: aspect_ratio.is_some() && size_style.width.is_auto(),
                overflow,
                contain,
                scrollbar_width: child_style.scrollbar_width(),
                position,
                direction: child_style.direction(),
                align_self: BlockItemStyle::align_self(&child_style),
                justify_self: child_style.justify_self(),
                inset: child_style.inset(),
                margin: child_style.margin(),
                padding,
                border,
                padding_border_sum: pb_sum,
                computed_size: Size::zero(),
                static_position: Point::zero(),
                can_be_collapsed_through: false,
                final_layout: None,
                oof_candidates: OofCandidates::new(),
            }
        })
        .collect()
}

/// Taffy's `resolve_stretch_height`.
fn resolve_stretch_height(
    height_style: Dimension,
    container_inner_height: Option<f32>,
    item_margin: Rect<f32>,
    margins_treated_as_zero: Line<bool>,
) -> Option<f32> {
    if !height_style.is_stretch() {
        return None;
    }
    let mut height = container_inner_height?;
    if !margins_treated_as_zero.start {
        height -= item_margin.top;
    }
    if !margins_treated_as_zero.end {
        height -= item_margin.bottom;
    }
    Some(height)
}

/// Taffy's `resolve_relative_offset`.
fn resolve_relative_offset<S: NodeStyle>(
    item: &Item,
    parent_size: Size<Option<f32>>,
    direction: Direction,
) -> Point<f32> {
    if item.position != Position::Relative {
        return Point::ZERO;
    }
    let inset = zip_size(item.inset, parent_size, |p, s| p.maybe_resolve(s, S::calc));
    let x = if direction.is_rtl() {
        inset.right.map(|x| -x).or(inset.left)
    } else {
        inset.left.or(inset.right.map(|x| -x))
    };
    Point {
        x: x.unwrap_or(0.0),
        y: inset.top.or(inset.bottom.map(|x| -x)).unwrap_or(0.0),
    }
}

/// Taffy's `determine_content_based_container_width`.
fn determine_content_based_container_width<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    items: &[Item],
    available_width: AvailableSpace,
) -> f32 {
    let calc = S::calc;
    let available_space = Size {
        width: available_width,
        height: AvailableSpace::MinContent,
    };
    let mut max_child_width = 0.0_f32;
    let mut float_contribution = FloatIntrinsicWidthCalculator::new(available_width);
    for item in items.iter().filter(|item| !item.position.is_out_of_flow()) {
        let known_dimensions = item.size.maybe_clamp(item.min_size, item.max_size);
        let item_x_margin_sum = item
            .margin
            .resolve_or_zero(available_space.width.into_option(), calc)
            .horizontal_axis_sum();
        let width = known_dimensions.width.unwrap_or_else(|| {
            let item_available_width = match keyword(item.size_style.width, None, None, calc) {
                Some(Keyword::Measure(available_width)) => available_width,
                Some(Keyword::Exact(width)) => AvailableSpace::Definite(width),
                None => available_space.width.maybe_sub(item_x_margin_sum),
            };
            view.measure_child_size(
                id(item.child),
                known_dimensions,
                Size::NONE,
                Size {
                    width: item_available_width,
                    height: available_space.height,
                },
                SizingMode::InherentSize,
                taffy::AbsoluteAxis::Horizontal,
                if item.is_in_same_bfc { Line::TRUE } else { Line::FALSE },
            )
        });
        let width = width.max(item.padding_border_sum.width) + item_x_margin_sum;
        if let Some(direction) = item.float.float_direction() {
            float_contribution.add_float(width, direction, item.clear);
            continue;
        }
        max_child_width = max_child_width.max(width);
    }
    max_child_width.max(float_contribution.result())
}

/// What `perform_final_layout_on_in_flow_children` reads of its container.
struct Flow {
    run_mode: RunMode,
    container_outer_width: f32,
    container_percentage_resolution_height: Option<f32>,
    content_box_inset: Rect<f32>,
    resolved_content_box_inset: Rect<f32>,
    resolved_border: Rect<f32>,
    text_align: TextAlign,
    justify_items: AlignItems,
    direction: Direction,
    own_margins_collapse_with_children: Line<bool>,
    children_margins_adjoin_own_edges: Line<bool>,
    is_scroll_container: bool,
}

/// The overflow a box placed at `location` contributes, in its container's
/// coordinates from the padding box's start (Taffy's in-flow overflow).
fn contribution(f: &Flow, location: Point<f32>, size: Size<f32>, rect: Rect<f32>, item: &Item) -> Rect<f32> {
    let contribution_location = if f.direction.is_rtl() {
        Point {
            x: f.container_outer_width - (location.x + size.width) - f.resolved_border.right,
            y: location.y - f.resolved_border.top,
        }
    } else {
        Point {
            x: location.x - f.resolved_border.left,
            y: location.y - f.resolved_border.top,
        }
    };
    overflow_contribution(
        contribution_location,
        size,
        rect,
        item.overflow,
        item.contain,
        f.is_scroll_container,
    )
}

/// Taffy's `perform_final_layout_on_in_flow_children`.
#[expect(clippy::too_many_lines, reason = "Taffy's in-flow pass, ported as one step")]
fn perform_final_layout_on_in_flow_children<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    items: &mut [Item],
    f: &Flow,
    block_ctx: &mut BlockContext<'_>,
) -> (Rect<f32>, f32, CollapsibleMarginSet, CollapsibleMarginSet, Option<f32>) {
    let calc = S::calc;
    let container_outer_width = f.container_outer_width;
    let resolved_content_box_inset = f.resolved_content_box_inset;
    let direction = f.direction;
    let own_margins_collapse_with_children = f.own_margins_collapse_with_children;
    let container_inner_width = (container_outer_width - resolved_content_box_inset.horizontal_axis_sum()).max(0.0);
    let container_percentage_resolution_height = f
        .container_percentage_resolution_height
        .maybe_sub(resolved_content_box_inset.vertical_axis_sum());
    let parent_size = Size {
        width: Some(container_inner_width),
        height: container_percentage_resolution_height,
    };
    let available_space = Size {
        width: AvailableSpace::Definite(container_inner_width),
        height: AvailableSpace::MaxContent,
    };

    if block_ctx.is_bfc_root() {
        block_ctx.set_width(container_outer_width);
        block_ctx.apply_content_box_inset([resolved_content_box_inset.left, resolved_content_box_inset.right]);
    }
    if !own_margins_collapse_with_children.start {
        block_ctx.commit_strut();
    }

    let mut inflow_overflow_rect = Rect::ZERO;
    let mut committed_y_offset = resolved_content_box_inset.top;
    let mut y_offset_for_absolute = resolved_content_box_inset.top;
    let mut first_child_top_margin_set = CollapsibleMarginSet::ZERO;
    let mut active_collapsible_margin_set = CollapsibleMarginSet::ZERO;
    let mut is_collapsing_with_first_margin_set = true;
    let mut first_baseline: Option<f32> = None;
    let mut active_margin_set_has_clearance = false;
    let mut has_active_floats = block_ctx.has_active_floats(committed_y_offset);

    for item in items.iter_mut() {
        if item.position.is_out_of_flow() {
            let x = match direction {
                Direction::Ltr => resolved_content_box_inset.left,
                Direction::Rtl => container_outer_width - resolved_content_box_inset.right,
            };
            item.static_position = Point {
                x,
                y: y_offset_for_absolute,
            };
            continue;
        }
        let item_margin = item
            .margin
            .map(|margin| margin.resolve_to_option(container_inner_width, calc));
        let item_non_auto_margin = item_margin.map(|m| m.unwrap_or(0.0));
        let item_non_auto_x_margin_sum = item_non_auto_margin.horizontal_axis_sum();

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

        // Floated boxes
        if let Some(float_direction) = item.float.float_direction() {
            has_active_floats = true;
            let available_width = (container_inner_width - item_non_auto_x_margin_sum).max(0.0);
            let (item_known_width, item_available_width) = match keyword(
                item.size_style.width,
                Some(available_width),
                Some(container_inner_width),
                calc,
            ) {
                Some(Keyword::Measure(available)) => (None, available),
                Some(Keyword::Exact(width)) => (Some(width), AvailableSpace::Definite(width)),
                None => (None, AvailableSpace::Definite(available_width)),
            };
            let item_known_height = resolve_stretch_height(
                item.size_style.height,
                container_percentage_resolution_height,
                item_non_auto_margin,
                Line::FALSE,
            );
            let mut item_layout = view.perform_child_layout(
                id(item.child),
                Size {
                    width: item_known_width,
                    height: item_known_height,
                },
                parent_size,
                Size {
                    width: item_available_width,
                    height: AvailableSpace::MaxContent,
                },
                SizingMode::InherentSize,
                Line::FALSE,
            );
            item.oof_candidates = item_layout.oof_candidates.take();
            let margin_box = Size {
                width: item_layout.size.width + item_non_auto_margin.horizontal_axis_sum(),
                height: item_layout.size.height + item_non_auto_margin.vertical_axis_sum(),
            };
            let adjoins_unresolved_strut =
                is_collapsing_with_first_margin_set && own_margins_collapse_with_children.start;
            let y_offset_for_float = if adjoins_unresolved_strut {
                committed_y_offset
            } else {
                committed_y_offset + active_collapsible_margin_set.resolve()
            };
            let mut location = block_ctx.place_floated_box(
                margin_box,
                y_offset_for_float,
                float_direction,
                item.clear,
                adjoins_unresolved_strut,
            );
            location.y += item_non_auto_margin.top;
            location.x += item_non_auto_margin.left;
            let inset_offset = resolve_relative_offset::<S>(item, parent_size, direction);
            location.x += inset_offset.x;
            location.y += inset_offset.y;
            item.final_layout = Some(Layout {
                order: item.order,
                size: item_layout.size,
                scrollable_overflow_rect: item_layout.scrollable_overflow_rect,
                scrollbar_size,
                location,
                padding: item.padding,
                border: item.border,
                margin: item_non_auto_margin,
            });
            inflow_overflow_rect = inflow_overflow_rect.union(contribution(
                f,
                location,
                item_layout.size,
                item_layout.scrollable_overflow_rect,
                item,
            ));
            continue;
        }

        // Non-floated boxes
        let mut y_margin_offset: f32 = 0.0;
        let mut item_avoids_floats = false;
        let mut item_pushed_below_float = false;

        let (stretch_width, float_avoiding_position, float_avoiding_width) = if item.is_in_same_bfc {
            let stretch_width = (container_inner_width - item_non_auto_x_margin_sum).max(0.0);
            (stretch_width, Point { x: 0.0, y: 0.0 }, 0.0)
        } else {
            if !is_collapsing_with_first_margin_set || !own_margins_collapse_with_children.start {
                y_margin_offset = active_collapsible_margin_set
                    .collapse_with_margin(item_non_auto_margin.top)
                    .resolve();
            }
            let min_y = committed_y_offset + y_margin_offset;
            if has_active_floats || block_ctx.has_active_floats(min_y) {
                let x_margins = [item_non_auto_margin.left, item_non_auto_margin.right];
                let min_auto_width = -item_non_auto_x_margin_sum;
                let mut slot_segment = None;
                let slot = loop {
                    let slot = block_ctx.find_bfc_slot(min_y, x_margins, direction, item.clear, slot_segment);
                    let Some(segment_id) = slot.segment_id else { break slot };
                    let width = item
                        .size
                        .width
                        .unwrap_or(slot.stretch_width.max(min_auto_width).max(0.0))
                        .maybe_clamp(item.min_size.width, item.max_size.width);
                    if width <= slot.border_width + FIT_TOLERANCE {
                        break slot;
                    }
                    slot_segment = Some(segment_id);
                };
                if slot.y > min_y {
                    item_pushed_below_float = true;
                }
                has_active_floats = slot.segment_id.is_some();
                item_avoids_floats = true;
                let stretch_width = slot.stretch_width.max(min_auto_width).max(0.0);
                (stretch_width, Point { x: slot.x, y: slot.y }, slot.border_width)
            } else {
                let stretch_width = (container_inner_width - item_non_auto_x_margin_sum).max(0.0);
                (
                    stretch_width,
                    Point {
                        x: resolved_content_box_inset.left,
                        y: min_y,
                    },
                    container_inner_width,
                )
            }
        };

        let justify_self =
            item.justify_self
                .unwrap_or(f.justify_items)
                .resolve_self_relative(item.direction, direction, true);
        let is_stretch = justify_self.keyword == AlignItemsKeyword::Stretch;
        let non_stretch_justify_self = match justify_self.keyword {
            AlignItemsKeyword::Normal | AlignItemsKeyword::Stretch => None,
            _ => Some(justify_self),
        };

        let known_dimensions = if (item.is_table || item.is_compressible_replaced) && !is_stretch {
            Size::NONE
        } else {
            let width_style = if non_stretch_justify_self.is_some() && item.size_style.width.is_auto() {
                Dimension::fit_content()
            } else {
                item.size_style.width
            };
            let keyword_width =
                keyword(width_style, Some(stretch_width), Some(container_inner_width), calc).map(|resolution| {
                    match resolution {
                        Keyword::Exact(width) => width,
                        Keyword::Measure(item_available_width) => view.measure_child_size(
                            id(item.child),
                            Size::NONE,
                            parent_size,
                            Size {
                                width: item_available_width,
                                height: AvailableSpace::MaxContent,
                            },
                            SizingMode::InherentSize,
                            taffy::AbsoluteAxis::Horizontal,
                            if item.is_in_same_bfc { Line::TRUE } else { Line::FALSE },
                        ),
                    }
                });
            let keyword_height = resolve_stretch_height(
                item.size_style.height,
                container_percentage_resolution_height,
                item_non_auto_margin,
                f.children_margins_adjoin_own_edges,
            );
            let height = item
                .size
                .height
                .or(keyword_height)
                .maybe_clamp(item.min_size.height, item.max_size.height);
            // An `auto` width its ratio gives from a set height is the
            // child's to find (at least its min-content width), not the
            // stretched one.
            let width = if item.width_from_ratio && item.size.width.is_none() && height.is_some() {
                None
            } else {
                Some(
                    item.size
                        .width
                        .or(keyword_width)
                        .unwrap_or(stretch_width)
                        .maybe_clamp(item.min_size.width, item.max_size.width),
                )
            };
            Size { width, height }.maybe_clamp(item.min_size, item.max_size)
        };

        let inputs = LayoutInput {
            run_mode: f.run_mode,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size,
            available_space: available_space.map_width(|_| AvailableSpace::Definite(stretch_width)),
            vertical_margins_are_collapsible: if item.is_in_same_bfc { Line::TRUE } else { Line::FALSE },
        };

        let clear_threshold = block_ctx.cleared_threshold(item.clear);
        let clear_pos = clear_threshold.unwrap_or(f32::NEG_INFINITY);
        let mut child_float_contribution = f32::NEG_INFINITY;

        let mut item_layout = if item.is_in_same_bfc {
            let width = known_dimensions
                .width
                .unwrap_or(stretch_width)
                .max(item.padding_border_sum.width);
            let inset_left = item_non_auto_margin.left + f.content_box_inset.left;
            let inset_right = container_outer_width - width - inset_left;
            let insets = [inset_left, inset_right];
            let estimated_y = committed_y_offset
                + active_collapsible_margin_set
                    .collapse_with_margin(item_non_auto_margin.top)
                    .resolve();
            let mut child_block_ctx = block_ctx.sub_context(estimated_y.max(clear_pos), insets);
            let output = view.child_layout(id(item.child), inputs, Some(&mut child_block_ctx));
            child_float_contribution = child_block_ctx.floated_content_height_contribution();
            let child_top_adjoining_floats = child_block_ctx.top_adjoining_floats();
            block_ctx.merge_adjoining_floats(child_top_adjoining_floats);
            output
        } else {
            view.child_layout(id(item.child), inputs, None)
        };
        item.oof_candidates = item_layout.oof_candidates.take();
        let final_size = item_layout.size;

        let top_margin_set = item_layout
            .top_margin
            .collapse_with_margin(item_margin.top.unwrap_or(0.0));
        let bottom_margin_set = item_layout
            .bottom_margin
            .collapse_with_margin(item_margin.bottom.unwrap_or(0.0));

        let free_x_space = (stretch_width - final_size.width).max(0.0);
        let x_axis_auto_margin_size = {
            let auto_margin_count =
                usize::from(item_margin.left.is_none()).saturating_add(usize::from(item_margin.right.is_none()));
            if auto_margin_count > 0 {
                free_x_space / usize_to_f32(auto_margin_count)
            } else {
                0.0
            }
        };
        let resolved_margin = Rect {
            left: item_margin.left.unwrap_or(x_axis_auto_margin_size),
            right: item_margin.right.unwrap_or(x_axis_auto_margin_size),
            top: item_margin.top.unwrap_or(0.0),
            bottom: item_margin.bottom.unwrap_or(0.0),
        };

        let inset_offset = resolve_relative_offset::<S>(item, parent_size, direction);

        if item.is_in_same_bfc && (!is_collapsing_with_first_margin_set || !own_margins_collapse_with_children.start) {
            y_margin_offset = active_collapsible_margin_set
                .collapse_with_set(top_margin_set)
                .resolve();
        }

        let mut has_clearance = false;
        if item.is_in_same_bfc
            && let Some(threshold) = clear_threshold
        {
            let hypothetical_y = committed_y_offset
                + active_collapsible_margin_set
                    .collapse_with_set(top_margin_set)
                    .resolve();
            let forced_clearance = block_ctx.has_adjoining_float(item.clear);
            if forced_clearance || hypothetical_y < threshold {
                has_clearance = true;
                let escaped_margin = if is_collapsing_with_first_margin_set && own_margins_collapse_with_children.start
                {
                    active_collapsible_margin_set.resolve()
                } else {
                    0.0
                };
                y_margin_offset = threshold - committed_y_offset - escaped_margin;
            }
        }

        item.computed_size = item_layout.size;
        item.can_be_collapsed_through = item_layout.margins_can_collapse_through && !has_clearance;
        item.static_position = if item.is_in_same_bfc {
            let uncleared_y = committed_y_offset + active_collapsible_margin_set.resolve();
            Point {
                x: match direction {
                    Direction::Ltr => resolved_content_box_inset.left,
                    Direction::Rtl => container_outer_width - resolved_content_box_inset.right - final_size.width,
                },
                y: uncleared_y.max(clear_pos),
            }
        } else {
            Point {
                x: match direction {
                    Direction::Ltr => float_avoiding_position.x,
                    Direction::Rtl => float_avoiding_position.x + float_avoiding_width - final_size.width,
                },
                y: float_avoiding_position.y,
            }
        };
        let mut location = if item.is_in_same_bfc {
            Point {
                x: match direction {
                    Direction::Ltr => resolved_content_box_inset.left + inset_offset.x + resolved_margin.left,
                    Direction::Rtl => {
                        container_outer_width
                            - resolved_content_box_inset.right
                            - final_size.width
                            - resolved_margin.right
                            + inset_offset.x
                    }
                },
                y: committed_y_offset + y_margin_offset + inset_offset.y,
            }
        } else {
            let (extra_margin_left, extra_margin_right) = if item_avoids_floats {
                (
                    resolved_margin.left - item_non_auto_margin.left,
                    resolved_margin.right - item_non_auto_margin.right,
                )
            } else {
                (resolved_margin.left, resolved_margin.right)
            };
            Point {
                x: match direction {
                    Direction::Ltr => float_avoiding_position.x + extra_margin_left + inset_offset.x,
                    Direction::Rtl => {
                        float_avoiding_position.x + float_avoiding_width - final_size.width - extra_margin_right
                            + inset_offset.x
                    }
                },
                y: float_avoiding_position.y + inset_offset.y,
            }
        };

        if item.is_in_same_bfc {
            block_ctx
                .add_child_floated_content_height_contribution(location.y - inset_offset.y + child_float_contribution);
        }

        let has_auto_x_margin = item_margin.left.is_none() || item_margin.right.is_none();
        let item_outer_width = item_layout.size.width + resolved_margin.horizontal_axis_sum();
        if let (Some(justify_self), false) = (non_stretch_justify_self, has_auto_x_margin) {
            let (container_start, free_x_space) = if item_avoids_floats {
                (
                    float_avoiding_position.x - resolved_margin.left,
                    float_avoiding_width - item_layout.size.width,
                )
            } else {
                (
                    resolved_content_box_inset.left,
                    container_inner_width - item_outer_width,
                )
            };
            let keyword = if matches!(justify_self.safety, AlignmentSafety::Safe) && free_x_space < 0.0 {
                AlignItemsKeyword::Start
            } else {
                justify_self.keyword
            };
            let align_to_end = match keyword {
                AlignItemsKeyword::Center => None,
                AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd | AlignItemsKeyword::SelfEnd => Some(true),
                _ => Some(false),
            };
            let offset = match align_to_end {
                None => free_x_space / 2.0,
                Some(end) if end != direction.is_rtl() => free_x_space,
                Some(_) => 0.0,
            };
            location.x = container_start + resolved_margin.left + inset_offset.x + offset;
        } else if non_stretch_justify_self.is_none() && item_outer_width < container_inner_width {
            let free_x_space = container_inner_width - item_outer_width;
            match (f.text_align, direction) {
                (TextAlign::Auto, _)
                | (TextAlign::LegacyLeft, Direction::Ltr)
                | (TextAlign::LegacyRight, Direction::Rtl) => {}
                (TextAlign::LegacyLeft, Direction::Rtl) => location.x -= free_x_space,
                (TextAlign::LegacyRight, Direction::Ltr) => location.x += free_x_space,
                (TextAlign::LegacyCenter, Direction::Ltr) => location.x += free_x_space / 2.0,
                (TextAlign::LegacyCenter, Direction::Rtl) => location.x -= free_x_space / 2.0,
            }
        }

        if first_baseline.is_none() {
            let child_baseline =
                child_first_baseline(item_layout.baselines.first, item_layout.size.height, item.overflow.y);
            first_baseline = child_baseline.map(|baseline| location.y - inset_offset.y + baseline);
        }

        let resolved_margin = {
            let mut margin = resolved_margin;
            if item_avoids_floats {
                if item_margin.left.is_none() {
                    margin.left += float_avoiding_position.x - resolved_content_box_inset.left;
                }
                if item_margin.right.is_none() {
                    margin.right += container_outer_width
                        - resolved_content_box_inset.right
                        - float_avoiding_position.x
                        - float_avoiding_width;
                }
            }
            margin
        };

        item.final_layout = Some(Layout {
            order: item.order,
            size: item_layout.size,
            scrollable_overflow_rect: item_layout.scrollable_overflow_rect,
            scrollbar_size,
            location,
            padding: item.padding,
            border: item.border,
            margin: resolved_margin,
        });
        inflow_overflow_rect = inflow_overflow_rect.union(contribution(
            f,
            location,
            final_size,
            item_layout.scrollable_overflow_rect,
            item,
        ));

        if is_collapsing_with_first_margin_set && item_pushed_below_float {
            is_collapsing_with_first_margin_set = false;
        }
        if is_collapsing_with_first_margin_set && has_clearance {
            is_collapsing_with_first_margin_set = false;
        } else if is_collapsing_with_first_margin_set {
            if item.can_be_collapsed_through {
                first_child_top_margin_set = first_child_top_margin_set
                    .collapse_with_set(top_margin_set)
                    .collapse_with_set(bottom_margin_set);
            } else {
                first_child_top_margin_set = first_child_top_margin_set.collapse_with_set(top_margin_set);
                is_collapsing_with_first_margin_set = false;
            }
        }

        if item.can_be_collapsed_through {
            active_collapsible_margin_set = active_collapsible_margin_set
                .collapse_with_set(top_margin_set)
                .collapse_with_set(bottom_margin_set);
            y_offset_for_absolute = committed_y_offset + item_layout.size.height + y_margin_offset;
        } else {
            committed_y_offset = location.y - inset_offset.y + item_layout.size.height;
            if has_clearance && item_layout.margins_can_collapse_through {
                committed_y_offset -= top_margin_set.resolve();
                active_collapsible_margin_set = top_margin_set.collapse_with_set(bottom_margin_set);
                active_margin_set_has_clearance = true;
            } else {
                active_collapsible_margin_set = bottom_margin_set;
                active_margin_set_has_clearance = false;
            }
            y_offset_for_absolute = committed_y_offset + active_collapsible_margin_set.resolve();
            block_ctx.commit_strut();
        }
    }

    let last_child_bottom_margin_set = if active_margin_set_has_clearance {
        CollapsibleMarginSet::ZERO
    } else {
        active_collapsible_margin_set
    };
    let bottom_y_margin_offset = if active_margin_set_has_clearance {
        active_collapsible_margin_set.resolve()
    } else if own_margins_collapse_with_children.end {
        0.0
    } else {
        last_child_bottom_margin_set.resolve()
    };

    committed_y_offset += resolved_content_box_inset.bottom + bottom_y_margin_offset;
    let content_height = committed_y_offset.max(0.0);
    (
        inflow_overflow_rect,
        content_height,
        first_child_top_margin_set,
        last_child_bottom_margin_set,
        first_baseline,
    )
}
