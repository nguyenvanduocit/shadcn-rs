//! What a node looks like and how it is laid out: the CSS subset shadcn's
//! Tailwind classes use, as one value per node. [`crate::styled::Styled`]
//! is the builder that writes it; [`Style::box_style`] hands the layout part
//! to the layout tree.

use taffy as ts;

use crate::color::Color;
use crate::element::{Side, SideAlign};
use crate::motion::MotionStyle;

/// A size or offset along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Length {
    #[default]
    Auto,
    Px(f32),
    /// A fraction of the containing block, `0.0..=1.0`.
    Fraction(f32),
    /// `fit-content`: the content's size, at most the space there is. A size
    /// keyword: as a margin or an inset it is `auto`.
    FitContent,
    /// `calc(fraction × 100% + px)` (`max-w-[calc(100%-2rem)]`).
    Calc {
        fraction: f32,
        px: f32,
    },
}

impl Length {
    /// Whether it reads the containing block's size.
    #[must_use]
    pub fn relative(self) -> bool {
        matches!(self, Self::Fraction(_) | Self::Calc { .. })
    }
}

/// One value per side.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Edges<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T: Copy> Edges<T> {
    #[must_use]
    pub const fn all(v: T) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    fn map<U>(self, f: impl Fn(T) -> U) -> ts::Rect<U> {
        ts::Rect {
            left: f(self.left),
            right: f(self.right),
            top: f(self.top),
            bottom: f(self.bottom),
        }
    }
}

/// Corner radii, clockwise from the top-left: pixels once laid out, each a
/// [`Radius`] as a style writes them.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Corners<T = f32> {
    pub top_left: T,
    pub top_right: T,
    pub bottom_right: T,
    pub bottom_left: T,
}

impl<T: Copy> Corners<T> {
    #[must_use]
    pub const fn all(r: T) -> Self {
        Self {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        }
    }
}

impl Corners<Radius> {
    /// The radii in pixels, against the `--radius` the node inherits.
    #[must_use]
    pub fn resolve(&self, var: f32) -> Corners {
        Corners {
            top_left: self.top_left.resolve(var),
            top_right: self.top_right.resolve(var),
            bottom_right: self.bottom_right.resolve(var),
            bottom_left: self.bottom_left.resolve(var),
        }
    }
}

/// One corner's radius as a style writes it: `calc(var(--radius) * times +
/// plus)`, shadcn's `--radius` scaled (`rounded-md` is `calc(var(--radius) *
/// 0.8)` in its theme) plus pixels, or pixels alone when `times` is zero. It
/// resolves against the `--radius` the node inherits
/// ([`ResolvedText::radius`]), so a subtree that sets it (`[--radius:1rem]`)
/// rounds every `rounded-sm` to `rounded-3xl` below it, as in CSS.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Radius {
    pub times: f32,
    pub plus: f32,
}

impl Radius {
    /// `rounded-sm`.
    pub const SM: Self = Self::times(0.6);
    /// `rounded-md`.
    pub const MD: Self = Self::times(0.8);
    /// `rounded-lg`.
    pub const LG: Self = Self::times(1.0);
    /// `rounded-xl`.
    pub const XL: Self = Self::times(1.4);
    /// `rounded-2xl`.
    pub const XL2: Self = Self::times(1.8);
    /// `rounded-3xl`.
    pub const XL3: Self = Self::times(2.2);

    const fn times(times: f32) -> Self {
        Self { times, plus: 0.0 }
    }

    /// Pixels, whatever `--radius` is.
    #[must_use]
    pub const fn px(px: f32) -> Self {
        Self { times: 0.0, plus: px }
    }

    /// `calc(var(--radius) + plus)` (`rounded-[calc(var(--radius)-5px)]` is `var(-5.0)`).
    #[must_use]
    pub const fn var(plus: f32) -> Self {
        Self { times: 1.0, plus }
    }

    /// Pixels, against an inherited `--radius` of `var`; never below zero.
    #[must_use]
    pub fn resolve(self, var: f32) -> f32 {
        var.mul_add(self.times, self.plus).max(0.0)
    }
}

impl From<f32> for Radius {
    fn from(px: f32) -> Self {
        Self::px(px)
    }
}

impl Corners {
    /// Every radius multiplied by `f`.
    #[must_use]
    pub fn scaled(&self, f: f32) -> Self {
        Self {
            top_left: self.top_left * f,
            top_right: self.top_right * f,
            bottom_right: self.bottom_right * f,
            bottom_left: self.bottom_left * f,
        }
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.top_left <= 0.0 && self.top_right <= 0.0 && self.bottom_right <= 0.0 && self.bottom_left <= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Block,
    Flex,
    Grid,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Relative,
    Absolute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    /// Children are clipped to the padding box.
    Hidden,
    /// Clipped, and the wheel scrolls the content.
    Scroll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

/// `align-items`, `align-self`, `justify-items`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
    Baseline,
}

/// `justify-content`, `align-content`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    Between,
    Around,
    Evenly,
    Stretch,
}

/// One grid track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Track {
    Fr(f32),
    Px(f32),
    Auto,
    MinContent,
    MaxContent,
}

impl Track {
    pub(crate) fn to_taffy(self) -> ts::TrackSizingFunction {
        match self {
            Self::Fr(f) => taffy::style_helpers::minmax(
                ts::MinTrackSizingFunction::length(0.0),
                ts::MaxTrackSizingFunction::fr(f),
            ),
            Self::Px(v) => taffy::style_helpers::length(v),
            Self::Auto => taffy::style_helpers::auto(),
            Self::MinContent => taffy::style_helpers::min_content(),
            Self::MaxContent => taffy::style_helpers::max_content(),
        }
    }
}

/// Where an item sits on one grid axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GridSpan {
    /// 1-based start line; `None` lets auto-placement choose.
    pub start: Option<i16>,
    pub span: Option<u16>,
}

impl GridSpan {
    fn to_taffy(self) -> ts::Line<ts::GridPlacement> {
        let start = self.start.map_or(ts::GridPlacement::Auto, taffy::style_helpers::line);
        let end = self.span.map_or(ts::GridPlacement::Auto, ts::GridPlacement::Span);
        ts::Line { start, end }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Text,
    NotAllowed,
    Grab,
    ResizeColumn,
    ResizeRow,
}

/// A box shadow; `inset` shadows are not drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

/// A solid outline drawn outside the border box (Tailwind's `ring`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub width: f32,
    pub color: Color,
}

/// The colour a gradient stop paints: a fixed one, or the text colour
/// (`currentColor`) through CSS relative-colour syntax, resolved where the
/// node is painted, as CSS resolves `currentColor` at used-value time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    Color(Color),
    /// `oklch(from currentColor max(floor, l + lift) c h / calc(alpha * alpha_scale + alpha_add))`.
    Current {
        lightness_floor: f32,
        lightness_lift: f32,
        alpha_scale: f32,
        alpha_add: f32,
    },
}

impl Ink {
    /// `currentColor` itself.
    pub const CURRENT: Self = Self::Current {
        lightness_floor: 0.0,
        lightness_lift: 0.0,
        alpha_scale: 1.0,
        alpha_add: 0.0,
    };

    /// The colour this ink paints in a node whose text colour is `current`.
    #[must_use]
    pub fn resolve(self, current: Color) -> Color {
        match self {
            Self::Color(c) => c,
            Self::Current {
                lightness_floor,
                lightness_lift,
                alpha_scale,
                alpha_add,
            } => {
                let [l, a, b] = current.to_oklab();
                let alpha = current.a.mul_add(alpha_scale, alpha_add).clamp(0.0, 1.0);
                // CSS clamps OKLCH lightness to 0..=1.
                let l = (l + lightness_lift).max(lightness_floor).clamp(0.0, 1.0);
                Color::oklab([l, a, b], alpha)
            }
        }
    }
}

/// A length mixing units, CSS `calc(<fraction × 100>% + <px>px + <ch>ch)`; the
/// percentage is of what the property measures against, `ch` the width of
/// the node's `0`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Calc {
    pub fraction: f32,
    pub px: f32,
    pub ch: f32,
}

impl Calc {
    #[must_use]
    pub const fn fraction(fraction: f32) -> Self {
        Self {
            fraction,
            px: 0.0,
            ch: 0.0,
        }
    }

    #[must_use]
    pub const fn px(px: f32) -> Self {
        Self {
            fraction: 0.0,
            px,
            ch: 0.0,
        }
    }

    /// Pixels, against `basis` for the percentage and `ch` pixels per `ch`.
    #[must_use]
    pub fn resolve(self, basis: f32, ch: f32) -> f32 {
        self.fraction.mul_add(basis, self.ch.mul_add(ch, self.px))
    }
}

/// One colour stop of a gradient, placed along the gradient line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub ink: Ink,
    pub at: Calc,
}

/// CSS `linear-gradient(<angle>, <stops>)`, interpolated in OKLab as
/// Tailwind's `bg-linear-*` asks.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    /// CSS degrees: 0 points up, 90 right, 180 down.
    pub angle: f32,
    pub stops: Vec<Stop>,
}

/// Where a background image shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackgroundClip {
    #[default]
    BorderBox,
    /// Only through the node's own text, which then paints no colour of its
    /// own (`bg-clip-text text-transparent`, `-webkit-text-fill-color: transparent`).
    Text,
}

/// A gradient background tile (`background-repeat: no-repeat`).
#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundImage {
    pub gradient: LinearGradient,
    /// The tile's width and height against the padding box (`background-size`).
    pub size: [Calc; 2],
    /// The tile's offset (`background-position`); as in CSS, the percentage is
    /// of the room left, padding box less tile.
    pub position: [Calc; 2],
    pub clip: BackgroundClip,
}

impl Default for BackgroundImage {
    /// Tailwind's empty `bg-linear-to-b`: transparent to transparent over the box.
    fn default() -> Self {
        let clear = |at| Stop {
            ink: Ink::Color(Color::TRANSPARENT),
            at: Calc::fraction(at),
        };
        Self {
            gradient: LinearGradient {
                angle: 180.0,
                stops: vec![clear(0.0), clear(1.0)],
            },
            size: [Calc::fraction(1.0); 2],
            position: [Calc::default(); 2],
            clip: BackgroundClip::BorderBox,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Scroll distance over which a scroll fade grows to its full size
/// (`--scroll-fade-reveal`, `spacing * 24`).
const FADE_REVEAL: f32 = 96.0;

/// shadcn's `scroll-fade`: a scroll container's edges fade where more content
/// lies beyond them. A mask gradient whose edge stops follow the scroll
/// position, as the utility's scroll-driven animation moves them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollFade {
    pub axis: Axis,
    /// Fade the top or left edge.
    pub start: bool,
    /// Fade the bottom or right edge.
    pub end: bool,
    /// Full fade size in pixels; `None` is `min(12%, 40px)` of the box.
    pub size: Option<f32>,
}

impl ScrollFade {
    /// The mask for a box `length` long on the fade's axis, scrolled to
    /// `offset` of `max`; `None` when nothing overflows, as a scroll timeline
    /// with no range applies no fade.
    #[must_use]
    pub fn gradient(&self, length: f32, offset: f32, max: f32) -> Option<LinearGradient> {
        if max <= 0.0 {
            return None;
        }
        let full = self.size.unwrap_or_else(|| (length * 0.12).min(40.0));
        let start = if self.start {
            full * ease_in_out((offset / FADE_REVEAL).clamp(0.0, 1.0))
        } else {
            0.0
        };
        let end = if self.end {
            full * (1.0 - ease_in_out(((offset - max + FADE_REVEAL) / FADE_REVEAL).clamp(0.0, 1.0)))
        } else {
            0.0
        };
        let stop = |color: Color, at: Calc| Stop {
            ink: Ink::Color(color),
            at,
        };
        Some(LinearGradient {
            angle: match self.axis {
                Axis::Horizontal => 90.0,
                Axis::Vertical => 180.0,
            },
            stops: vec![
                stop(Color::TRANSPARENT, Calc::default()),
                stop(Color::BLACK, Calc::px(start)),
                stop(
                    Color::BLACK,
                    Calc {
                        fraction: 1.0,
                        px: -end,
                        ch: 0.0,
                    },
                ),
                stop(Color::TRANSPARENT, Calc::fraction(1.0)),
            ],
        })
    }
}

/// CSS `ease-in-out` at progress `x`.
fn ease_in_out(x: f32) -> f32 {
    crate::motion::Easing::EASE_IN_OUT.at(x)
}

/// What masks a node's painting, children included (`mask-image`).
#[derive(Clone, Debug, PartialEq)]
pub enum MaskImage {
    Linear(LinearGradient),
    ScrollFade(ScrollFade),
}

/// A CSS `filter` function, applied to the node's painting, children included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    /// `brightness(k)`: every channel times `k`.
    Brightness(f32),
    /// `grayscale(amount)`: towards the luminance, Filter Effects' matrix.
    Grayscale(f32),
}

/// Paint beyond a solid box: a gradient background, a mask and filters over
/// the node's painting.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Effects {
    pub background_image: Option<BackgroundImage>,
    pub mask: Option<MaskImage>,
    pub filters: Vec<Filter>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum FontFamily {
    #[default]
    Sans,
    Mono,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextAlign {
    /// The side lines start from: left, or right under [`Direction::Rtl`]
    /// (CSS `start`, the initial value).
    #[default]
    Start,
    /// The side lines end on (CSS `end`).
    End,
    Left,
    Center,
    Right,
}

/// The inline direction (CSS `direction`, set by HTML's `dir`): which way
/// text, flex rows, grid columns and horizontal overflow run. Inherited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

/// How wrapped text chooses its line breaks (CSS `text-wrap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum WrapStyle {
    /// Each line takes all the words that fit.
    #[default]
    Auto,
    /// Lines of even width, as narrow as their count allows (`balance`).
    Balance,
    /// No last line of a single word (`pretty`).
    Pretty,
}

/// Line height: a multiple of the font size or an absolute size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Relative(f32),
    Px(f32),
}

/// The inherited text properties a node sets; `None` takes the parent's.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct TextStyle {
    pub color: Option<Color>,
    pub size: Option<f32>,
    pub line_height: Option<LineHeight>,
    pub weight: Option<u16>,
    pub family: Option<FontFamily>,
    pub align: Option<TextAlign>,
    pub direction: Option<Direction>,
    /// Keep the text on one line.
    pub nowrap: Option<bool>,
    /// Cut overflowing text with an ellipsis (implies one line).
    pub truncate: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// How far below the baseline an underline sits (`underline-offset-*`);
    /// `None` takes the font's own position.
    pub underline_offset: Option<f32>,
    pub line_through: Option<bool>,
    /// Letter spacing in em.
    pub tracking: Option<f32>,
    /// Digits of one width (`tabular-nums`, OpenType `tnum`).
    pub tabular_nums: Option<bool>,
    /// Show at most this many lines, the last ending in an ellipsis.
    pub line_clamp: Option<u16>,
    /// How wrapped lines are chosen (`text-wrap`).
    pub wrap_style: Option<WrapStyle>,
    /// The size of icons below that set none (shadcn's `[&_svg]:size-3`).
    pub icon_size: Option<f32>,
    /// shadcn's `--radius` (`[--radius:1rem]`), which every [`Radius`]
    /// below derives from.
    pub radius: Option<f32>,
}

/// Text properties after inheritance: what a text node is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedText {
    pub color: Color,
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
    pub family: FontFamily,
    pub align: TextAlign,
    pub direction: Direction,
    pub nowrap: bool,
    pub truncate: bool,
    pub italic: bool,
    pub underline: bool,
    pub underline_offset: Option<f32>,
    pub line_through: bool,
    pub tracking: f32,
    pub tabular_nums: bool,
    pub line_clamp: Option<u16>,
    pub wrap_style: WrapStyle,
    /// Width and height of an icon that sets no size of its own.
    pub icon_size: f32,
    /// shadcn's `--radius`, inherited as a CSS custom property is.
    pub radius: f32,
}

impl ResolvedText {
    /// The root's text: 16px/24px normal weight.
    #[must_use]
    pub fn root(color: Color) -> Self {
        Self {
            color,
            size: 16.0,
            line_height: 24.0,
            weight: 400,
            family: FontFamily::Sans,
            align: TextAlign::Start,
            direction: Direction::Ltr,
            nowrap: false,
            truncate: false,
            italic: false,
            underline: false,
            underline_offset: None,
            line_through: false,
            tracking: 0.0,
            tabular_nums: false,
            line_clamp: None,
            wrap_style: WrapStyle::Auto,
            // Tailwind's `size-4`, lucide's size in shadcn.
            icon_size: 16.0,
            radius: crate::styled::RADIUS,
        }
    }

    /// The alignment on the page: `Start` and `End` resolved by the direction.
    #[must_use]
    pub fn physical_align(&self) -> TextAlign {
        match (self.align, self.direction) {
            (TextAlign::Start, Direction::Ltr) | (TextAlign::End, Direction::Rtl) => TextAlign::Left,
            (TextAlign::Start, Direction::Rtl) | (TextAlign::End, Direction::Ltr) => TextAlign::Right,
            (other, _) => other,
        }
    }

    /// These properties with `own` applied on top, as CSS inheritance does.
    #[must_use]
    pub fn inherit(&self, own: &TextStyle) -> Self {
        let size = own.size.unwrap_or(self.size);
        // A new font size without a new line height keeps the parent's ratio,
        // as Tailwind's `text-*` utilities set both.
        let line_height = match own.line_height {
            Some(LineHeight::Px(px)) => px,
            Some(LineHeight::Relative(r)) => r * size,
            None if own.size.is_some() => size * (self.line_height / self.size.max(1.0)),
            None => self.line_height,
        };
        Self {
            color: own.color.unwrap_or(self.color),
            size,
            line_height,
            weight: own.weight.unwrap_or(self.weight),
            family: own.family.unwrap_or(self.family),
            align: own.align.unwrap_or(self.align),
            direction: own.direction.unwrap_or(self.direction),
            nowrap: own.nowrap.unwrap_or(self.nowrap),
            truncate: own.truncate.unwrap_or(self.truncate),
            italic: own.italic.unwrap_or(self.italic),
            underline: own.underline.unwrap_or(self.underline),
            underline_offset: own.underline_offset.or(self.underline_offset),
            line_through: own.line_through.unwrap_or(self.line_through),
            tracking: own.tracking.unwrap_or(self.tracking),
            tabular_nums: own.tabular_nums.unwrap_or(self.tabular_nums),
            line_clamp: own.line_clamp.or(self.line_clamp),
            wrap_style: own.wrap_style.unwrap_or(self.wrap_style),
            icon_size: own.icon_size.unwrap_or(self.icon_size),
            radius: own.radius.unwrap_or(self.radius),
        }
    }
}

/// The point a node scales and turns about (`transform-origin`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Origin {
    /// Against the node's own border box (`origin-top` is `[50%, 0]`).
    At([Calc; 2]),
    /// The edge facing the anchor of the layer the node is in, lined up as
    /// the layer is (reka's `--reka-popper-transform-origin`); the centre
    /// outside an anchored layer.
    Anchor,
}

impl Origin {
    /// `origin-center`, the default.
    pub const CENTER: Self = Self::At([Calc::fraction(0.5), Calc::fraction(0.5)]);
    /// `origin-top`.
    pub const TOP: Self = Self::At([Calc::fraction(0.5), Calc::fraction(0.0)]);
}

/// CSS's individual transforms: where a node's painting moves, how much it
/// scales and how far it turns, about [`Origin`]. Moving takes the pointer
/// along, as in CSS; scaling and turning are painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// `translate`: against the node's own border box.
    pub translate: [Calc; 2],
    /// Pixels toward the anchor of the layer the node is in, on whichever
    /// side the layer opened: tw-animate's `slide-in-from-*` under reka's
    /// `data-side`.
    pub toward_anchor: f32,
    pub scale: f32,
    /// Degrees, clockwise.
    pub rotate: f32,
    pub origin: Origin,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translate: [Calc::default(); 2],
            toward_anchor: 0.0,
            scale: 1.0,
            rotate: 0.0,
            origin: Origin::CENTER,
        }
    }
}

/// How a layer was placed against its anchor: the side it opened on and how
/// it lines up there; `None` outside an anchored layer.
pub(crate) type Opened = Option<(Side, SideAlign)>;

impl Transform {
    /// How far the node's box moves, given its size and how its layer was placed.
    #[must_use]
    pub(crate) fn offset(&self, size: (f32, f32), placed: Opened) -> (f32, f32) {
        let (mut x, mut y) = (
            self.translate[0].resolve(size.0, 0.0),
            self.translate[1].resolve(size.1, 0.0),
        );
        let d = self.toward_anchor;
        match placed.map(|(side, _)| side) {
            Some(Side::Bottom) => y -= d,
            Some(Side::Top) => y += d,
            Some(Side::Right) => x -= d,
            Some(Side::Left) => x += d,
            None => {}
        }
        (x, y)
    }

    /// The scale and turn to paint a node of `bounds` with.
    #[must_use]
    pub(crate) fn painted(&self, bounds: crate::geometry::Bounds, placed: Opened) -> Painted {
        let along = |align: SideAlign| match align {
            SideAlign::Start => 0.0,
            SideAlign::Center => 0.5,
            SideAlign::End => 1.0,
        };
        let (fx, fy) = match (self.origin, placed) {
            (Origin::At([x, y]), _) => (
                x.resolve(bounds.width, 0.0) / bounds.width.max(f32::EPSILON),
                y.resolve(bounds.height, 0.0) / bounds.height.max(f32::EPSILON),
            ),
            (Origin::Anchor, Some((Side::Bottom, a))) => (along(a), 0.0),
            (Origin::Anchor, Some((Side::Top, a))) => (along(a), 1.0),
            (Origin::Anchor, Some((Side::Right, a))) => (0.0, along(a)),
            (Origin::Anchor, Some((Side::Left, a))) => (1.0, along(a)),
            (Origin::Anchor, None) => (0.5, 0.5),
        };
        Painted {
            scale: self.scale,
            rotate: self.rotate,
            origin: (bounds.width * fx, bounds.height * fy),
        }
    }
}

/// A node's scale and turn as painted, about `origin`, in pixels from the
/// top-left corner of its border box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Painted {
    pub scale: f32,
    pub rotate: f32,
    pub origin: (f32, f32),
}

impl Painted {
    /// Paints as laid out.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        (self.scale - 1.0).abs() < 1e-4 && self.rotate.rem_euclid(360.0) < 1e-4
    }
}

/// Everything about one node's box: layout, paint and the text it passes down.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub display: Display,
    /// Inline-level (`inline-flex`, `inline-block`): in block flow the box
    /// shrinks to fit its content instead of filling the line.
    pub inline: bool,
    pub position: Position,
    pub inset: Edges<Length>,
    pub width: Length,
    pub height: Length,
    pub min_width: Length,
    pub min_height: Length,
    pub max_width: Length,
    pub max_height: Length,
    pub aspect_ratio: Option<f32>,
    pub margin: Edges<Length>,
    /// `margin-inline-start` (`ms-*`): the left margin, or the right one
    /// under [`Direction::Rtl`]; it wins over the physical side it lands on.
    pub margin_start: Option<Length>,
    /// `margin-inline-end` (`me-*`).
    pub margin_end: Option<Length>,
    pub padding: Edges<f32>,
    pub border_widths: Edges<f32>,
    pub flex_direction: FlexDirection,
    pub flex_wrap: bool,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Length,
    pub align_items: Option<Align>,
    pub align_self: Option<Align>,
    pub justify_items: Option<Align>,
    pub justify_self: Option<Align>,
    pub justify_content: Option<Justify>,
    pub align_content: Option<Justify>,
    /// Space between rows (`gap-y`).
    pub row_gap: f32,
    /// Space between columns (`gap-x`).
    pub column_gap: f32,
    pub grid_columns: Vec<Track>,
    pub grid_rows: Vec<Track>,
    pub grid_auto_rows: Vec<Track>,
    pub grid_flow_column: bool,
    pub grid_column: GridSpan,
    pub grid_row: GridSpan,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,

    pub background: Option<Color>,
    pub border_color: Option<Color>,
    pub radius: Corners<Radius>,
    pub shadows: Vec<Shadow>,
    pub ring: Option<Ring>,
    /// Multiplies the alpha of everything this node and its children paint.
    pub opacity: f32,
    pub cursor: Option<Cursor>,
    /// Where this node's painting moves, scales and turns (`translate`, `scale`, `rotate`).
    pub transform: Transform,
    /// Hidden but still laid out (`invisible`).
    pub invisible: bool,
    pub effects: Effects,
    /// How it moves: transitions and keyframe animations. Boxed, as few
    /// nodes move and every node carries a style.
    pub motion: Option<Box<MotionStyle>>,
    /// A scroll container shows no scrollbar (`scrollbar-width: none`).
    pub scrollbar_none: bool,
    pub text: TextStyle,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            display: Display::Block,
            inline: false,
            position: Position::Relative,
            inset: Edges::all(Length::Auto),
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            aspect_ratio: None,
            margin: Edges::all(Length::Px(0.0)),
            margin_start: None,
            margin_end: None,
            padding: Edges::all(0.0),
            border_widths: Edges::all(0.0),
            flex_direction: FlexDirection::Row,
            flex_wrap: false,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Length::Auto,
            align_items: None,
            align_self: None,
            justify_items: None,
            justify_self: None,
            justify_content: None,
            align_content: None,
            row_gap: 0.0,
            column_gap: 0.0,
            grid_columns: Vec::new(),
            grid_rows: Vec::new(),
            grid_auto_rows: Vec::new(),
            grid_flow_column: false,
            grid_column: GridSpan::default(),
            grid_row: GridSpan::default(),
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            background: None,
            border_color: None,
            radius: Corners::default(),
            shadows: Vec::new(),
            ring: None,
            opacity: 1.0,
            cursor: None,
            transform: Transform::default(),
            invisible: false,
            effects: Effects::default(),
            motion: None,
            scrollbar_none: false,
            text: TextStyle::default(),
        }
    }
}

fn align(a: Align) -> ts::AlignItems {
    match a {
        Align::Start => ts::AlignItems::START,
        Align::Center => ts::AlignItems::CENTER,
        Align::End => ts::AlignItems::END,
        Align::Stretch => ts::AlignItems::STRETCH,
        Align::Baseline => ts::AlignItems::BASELINE,
    }
}

fn justify(j: Justify) -> ts::JustifyContent {
    match j {
        Justify::Start => ts::JustifyContent::START,
        Justify::Center => ts::JustifyContent::CENTER,
        Justify::End => ts::JustifyContent::END,
        Justify::Between => ts::JustifyContent::SPACE_BETWEEN,
        Justify::Around => ts::JustifyContent::SPACE_AROUND,
        Justify::Evenly => ts::JustifyContent::SPACE_EVENLY,
        Justify::Stretch => ts::JustifyContent::STRETCH,
    }
}

fn overflow(o: Overflow) -> ts::Overflow {
    match o {
        Overflow::Visible => ts::Overflow::Visible,
        // Hidden, not Clip: like CSS `overflow: hidden`, the box may shrink
        // below its content (a flex item's automatic minimum size is 0).
        Overflow::Hidden => ts::Overflow::Hidden,
        Overflow::Scroll => ts::Overflow::Scroll,
    }
}

impl Style {
    /// Whether this node clips what its children paint.
    #[must_use]
    pub fn clips(&self) -> bool {
        self.overflow_x != Overflow::Visible || self.overflow_y != Overflow::Visible
    }

    /// The layout half of this style, as the layout tree keeps it and Taffy
    /// reads it, in the node's inherited `direction`.
    #[must_use]
    pub fn box_style(&self, direction: Direction) -> crate::layout::BoxStyle {
        let mut margin = self.margin;
        let (start, end) = match direction {
            Direction::Ltr => (&mut margin.left, &mut margin.right),
            Direction::Rtl => (&mut margin.right, &mut margin.left),
        };
        if let Some(m) = self.margin_start {
            *start = m;
        }
        if let Some(m) = self.margin_end {
            *end = m;
        }
        use crate::layout::boxstyle::Len;
        let len = |l: Length| Len::from(l);
        crate::layout::BoxStyle {
            direction: match direction {
                Direction::Ltr => ts::Direction::Ltr,
                Direction::Rtl => ts::Direction::Rtl,
            },
            display: match self.display {
                Display::Block => ts::Display::Block,
                Display::Flex => ts::Display::Flex,
                Display::Grid => ts::Display::Grid,
                Display::None => ts::Display::None,
            },
            position: match self.position {
                Position::Relative => ts::Position::Relative,
                Position::Absolute => ts::Position::Absolute,
            },
            inset: self.inset.map(len),
            size: ts::Size {
                width: len(self.width),
                height: len(self.height),
            },
            min_size: ts::Size {
                width: len(self.min_width),
                height: len(self.min_height),
            },
            max_size: ts::Size {
                width: len(self.max_width),
                height: len(self.max_height),
            },
            aspect_ratio: self.aspect_ratio,
            margin: margin.map(len),
            padding: self.padding.map(|v| v),
            border: self.border_widths.map(|v| v),
            flex_direction: match self.flex_direction {
                FlexDirection::Row => ts::FlexDirection::Row,
                FlexDirection::Column => ts::FlexDirection::Column,
                FlexDirection::RowReverse => ts::FlexDirection::RowReverse,
                FlexDirection::ColumnReverse => ts::FlexDirection::ColumnReverse,
            },
            flex_wrap: if self.flex_wrap {
                ts::FlexWrap::Wrap
            } else {
                ts::FlexWrap::NoWrap
            },
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            flex_basis: len(self.flex_basis),
            align_items: self.align_items.map_or(ts::AlignItems::NORMAL, align),
            align_self: self.align_self.map(align),
            justify_items: self.justify_items.map_or(ts::AlignItems::NORMAL, align),
            justify_self: self.justify_self.map(align),
            justify_content: self.justify_content.map_or(ts::JustifyContent::NORMAL, justify),
            align_content: self.align_content.map_or(ts::AlignContent::NORMAL, justify),
            gap: ts::Size {
                width: self.column_gap,
                height: self.row_gap,
            },
            grid_template_columns: self.grid_columns.to_vec(),
            grid_template_rows: self.grid_rows.to_vec(),
            grid_auto_rows: self.grid_auto_rows.to_vec(),
            grid_auto_flow: if self.grid_flow_column {
                ts::GridAutoFlow::Column
            } else {
                ts::GridAutoFlow::Row
            },
            grid_column: self.grid_column.to_taffy(),
            grid_row: self.grid_row.to_taffy(),
            overflow: ts::Point {
                x: overflow(self.overflow_x),
                y: overflow(self.overflow_y),
            },
            reads_parent: ts::Size {
                width: false,
                height: false,
            },
        }
        .settled()
    }
}
