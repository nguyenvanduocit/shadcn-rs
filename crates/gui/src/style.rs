//! What a node looks like and how it is laid out: a subset of CSS's
//! properties, as one value per node. [`crate::styled::Styled`] is the
//! builder that writes it, one setter per property, each taking a value or
//! a closure ([`crate::value`]); [`Style::box_style`] hands the layout part
//! to the layout tree.

use std::rc::Rc;

use taffy as ts;

use crate::color::Color;
use crate::element::Side;
use crate::motion::MotionStyle;
use crate::placement::Placement;
use crate::value::{Convert, Inline};

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
    /// `calc(fraction × 100% + px)`.
    #[must_use]
    pub const fn calc(fraction: f32, px: f32) -> Self {
        Self::Calc { fraction, px }
    }

    /// Whether it reads the containing block's size.
    #[must_use]
    pub fn relative(self) -> bool {
        matches!(self, Self::Fraction(_) | Self::Calc { .. })
    }
}

/// A length in pixels.
#[must_use]
pub const fn px(v: f32) -> Length {
    Length::Px(v)
}

/// A percentage of the containing block (`pct(100.0)` is `100%`), kept as
/// the fraction `percent / 100`.
#[must_use]
pub const fn pct(percent: f32) -> Length {
    Length::Fraction(percent / 100.0)
}

impl Convert<Length> for f32 {
    /// Pixels.
    fn convert(self) -> Length {
        Length::Px(self)
    }
}

impl Convert<Edges<f32>> for f32 {
    /// The same on every side.
    fn convert(self) -> Edges<f32> {
        Edges::all(self)
    }
}

impl Convert<Edges<Length>> for f32 {
    /// The same pixels on every side.
    fn convert(self) -> Edges<Length> {
        Edges::all(Length::Px(self))
    }
}

impl Convert<Edges<Length>> for Length {
    /// The same on every side.
    fn convert(self) -> Edges<Length> {
        Edges::all(self)
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

/// Corner radii in pixels, clockwise from the top-left.
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

impl Convert<Corners> for f32 {
    /// The same radius at every corner.
    fn convert(self) -> Corners {
        Corners::all(self)
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

    /// The radii, none below zero.
    #[must_use]
    pub fn clamped(&self) -> Self {
        Self {
            top_left: self.top_left.max(0.0),
            top_right: self.top_right.max(0.0),
            bottom_right: self.bottom_right.max(0.0),
            bottom_left: self.bottom_left.max(0.0),
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
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
}

/// A node's box shadows, painted first to last, kept inline
/// ([`crate::value::Inline`]): a setter takes one [`Shadow`], an array of
/// them, or `Shadows::new()` for none.
pub type Shadows = Inline<Shadow, 2>;

/// Grid tracks, kept inline: a setter takes one [`Track`], an array, a
/// slice, a `Vec`, or `Tracks::repeat(n, track)`.
pub type Tracks = Inline<Track, 6>;

/// Filters applied in order, kept inline.
pub type Filters = Inline<Filter, 2>;

/// A focus ring: a solid band `offset` outside the node's shape,
/// following its corners. The gap an offset leaves is not painted.
/// [`Ring::new`] is Tailwind's `ring-*` (with `ring-offset-*`), a box
/// shadow's spread; [`Ring::outline`] is CSS `outline` with
/// `outline-offset`. The two differ only at corners smaller than the
/// distance out ([`RingCorners`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub width: f32,
    /// Pixels between the border box and the ring; negative draws it inside.
    pub offset: f32,
    pub color: Color,
    /// How the ring's corners grow from the shape's.
    pub corners: RingCorners,
}

/// How a ring's corners grow from the shape's, as Chrome draws each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RingCorners {
    /// A box shadow's spread ([`Corners::spread`]): Tailwind's `ring-*`.
    #[default]
    Spread,
    /// An outline's ([`Corners::outset`]): CSS `outline`.
    Outline,
}

impl Ring {
    /// Tailwind's ring of `width` in `color` against the border box (a
    /// box shadow's spread, its corners grown as [`Corners::spread`]).
    #[must_use]
    pub const fn new(width: f32, color: Color) -> Self {
        Self {
            width,
            offset: 0.0,
            color,
            corners: RingCorners::Spread,
        }
    }

    /// CSS `outline: <width> solid <color>` (its corners grown as
    /// [`Corners::outset`]).
    #[must_use]
    pub const fn outline(width: f32, color: Color) -> Self {
        Self {
            corners: RingCorners::Outline,
            ..Self::new(width, color)
        }
    }

    /// The same ring `offset` pixels out from the border box.
    #[must_use]
    pub const fn offset(self, offset: f32) -> Self {
        Self { offset, ..self }
    }

    /// The radii of the ring's edge `d` out from a shape of radii `r`.
    #[must_use]
    pub fn radii(&self, r: Corners, d: f32) -> Corners {
        match self.corners {
            RingCorners::Spread => r.spread(d),
            RingCorners::Outline => r.outset(d),
        }
    }
}

/// The colour a gradient stop paints: a fixed one, or the content colour
/// (`currentColor`, the [`ContentStyle::color`] provided to the node)
/// through CSS relative-colour syntax, resolved where the node is painted,
/// as CSS resolves `currentColor` at used-value time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StopColor {
    Color(Color),
    /// `oklch(from currentColor max(floor, l + lift) c h / calc(alpha * alpha_scale + alpha_add))`.
    Current {
        lightness_floor: f32,
        lightness_lift: f32,
        alpha_scale: f32,
        alpha_add: f32,
    },
}

impl StopColor {
    /// `currentColor` itself.
    pub const CURRENT: Self = Self::Current {
        lightness_floor: 0.0,
        lightness_lift: 0.0,
        alpha_scale: 1.0,
        alpha_add: 0.0,
    };

    /// The colour this stop paints in a node whose text colour is `current`.
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

impl Convert<Calc> for f32 {
    /// Pixels.
    fn convert(self) -> Calc {
        Calc::px(self)
    }
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
    pub color: StopColor,
    pub at: Calc,
}

/// A gradient's colour stops, kept inline ([`crate::value::Inline`]), so
/// a style holding a gradient clones without allocating: `[a, b].into()`.
pub type Stops = Inline<Stop, 4>;

/// CSS `linear-gradient(<angle>, <stops>)`, interpolated in OKLab.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    /// CSS degrees: 0 points up, 90 right, 180 down.
    pub angle: f32,
    pub stops: Stops,
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

impl From<LinearGradient> for BackgroundImage {
    /// `gradient` over the whole box (`background-size: 100% 100%`), at its
    /// top-left, through the border box.
    fn from(gradient: LinearGradient) -> Self {
        Self {
            gradient,
            size: [Calc::fraction(1.0); 2],
            position: [Calc::default(); 2],
            clip: BackgroundClip::BorderBox,
        }
    }
}

impl Convert<Option<BackgroundImage>> for LinearGradient {
    fn convert(self) -> Option<BackgroundImage> {
        Some(self.into())
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

/// A scroll container's edges fade where more content
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
            color: StopColor::Color(color),
            at,
        };
        Some(LinearGradient {
            angle: match self.axis {
                Axis::Horizontal => 90.0,
                Axis::Vertical => 180.0,
            },
            stops: [
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
            ]
            .into(),
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

impl Convert<Option<MaskImage>> for LinearGradient {
    fn convert(self) -> Option<MaskImage> {
        Some(MaskImage::Linear(self))
    }
}

impl Convert<Option<MaskImage>> for ScrollFade {
    fn convert(self) -> Option<MaskImage> {
        Some(MaskImage::ScrollFade(self))
    }
}

/// Paint beyond a solid box: a gradient background, a mask and filters over
/// the node's painting.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Effects {
    pub background_image: Option<BackgroundImage>,
    pub mask: Option<MaskImage>,
    pub filters: Filters,
}

/// The generic family a [`FontFamily`] ends in, as CSS names them; what
/// none of its named fonts has falls back to it, then per script.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenericFamily {
    SansSerif,
    Serif,
    Monospace,
    SystemUi,
}

/// A font stack, CSS `font-family`: fonts by name, first to last, then a
/// generic family. `gui` bundles Inter and Geist Mono ([`Self::INTER`],
/// [`Self::GEIST_MONO`]); which of them is a design system's "sans" or
/// "mono" is the design system's to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFamily {
    pub names: &'static [&'static str],
    pub generic: GenericFamily,
}

impl FontFamily {
    /// The bundled Inter, then `sans-serif` (CSS `"Inter", sans-serif`).
    /// The root's family.
    pub const INTER: Self = Self {
        names: &[crate::text::SANS],
        generic: GenericFamily::SansSerif,
    };
    /// The bundled Geist Mono, then `monospace` (CSS `"Geist Mono", monospace`).
    pub const GEIST_MONO: Self = Self {
        names: &[crate::text::MONO],
        generic: GenericFamily::Monospace,
    };

    /// `names`, then `generic`.
    #[must_use]
    pub const fn new(names: &'static [&'static str], generic: GenericFamily) -> Self {
        Self { names, generic }
    }
}

impl Default for FontFamily {
    fn default() -> Self {
        Self::INTER
    }
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
/// text, flex rows, grid columns and horizontal overflow run. A node
/// that sets none takes its parent's: the one property that flows to every
/// node (ADR-0004, ADR-0022).
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

impl Convert<LineHeight> for f32 {
    /// Pixels.
    fn convert(self) -> LineHeight {
        LineHeight::Px(self)
    }
}

/// The type of text a container provides (ADR-0022's `ContentStyle::text`):
/// each field it sets is its subtree's, `None` leaves the enclosing
/// provider's. Only paragraphs and text fields read it.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct TextStyle {
    pub size: Option<f32>,
    pub line_height: Option<LineHeight>,
    pub weight: Option<u16>,
    pub family: Option<FontFamily>,
    pub align: Option<TextAlign>,
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
    /// Digits of one width (`tabular-nums`, OpenType `tnum`). ADR-0005's
    /// inherited property is this field of the provided text style.
    pub tabular_nums: Option<bool>,
    /// Show at most this many lines, the last ending in an ellipsis.
    pub line_clamp: Option<u16>,
    /// How wrapped lines are chosen (`text-wrap`).
    pub wrap_style: Option<WrapStyle>,
}

/// What a container provides to the content of its subtree (ADR-0022,
/// research §7.10): the colour, the type of text and the size of icons,
/// as Compose's `ProvideContentColorTextStyle` and Flutter's
/// `DefaultTextStyle` with `IconTheme` do. It is not CSS inheritance:
/// - only a paragraph, a text field and an icon read it, and only for what
///   they do not set themselves; a span reads its paragraph's;
/// - a box reads nothing of it but its colour, as `currentColor`
///   ([`StopColor::Current`], a scroll container's thumb), and its type
///   for the `ch` unit;
/// - nothing else flows down the tree but [`Direction`], which layout
///   reads at every node.
///
/// A node's [`Style::content`] is what it provides: each field it sets
/// replaces the enclosing provider's for its subtree, and `None` leaves
/// it. The colour is read where the reader paints and the type where it
/// lays out: a container whose colour changes on hover repaints its text
/// and icons and lays out nothing.
///
/// Research §7.10 drew it as fields that are always set, a provider
/// writing `ContentStyle { color, ..content() }` with `content()` read in
/// render. Here the provider sets only what it changes and the retained
/// tree merges it field by field, where the node resolves; the reasons
/// are in §7.10's "Gộp theo field" note:
/// - an element is a value built apart from the tree it goes into (a
///   child often before its container), so in render there is no
///   enclosing provider to read;
/// - a read in render would subscribe the render to the enclosing
///   provider, so a hover colour or a theme switch above would render the
///   instance again instead of repainting its text;
/// - the merge is the references' own: Compose's `ProvideTextStyle` and
///   `ProvideContentColorTextStyle` provide `LocalTextStyle.current.merge`,
///   Flutter's `DefaultTextStyle.merge` and `IconTheme.merge` take the
///   enclosing value for each field they leave null.
///
/// What keeps it from being CSS's `inherit` is that one typed struct is
/// provided by a container that says so, and read only by text, icons and
/// `currentColor`; no other property flows down. A colour that is "the
/// enclosing content colour" (`Ink::Content` in `shadcn`) is a provider
/// leaving [`ContentStyle::color`] `None`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ContentStyle {
    /// The colour of text and icons, and CSS's `currentColor`.
    pub color: Option<Color>,
    pub text: TextStyle,
    /// The width and height of an icon that sets no size of its own.
    pub icon_size: Option<f32>,
}

/// The content a node holds or passes on, every provider above it applied
/// ([`ContentStyle`]), with the node's [`Direction`]: what a paragraph,
/// an icon or a text field is drawn with.
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
}

impl ResolvedText {
    /// The root's content: 16px/24px normal weight.
    #[must_use]
    pub fn root(color: Color) -> Self {
        Self {
            color,
            size: 16.0,
            line_height: 24.0,
            weight: 400,
            family: FontFamily::INTER,
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
            // Lucide's own 24-unit grid drawn at 16px.
            icon_size: 16.0,
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

    /// This content with what `own` provides in place of its fields; the
    /// direction stays (it is not content).
    #[must_use]
    pub fn provide(&self, own: &ContentStyle) -> Self {
        let t = &own.text;
        let size = t.size.unwrap_or(self.size);
        // A new font size without a new line height keeps the enclosing ratio.
        let line_height = match t.line_height {
            Some(LineHeight::Px(px)) => px,
            Some(LineHeight::Relative(r)) => r * size,
            None if t.size.is_some() => size * (self.line_height / self.size.max(1.0)),
            None => self.line_height,
        };
        Self {
            color: own.color.unwrap_or(self.color),
            size,
            line_height,
            weight: t.weight.unwrap_or(self.weight),
            family: t.family.unwrap_or(self.family),
            align: t.align.unwrap_or(self.align),
            direction: self.direction,
            nowrap: t.nowrap.unwrap_or(self.nowrap),
            truncate: t.truncate.unwrap_or(self.truncate),
            italic: t.italic.unwrap_or(self.italic),
            underline: t.underline.unwrap_or(self.underline),
            underline_offset: t.underline_offset.or(self.underline_offset),
            line_through: t.line_through.unwrap_or(self.line_through),
            tracking: t.tracking.unwrap_or(self.tracking),
            tabular_nums: t.tabular_nums.unwrap_or(self.tabular_nums),
            line_clamp: t.line_clamp.or(self.line_clamp),
            wrap_style: t.wrap_style.unwrap_or(self.wrap_style),
            icon_size: own.icon_size.unwrap_or(self.icon_size),
        }
    }

    /// What of it shapes and lays out text: the fields only paint reads
    /// (colour, underline, line-through) taken out. Two contents of one
    /// shape lay out alike.
    #[must_use]
    pub fn shape(self) -> Self {
        Self {
            color: Color::TRANSPARENT,
            underline: false,
            underline_offset: None,
            line_through: false,
            ..self
        }
    }

    /// The fields only paint reads, from `paint`; the rest from `self`.
    #[must_use]
    pub fn painted_as(self, paint: &Self) -> Self {
        Self {
            color: paint.color,
            underline: paint.underline,
            underline_offset: paint.underline_offset,
            line_through: paint.line_through,
            ..self
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

impl Transform {
    /// How far the node's box moves, given its size and how its layer was placed.
    #[must_use]
    pub(crate) fn offset(&self, size: (f32, f32), placed: Option<Placement>) -> (f32, f32) {
        let (mut x, mut y) = (
            self.translate[0].resolve(size.0, 0.0),
            self.translate[1].resolve(size.1, 0.0),
        );
        let d = self.toward_anchor;
        match placed.map(|p| p.side) {
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
    pub(crate) fn painted(&self, bounds: crate::geometry::Bounds, placed: Option<Placement>) -> Painted {
        let origin = match (self.origin, placed) {
            (Origin::At([x, y]), _) => (x.resolve(bounds.width, 0.0), y.resolve(bounds.height, 0.0)),
            (Origin::Anchor, Some(p)) => p.facing(bounds.width, bounds.height),
            (Origin::Anchor, None) => (bounds.width * 0.5, bounds.height * 0.5),
        };
        Painted {
            scale: self.scale,
            rotate: self.rotate,
            origin,
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

/// How a flex item grows and shrinks, and from what size (CSS `flex`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flex {
    pub grow: f32,
    pub shrink: f32,
    pub basis: Length,
}

impl Flex {
    /// CSS `flex: auto`: `1 1 auto`.
    pub const AUTO: Self = Self {
        grow: 1.0,
        shrink: 1.0,
        basis: Length::Auto,
    };
    /// CSS `flex: none`: `0 0 auto`.
    pub const NONE: Self = Self {
        grow: 0.0,
        shrink: 0.0,
        basis: Length::Auto,
    };

    /// CSS `flex: <grow>`: `grow 1 0%` (`flex: 1` is `Flex::grow(1.0)`).
    #[must_use]
    pub const fn grow(grow: f32) -> Self {
        Self {
            grow,
            shrink: 1.0,
            basis: Length::Fraction(0.0),
        }
    }
}

/// Which way a grid places items that name no line (`grid-auto-flow`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GridAutoFlow {
    #[default]
    Row,
    Column,
}

/// Everything about one node's box: layout, paint and the content it provides.
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
    pub grid_columns: Tracks,
    pub grid_rows: Tracks,
    pub grid_auto_rows: Tracks,
    pub grid_auto_flow: GridAutoFlow,
    pub grid_column: GridSpan,
    pub grid_row: GridSpan,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,

    pub background: Option<Color>,
    pub border_color: Option<Color>,
    /// The outline everything of the node follows: background, border,
    /// ring, shadows, clip, hit testing.
    pub shape: crate::shape::Shape,
    pub shadows: Shadows,
    pub ring: Option<Ring>,
    /// Decoration beyond the box: an edge bar, an inset layer, a border
    /// over the content, a blurred backdrop.
    pub decoration: crate::shape::Decoration,
    /// How far past its border box the node takes the pointer, per side
    /// (negative: less of it).
    pub hit_slop: Edges<f32>,
    /// Multiplies the alpha of everything this node and its children paint.
    pub opacity: f32,
    pub cursor: Option<Cursor>,
    /// Where this node's painting moves, scales and turns (`translate`, `scale`, `rotate`).
    pub transform: Transform,
    /// Hidden but still laid out (`invisible`).
    pub invisible: bool,
    pub effects: Effects,
    /// How it moves: transitions and keyframe animations. Shared, as few
    /// nodes move and every node carries a style: cloning a style to
    /// resolve its bindings copies a pointer.
    pub motion: Option<Rc<MotionStyle>>,
    /// A scroll container shows no scrollbar (`scrollbar-width: none`).
    pub scrollbar_hidden: bool,
    /// An icon's line width in its 24-unit grid (`stroke-width`; lucide's
    /// default is 2). Only an icon reads it.
    pub stroke_width: f32,
    /// A text input's placeholder colour (`::placeholder { color }`);
    /// `None` takes the text colour at half alpha. Only an input reads it.
    pub placeholder_color: Option<Color>,
    /// What a text input paints behind its selected text
    /// (`::selection { background }`); `None` takes the platform's. Only
    /// an input reads it.
    pub selection_color: Option<Color>,
    /// The inline direction of this node and what it holds (`dir`); `None`
    /// takes its parent's.
    pub direction: Option<Direction>,
    /// What it provides to the text and icons of its subtree.
    pub content: ContentStyle,
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
            grid_columns: Tracks::new(),
            grid_rows: Tracks::new(),
            grid_auto_rows: Tracks::new(),
            grid_auto_flow: GridAutoFlow::Row,
            grid_column: GridSpan::default(),
            grid_row: GridSpan::default(),
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            background: None,
            border_color: None,
            shape: crate::shape::Shape::Rect,
            shadows: Shadows::new(),
            ring: None,
            decoration: crate::shape::Decoration::default(),
            hit_slop: Edges::all(0.0),
            opacity: 1.0,
            cursor: None,
            transform: Transform::default(),
            invisible: false,
            effects: Effects::default(),
            motion: None,
            scrollbar_hidden: false,
            stroke_width: 2.0,
            placeholder_color: None,
            selection_color: None,
            direction: None,
            content: ContentStyle::default(),
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
            grid_template_columns: self.grid_columns.clone(),
            grid_template_rows: self.grid_rows.clone(),
            grid_auto_rows: self.grid_auto_rows.clone(),
            grid_auto_flow: match self.grid_auto_flow {
                GridAutoFlow::Row => ts::GridAutoFlow::Row,
                GridAutoFlow::Column => ts::GridAutoFlow::Column,
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
