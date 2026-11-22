//! The layout half of a node's [`crate::style::Style`], as the retained tree
//! keeps it (ADR-0014): built from the style when the style diff finds a
//! change, compared before anything is dirtied, and read by Taffy's
//! algorithms through its style traits, field by field. It holds Taffy's
//! own compact value types (a length is one word) and nothing a box does
//! not lay out by: no colour, shadow, effect or motion.

use taffy as ts;
use taffy::{
    AlignContent, AlignItems, AlignSelf, BlockContainerStyle, BlockItemStyle, CoreStyle, Dimension, FlexDirection,
    FlexWrap, FlexboxContainerStyle, FlexboxItemStyle, GridAutoFlow, GridContainerStyle, GridItemStyle, GridPlacement,
    GridTemplateArea, JustifyContent, LengthPercentage, LengthPercentageAuto, Line, OofItemStyle, Overflow, Point,
    Rect, Size, TrackSizingFunction,
};

use super::engine::calc;
use super::engine::{Job, Measure, Node, NodeStyle, Stats};
use crate::style::{Length, Track};

/// A length as the tree keeps it: Taffy's own compact value holds a
/// pointer (for `calc()`), which no thread but its maker may hold, so the
/// tree keeps this and hands Taffy its value when read. A `calc()` is
/// interned once, when the style is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Len {
    Auto,
    Px(f32),
    /// A fraction of the containing block (`0.5` is 50%).
    Percent(f32),
    FitContent,
    /// An interned [`calc::CalcSum`].
    Calc(usize),
}

impl Len {
    /// As Taffy reads a size or flex basis.
    #[must_use]
    #[inline(always)]
    pub fn dimension(self) -> Dimension {
        match self {
            Self::Auto => Dimension::auto(),
            Self::Px(v) => Dimension::length(v),
            Self::Percent(f) => Dimension::percent(f),
            Self::FitContent => Dimension::fit_content(),
            Self::Calc(i) => Dimension::calc(calc::handle(i)),
        }
    }

    /// As Taffy reads a margin, an inset, a min or max size.
    #[must_use]
    #[inline(always)]
    pub fn auto_or_length(self) -> LengthPercentageAuto {
        match self {
            Self::Auto | Self::FitContent => LengthPercentageAuto::auto(),
            Self::Px(v) => LengthPercentageAuto::length(v),
            Self::Percent(f) => LengthPercentageAuto::percent(f),
            Self::Calc(i) => LengthPercentageAuto::calc(calc::handle(i)),
        }
    }

    /// Whether it reads the containing block's size.
    #[must_use]
    pub fn relative(self) -> bool {
        matches!(self, Self::Percent(_) | Self::Calc(_))
    }
}

impl From<Length> for Len {
    fn from(l: Length) -> Self {
        match l {
            Length::Auto => Self::Auto,
            Length::Px(v) => Self::Px(v),
            Length::Fraction(f) => Self::Percent(f),
            Length::FitContent => Self::FitContent,
            Length::Calc { fraction, px } => Self::Calc(calc::intern(calc::CalcSum { fraction, px })),
        }
    }
}

#[inline(always)]
fn rect<T: Copy, U>(r: Rect<T>, f: impl Fn(T) -> U) -> Rect<U> {
    Rect {
        left: f(r.left),
        right: f(r.right),
        top: f(r.top),
        bottom: f(r.bottom),
    }
}

/// What a node lays out by.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxStyle {
    pub display: ts::Display,
    pub direction: ts::Direction,
    pub position: ts::Position,
    pub inset: Rect<Len>,
    pub size: Size<Len>,
    pub min_size: Size<Len>,
    pub max_size: Size<Len>,
    pub aspect_ratio: Option<f32>,
    pub margin: Rect<Len>,
    pub padding: Rect<f32>,
    pub border: Rect<f32>,
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Len,
    pub align_items: AlignItems,
    pub align_self: Option<AlignSelf>,
    pub justify_items: AlignItems,
    pub justify_self: Option<AlignSelf>,
    pub justify_content: JustifyContent,
    pub align_content: AlignContent,
    pub gap: Size<f32>,
    pub grid_template_columns: crate::style::Tracks,
    pub grid_template_rows: crate::style::Tracks,
    pub grid_auto_rows: crate::style::Tracks,
    pub grid_auto_flow: GridAutoFlow,
    pub grid_column: Line<GridPlacement<String>>,
    pub grid_row: Line<GridPlacement<String>>,
    pub overflow: Point<Overflow>,
    /// Which axes of the parent's size its percentages read (the cache key,
    /// L3), as [`BoxStyle::settled`] last found.
    pub reads_parent: Size<bool>,
}

impl BoxStyle {
    /// Note what its values read: call after changing them.
    #[must_use]
    pub fn settled(mut self) -> Self {
        let m = &self.margin;
        self.reads_parent = Size {
            width: self.size.width.relative()
                || self.min_size.width.relative()
                || self.max_size.width.relative()
                || self.inset.left.relative()
                || self.inset.right.relative()
                || [m.top, m.right, m.bottom, m.left].iter().any(|v| v.relative()),
            height: self.size.height.relative()
                || self.min_size.height.relative()
                || self.max_size.height.relative()
                || self.inset.top.relative()
                || self.inset.bottom.relative(),
        };
        self
    }
}

impl Default for BoxStyle {
    /// CSS's initial values, as `crate::style::Style::default()` lays out.
    fn default() -> Self {
        crate::style::Style::default().box_style(crate::style::Direction::Ltr)
    }
}

impl NodeStyle for BoxStyle {
    type Ident = String;
    type Ref<'a> = &'a Self;

    fn view(&self) -> &Self {
        self
    }

    fn display(&self) -> ts::Display {
        self.display
    }

    fn reads_parent(&self) -> Size<bool> {
        self.reads_parent
    }

    fn baseline(&self) -> bool {
        self.align_items == AlignItems::BASELINE
            || self.align_self == Some(AlignItems::BASELINE)
            || self.justify_items == AlignItems::BASELINE
            || self.justify_self == Some(AlignItems::BASELINE)
    }

    fn calc(handle: *const (), basis: f32) -> f32 {
        calc::resolve(handle, basis)
    }

    fn run_jobs<L: Measure>(groups: &mut [Vec<&mut Node<Self, L>>], cx: &mut L::Cx, run: &Job<'_, Self, L>) -> Stats {
        super::engine::parallel(groups, cx, run)
    }
}

impl CoreStyle for BoxStyle {
    type CustomIdent = String;

    #[inline]
    fn box_generation_mode(&self) -> ts::BoxGenerationMode {
        if self.display == ts::Display::None {
            ts::BoxGenerationMode::None
        } else {
            ts::BoxGenerationMode::Normal
        }
    }
    #[inline]
    fn is_block(&self) -> bool {
        self.display == ts::Display::Block
    }
    #[inline]
    fn direction(&self) -> ts::Direction {
        self.direction
    }
    #[inline]
    fn overflow(&self) -> Point<Overflow> {
        self.overflow
    }
    #[inline]
    fn position(&self) -> ts::Position {
        self.position
    }
    #[inline(always)]
    fn inset(&self) -> Rect<LengthPercentageAuto> {
        rect(self.inset, Len::auto_or_length)
    }
    #[inline(always)]
    fn size(&self) -> Size<Dimension> {
        Size {
            width: self.size.width.dimension(),
            height: self.size.height.dimension(),
        }
    }
    #[inline(always)]
    fn min_size(&self) -> Size<LengthPercentageAuto> {
        Size {
            width: self.min_size.width.auto_or_length(),
            height: self.min_size.height.auto_or_length(),
        }
    }
    #[inline(always)]
    fn max_size(&self) -> Size<LengthPercentageAuto> {
        Size {
            width: self.max_size.width.auto_or_length(),
            height: self.max_size.height.auto_or_length(),
        }
    }
    #[inline]
    fn aspect_ratio(&self) -> Option<f32> {
        self.aspect_ratio
    }
    #[inline(always)]
    fn margin(&self) -> Rect<LengthPercentageAuto> {
        rect(self.margin, Len::auto_or_length)
    }
    #[inline(always)]
    fn padding(&self) -> Rect<LengthPercentage> {
        rect(self.padding, LengthPercentage::length)
    }
    #[inline(always)]
    fn border(&self) -> Rect<LengthPercentage> {
        rect(self.border, LengthPercentage::length)
    }
}

impl OofItemStyle for BoxStyle {
    #[inline]
    fn align_self(&self) -> Option<AlignSelf> {
        self.align_self
    }
    #[inline]
    fn justify_self(&self) -> Option<AlignSelf> {
        self.justify_self
    }
    #[inline]
    fn grid_row(&self) -> Line<GridPlacement<String>> {
        self.grid_row.clone()
    }
    #[inline]
    fn grid_column(&self) -> Line<GridPlacement<String>> {
        self.grid_column.clone()
    }
}

impl BlockContainerStyle for BoxStyle {
    #[inline]
    fn align_content(&self) -> AlignContent {
        self.align_content
    }
    #[inline]
    fn justify_items(&self) -> AlignItems {
        self.justify_items
    }
}

impl BlockItemStyle for BoxStyle {
    #[inline]
    fn align_self(&self) -> Option<AlignSelf> {
        self.align_self
    }
    #[inline]
    fn justify_self(&self) -> Option<AlignSelf> {
        self.justify_self
    }
    #[inline]
    fn align_content(&self) -> AlignContent {
        self.align_content
    }
}

impl FlexboxContainerStyle for BoxStyle {
    #[inline]
    fn flex_direction(&self) -> FlexDirection {
        self.flex_direction
    }
    #[inline]
    fn flex_wrap(&self) -> FlexWrap {
        self.flex_wrap
    }
    #[inline]
    fn gap(&self) -> Size<LengthPercentage> {
        Size {
            width: LengthPercentage::length(self.gap.width),
            height: LengthPercentage::length(self.gap.height),
        }
    }
    #[inline]
    fn align_content(&self) -> AlignContent {
        self.align_content
    }
    #[inline]
    fn align_items(&self) -> AlignItems {
        self.align_items
    }
    #[inline]
    fn justify_content(&self) -> JustifyContent {
        self.justify_content
    }
}

impl FlexboxItemStyle for BoxStyle {
    #[inline]
    fn flex_basis(&self) -> Dimension {
        self.flex_basis.dimension()
    }
    #[inline]
    fn flex_grow(&self) -> f32 {
        self.flex_grow
    }
    #[inline]
    fn flex_shrink(&self) -> f32 {
        self.flex_shrink
    }
    #[inline]
    fn align_self(&self) -> Option<AlignSelf> {
        self.align_self
    }
}

type Components<'a> = std::iter::Map<
    std::slice::Iter<'a, Track>,
    fn(&'a Track) -> ts::GenericGridTemplateComponent<String, &'a ts::GridTemplateRepetition<String>>,
>;

type Names<'a> = std::iter::Map<std::slice::Iter<'a, Vec<String>>, fn(&Vec<String>) -> std::slice::Iter<'_, String>>;

#[inline]
fn single(t: &Track) -> ts::GenericGridTemplateComponent<String, &ts::GridTemplateRepetition<String>> {
    ts::GenericGridTemplateComponent::Single(t.to_taffy())
}

impl GridContainerStyle for BoxStyle {
    type Repetition<'a>
        = &'a ts::GridTemplateRepetition<String>
    where
        Self: 'a;
    type TemplateTrackList<'a>
        = Components<'a>
    where
        Self: 'a;
    type AutoTrackList<'a>
        = std::iter::Map<std::slice::Iter<'a, Track>, fn(&Track) -> TrackSizingFunction>
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = Names<'a>
    where
        Self: 'a;
    type GridTemplateAreas<'a>
        = std::iter::Cloned<std::slice::Iter<'a, GridTemplateArea<String>>>
    where
        Self: 'a;

    #[inline]
    fn grid_template_rows(&self) -> Option<Components<'_>> {
        Some(self.grid_template_rows.iter().map(single))
    }
    #[inline]
    fn grid_template_columns(&self) -> Option<Components<'_>> {
        Some(self.grid_template_columns.iter().map(single))
    }
    #[inline]
    fn grid_auto_rows(&self) -> Self::AutoTrackList<'_> {
        self.grid_auto_rows.iter().map(|t| t.to_taffy())
    }
    #[inline]
    fn grid_auto_columns(&self) -> Self::AutoTrackList<'_> {
        [].iter().map(|t| t.to_taffy())
    }
    #[inline]
    fn grid_template_areas(&self) -> Option<Self::GridTemplateAreas<'_>> {
        None
    }
    #[inline]
    fn grid_template_column_names(&self) -> Option<Names<'_>> {
        None
    }
    #[inline]
    fn grid_template_row_names(&self) -> Option<Names<'_>> {
        None
    }
    #[inline]
    fn grid_auto_flow(&self) -> GridAutoFlow {
        self.grid_auto_flow
    }
    #[inline]
    fn gap(&self) -> Size<LengthPercentage> {
        Size {
            width: LengthPercentage::length(self.gap.width),
            height: LengthPercentage::length(self.gap.height),
        }
    }
    #[inline]
    fn align_content(&self) -> AlignContent {
        self.align_content
    }
    #[inline]
    fn justify_content(&self) -> JustifyContent {
        self.justify_content
    }
    #[inline]
    fn align_items(&self) -> AlignItems {
        self.align_items
    }
    #[inline]
    fn justify_items(&self) -> AlignItems {
        self.justify_items
    }
}

impl GridItemStyle for BoxStyle {
    #[inline]
    fn grid_row(&self) -> Line<GridPlacement<String>> {
        self.grid_row.clone()
    }
    #[inline]
    fn grid_column(&self) -> Line<GridPlacement<String>> {
        self.grid_column.clone()
    }
    #[inline]
    fn align_self(&self) -> Option<AlignSelf> {
        self.align_self
    }
    #[inline]
    fn justify_self(&self) -> Option<AlignSelf> {
        self.justify_self
    }
}
