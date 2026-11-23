//! The style builder: one setter per property of [`Style`], named for the
//! property it writes and taking its value (`.padding(16.0)`,
//! `.radius(8.0)`, `.align_items(Align::Center)`), never a step of a
//! design system's scale. Lengths are logical pixels; a [`Length`] also
//! takes [`pct`](crate::style::pct), `Length::Auto` and `Length::Calc`.
//!
//! Every setter takes a value or a closure giving one
//! (`.background(move || if on.get() { a } else { b })`): the closure is
//! bound to the node and runs where the node resolves, so a signal it reads
//! dirties only that node, and the style diff only the phase the property
//! belongs to ([`crate::value`]).

use crate::color::Color;
use crate::motion::{Animation, Easing, MotionStyle, Properties};
use crate::shape::{Backdrop, EdgeBar, InsetLayer, OverlayBorder, Shape};
use crate::style::{
    Align, BackgroundImage, Calc, ContentStyle, Corners, Cursor, Direction, Display, Edges, Filters, Flex,
    FlexDirection, FontFamily, GridAutoFlow, GridSpan, Justify, Length, LineHeight, MaskImage, Origin, Overflow,
    Position, Ring, Shadows, Style, TextAlign, Tracks, WrapStyle,
};
use crate::value::{Bindings, Field, Fields, IntoValue, set};

/// Conditional building, for elements and styles.
pub trait Fluent: Sized {
    /// Apply `f` when `condition` holds.
    #[must_use]
    fn when(self, condition: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if condition { f(self) } else { self }
    }

    /// Apply `f` with the value when there is one.
    #[must_use]
    fn when_some<T>(self, value: Option<T>, f: impl FnOnce(Self, T) -> Self) -> Self {
        match value {
            Some(v) => f(self, v),
            None => self,
        }
    }
}

/// The node's motion, made if it has none (shared, so this copies it only
/// when another style holds it too).
fn motion(s: &mut Style) -> &mut MotionStyle {
    std::rc::Rc::make_mut(s.motion.get_or_insert_default())
}

macro_rules! props {
    ($($(#[$doc:meta])* $name:ident: $ty:ty [$($field:ident),+] => |$s:ident, $v:ident| $body:expr;)*) => {
        $(
            $(#[$doc])*
            #[must_use]
            fn $name<M>(mut self, value: impl IntoValue<$ty, M>) -> Self {
                let (style, bindings) = self.style_and_bindings();
                const FIELDS: Fields = Fields::of(&[$(Field::$field),+]);
                set(style, bindings, FIELDS, value, |$s: &mut Style, $v: $ty| $body);
                self
            }
        )*
    };
}

/// Anything with a [`Style`]: elements, the types wrapping one, and a bare
/// [`Style`].
pub trait Styled: Sized {
    /// The fixed style the setters write, and where the closures given to
    /// setters are kept to run where the node resolves. An element returns
    /// its own bindings, and a type wrapping an element returns the
    /// element's (`self.el.style_and_bindings()`). `None` runs a closure
    /// as it is given, tracked by whatever runs the setter: a bare
    /// [`Style`] and a [`crate::Span`] (its closures are read by the render
    /// that builds its paragraph).
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut Bindings>);

    /// The [`Interaction`](crate::Interaction) the node this styles carries,
    /// if any: what a caller restyling a part reads to keep the part's
    /// state look over its own fixed values. A type wrapping an element
    /// returns the element's.
    fn node_interaction(&self) -> Option<crate::Interaction> {
        None
    }

    /// The fixed style alone. A write through it is the node's fixed
    /// value; a binding of the same property still replays over it.
    fn style_mut(&mut self) -> &mut Style {
        self.style_and_bindings().0
    }

    props! {
        // ── display and position ──
        /// `display`: block, flex, grid, or none (not laid out, not painted).
        display: Display [Display] => |s, v| s.display = v;
        /// Inline-level (`inline-flex`, `inline-block`): in block flow the
        /// box shrinks to fit its content instead of filling the line.
        inline: bool [Inline] => |s, v| s.inline = v;
        /// Laid out but not painted (`visibility: hidden`).
        invisible: bool [Invisible] => |s, v| s.invisible = v;
        position: Position [Position] => |s, v| s.position = v;
        /// All four insets.
        inset: Edges<Length> [InsetTop, InsetRight, InsetBottom, InsetLeft] => |s, v| s.inset = v;
        top: Length [InsetTop] => |s, v| s.inset.top = v;
        right: Length [InsetRight] => |s, v| s.inset.right = v;
        bottom: Length [InsetBottom] => |s, v| s.inset.bottom = v;
        left: Length [InsetLeft] => |s, v| s.inset.left = v;

        // ── flex ──
        flex_direction: FlexDirection [FlexDirection] => |s, v| s.flex_direction = v;
        flex_wrap: bool [FlexWrap] => |s, v| s.flex_wrap = v;
        /// `flex`: grow, shrink and basis at once.
        flex: Flex [FlexGrow, FlexShrink, FlexBasis] => |s, v| { s.flex_grow = v.grow; s.flex_shrink = v.shrink; s.flex_basis = v.basis };
        flex_grow: f32 [FlexGrow] => |s, v| s.flex_grow = v;
        flex_shrink: f32 [FlexShrink] => |s, v| s.flex_shrink = v;
        flex_basis: Length [FlexBasis] => |s, v| s.flex_basis = v;
        align_items: Align [AlignItems] => |s, v| s.align_items = Some(v);
        align_self: Align [AlignSelf] => |s, v| s.align_self = Some(v);
        justify_items: Align [JustifyItems] => |s, v| s.justify_items = Some(v);
        justify_self: Align [JustifySelf] => |s, v| s.justify_self = Some(v);
        justify_content: Justify [JustifyContent] => |s, v| s.justify_content = Some(v);
        align_content: Justify [AlignContent] => |s, v| s.align_content = Some(v);
        /// Space between rows and between columns, in pixels.
        gap: f32 [RowGap, ColumnGap] => |s, v| { s.row_gap = v; s.column_gap = v };
        row_gap: f32 [RowGap] => |s, v| s.row_gap = v;
        column_gap: f32 [ColumnGap] => |s, v| s.column_gap = v;

        // ── grid ──
        /// `grid-template-columns` (`Tracks::repeat(3, Track::Fr(1.0))` is `repeat(3, 1fr)`).
        grid_columns: Tracks [GridColumns] => |s, v| s.grid_columns = v;
        grid_rows: Tracks [GridRows] => |s, v| s.grid_rows = v;
        grid_auto_rows: Tracks [GridAutoRows] => |s, v| s.grid_auto_rows = v;
        grid_auto_flow: GridAutoFlow [GridAutoFlow] => |s, v| s.grid_auto_flow = v;
        /// Where the item sits among the columns: start line and span.
        grid_column: GridSpan [GridColumnStart, GridColumnSpan] => |s, v| s.grid_column = v;
        /// The column line it starts at, from 1, keeping its span.
        grid_column_start: i16 [GridColumnStart] => |s, v| s.grid_column.start = Some(v);
        /// How many columns it spans, keeping its start.
        grid_column_span: u16 [GridColumnSpan] => |s, v| s.grid_column.span = Some(v);
        grid_row: GridSpan [GridRowStart, GridRowSpan] => |s, v| s.grid_row = v;
        grid_row_start: i16 [GridRowStart] => |s, v| s.grid_row.start = Some(v);
        grid_row_span: u16 [GridRowSpan] => |s, v| s.grid_row.span = Some(v);

        // ── spacing ──
        /// Padding in pixels: one value for every side, or [`Edges`].
        padding: Edges<f32> [PaddingTop, PaddingRight, PaddingBottom, PaddingLeft] => |s, v| s.padding = v;
        /// Left and right padding.
        padding_x: f32 [PaddingLeft, PaddingRight] => |s, v| { s.padding.left = v; s.padding.right = v };
        /// Top and bottom padding.
        padding_y: f32 [PaddingTop, PaddingBottom] => |s, v| { s.padding.top = v; s.padding.bottom = v };
        padding_top: f32 [PaddingTop] => |s, v| s.padding.top = v;
        padding_right: f32 [PaddingRight] => |s, v| s.padding.right = v;
        padding_bottom: f32 [PaddingBottom] => |s, v| s.padding.bottom = v;
        padding_left: f32 [PaddingLeft] => |s, v| s.padding.left = v;
        /// Margin: one length for every side, or [`Edges`].
        margin: Edges<Length> [MarginTop, MarginRight, MarginBottom, MarginLeft] => |s, v| s.margin = v;
        margin_x: Length [MarginLeft, MarginRight] => |s, v| { s.margin.left = v; s.margin.right = v };
        margin_y: Length [MarginTop, MarginBottom] => |s, v| { s.margin.top = v; s.margin.bottom = v };
        margin_top: Length [MarginTop] => |s, v| s.margin.top = v;
        margin_right: Length [MarginRight] => |s, v| s.margin.right = v;
        margin_bottom: Length [MarginBottom] => |s, v| s.margin.bottom = v;
        margin_left: Length [MarginLeft] => |s, v| s.margin.left = v;
        /// `margin-inline-start`: the left margin, or the right one under
        /// [`Direction::Rtl`]; it wins over the physical side it lands on.
        margin_start: Length [MarginStart] => |s, v| s.margin_start = Some(v);
        /// `margin-inline-end`.
        margin_end: Length [MarginEnd] => |s, v| s.margin_end = Some(v);

        // ── sizing ──
        width: Length [Width] => |s, v| s.width = v;
        height: Length [Height] => |s, v| s.height = v;
        /// Width and height.
        size: Length [Width, Height] => |s, v| { s.width = v; s.height = v };
        min_width: Length [MinWidth] => |s, v| s.min_width = v;
        min_height: Length [MinHeight] => |s, v| s.min_height = v;
        max_width: Length [MaxWidth] => |s, v| s.max_width = v;
        max_height: Length [MaxHeight] => |s, v| s.max_height = v;
        /// Width over height.
        aspect_ratio: f32 [AspectRatio] => |s, v| s.aspect_ratio = Some(v);
        /// The size in pixels it provides to the icons below that set none.
        icon_size: f32 [IconSize] => |s, v| s.content.icon_size = Some(v);

        // ── overflow ──
        /// Both axes.
        overflow: Overflow [OverflowX, OverflowY] => |s, v| { s.overflow_x = v; s.overflow_y = v };
        overflow_x: Overflow [OverflowX] => |s, v| s.overflow_x = v;
        overflow_y: Overflow [OverflowY] => |s, v| s.overflow_y = v;
        /// A scroll container shows no scrollbar (`scrollbar-width: none`).
        scrollbar_hidden: bool [ScrollbarHidden] => |s, v| s.scrollbar_hidden = v;

        // ── background, border, corners ──
        /// The fill; `None` paints none.
        background: Option<Color> [Background] => |s, v| s.background = v;
        /// A gradient tile over the background; a [`crate::style::LinearGradient`]
        /// covers the whole box.
        background_image: Option<BackgroundImage> [BackgroundImage] => |s, v| s.effects.background_image = v;
        /// Border widths in pixels: one for every side, or [`Edges`].
        border: Edges<f32> [BorderTop, BorderRight, BorderBottom, BorderLeft] => |s, v| s.border_widths = v;
        border_top: f32 [BorderTop] => |s, v| s.border_widths.top = v;
        border_right: f32 [BorderRight] => |s, v| s.border_widths.right = v;
        border_bottom: f32 [BorderBottom] => |s, v| s.border_widths.bottom = v;
        border_left: f32 [BorderLeft] => |s, v| s.border_widths.left = v;
        border_color: Option<Color> [BorderColor] => |s, v| s.border_color = v;
        /// The node's shape ([`Shape`]): what its background, border, ring,
        /// shadows, clip and hit testing follow; a radius (`f32` or
        /// [`Corners`]) gives round corners.
        shape: Shape [RadiusTopLeft, RadiusTopRight, RadiusBottomRight, RadiusBottomLeft] => |s, v| s.shape = v;
        /// Round corners of these radii in pixels: one for every corner, or
        /// [`Corners`]. Overlapping radii scale down as CSS's do, so
        /// [`Corners::FULL`] is a stadium.
        radius: Corners [RadiusTopLeft, RadiusTopRight, RadiusBottomRight, RadiusBottomLeft] => |s, v| s.shape = Shape::Rounded(v);
        /// The top-left and top-right radii, keeping the shape's curve.
        radius_top: f32 [RadiusTopLeft, RadiusTopRight] => |s, v| s.shape = s.shape.with_radii(|r| { r.top_left = v; r.top_right = v });
        radius_bottom: f32 [RadiusBottomLeft, RadiusBottomRight] => |s, v| s.shape = s.shape.with_radii(|r| { r.bottom_left = v; r.bottom_right = v });
        radius_left: f32 [RadiusTopLeft, RadiusBottomLeft] => |s, v| s.shape = s.shape.with_radii(|r| { r.top_left = v; r.bottom_left = v });
        radius_right: f32 [RadiusTopRight, RadiusBottomRight] => |s, v| s.shape = s.shape.with_radii(|r| { r.top_right = v; r.bottom_right = v });
        /// A bar along one side, over the border ([`EdgeBar`]); `None` draws none.
        edge_bar: Option<EdgeBar> [EdgeBar] => |s, v| s.decoration.edge_bar = v;
        /// A box inset into this one, painted behind the content ([`InsetLayer`]).
        inset_layer: Option<InsetLayer> [InsetLayer] => |s, v| s.decoration.inset = v;
        /// A border over the content and children, blended ([`OverlayBorder`]).
        overlay_border: Option<OverlayBorder> [OverlayBorder] => |s, v| s.decoration.overlay = v;
        /// What lies behind, blurred and saturated inside the shape ([`Backdrop`]).
        backdrop: Option<Backdrop> [Backdrop] => |s, v| s.decoration.backdrop = v;

        // ── effects ──
        /// Box shadows: one [`crate::style::Shadow`], an array of them, or
        /// `Shadows::new()` for none. Kept inline: writing them allocates nothing.
        shadow: Shadows [Shadows] => |s, v| s.shadows = v;
        /// A focus ring outside the shape, at its offset ([`Ring`]); `None` draws none.
        ring: Option<Ring> [Ring] => |s, v| s.ring = v;
        /// How far past its border box the node takes the pointer, per side:
        /// a larger target than it paints (a slider thumb's `after:-inset-2`).
        hit_slop: Edges<f32> [HitSlop] => |s, v| s.hit_slop = v;
        /// Multiplies the alpha of everything this node and its children paint.
        opacity: f32 [Opacity] => |s, v| s.opacity = v;
        /// Turn the node's painting `degrees` clockwise about its origin.
        rotate: f32 [Rotate] => |s, v| s.transform.rotate = v;
        /// Scale the node's painting about its origin (`1.0` as laid out).
        scale: f32 [Scale] => |s, v| s.transform.scale = v;
        /// Move the node: a fraction of its own width plus pixels.
        translate_x: Calc [TranslateX] => |s, v| s.transform.translate[0] = v;
        translate_y: Calc [TranslateY] => |s, v| s.transform.translate[1] = v;
        /// The point it scales and turns about.
        origin: Origin [Origin] => |s, v| s.transform.origin = v;
        /// `mask-image` over this node and its children: a gradient, or a
        /// scroll container's [`crate::style::ScrollFade`]; `None` masks nothing.
        mask: Option<MaskImage> [Mask] => |s, v| s.effects.mask = v;
        /// CSS `filter`s over the node's painting, children included, in order.
        filters: Filters [Filters] => |s, v| s.effects.filters = v;
        /// The pointer's cursor over it; `None` shows its parent's.
        cursor: Option<Cursor> [Cursor] => |s, v| s.cursor = v;

        // ── motion ──
        /// What eases to a new value instead of jumping to it.
        transition: Properties [Transition] => |s, v| motion(s).transition = v;
        /// How long transitions and the node's animations take, in milliseconds.
        duration_ms: f32 [Duration] => |s, v| motion(s).duration = Some(v / 1000.0);
        /// How long a transition waits before it starts, in milliseconds.
        delay_ms: f32 [Delay] => |s, v| motion(s).delay = Some(v / 1000.0);
        /// The transitions' and animations' timing.
        ease: Easing [Easing] => |s, v| motion(s).easing = Some(v);
        /// What plays from the frame the node first has it; `None` plays nothing.
        animation: Option<Animation> [Animation] => |s, v| motion(s).animation = v.map(std::rc::Rc::new);
        /// What plays when the node leaves the tree, which keeps it painted,
        /// inert, until it ends.
        exit_animation: Option<Animation> [ExitAnimation] => |s, v| motion(s).exit = v.map(std::rc::Rc::new);

        // ── content (ADR-0022's `ContentStyle`, provided to the subtree) ──
        /// Everything it provides to the text and icons of its subtree at
        /// once ([`ContentStyle`]): each field set replaces the enclosing
        /// provider's, `None` leaves it.
        provide_content: ContentStyle [TextColor, TextSize, LineHeight, FontWeight, FontFamily, TextAlign, Nowrap, Truncate, LineClamp, TextWrap, Italic, Underline, UnderlineOffset, LineThrough, Tracking, TabularNums, IconSize] => |s, v| s.content = v;
        /// The colour it provides to its text and icons, and its
        /// `currentColor`; `None` leaves the enclosing provider's. Read
        /// where they paint: a change repaints them and lays out nothing.
        text_color: Option<Color> [TextColor] => |s, v| s.content.color = v;
        /// Font size in pixels. Without a line height of its own the node
        /// keeps its parent's ratio of line height to size.
        text_size: f32 [TextSize] => |s, v| s.content.text.size = Some(v);
        /// Line height: pixels, or `LineHeight::Relative(k)` times the font size.
        line_height: LineHeight [LineHeight] => |s, v| s.content.text.line_height = Some(v);
        /// Font weight, 100 to 900.
        font_weight: u16 [FontWeight] => |s, v| s.content.text.weight = Some(v);
        font_family: FontFamily [FontFamily] => |s, v| s.content.text.family = Some(v);
        text_align: TextAlign [TextAlign] => |s, v| s.content.text.align = Some(v);
        /// The inline direction of this node and what it holds (`dir`).
        direction: Direction [Direction] => |s, v| s.direction = Some(v);
        /// Keep the text on one line (`white-space: nowrap`).
        nowrap: bool [Nowrap] => |s, v| s.content.text.nowrap = Some(v);
        /// Cut overflowing text with an ellipsis (`text-overflow: ellipsis`), on one line.
        truncate: bool [Truncate] => |s, v| s.content.text.truncate = Some(v);
        /// Show at most this many lines, the last ending in an ellipsis.
        line_clamp: u16 [LineClamp] => |s, v| s.content.text.line_clamp = Some(v);
        /// How wrapped lines are chosen (`text-wrap`).
        text_wrap: WrapStyle [TextWrap] => |s, v| s.content.text.wrap_style = Some(v);
        italic: bool [Italic] => |s, v| s.content.text.italic = Some(v);
        /// Underline the text it holds (`text-decoration-line:
        /// underline`): a paragraph's line by its font and colour under
        /// all its runs, a span's by the span's own. `false` declares no
        /// underline of its own; it cannot take off one an enclosing node
        /// or paragraph declares, which CSS draws over everything inside
        /// it whatever its descendants say.
        underline: bool [Underline] => |s, v| s.content.text.underline = Some(v);
        /// The underline's top this many pixels below the baseline
        /// (`text-underline-offset`), with no gap added.
        underline_offset: f32 [UnderlineOffset] => |s, v| s.content.text.underline_offset = Some(v);
        /// Strike its text through (`text-decoration-line: line-through`);
        /// as with `underline`, `false` cannot take off an enclosing
        /// node's.
        line_through: bool [LineThrough] => |s, v| s.content.text.line_through = Some(v);
        /// Letter spacing in em.
        tracking: f32 [Tracking] => |s, v| s.content.text.tracking = Some(v);
        /// Every digit as wide as the others (OpenType `tnum`).
        tabular_nums: bool [TabularNums] => |s, v| s.content.text.tabular_nums = Some(v);
    }
}

impl Fluent for Style {}

impl Styled for Style {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut Bindings>) {
        (self, None)
    }
}
