//! Text (ADR-0019): shaped and broken into lines by Parley, fonts found and
//! fallen back between by fontique, glyphs rasterized by `gui` itself
//! ([`fonts`], [`raster`]) with Chrome's parameters for the OS ([`gamma`],
//! [`settings`]). Inter (sans, variable in `opsz` and `wght`) and Geist Mono
//! are embedded so text looks the same on every machine.
//!
//! A [`Paragraph`] is one string laid out as a whole; its [`Run`]s give byte
//! ranges of it a style of their own (CSS inline elements: a link, a `<code>`,
//! a highlighted token), shaped together so they wrap as one paragraph
//! (ADR-0001). The text is split at hard line breaks into pieces, each
//! shaped once and cached by its content and resolved styles, never by the
//! width it is broken at: a new width only breaks lines again, and a
//! keystroke in a long text reshapes the one piece it changed.

mod access;
mod color_glyph;
mod fonts;
pub mod gamma;
mod ink;
mod raster;
pub mod settings;

use foldhash::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use parley::{
    Affinity, Alignment, AlignmentOptions, BaseDirection, Cursor as ParleyCursor, FontFamily as ParleyFamily,
    FontFeature, FontStyle, FontVariation, FontWeight, InlineBox, InlineBoxKind, Layout, LineHeight, OverflowWrap,
    PositionedLayoutItem, Selection, StyleProperty, VerticalAlign,
};

pub use self::access::TextRun;
pub(crate) use self::fonts::CURVE_EM;
pub use self::fonts::{Coverage, GlyphContent, GlyphImage, GlyphKey, Glyphs};
use self::fonts::{Fonts, InkKey};
pub(crate) use self::fonts::{MONO, SANS};
use self::gamma::{GammaParams, MaskGamma};
use self::settings::TextSettings;
use crate::color::Color;
use crate::convert::{f32_to_i32, f32_to_u32, i32_to_f32};
use crate::geometry::Bounds;
use crate::scene::Item;
use crate::style::{
    ContentStyle, Direction, Edges, FontFamily, GenericFamily, ResolvedText, TextAlign, TextDecoration,
    TextDecorations, WrapStyle,
};

/// Subdivisions of a pixel lengths are kept in (Blink's `LayoutUnit`).
const LAYOUT_UNITS: f32 = 64.0;

/// What CSS draws where text overflows its line (`text-overflow: ellipsis`).
const ELLIPSIS: &str = "\u{2026}";

/// What is shaped to read a font's metrics.
const PROBE: &str = "x";

/// U+FFFC OBJECT REPLACEMENT CHARACTER: the text an inline box stands on.
pub const OBJECT: char = '\u{FFFC}';

/// Bytes of a paragraph in an inline style of their own (`<a>`, `<code>`,
/// `<strong>`, a Shiki token).
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The bytes of the paragraph's content this run covers.
    pub range: Range<usize>,
    /// The paragraph's text style with the run's own applied on top
    /// ([`Run::style_in`]). Its line height is not used: a run takes the
    /// paragraph's lines. Its `underline` and `line_through` are only what
    /// the run itself declares: the paragraph's are drawn under every run
    /// anyway (CSS's propagation, which a run cannot undo).
    pub text: ResolvedText,
    /// Painted behind the run's glyphs, over the font's height.
    pub background: Option<Color>,
    /// Corner radius of that background.
    pub radius: f32,
    /// Left and right add space in the line before and after the run; top
    /// and bottom only grow its background, as CSS pads an inline box.
    pub padding: Edges<f32>,
}

impl Run {
    /// The style of a run whose own content style is `own` in a paragraph
    /// styled `paragraph`: the paragraph's with what `own` sets in place,
    /// but with only the decorations `own` declares. CSS draws a
    /// decoration for the box that declares it, over everything inside
    /// it, by that box's font and colour (Blink's decorating box):
    /// [`TextSystem::draw`] draws the paragraph's under every run, so a
    /// run's `underline(false)` leaves it, and a run that declares one
    /// draws its own besides, by its own font, over itself.
    #[must_use]
    pub fn style_in(paragraph: &ResolvedText, own: &ContentStyle) -> ResolvedText {
        // Provided to it without the paragraph's boxes, its chain is its
        // own box alone: found while the run holds it, never the
        // paragraph's chain with its box on top, made and let go.
        paragraph.clone().undecorated().provide(own)
    }
}

/// Where an inline box ([`InlineSlot`]) sits, relative to the text box,
/// and whether it shows: a line clamp or an ellipsis can hide it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedBox {
    pub id: u64,
    pub bounds: Bounds,
    pub shown: bool,
}

/// A box laid out in a line as a glyph is (an icon, a chip, a button): it
/// takes its width in the line, wraps with the words around it, and sits
/// on the baseline by its own (CSS `vertical-align: baseline` of an atomic
/// inline), the line growing to hold it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineSlot {
    /// The byte offset in the content of the [`OBJECT`] character it
    /// stands on (one box to each); a box anywhere else is not placed.
    pub at: usize,
    pub width: f32,
    pub height: f32,
    /// Its baseline below its top; none puts its bottom edge on the line's
    /// baseline (an `inline-block` with no line box in it).
    pub baseline: Option<f32>,
    /// Names it in [`TextSystem::placed_inline_boxes`].
    pub id: u64,
}

/// `content` as a reader or a copy has it: without the [`OBJECT`]s its
/// atomic inlines stand on (each is a node of its own).
#[must_use]
pub fn readable(content: &str) -> std::borrow::Cow<'_, str> {
    if content.contains(OBJECT) {
        std::borrow::Cow::Owned(content.replace(OBJECT, ""))
    } else {
        std::borrow::Cow::Borrowed(content)
    }
}

/// What text shapes and draws: `content`, with `runs` (sorted, not
/// overlapping, on char boundaries) styled on their own, and `boxes` (sorted
/// by offset) placed in its lines; the rest takes the paragraph's style.
#[derive(Clone, Copy, Debug)]
pub struct Paragraph<'a> {
    pub content: &'a str,
    pub runs: &'a [Run],
    pub boxes: &'a [InlineSlot],
}

impl<'a> Paragraph<'a> {
    #[must_use]
    pub fn new(content: &'a str, runs: &'a [Run]) -> Self {
        Self {
            content,
            runs,
            boxes: &[],
        }
    }
}

impl<'a> From<&'a str> for Paragraph<'a> {
    fn from(content: &'a str) -> Self {
        Self::new(content, &[])
    }
}

impl<'a> From<&'a String> for Paragraph<'a> {
    fn from(content: &'a String) -> Self {
        Self::from(content.as_str())
    }
}

/// Where a caret sits: its top-left and height, relative to the text box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub x: f32,
    pub y: f32,
    pub height: f32,
}

/// A move of a caret through text, as the keys of a text field make it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// One grapheme cluster left or right on screen (the arrow keys), across
    /// a change of direction as the eye reads it.
    Left,
    Right,
    /// To the start of the word left of the caret, or past the end of the
    /// word right of it (Alt or Ctrl with an arrow).
    WordLeft,
    WordRight,
    /// To the start or end of the caret's line as it wraps (Home, End,
    /// Cmd with an arrow on macOS).
    LineStart,
    LineEnd,
    /// A line up or down, keeping the caret's horizontal position.
    Up,
    Down,
    /// Where Backspace deletes back to: the character before the caret, or
    /// the whole cluster for an emoji or a line break (as `PlainEditor`).
    DeleteBack,
    /// Where Delete deletes to: the whole cluster after the caret.
    DeleteForward,
    /// The start of the word before the caret (Alt/Ctrl + Backspace).
    DeleteWordBack,
    /// The end of the word after the caret (Alt/Ctrl + Delete).
    DeleteWordForward,
}

/// What of a text style changes how it is shaped (not its colour, its
/// decorations, its alignment or how it wraps).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StyleKey {
    size: u32,
    line_height: u32,
    weight: u16,
    family: FontFamily,
    italic: bool,
    tracking: u32,
    tabular_nums: bool,
}

impl StyleKey {
    fn of(t: &ResolvedText, line_height: f32) -> Self {
        Self {
            size: t.size.to_bits(),
            line_height: line_height.to_bits(),
            weight: t.weight,
            family: t.family,
            italic: t.italic,
            tracking: t.tracking.to_bits(),
            tabular_nums: t.tabular_nums,
        }
    }
}

/// A run within one piece of a paragraph: its bytes from the piece's start,
/// its style, its padding, and its index in the paragraph's runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PieceRun {
    range: (usize, usize),
    style: StyleKey,
    padding: (u32, u32),
    /// Pads its start (left) or end (right) here: the run starts or ends in this piece.
    starts: bool,
    ends: bool,
    index: u32,
}

/// Everything but the content a piece is shaped from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShapeKey {
    base: StyleKey,
    runs: Vec<PieceRun>,
    /// Each inline box: its offset, width, height, baseline (`u32::MAX`
    /// for none, which no finite baseline's bits are) and id.
    boxes: Vec<(usize, u32, u32, u32, u64)>,
    /// Bytes put before the text: a U+FFFC for a leading run's padding.
    lead: usize,
    rtl: bool,
    /// Shaped apart from the same text in a paragraph ([`Apart`]).
    apart: Apart,
}

/// Why a piece is shaped apart from the same text in its paragraph, to be
/// broken otherwise: each its own cache entry, so none breaks another's
/// lines again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Apart {
    /// Not: the paragraph's own piece.
    No,
    /// A clamped line, broken where its ellipsis leaves room, to find its
    /// last soft wrap opportunity that does (css-overflow-4 block-ellipsis).
    /// One text clamped at a few widths keeps each breaking ([`Broken`]).
    Probe,
    /// What a clamped line keeps, laid out as a line of its own (its
    /// bidi resolved, its runs' padding added, as if it were all there is).
    Kept,
}

/// The most pieces shaped apart ([`Apart`]) a [`TextSystem`] keeps.
const APART_MAX: usize = 512;

/// The most texts whose hidden lines a [`TextSystem`] keeps cut
/// ([`TextSystem::bound_hidden`]), unless its working set reads more.
const HIDDEN_MAX: usize = 16;

/// How a piece's lines are broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Breaking {
    /// The width lines break at; none for one unbroken line per piece.
    width: Option<u32>,
    nowrap: bool,
    wrap_style: WrapStyle,
    align: TextAlign,
    /// The most lines broken (a clamp's, less those before the piece):
    /// what follows them is never broken, however many lines it would
    /// wrap into ([`TextSystem::lay_out_into`]). None: all of them.
    lines: Option<u32>,
}

impl Breaking {
    fn of(t: &ResolvedText, width: Option<f32>) -> Self {
        Self {
            width: width.map(f32::to_bits),
            nowrap: t.nowrap || t.truncate,
            wrap_style: t.wrap_style,
            align: t.physical_align(),
            lines: None,
        }
    }

    fn width(&self) -> Option<f32> {
        self.width.map(f32::from_bits)
    }

    /// One unbroken line, as an ellipsis is laid out whatever the
    /// paragraph it ends breaks by: one breaking for every paragraph, so
    /// paragraphs that break differently never break it again.
    const ONE_LINE: Self = Self {
        width: None,
        nowrap: true,
        wrap_style: WrapStyle::Auto,
        align: TextAlign::Left,
        lines: None,
    };
}

/// One shaped piece, kept across frames.
struct Shaped {
    content: String,
    key: ShapeKey,
    layout: Layout<u32>,
    /// How its lines are broken now, and how far right they then sit.
    broken: Option<Breaking>,
    /// Its lines' boxes as so broken, by Blink's rules ([`rows`]).
    rows: Vec<Row>,
    shift: f32,
    /// Where `pretty` and `balance` settled its lines, by how they were asked.
    settled: Vec<(Breaking, Fit)>,
    /// Each line's size and baseline under the last few breakings, so
    /// layout's probes at alternating widths measure without breaking
    /// the lines again.
    lines: Vec<(Breaking, Vec<LineSize>)>,
    /// Its min-content width.
    min: Option<f32>,
    used: u64,
    /// Where each of its inline boxes stands in its text, by id: a box is
    /// found at once, however many there are ([`box_range`]).
    boxes: HashMap<u64, usize>,
    /// Its lines as broken the last time or two before ([`Broken`]): a
    /// text drawn at a few widths (labels of one text in boxes of their
    /// own) swaps between them rather than breaking again.
    spare: Vec<Broken>,
}

/// A piece's lines as broken once, kept aside: its layout, rows and shift.
struct Broken {
    how: Breaking,
    layout: Layout<u32>,
    rows: Vec<Row>,
    shift: f32,
}

impl Broken {
    /// Swap with what `s` has now, broken as `s.broken` says.
    fn swap(&mut self, s: &mut Shaped) {
        std::mem::swap(&mut self.layout, &mut s.layout);
        std::mem::swap(&mut self.rows, &mut s.rows);
        std::mem::swap(&mut self.shift, &mut s.shift);
        let how = s.broken.replace(self.how);
        if let Some(how) = how {
            self.how = how;
        }
    }
}

/// How many breakings a piece keeps aside besides its own ([`Broken`]).
const SPARE: usize = 2;

/// One line of a piece as layout measures it: its width, its height, and
/// its baseline below its top.
#[derive(Clone, Copy, Debug)]
struct LineSize {
    width: f32,
    height: f32,
    baseline: f32,
}

/// The first and last of a paragraph's baselines seen so far.
#[derive(Clone, Copy, Debug, Default)]
struct Lines {
    first: Option<f32>,
    last: Option<f32>,
}

impl Lines {
    fn add(&mut self, baseline: f32) {
        self.first.get_or_insert(baseline);
        self.last = Some(baseline);
    }
}

/// What a paragraph takes ([`TextSystem::metrics`]): its width and height,
/// and its first and last baselines below its top; none when it has no
/// line box (no text and no inline box).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    pub first_baseline: Option<f32>,
    pub last_baseline: Option<f32>,
}

impl TextMetrics {
    /// Its widest line (rounded up to a LayoutUnit), its lines' height (at
    /// least one line's), and its baselines.
    fn of(widest: f32, height: f32, t: &ResolvedText, baselines: Lines) -> Self {
        Self {
            width: (widest * LAYOUT_UNITS).ceil() / LAYOUT_UNITS,
            height: height.max(t.line_height),
            first_baseline: baselines.first,
            last_baseline: baselines.last,
        }
    }
}

/// A piece of a paragraph: bytes `start..end` of its content, its cache entry.
#[derive(Clone, Copy, Debug)]
struct Piece {
    start: usize,
    end: usize,
    hash: u64,
}

/// Fonts, shaped text and how glyphs are rasterized, kept across frames.
pub struct TextSystem {
    fonts: Fonts,
    shaped: HashMap<u64, Shaped>,
    frame: u64,
    /// Pieces shaped since last asked ([`TextSystem::take_shaped`]).
    shapes: u64,
    settings: TextSettings,
    gamma: GammaParams,
    table: std::sync::Arc<MaskGamma>,
    /// Bumped whenever the settings change how glyphs rasterize.
    generation: u64,
    /// The generation the renderer's [`Glyphs`] was last sent.
    sent_generation: Option<u64>,
    /// The pieces drawn since last taken ([`TextSystem::take_drawn`]).
    drawn: Scratch<u64>,
    /// Scratch: the runs and inline boxes of the key a piece is looked up by.
    key_runs: Vec<PieceRun>,
    key_boxes: Vec<(usize, u32, u32, u32, u64)>,
    /// Scratch [`TextSystem::draw`] works in.
    scratch: DrawScratch,
    /// Pieces kept shaped while a retained paint chunk shows them (ADR-0016),
    /// however long since a frame drew them; by how many chunks.
    pinned: HashMap<u64, u32>,
    /// Glyph origins round to whole pixels at 1x (ADR-0020: Chrome on
    /// Linux), the ellipsis's as the text's.
    whole_origins: bool,
    /// Scratch: a cut line's units ([`Units`]).
    units: Units,
    /// The padding of runs the last cut dropped that kept units hold, by
    /// the byte where each holding unit ends, sorted ([`trim_of`]): as
    /// many as there are, in room kept from cut to cut.
    trims: Vec<(usize, f32)>,
    /// Where clamps keep their last lines to, by line, room and end
    /// ([`TextSystem::kept_fit`]), and the frame each was last asked.
    fits: HashMap<u64, (usize, u64)>,
    /// Scratch: a clamped line's word boundaries, and the prefixes a clamp
    /// may keep of it ([`TextSystem::kept_fit`]).
    ends: Vec<usize>,
    prefixes: Vec<Prefix>,
    /// Lines broken, ever: a clamp breaks only those it shows.
    lines_broken: u64,
    /// The lines clamps hide, never laid out, as a screen reader reads
    /// them ([`TextSystem::text_runs`]), by text: cut into characters and
    /// words once per text, whatever width it is laid out at; as many
    /// texts as the working set reads ([`TextSystem::bound_hidden`]).
    hidden_lines: HashMap<u64, access::HiddenLines>,
    /// Hidden lines cut into characters, ever.
    hidden_segmented: u64,
    /// Scratch: hidden lines' texts by which go first ([`TextSystem::bound_hidden`]).
    hidden_order: Vec<(u64, u64)>,
    /// Scratch: pieces shaped apart by which go first ([`TextSystem::bound_apart`]).
    apart_order: Vec<(u64, u64)>,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

/// A text system's caches, counts and scratch, set aside while a cold
/// frame lays text out with fresh ones ([`TextSystem::enter_cold`]): it
/// reads no cache and warms none for the live frames. The faces and their
/// ids are shared: append-only, so sharing them changes no result.
#[cfg(debug_assertions)]
pub(crate) struct LiveCaches {
    shaped: HashMap<u64, Shaped>,
    shapes: u64,
    drawn: Scratch<u64>,
    key_runs: Vec<PieceRun>,
    key_boxes: Vec<(usize, u32, u32, u32, u64)>,
    scratch: DrawScratch,
    pinned: HashMap<u64, u32>,
    units: Units,
    trims: Vec<(usize, f32)>,
    fits: HashMap<u64, (usize, u64)>,
    ends: Vec<usize>,
    prefixes: Vec<Prefix>,
    lines_broken: u64,
    hidden_lines: HashMap<u64, access::HiddenLines>,
    hidden_segmented: u64,
    hidden_order: Vec<(u64, u64)>,
    apart_order: Vec<(u64, u64)>,
    ink: fonts::InkCache,
}

#[cfg(debug_assertions)]
impl TextSystem {
    /// Lay text out from fresh caches until [`Self::exit_cold`]: the
    /// live ones, set aside.
    pub(crate) fn enter_cold(&mut self) -> LiveCaches {
        LiveCaches {
            shaped: std::mem::take(&mut self.shaped),
            shapes: std::mem::take(&mut self.shapes),
            drawn: std::mem::take(&mut self.drawn),
            key_runs: std::mem::take(&mut self.key_runs),
            key_boxes: std::mem::take(&mut self.key_boxes),
            scratch: std::mem::take(&mut self.scratch),
            pinned: std::mem::take(&mut self.pinned),
            units: std::mem::take(&mut self.units),
            trims: std::mem::take(&mut self.trims),
            fits: std::mem::take(&mut self.fits),
            ends: std::mem::take(&mut self.ends),
            prefixes: std::mem::take(&mut self.prefixes),
            lines_broken: std::mem::take(&mut self.lines_broken),
            hidden_lines: std::mem::take(&mut self.hidden_lines),
            hidden_segmented: std::mem::take(&mut self.hidden_segmented),
            hidden_order: std::mem::take(&mut self.hidden_order),
            apart_order: std::mem::take(&mut self.apart_order),
            ink: self.fonts.swap_ink(fonts::InkCache::default()),
        }
    }

    /// The live caches back, the cold frame's dropped.
    pub(crate) fn exit_cold(&mut self, live: LiveCaches) {
        let LiveCaches {
            shaped,
            shapes,
            drawn,
            key_runs,
            key_boxes,
            scratch,
            pinned,
            units,
            trims,
            fits,
            ends,
            prefixes,
            lines_broken,
            hidden_lines,
            hidden_segmented,
            hidden_order,
            apart_order,
            ink,
        } = live;
        self.shaped = shaped;
        self.shapes = shapes;
        self.drawn = drawn;
        self.key_runs = key_runs;
        self.key_boxes = key_boxes;
        self.scratch = scratch;
        self.pinned = pinned;
        self.units = units;
        self.trims = trims;
        self.fits = fits;
        self.ends = ends;
        self.prefixes = prefixes;
        self.lines_broken = lines_broken;
        self.hidden_lines = hidden_lines;
        self.hidden_segmented = hidden_segmented;
        self.hidden_order = hidden_order;
        self.apart_order = apart_order;
        drop(self.fonts.swap_ink(ink));
    }
}

/// The gamma parameters Chrome uses on this OS, with the user's own where
/// the OS has them (Windows' ClearType tuner).
pub(crate) fn gamma_for(settings: &TextSettings) -> GammaParams {
    let mut params = if cfg!(target_os = "macos") {
        GammaParams::macos()
    } else if cfg!(windows) {
        GammaParams::windows()
    } else {
        GammaParams::linux()
    };
    if let Some(c) = settings.contrast {
        params.contrast = c;
    }
    if let Some(g) = settings.gamma {
        params.gamma = gamma::Gamma::Power(g);
    }
    params
}

/// Where the piece of `content` starting at `start` ends: its next `\n`, or
/// the content's end.
fn piece_end(content: &str, start: usize) -> usize {
    content
        .get(start..)
        .and_then(|s| s.find('\n'))
        .map_or(content.len(), |i| start.saturating_add(i))
}

/// How many lines a paragraph in style `t` shows: one when truncated, its
/// line clamp's, or all (none).
fn shown_lines(t: &ResolvedText) -> Option<usize> {
    if t.truncate {
        Some(1)
    } else {
        t.line_clamp.map(usize::from)
    }
}

fn hash_of(content: &str, key: &ShapeKey) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut h);
    key.hash(&mut h);
    h.finish()
}

/// The font stack of `family`: its named fonts, then its generic family as
/// Chrome resolves it, as CSS's `"Inter", sans-serif` and `"Geist Mono",
/// monospace` do; what none has falls back per script. Chrome's
/// `monospace` on macOS is Menlo (its default fixed font), where
/// fontique's is Courier, which lacks the symbols code shows (✓, ✗).
fn family(family: FontFamily) -> ParleyFamily<'static> {
    use parley::style::{FontFamilyName, GenericFamily as Generic};
    use std::borrow::Cow;
    let mut stack = Vec::with_capacity(family.names.len().saturating_add(2));
    stack.extend(family.names.iter().map(|n| FontFamilyName::Named(Cow::Borrowed(*n))));
    let generic = match family.generic {
        GenericFamily::SansSerif => Generic::SansSerif,
        GenericFamily::Serif => Generic::Serif,
        GenericFamily::Monospace => {
            if cfg!(target_os = "macos") {
                stack.push(FontFamilyName::Named(Cow::Borrowed("Menlo")));
            }
            Generic::Monospace
        }
        GenericFamily::SystemUi => Generic::SystemUi,
    };
    stack.push(FontFamilyName::Generic(generic));
    ParleyFamily::List(Cow::Owned(stack))
}

/// Parley properties for a style: the font, its size (also its `opsz`, as
/// Chrome's `font-optical-sizing: auto` sets it), weight, slant, tracking,
/// `tnum`, the line height, and overflow wrapping (`break-word`).
fn properties(s: &StyleKey, brush: u32) -> Vec<StyleProperty<'static, u32>> {
    let size = f32::from_bits(s.size);
    let mut props = vec![
        StyleProperty::FontFamily(family(s.family)),
        StyleProperty::FontSize(size),
        StyleProperty::FontWeight(FontWeight::new(f32::from(s.weight))),
        StyleProperty::FontStyle(if s.italic { FontStyle::Italic } else { FontStyle::Normal }),
        StyleProperty::LetterSpacing(f32::from_bits(s.tracking) * size),
        StyleProperty::LineHeight(LineHeight::Absolute(f32::from_bits(s.line_height))),
        StyleProperty::OverflowWrap(OverflowWrap::BreakWord),
        StyleProperty::FontVariations(parley::FontVariations::List(std::borrow::Cow::Owned(vec![
            FontVariation::new(parley::setting::Tag::new(b"opsz"), size),
        ]))),
        StyleProperty::Brush(brush),
    ];
    if s.tabular_nums {
        props.push(StyleProperty::FontFeatures(parley::FontFeatures::List(
            std::borrow::Cow::Owned(vec![FontFeature::new(parley::setting::Tag::new(b"tnum"), 1)]),
        )));
    }
    props
}

/// The inline box id of the padding at the start of run `index`.
fn padding_id(index: u32) -> u64 {
    u64::from(index) | 1 << 63
}

/// The run an inline box id names, when it is a run's padding.
fn padding_of(id: u64) -> Option<u32> {
    (id & 1 << 63 != 0).then(|| u32::try_from(id & !(1 << 63)).unwrap_or(0))
}

impl TextSystem {
    #[must_use]
    pub fn new() -> Self {
        let settings = settings::read_with_override();
        let gamma = gamma_for(&settings);
        Self {
            fonts: Fonts::new(),
            shaped: HashMap::default(),
            frame: 0,
            shapes: 0,
            table: std::sync::Arc::new(MaskGamma::new(&gamma)),
            gamma,
            settings,
            generation: 0,
            sent_generation: None,
            drawn: Scratch::default(),
            key_runs: Vec::new(),
            key_boxes: Vec::new(),
            scratch: DrawScratch::default(),
            pinned: HashMap::default(),
            whole_origins: cfg!(all(unix, not(target_os = "macos"))),
            units: Units::default(),
            trims: Vec::new(),
            apart_order: Vec::new(),
            fits: HashMap::default(),
            ends: Vec::new(),
            prefixes: Vec::new(),
            lines_broken: 0,
            hidden_lines: HashMap::default(),
            hidden_segmented: 0,
            hidden_order: Vec::new(),
        }
    }

    /// Read the user's text settings again (the window regained focus); a
    /// change empties what renderers rasterized ([`Self::generation`]).
    pub fn reload_settings(&mut self) {
        let settings = settings::read_with_override();
        if settings != self.settings {
            self.gamma = gamma_for(&settings);
            self.table = std::sync::Arc::new(MaskGamma::new(&self.gamma));
            self.settings = settings;
            self.generation = self.generation.saturating_add(1);
        }
    }

    /// The settings glyphs rasterize with.
    #[must_use]
    pub fn settings(&self) -> &TextSettings {
        &self.settings
    }

    /// Changes each time the settings change how glyphs look.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The pieces of `p` (split after each `\n`) in style `t`, each shaped.
    fn pieces(&mut self, p: Paragraph<'_>, t: &ResolvedText) -> Vec<Piece> {
        let content = p.content;
        let mut pieces = Vec::new();
        let mut start = 0;
        loop {
            let end = piece_end(content, start);
            let hash = self.piece(p, t, start, end);
            pieces.push(Piece { start, end, hash });
            if end >= content.len() {
                break;
            }
            start = end.saturating_add(1);
        }
        pieces
    }

    /// Shape bytes `start..end` of `p` (no line break among them), or find
    /// them shaped: found, nothing is allocated (the key is built in
    /// scratch and compared in place).
    fn piece(&mut self, p: Paragraph<'_>, t: &ResolvedText, start: usize, end: usize) -> u64 {
        self.piece_as(p, t, start, end, Apart::No)
    }

    /// [`Self::piece`], or shaped `apart` from it ([`ShapeKey::apart`]).
    fn piece_as(&mut self, p: Paragraph<'_>, t: &ResolvedText, start: usize, end: usize, apart: Apart) -> u64 {
        let text = p.content.get(start..end).unwrap_or("");
        let base = StyleKey::of(t, t.line_height);
        let mut runs = std::mem::take(&mut self.key_runs);
        runs.clear();
        runs.extend(
            p.runs
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.range.start < end && r.range.end > start || (r.range.is_empty() && r.range.start == start)
                })
                .map(|(i, r)| PieceRun {
                    range: (
                        r.range.start.max(start).saturating_sub(start),
                        r.range.end.min(end).saturating_sub(start),
                    ),
                    style: StyleKey::of(&r.text, r.text.line_height),
                    padding: (r.padding.left.to_bits(), r.padding.right.to_bits()),
                    starts: r.range.start >= start,
                    ends: r.range.end <= end,
                    index: u32::try_from(i).unwrap_or(u32::MAX),
                }),
        );
        // An in-flow inline box sits on the U+FFFC its caller put in the
        // text for it (Parley's contract); one elsewhere is left out.
        let mut boxes = std::mem::take(&mut self.key_boxes);
        boxes.clear();
        boxes.extend(
            p.boxes
                .iter()
                .filter(|b| b.at >= start && b.at < end && p.content.get(b.at..).is_some_and(|r| r.starts_with(OBJECT)))
                .map(|b| {
                    (
                        b.at.saturating_sub(start),
                        b.width.to_bits(),
                        b.height.to_bits(),
                        b.baseline.map_or(u32::MAX, f32::to_bits),
                        b.id,
                    )
                }),
        );
        let lead = if runs
            .iter()
            .any(|r| r.starts && r.range.0 == 0 && f32::from_bits(r.padding.0) > 0.0)
        {
            OBJECT.len_utf8()
        } else {
            0
        };
        let key = ShapeKey {
            base,
            runs,
            boxes,
            lead,
            rtl: t.direction == Direction::Rtl,
            apart,
        };
        let hash = hash_of(text, &key);
        let frame = self.frame;
        let found = self
            .shaped
            .get_mut(&hash)
            .filter(|s| s.content.get(lead..) == Some(text) && s.key == key);
        if let Some(s) = found {
            s.used = frame;
            (self.key_runs, self.key_boxes) = (key.runs, key.boxes);
            return hash;
        }
        // Kept with the shaped piece at the size it is; the scratch stays.
        let kept = key.clone();
        (self.key_runs, self.key_boxes) = (key.runs, key.boxes);
        let key = kept;
        let mut content = String::with_capacity(text.len().saturating_add(lead));
        if lead > 0 {
            content.push(OBJECT);
        }
        content.push_str(text);
        let layout = self.shape(&content, &key);
        let boxes = layout.inline_boxes().map(|b| (b.id, b.index)).collect();
        self.shapes = self.shapes.saturating_add(1);
        self.shaped.insert(
            hash,
            Shaped {
                content,
                key,
                layout,
                broken: None,
                rows: Vec::new(),
                shift: 0.0,
                settled: Vec::new(),
                lines: Vec::new(),
                min: None,
                used: frame,
                boxes,
                spare: Vec::new(),
            },
        );
        hash
    }

    /// Shape `text` (one piece, after its lead) with Parley.
    fn shape(&mut self, text: &str, key: &ShapeKey) -> Layout<u32> {
        let lead = key.lead;
        let fonts = &mut self.fonts;
        let mut b = fonts.lcx.ranged_builder(&mut fonts.fcx, text, 1.0, false);
        for prop in properties(&key.base, 0) {
            b.push_default(prop);
        }
        for run in &key.runs {
            let range = run.range.0.saturating_add(lead)..run.range.1.saturating_add(lead);
            if range.is_empty() {
                continue;
            }
            for prop in properties(&run.style, run.index.saturating_add(1)) {
                b.push(prop, range.clone());
            }
        }
        // A run's horizontal padding takes room in the line as more letter
        // spacing after the character before it and after its own last
        // one, so no break opportunity comes between a padding and its
        // text (an inline box would bring one each side, as CSS's atomic
        // inlines have).
        let mut extra: Vec<(usize, f32)> = Vec::new();
        for run in &key.runs {
            let (left, right) = (f32::from_bits(run.padding.0), f32::from_bits(run.padding.1));
            if left > 0.0 && run.starts && run.range.0 > 0 {
                extra.push((run.range.0, left));
            }
            if right > 0.0 && run.ends && run.range.1 > run.range.0 {
                extra.push((run.range.1, right));
            }
        }
        extra.sort_by_key(|e| e.0);
        let mut at = 0;
        while let Some(&(end, _)) = extra.get(at) {
            let mut add = 0.0;
            while let Some(&(e, w)) = extra.get(at)
                && e == end
            {
                add += w;
                at = at.saturating_add(1);
            }
            // The character that ends at `end`, and the style it is in.
            let Some((start, _)) = text
                .get(lead..end.saturating_add(lead))
                .and_then(|s| s.char_indices().next_back())
            else {
                continue;
            };
            let style = key
                .runs
                .iter()
                .rev()
                .find(|r| r.range.0 <= start && start < r.range.1)
                .map_or(&key.base, |r| &r.style);
            let tracking = f32::from_bits(style.tracking) * f32::from_bits(style.size);
            b.push(
                StyleProperty::LetterSpacing(tracking + add),
                start.saturating_add(lead)..end.saturating_add(lead),
            );
        }
        let mut boxes: Vec<InlineBox> = key
            .boxes
            .iter()
            .map(|&(at, w, h, baseline, id)| InlineBox {
                id,
                kind: InlineBoxKind::InFlow,
                index: at.saturating_add(lead),
                width: f32::from_bits(w),
                height: f32::from_bits(h),
                baseline: (baseline != u32::MAX).then(|| f32::from_bits(baseline)),
                vertical_align: VerticalAlign::BASELINE,
            })
            .collect();
        if lead > 0
            && let Some(run) = key
                .runs
                .iter()
                .find(|r| r.starts && r.range.0 == 0 && f32::from_bits(r.padding.0) > 0.0)
        {
            boxes.insert(
                0,
                InlineBox {
                    id: padding_id(run.index),
                    kind: InlineBoxKind::InFlow,
                    index: 0,
                    width: f32::from_bits(run.padding.0),
                    height: 0.0,
                    baseline: None,
                    vertical_align: VerticalAlign::BASELINE,
                },
            );
        }
        for ib in boxes {
            b.push_inline_box(ib);
        }
        b.set_base_direction(if key.rtl {
            BaseDirection::Rtl
        } else {
            BaseDirection::Ltr
        });
        b.build(text)
    }

    /// Break the lines of piece `hash` as `how` asks, unless they already are.
    fn break_piece(&mut self, hash: u64, how: Breaking) {
        let Some(s) = self.shaped.get_mut(&hash) else { return };
        if s.broken == Some(how) {
            return;
        }
        // Broken so before: swapped back (and what it had kept aside in
        // its place), allocating nothing.
        let mut spare = std::mem::take(&mut s.spare);
        let found = spare.iter().position(|b| b.how == how);
        if let Some(i) = found
            && let Some(b) = spare.get_mut(i)
        {
            b.swap(s);
            // Most recently used last.
            if let Some(after) = spare.get_mut(i..) {
                after.rotate_left(1);
            }
            s.spare = spare;
            return;
        }
        // Else what it has now is kept aside (the least recently used
        // aside giving way) and broken again.
        if let Some(prev) = s.broken {
            if spare.len() < SPARE {
                spare.push(Broken {
                    how: prev,
                    layout: s.layout.clone(),
                    rows: s.rows.clone(),
                    shift: s.shift,
                });
            } else if let Some(b) = spare.first_mut() {
                b.swap(s);
                spare.rotate_left(1);
            }
        }
        s.spare = spare;
        let align = match how.align {
            TextAlign::Center => Alignment::Center,
            TextAlign::Right => Alignment::Right,
            TextAlign::Left | TextAlign::Start | TextAlign::End => Alignment::Left,
        };
        let factor = match how.align {
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.0,
            TextAlign::Left | TextAlign::Start | TextAlign::End => 0.0,
        };
        s.layout.set_alignment(align, AlignmentOptions::default());
        let width = how.width();
        s.shift = 0.0;
        if how.nowrap || width.is_none() {
            s.layout.break_all_lines(None);
            if let Some(w) = width {
                // CSS Text §7.1: a line too long for its box is start
                // aligned, overflowing its end edge (the left in RTL).
                let room = w - s.layout.width();
                s.shift = if room >= 0.0 {
                    room * factor
                } else if s.key.rtl {
                    room
                } else {
                    0.0
                };
            }
        } else if let Some(w) = width {
            let settled = s.settled.iter().find(|(b, _)| *b == how).map(|(_, f)| *f);
            let fit = match settled {
                Some(fit) => fit,
                None => {
                    let fit = match how.wrap_style {
                        WrapStyle::Auto => Fit::whole(w),
                        WrapStyle::Pretty => pretty(&mut s.layout, &s.content, w, how.lines, &mut self.lines_broken),
                        WrapStyle::Balance => balance(&mut s.layout, w, how.lines, &mut self.lines_broken),
                    };
                    if s.settled.len() >= 8 {
                        s.settled.clear();
                    }
                    s.settled.push((how, fit));
                    fit
                }
            };
            // Up to the most lines it asks for (Parley breaks line by
            // line): a clamped document's first line wrapping into
            // thousands breaks only the lines shown. Every line is aligned
            // in the box, narrowed ones too: no shift.
            let most = how
                .lines
                .map_or(usize::MAX, |m| usize::try_from(m).unwrap_or(usize::MAX));
            break_fit(&mut s.layout, w, fit, most);
        }
        rows(&s.layout, &s.key, &mut s.rows);
        s.broken = Some(how);
        self.lines_broken = self
            .lines_broken
            .saturating_add(u64::try_from(s.layout.len()).unwrap_or(u64::MAX));
    }

    /// The pieces of `p`, shaped and broken for `width`, with the top of each.
    fn laid_out(&mut self, p: Paragraph<'_>, t: &ResolvedText, width: Option<f32>) -> Vec<(Piece, f32)> {
        let mut out = Vec::new();
        self.lay_out_into(p, t, width, &mut out);
        out
    }

    /// [`Self::laid_out`] written over `out`: the pieces of `p` (split
    /// after each `\n`), each shaped and broken for `width`, with its top.
    /// Cut short (a line clamp, a truncation), only the pieces up to the
    /// last line shown: what the clamp hides past them is never shaped,
    /// so a one-line preview of a long document holds one piece.
    fn lay_out_into(&mut self, p: Paragraph<'_>, t: &ResolvedText, width: Option<f32>, out: &mut Vec<(Piece, f32)>) {
        out.clear();
        let mut how = Breaking::of(t, width);
        let clamp = shown_lines(t);
        let wraps = !how.nowrap && how.width.is_some();
        let content = p.content;
        let mut start = 0;
        let mut y = 0.0;
        let mut lines = 0_usize;
        loop {
            let end = piece_end(content, start);
            let hash = self.piece(p, t, start, end);
            if wraps {
                how.lines = clamp.map(|c| u32::try_from(c.saturating_sub(lines)).unwrap_or(u32::MAX));
            }
            self.break_piece(hash, how);
            out.push((Piece { start, end, hash }, y));
            let s = self.shaped.get(&hash);
            y += s.map_or(t.line_height, |s| height(s, t));
            lines = lines.saturating_add(s.map_or(1, |s| s.layout.len().max(1)));
            if end >= content.len() || clamp.is_some_and(|c| lines >= c) {
                break;
            }
            start = end.saturating_add(1);
        }
    }

    /// Where the lines of `piece` as broken end, in the paragraph: its end,
    /// or short of it when a clamp stopped breaking them
    /// ([`Breaking::lines`]): what follows was never broken.
    fn laid_end(&self, piece: &Piece) -> usize {
        let Some(s) = self.shaped.get(&piece.hash) else {
            return piece.end;
        };
        if s.broken.is_none_or(|b| b.lines.is_none()) {
            return piece.end;
        }
        let Some(last) = s.layout.len().checked_sub(1).and_then(|l| s.layout.get(l)) else {
            return piece.end;
        };
        let end = last.text_range().end.saturating_sub(s.key.lead);
        piece.start.saturating_add(end).min(piece.end)
    }

    /// The last shown line of `p` (laid out as `laid`), when it is cut
    /// short with an ellipsis: [`Cut`].
    /// What a truncation drops of the padding kept units hold is left in
    /// [`TextSystem::trims`] (none for a clamp), until the next cut.
    fn cut(&mut self, p: Paragraph<'_>, laid: &[(Piece, f32)], t: &ResolvedText, width: Option<f32>) -> Option<Cut> {
        self.trims.clear();
        let width = width?;
        let clamp = shown_lines(t)?;
        // The last line shown, and whether more follow it: lines of its
        // piece, or pieces after it (laid out or not: [`Self::lay_out_into`]
        // stops at the last line shown).
        let mut seen = 0_usize;
        let mut last = None;
        let mut more = false;
        for (i, (piece, _)) in laid.iter().enumerate() {
            let lines = self.shaped.get(&piece.hash).map_or(1, |s| s.layout.len().max(1));
            if seen.saturating_add(lines) >= clamp {
                let line = clamp.saturating_sub(seen).saturating_sub(1);
                last = Some((i, line));
                more = line.saturating_add(1) < lines || self.laid_end(piece) < p.content.len();
                break;
            }
            seen = seen.saturating_add(lines);
        }
        let (pi, line) = last?;
        let &(piece, _) = laid.get(pi)?;
        let s = self.shaped.get(&piece.hash)?;
        let l = s.layout.get(line)?;
        let rtl = s.key.rtl;
        // The line by its units (whole grapheme clusters, a ligature's
        // together, inline boxes), in visual order: its edges, and its
        // content's extent (without the whitespace that hangs at its end).
        let (left, right, from_content, to_content) = extent(s, &l);
        let overflows = to_content - from_content > width + noise(width);
        if !more && !overflows {
            return None;
        }
        let ellipsis = self.ellipsis_width(t);
        let room = width - ellipsis;
        // The kept text's width and left edge (where it starts, closed up
        // or laid out alone), what is kept, and the pieces shaped apart.
        let (keep, kept, kept_left, probe, prefix) = if t.truncate {
            // css-overflow-3 §5.1: what is hidden is what is visually at
            // the line's end edge (the right in LTR, the left in RTL),
            // whole units, leaving the ellipsis room, and the whitespace
            // before the ellipsis.
            let (from, to, trimmed) = self.visual_keep(piece.hash, line, room, left, right)?;
            (Keep::Visual { from, to }, to - from - trimmed, left, None, None)
        } else {
            // css-overflow-4 block-ellipsis: a logical prefix of the line,
            // laid out as a line of its own: shaped apart from the
            // paragraph, its bidi resolved as if it were all the line holds
            // and its runs' padding only theirs (none for a run it drops).
            let (end, probe, soft) = self.block_ellipsis(p, t, piece, line, room);
            let s = self.shaped.get(&piece.hash)?;
            let start = s.layout.get(line)?.text_range().start;
            let lead = s.key.lead;
            let at = |i: usize| piece.start.saturating_add(i.saturating_sub(lead));
            let from = at(start);
            // Shaped alone, what is kept can be wider than in the line
            // (Arabic's last letter kept turns final): the longest prefix
            // that, shaped alone, leaves the ellipsis room.
            let to = self.kept_fit(p, t, &piece, line, soft.then_some(end), room);
            let hash = self.piece_as(p, t, from, to, Apart::Kept);
            self.break_piece(hash, Breaking::ONE_LINE);
            let (k_left, k_width) = self.alone_extent(hash);
            let end = to.saturating_sub(piece.start).saturating_add(lead);
            (
                Keep::Logical { end },
                k_width,
                k_left,
                probe,
                Some(Piece {
                    start: from,
                    end: to,
                    hash,
                }),
            )
        };
        // css-overflow-4 block-ellipsis: the kept text and the ellipsis
        // are aligned as one in the box, start aligned when they overflow
        // it (the ellipsis alone wider than the box).
        let total = kept + ellipsis;
        let factor = match t.physical_align() {
            TextAlign::Center => 0.5,
            TextAlign::Right => 1.0,
            TextAlign::Left => 0.0,
            _ if rtl => 1.0,
            _ => 0.0,
        };
        let start = if total <= width + noise(width) {
            (width - total).max(0.0) * factor
        } else if rtl {
            width - total
        } else {
            0.0
        };
        let (dx, ellipsis_at) = if rtl {
            (start + ellipsis - kept_left, start)
        } else {
            (start - kept_left, start + kept)
        };
        Some(Cut {
            piece: pi,
            line,
            keep,
            kept,
            dx,
            ellipsis_at,
            ellipsis,
            probe,
            prefix,
        })
    }

    /// css-overflow-4 block-ellipsis: where line `line` of `piece`, the
    /// last a clamp shows, keeps its content to (a byte of its layout's
    /// text), content displaced from its end as if it wrapped at `room`:
    /// to the last soft wrap opportunity that leaves the ellipsis room
    /// (not one `overflow-wrap` adds, none under `nowrap`), its whitespace
    /// at the end dropped; all of it when there is none. With the line
    /// shaped apart and broken at `room` to find it. Whether it found one
    /// (else what fits of the line as laid out, by whole units, and
    /// [`TextSystem::kept_fit`] looks over the whole line).
    fn block_ellipsis(
        &mut self,
        p: Paragraph<'_>,
        t: &ResolvedText,
        piece: Piece,
        line: usize,
        room: f32,
    ) -> (usize, Option<u64>, bool) {
        let Some(s) = self.shaped.get(&piece.hash) else {
            return (0, None, false);
        };
        let Some(range) = s.layout.get(line).map(|l| l.text_range()) else {
            return (0, None, false);
        };
        let lead = s.key.lead;
        if t.nowrap || room <= 0.0 {
            return (self.logical_prefix(piece.hash, line, room), None, false);
        }
        // The line's bytes in the paragraph (a lead is not content).
        let at = |i: usize| piece.start.saturating_add(i.saturating_sub(lead));
        let (from, to) = (at(range.start), at(range.end));
        let apart = self.piece_as(p, t, from, to, Apart::Probe);
        self.break_piece(
            apart,
            Breaking {
                width: Some(room.to_bits()),
                nowrap: false,
                wrap_style: WrapStyle::Auto,
                align: TextAlign::Left,
                lines: Some(1),
            },
        );
        // Where its first line ends (its whitespace at the end dropped),
        // unless it was broken where `overflow-wrap` allows alone, or is
        // wider than the room: then no soft wrap opportunity leaves the
        // ellipsis room.
        let wrapped = self.shaped.get(&apart).and_then(|a| {
            let first = a.layout.get(0)?;
            let (_, _, content_from, content_to) = extent(a, &first);
            if first.break_reason() == parley::BreakReason::Emergency || content_to - content_from > room + noise(room)
            {
                return None;
            }
            let end = first.text_range().end;
            Some(
                a.content
                    .get(..end)
                    .map_or(end, |c| c.trim_end().len())
                    .max(a.key.lead)
                    .saturating_sub(a.key.lead),
            )
        });
        let Some(kept) = wrapped else {
            return (self.logical_prefix(piece.hash, line, room), Some(apart), false);
        };
        // Back in the piece's layout's bytes.
        let end = at(range.start).saturating_add(kept);
        (
            end.saturating_sub(piece.start).saturating_add(lead).min(range.end),
            Some(apart),
            true,
        )
    }

    /// Where line `line` of piece `hash` keeps its content to when no soft
    /// wrap opportunity leaves its ellipsis room (`nowrap`, one long
    /// word): the longest logical prefix of whole units (grapheme
    /// clusters, a ligature's together, inline boxes) that fits `room`,
    /// its whitespace at the end dropped, so as much of it as fits shows
    /// before the ellipsis rather than the ellipsis alone. Its units are
    /// put in logical order in a buffer the system keeps: allocates
    /// nothing once warm.
    /// A unit's padding for the run after it is that run's: it counts
    /// toward what fits only once that run's first unit is kept.
    fn logical_prefix(&mut self, hash: u64, line: usize, room: f32) -> usize {
        let mut buf = std::mem::take(&mut self.units);
        let mut end = 0;
        if let Some(s) = self.shaped.get(&hash)
            && let Some(l) = s.layout.get(line)
        {
            end = l.text_range().start;
            buf.fill(s, &l);
            let mut sum = 0.0;
            for &(_, i) in &buf.order {
                let Some(u) = buf.units.get(i) else { continue };
                sum += u.advance;
                if sum - u.pad > room + noise(room) {
                    break;
                }
                if !u.space {
                    end = u.range.1;
                }
            }
        }
        self.units = buf;
        end
    }

    /// css-overflow-3 §5.1, `text-overflow`: what line `line` of piece
    /// `hash` (edges `left`, `right`) keeps visually from its start edge in
    /// `room`, whole units: x from and to (the whitespace before the
    /// ellipsis left out), and the padding of the runs it drops that kept
    /// units hold ([`Trims`]), wherever bidi puts them. A unit's padding
    /// for the run after it counts toward what fits only once that run's
    /// first unit is kept. Its units are kept in a buffer the system
    /// keeps: allocates nothing once warm.
    fn visual_keep(&mut self, hash: u64, line: usize, room: f32, left: f32, right: f32) -> Option<(f32, f32, f32)> {
        let mut buf = std::mem::take(&mut self.units);
        let mut trims = std::mem::take(&mut self.trims);
        trims.clear();
        let out = self.shaped.get(&hash).and_then(|s| {
            let l = s.layout.get(line)?;
            buf.fill(s, &l);
            let n = buf.units.len();
            let rtl = s.key.rtl;
            // The kth unit from the start edge (the left in LTR, the right
            // in RTL), and whether unit i comes before it.
            let at = |k: usize| if rtl { n.saturating_sub(k).saturating_sub(1) } else { k };
            let before = |i: usize, k: usize| if rtl { i > at(k) } else { i < at(k) };
            let (mut sum, mut pending) = (0.0_f32, 0.0_f32);
            // How many units from the start edge are kept: to the last
            // that fits and is not whitespace.
            let mut content = 0;
            for k in 0..n {
                let Some(u) = buf.units.get(at(k)).copied() else { break };
                sum += u.advance;
                if u.pad > 0.0 && u.next.is_none_or(|j| !before(j, k)) {
                    pending += u.pad;
                }
                if let Some(o) = u.owner.filter(|&i| before(i, k)).and_then(|i| buf.units.get(i)) {
                    pending -= o.pad;
                }
                if sum - pending > room + noise(room) {
                    break;
                }
                if !u.space {
                    content = k.saturating_add(1);
                }
            }
            let kept = |i: usize| {
                if rtl {
                    i >= n.saturating_sub(content)
                } else {
                    i < content
                }
            };
            let (from, to) = match content.checked_sub(1).and_then(|k| buf.units.get(at(k))) {
                None if rtl => (right, right),
                None => (left, left),
                Some(u) if rtl => (u.x, right),
                Some(u) => (left, u.x + u.advance),
            };
            let mut trimmed = 0.0;
            for (i, u) in buf.units.iter().enumerate() {
                if kept(i) && u.pad > 0.0 && u.next.is_none_or(|j| !kept(j)) {
                    trims.push((u.range.1, u.pad));
                    trimmed += u.pad;
                }
            }
            trims.sort_unstable_by_key(|t| t.0);
            Some((from, to, trimmed))
        });
        self.units = buf;
        self.trims = trims;
        out
    }

    /// The left edge and content width of the one line of piece `hash`.
    fn alone_extent(&self, hash: u64) -> (f32, f32) {
        self.shaped
            .get(&hash)
            .and_then(|k| {
                let l = k.layout.get(0)?;
                let (left, _, content_from, content_to) = extent(k, &l);
                Some((left, content_to - content_from))
            })
            .unwrap_or((0.0, 0.0))
    }

    /// Where (a byte of the paragraph) a clamp keeps line `line` of
    /// `piece` to: to the soft wrap opportunity `soft` (a byte of its
    /// layout's text) when that, shaped alone, fits `room`; else the longest
    /// prefix of the line (short of `soft`, when given), at whole units,
    /// that fits it by the line's own advances, walked once ([`word_fit`]):
    /// shaped alone again at most once, where it cuts a word. The prefix
    /// shaped alone that fits is kept (it is the one shown); the answer is kept by the piece, line,
    /// room and limit, from the piece's start (a line shared by paragraphs
    /// that start it at other bytes finds it too), so a steady frame shapes
    /// none and allocates nothing.
    fn kept_fit(
        &mut self,
        p: Paragraph<'_>,
        t: &ResolvedText,
        piece: &Piece,
        line: usize,
        soft: Option<usize>,
        room: f32,
    ) -> usize {
        let Some(s) = self.shaped.get(&piece.hash) else {
            return piece.start;
        };
        let Some(l) = s.layout.get(line) else {
            return piece.start;
        };
        let lead = s.key.lead;
        let at = |i: usize| piece.start.saturating_add(i.saturating_sub(lead));
        let from = at(l.text_range().start);
        let line_end = l.text_range().end;
        let key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (piece.hash, line, soft, room.to_bits()).hash(&mut h);
            h.finish()
        };
        let frame = self.frame;
        if let Some(found) = self.fits.get_mut(&key) {
            found.1 = frame;
            return piece.start.saturating_add(found.0);
        }
        // The prefix to `to` shaped alone: its width. One shaped only for
        // this is let go, but one that fits: it is the one shown.
        let alone = |ts: &mut Self, to: usize| {
            let known = ts.shaped.len();
            let hash = ts.piece_as(p, t, from, to, Apart::Kept);
            let fresh = ts.shaped.len() > known;
            ts.break_piece(hash, Breaking::ONE_LINE);
            let width = ts.alone_extent(hash).1;
            if fresh && width > room + noise(room) {
                ts.shaped.remove(&hash);
            }
            width
        };
        let to = match soft {
            Some(end) if alone(self, at(end)) <= room + noise(room) => at(end),
            _ => {
                // The prefixes (each unit's end that is not whitespace, in
                // the text's order, short of the limit): their widths in
                // the line, walked once (a unit's padding for the run
                // after it counting once that run's first unit is in),
                // kept monotone, and whether each ends a word, by the
                // word boundaries Parley's analysis gave the line's
                // clusters (dictionary-aware, as hidden text's are).
                let limit = soft.unwrap_or(line_end.saturating_add(1));
                let mut buf = std::mem::take(&mut self.units);
                let mut prefixes = std::mem::take(&mut self.prefixes);
                let mut bounds = std::mem::take(&mut self.ends);
                prefixes.clear();
                bounds.clear();
                let text_end = at(line_end);
                if let Some(s) = self.shaped.get(&piece.hash)
                    && let Some(l) = s.layout.get(line)
                {
                    let mut clusters = None;
                    for item in l.items() {
                        if let PositionedLayoutItem::GlyphRun(run) = item {
                            for c in run_clusters(&mut clusters, &run, || run.run().visual_clusters()) {
                                if c.is_word_boundary() {
                                    bounds.push(at(c.text_range().start));
                                }
                            }
                        }
                    }
                    bounds.push(text_end);
                    bounds.sort_unstable();
                    buf.fill(s, &l);
                    let (mut sum, mut widest) = (0.0_f32, 0.0_f32);
                    // The prefixes come in the text's order, each past the
                    // last: one pointer over the sorted boundaries meets
                    // each's end, never stepping back.
                    let mut next = 0_usize;
                    for &(_, i) in &buf.order {
                        let Some(u) = buf.units.get(i) else { continue };
                        sum += u.advance;
                        widest = widest.max(sum - u.pad);
                        if u.space || u.range.1 >= limit || u.range.1 <= lead {
                            continue;
                        }
                        let e = at(u.range.1);
                        if prefixes.last().is_none_or(|q: &Prefix| q.end < e) {
                            while bounds.get(next).is_some_and(|&b| {
                                #[cfg(test)]
                                tests::BOUND_LOOKS.with(|n| n.set(n.get().saturating_add(1)));
                                b < e
                            }) {
                                next = next.saturating_add(1);
                            }
                            prefixes.push(Prefix {
                                end: e,
                                inline: widest,
                                closes: bounds.get(next) == Some(&e),
                            });
                        }
                    }
                }
                self.units = buf;
                self.ends = bounds;
                let k = word_fit(&prefixes, room, |k| {
                    let to = k.checked_sub(1).and_then(|i| prefixes.get(i)).map_or(from, |q| q.end);
                    alone(self, to)
                });
                let to = k.checked_sub(1).and_then(|i| prefixes.get(i)).map_or(from, |q| q.end);
                self.prefixes = prefixes;
                to
            }
        };
        self.fits.insert(key, (to.saturating_sub(piece.start), frame));
        to
    }

    fn ellipsis_width(&mut self, t: &ResolvedText) -> f32 {
        let hash = self.piece(Paragraph::from(ELLIPSIS), t, 0, ELLIPSIS.len());
        self.break_piece(hash, Breaking::ONE_LINE);
        self.shaped.get(&hash).map_or(0.0, |s| s.layout.width())
    }

    /// The size `p` takes in style `t`, wrapped to `width` when given: its
    /// widest line (the text, not where alignment put it) and its lines'
    /// height, up to a line clamp.
    pub fn measure<'p>(&mut self, p: impl Into<Paragraph<'p>>, t: &ResolvedText, width: Option<f32>) -> (f32, f32) {
        let m = self.metrics(p, t, width);
        (m.width, m.height)
    }

    /// [`Self::measure`] with the paragraph's first and last baselines, as
    /// [`Self::draw`] places them (Blink's rounding, ADR-0005), for layout
    /// to align it by.
    pub fn metrics<'p>(&mut self, p: impl Into<Paragraph<'p>>, t: &ResolvedText, width: Option<f32>) -> TextMetrics {
        let p = p.into();
        if t.truncate || t.line_clamp.is_some() {
            return self.measure_cut(p, t, width);
        }
        let how = Breaking::of(t, width);
        let empty = p.content.is_empty() && p.boxes.is_empty();
        let pieces = self.pieces(p, t);
        let several = pieces.len() > 1;
        let mut widest = 0.0_f32;
        let mut height = 0.0;
        let mut baselines = Lines::default();
        for piece in pieces {
            let cached = self
                .shaped
                .get(&piece.hash)
                .is_some_and(|s| s.lines.iter().any(|(b, _)| *b == how));
            if !cached {
                self.break_piece(piece.hash, how);
                if let Some(s) = self.shaped.get_mut(&piece.hash) {
                    let lines = s
                        .layout
                        .lines()
                        .zip(&s.rows)
                        .map(|(l, r)| {
                            let m = l.metrics();
                            LineSize {
                                width: m.advance - m.hanging_advance,
                                height: r.height,
                                baseline: r.baseline,
                            }
                        })
                        .collect();
                    if s.lines.len() >= 4 {
                        s.lines.remove(0);
                    }
                    s.lines.push((how, lines));
                }
            }
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
            let Some((_, lines)) = s.lines.iter().find(|(b, _)| *b == how) else {
                continue;
            };
            if lines.is_empty() {
                // An empty line between forced breaks still has a baseline;
                // an empty paragraph has none.
                if several {
                    baselines.add(height + empty_baseline(&s.layout, t));
                }
                height += t.line_height;
            }
            for l in lines {
                widest = widest.max(l.width);
                baselines.add(height + l.baseline);
                height += l.height;
            }
        }
        TextMetrics::of(widest, height, t, if empty { Lines::default() } else { baselines })
    }

    /// [`Self::metrics`] for a paragraph cut at a line clamp, or truncated.
    fn measure_cut(&mut self, p: Paragraph<'_>, t: &ResolvedText, width: Option<f32>) -> TextMetrics {
        let empty = p.content.is_empty() && p.boxes.is_empty();
        let laid = self.laid_out(p, t, width);
        let cut = self.cut(p, &laid, t, width);
        let clamp = if t.truncate {
            Some(1)
        } else {
            t.line_clamp.map(usize::from)
        };
        let several = laid.len() > 1;
        let mut widest = 0.0_f32;
        let mut height = 0.0;
        let mut lines = 0_usize;
        let mut baselines = Lines::default();
        'pieces: for (i, (piece, _)) in laid.iter().enumerate() {
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
            if s.layout.is_empty() {
                lines = lines.saturating_add(1);
                if several {
                    baselines.add(height + empty_baseline(&s.layout, t));
                }
                height += t.line_height;
                if clamp.is_some_and(|c| lines >= c) {
                    break;
                }
                continue;
            }
            for (j, (line, r)) in s.layout.lines().zip(&s.rows).enumerate() {
                let m = line.metrics();
                let w = match cut {
                    Some(c) if c.piece == i && c.line == j => c.kept + c.ellipsis,
                    _ => m.advance - m.hanging_advance,
                };
                widest = widest.max(w);
                baselines.add(height + r.baseline);
                height += r.height;
                lines = lines.saturating_add(1);
                if clamp.is_some_and(|c| lines >= c) {
                    break 'pieces;
                }
            }
        }
        TextMetrics::of(widest, height, t, if empty { Lines::default() } else { baselines })
    }

    /// The min-content width of `p` in style `t`: its widest word, or all of
    /// it when it does not wrap. Breaking inside a word
    /// (`overflow-wrap: break-word`) does not narrow it, as in CSS.
    pub fn min_content<'p>(&mut self, p: impl Into<Paragraph<'p>>, t: &ResolvedText) -> f32 {
        let p = p.into();
        if t.nowrap || t.truncate {
            return self.measure(p, t, None).0;
        }
        let pieces = self.pieces(p, t);
        let mut min = 0.0_f32;
        for piece in &pieces {
            if let Some(s) = self.shaped.get_mut(&piece.hash) {
                let m = *s.min.get_or_insert_with(|| s.layout.calculate_content_widths().min);
                min = min.max(m);
            }
        }
        (min * LAYOUT_UNITS).ceil() / LAYOUT_UNITS
    }

    /// The byte offset in `content` nearest to `(x, y)`, relative to the text box.
    /// On a line cut short, what it shows is hit (never an offset in the
    /// text it hides); over its ellipsis, past it, or below the line, the
    /// text's end, the hidden text included: the ellipsis stands for what
    /// it hides, as Chrome selects to the end on a drag past it. A screen
    /// reader's point query agrees: what is hidden is read zero wide at the
    /// ellipsis's start ([`TextSystem::text_runs`]), so a point over the
    /// ellipsis is past the end of the last run.
    pub fn hit(&mut self, content: &str, t: &ResolvedText, width: Option<f32>, x: f32, y: f32) -> usize {
        let p = Paragraph::from(content);
        let laid = self.laid_out(p, t, width);
        let cut = self.cut(p, &laid, t, width);
        let index = laid.iter().rposition(|(_, top)| *top <= y).unwrap_or(0);
        if cut.is_some_and(|c| index > c.piece) {
            return content.len();
        }
        let Some((piece, top)) = laid.get(index) else {
            return content.len();
        };
        let Some(s) = self.shaped.get(&piece.hash) else {
            return piece.start;
        };
        let local = y - top;
        let line = s.rows.iter().rposition(|r| r.top <= local).unwrap_or(0);
        if let Some(c) = cut.filter(|c| c.piece == index && line >= c.line) {
            let below = s.rows.get(c.line).is_none_or(|r| local >= r.top + r.height);
            if line > c.line || below {
                return content.len();
            }
            return self.hit_cut(&laid, c, x, content.len());
        }
        let local = s.rows.get(line).map_or(local, |r| r.parley(local));
        let cursor = ParleyCursor::from_point(&s.layout, x - s.shift, local);
        piece.start.saturating_add(cursor.index()).min(piece.end)
    }

    /// [`Self::hit`] on line cut short `c` at `x`: in the kept prefix a
    /// clamp lays out alone, or among the units a truncation keeps, as
    /// they are drawn; past the ellipsis, `len`, the text's end.
    fn hit_cut(&self, laid: &[(Piece, f32)], c: Cut, x: f32, len: usize) -> usize {
        let Some((piece, _)) = laid.get(c.piece) else { return 0 };
        let Some(s) = self.shaped.get(&piece.hash) else {
            return piece.start;
        };
        // Over the ellipsis or past it (its side: the right in LTR, the
        // left in RTL): the text's end.
        let past = if s.key.rtl {
            x < c.ellipsis_at + c.ellipsis
        } else {
            x >= c.ellipsis_at
        };
        if past {
            return len;
        }
        let lead = s.key.lead;
        let back = |i: usize| piece.start.saturating_add(i.saturating_sub(lead)).min(piece.end);
        if let Some(k) = c.prefix {
            let Some(ks) = self.shaped.get(&k.hash) else {
                return k.end;
            };
            let y = ks.rows.first().map_or(0.0, |r| r.parley_baseline);
            let cursor = ParleyCursor::from_point(&ks.layout, x - c.dx, y);
            let i = cursor.index().saturating_sub(ks.key.lead);
            return k.start.saturating_add(i).min(k.end);
        }
        // The kept unit nearest `x`, and where in it (Parley's x).
        let mut best: Option<(f32, f32, (usize, usize))> = None;
        placed_units(s, c, &self.trims, |bx, shown, px, _, range| {
            let into = (x - bx).clamp(0.0, shown.max(0.0));
            let off = (x - bx - into).abs();
            if best.is_none_or(|b| off < b.0) {
                best = Some((off, px + into.min(shown * 0.999), range));
            }
        });
        let Some((_, px, range)) = best else {
            return back(s.layout.get(c.line).map_or(lead, |l| l.text_range().start));
        };
        let y = s.rows.get(c.line).map_or(0.0, |r| r.parley_baseline);
        let i = ParleyCursor::from_point(&s.layout, px, y).index();
        back(i.clamp(range.0, range.1))
    }

    /// Where the caret for byte offset `at` sits, relative to the text box.
    /// On a line cut short it sits where what it shows is drawn; at an
    /// offset in the text the cut hides (or below it), at the kept text's
    /// end, by the ellipsis.
    pub fn caret(&mut self, content: &str, t: &ResolvedText, width: Option<f32>, at: usize) -> Caret {
        let p = Paragraph::from(content);
        let laid = self.laid_out(p, t, width);
        let cut = self.cut(p, &laid, t, width);
        let index = laid
            .iter()
            .position(|(p, _)| at <= p.end)
            .unwrap_or(laid.len().saturating_sub(1));
        let Some((piece, top)) = laid.get(index) else {
            return Caret {
                x: 0.0,
                y: 0.0,
                height: t.line_height,
            };
        };
        let Some(s) = self.shaped.get(&piece.hash) else {
            return Caret {
                x: 0.0,
                y: *top,
                height: t.line_height,
            };
        };
        if let Some(c) = cut {
            let local = at.saturating_sub(piece.start).saturating_add(s.key.lead);
            let line = s
                .layout
                .lines()
                .position(|l| local < l.text_range().end)
                .unwrap_or(s.layout.len());
            if index > c.piece || (index == c.piece && line >= c.line) {
                return self.caret_cut(&laid, c, at, t);
            }
        }
        let local = at.saturating_sub(piece.start).min(s.content.len());
        let cursor = ParleyCursor::from_byte_index(&s.layout, local, Affinity::Downstream);
        let g = cursor.geometry(&s.layout, 1.0);
        let (y, height) = if s.layout.is_empty() {
            (0.0, t.line_height)
        } else {
            let (y0, y1) = (crate::convert::f64_to_f32(g.y0), crate::convert::f64_to_f32(g.y1));
            match row_at(&s.rows, y0) {
                Some(r) if (y0 - r.parley_top).abs() < 0.01 && (y1 - y0 - r.parley_height).abs() < 0.01 => {
                    (r.top, r.height)
                }
                Some(r) => (r.place(y0), y1 - y0),
                None => (y0, y1 - y0),
            }
        };
        Caret {
            x: crate::convert::f64_to_f32(g.x0) + s.shift,
            y: top + y,
            height,
        }
    }

    /// [`Self::caret`] on line cut short `c`, for byte `at`.
    fn caret_cut(&self, laid: &[(Piece, f32)], c: Cut, at: usize, t: &ResolvedText) -> Caret {
        let Some((piece, top)) = laid.get(c.piece) else {
            return Caret {
                x: 0.0,
                y: 0.0,
                height: t.line_height,
            };
        };
        let Some(s) = self.shaped.get(&piece.hash) else {
            return Caret {
                x: 0.0,
                y: *top,
                height: t.line_height,
            };
        };
        let (y, height) = s.rows.get(c.line).map_or((0.0, t.line_height), |r| (r.top, r.height));
        let rtl = s.key.rtl;
        // By the ellipsis: where the kept text ends.
        let edge = if rtl { c.ellipsis_at + c.ellipsis } else { c.ellipsis_at };
        let caret = |x: f32| Caret { x, y: top + y, height };
        if let Some(k) = c.prefix {
            let Some(ks) = self.shaped.get(&k.hash) else {
                return caret(edge);
            };
            if at > k.end {
                return caret(edge);
            }
            let local = at.saturating_sub(k.start).saturating_add(ks.key.lead);
            let g = ParleyCursor::from_byte_index(&ks.layout, local, Affinity::Downstream).geometry(&ks.layout, 1.0);
            return caret(crate::convert::f64_to_f32(g.x0) + c.dx);
        }
        let local = at.saturating_sub(piece.start).saturating_add(s.key.lead);
        let g = ParleyCursor::from_byte_index(&s.layout, local, Affinity::Downstream).geometry(&s.layout, 1.0);
        let px = crate::convert::f64_to_f32(g.x0);
        // The kept unit the caret stands at an edge of, at Parley's x.
        let mut found: Option<(f32, f32)> = None;
        placed_units(s, c, &self.trims, |bx, shown, x, advance, range| {
            let edge_of = range.0 == local || range.1 == local || (range.0 < local && local < range.1);
            if edge_of {
                let on = px >= x - noise(x) && px <= x + advance + noise(x + advance);
                let off = if on {
                    0.0
                } else {
                    (px - x).abs().min((px - x - advance).abs())
                };
                if found.is_none_or(|f| off < f.0) {
                    found = Some((off, bx + (px - x).clamp(0.0, shown.max(0.0))));
                }
            }
        });
        caret(found.map_or(edge, |f| f.1))
    }

    /// The boxes covering bytes `from..to`, relative to the text box.
    /// On a line cut short, what it shows covers them as it is drawn; the
    /// text it hides, and what is below it, has none.
    pub fn selection<'p>(
        &mut self,
        p: impl Into<Paragraph<'p>>,
        t: &ResolvedText,
        width: Option<f32>,
        from: usize,
        to: usize,
    ) -> Vec<Bounds> {
        let p = p.into();
        let laid = self.laid_out(p, t, width);
        let cut = self.cut(p, &laid, t, width);
        let mut boxes = Vec::new();
        for (pi, (piece, top)) in laid.iter().enumerate() {
            if cut.is_some_and(|c| pi > c.piece) {
                break;
            }
            if to < piece.start || from > piece.end || (from == to && from != piece.start) {
                continue;
            }
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
            let cut_line = cut.filter(|c| c.piece == pi).map(|c| c.line);
            let a = from
                .max(piece.start)
                .saturating_sub(piece.start)
                .saturating_add(s.key.lead);
            let b = to.min(piece.end).saturating_sub(piece.start).saturating_add(s.key.lead);
            if s.layout.is_empty() || a == b {
                // An empty line inside the selection shows a sliver.
                if from < piece.start || (from <= piece.start && to > piece.end) {
                    boxes.push(Bounds {
                        x: s.shift,
                        y: *top,
                        width: 1.0,
                        height: t.line_height,
                    });
                }
                continue;
            }
            let sel = Selection::new(
                ParleyCursor::from_byte_index(&s.layout, a, Affinity::Downstream),
                ParleyCursor::from_byte_index(&s.layout, b, Affinity::Upstream),
            );
            for (bb, _) in sel.geometry(&s.layout) {
                let (y0, y1) = (crate::convert::f64_to_f32(bb.y0), crate::convert::f64_to_f32(bb.y1));
                let line = s.rows.iter().rposition(|r| r.parley_top <= y0 + 0.01).unwrap_or(0);
                if cut_line.is_some_and(|c| line >= c) {
                    continue;
                }
                // Our line's box where Parley covers its whole line.
                let (y0, y1) = match s.rows.get(line) {
                    Some(r) if (y0 - r.parley_top).abs() < 0.01 && (y1 - y0 - r.parley_height).abs() < 0.01 => {
                        (r.top, r.top + r.height)
                    }
                    Some(r) => (r.place(y0), r.place(y1)),
                    None => (y0, y1),
                };
                boxes.push(Bounds {
                    x: crate::convert::f64_to_f32(bb.x0) + s.shift,
                    y: top + y0,
                    width: crate::convert::f64_to_f32(bb.x1 - bb.x0).max(1.0),
                    height: y1 - y0,
                });
            }
            if let Some(c) = cut.filter(|c| c.piece == pi) {
                self.select_cut(s, c, *top, (a, b), (from, to), &mut boxes);
            }
        }
        boxes
    }

    /// The boxes [`Self::selection`] gives line cut short `c` of `s` (its
    /// top `top`), for its layout's bytes `local` (the paragraph's
    /// `bytes`): over the kept prefix a clamp lays out alone, or each kept
    /// unit a truncation shows, as drawn.
    fn select_cut(
        &self,
        s: &Shaped,
        c: Cut,
        top: f32,
        local: (usize, usize),
        bytes: (usize, usize),
        boxes: &mut Vec<Bounds>,
    ) {
        let Some(r) = s.rows.get(c.line) else { return };
        let (y, height) = (top + r.top, r.height);
        if let Some(k) = c.prefix {
            let Some(ks) = self.shaped.get(&k.hash) else { return };
            let (a, b) = (bytes.0.max(k.start), bytes.1.min(k.end));
            if a >= b {
                return;
            }
            let at = |i: usize| i.saturating_sub(k.start).saturating_add(ks.key.lead);
            let sel = Selection::new(
                ParleyCursor::from_byte_index(&ks.layout, at(a), Affinity::Downstream),
                ParleyCursor::from_byte_index(&ks.layout, at(b), Affinity::Upstream),
            );
            for (bb, _) in sel.geometry(&ks.layout) {
                boxes.push(Bounds {
                    x: crate::convert::f64_to_f32(bb.x0) + c.dx,
                    y,
                    width: crate::convert::f64_to_f32(bb.x1 - bb.x0).max(1.0),
                    height,
                });
            }
            return;
        }
        let first = boxes.len();
        let x_of = |i: usize| {
            let g = ParleyCursor::from_byte_index(&s.layout, i, Affinity::Downstream).geometry(&s.layout, 1.0);
            crate::convert::f64_to_f32(g.x0)
        };
        placed_units(s, c, &self.trims, |bx, shown, px, advance, range| {
            let (a, b) = (range.0.max(local.0), range.1.min(local.1));
            if a >= b {
                return;
            }
            // Part of a unit (a letter of a ligature): its share, between
            // where Parley puts carets inside it (from its start edge: its
            // left, or its right when right to left).
            let (from, to) = if (a, b) == range {
                (0.0, shown)
            } else {
                let start = x_of(range.0) - px;
                let end = if start < advance * 0.5 { advance } else { 0.0 };
                let xa = if a == range.0 { start } else { x_of(a) - px };
                let xb = if b == range.1 { end } else { x_of(b) - px };
                (xa.min(xb).clamp(0.0, shown), xa.max(xb).clamp(0.0, shown))
            };
            let (x0, x1) = (bx + from, bx + to);
            // Next to the last box: one box.
            if boxes.len() > first
                && let Some(last) = boxes.last_mut()
                && (last.x + last.width - x0).abs() < 0.5
            {
                last.width = x1 - last.x;
                return;
            }
            boxes.push(Bounds {
                x: x0,
                y,
                width: (x1 - x0).max(1.0),
                height,
            });
        });
    }

    /// Where `p`'s inline boxes ([`Paragraph::boxes`]) sit, relative to
    /// the text box, and whether each shows: one on a line a clamp hides,
    /// or that a cut drops on the last line (its place taken by the
    /// ellipsis, or past it), does not (css-overflow-3 §5.1 hides what the
    /// ellipsis replaces, atomic inlines too); one a cut keeps sits where
    /// the kept text closes up.
    pub fn placed_inline_boxes<'p>(
        &mut self,
        p: impl Into<Paragraph<'p>>,
        t: &ResolvedText,
        width: Option<f32>,
    ) -> Vec<PlacedBox> {
        let p = p.into();
        let laid = self.laid_out(p, t, width);
        let cut = self.cut(p, &laid, t, width);
        let clamp = if t.truncate {
            Some(1)
        } else {
            t.line_clamp.map(usize::from)
        };
        let mut out = Vec::new();
        let mut lines = 0_usize;
        for (pi, (piece, top)) in laid.iter().enumerate() {
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
            if s.layout.is_empty() {
                lines = lines.saturating_add(1);
                continue;
            }
            for (li, (line, row)) in s.layout.lines().zip(&s.rows).enumerate() {
                let line_shown = clamp.is_none_or(|c| lines < c);
                lines = lines.saturating_add(1);
                let cut_line = cut.filter(|c| c.piece == pi && c.line == li);
                let dx = cut_line.map_or(s.shift, |c| c.dx);
                // A clamp's line: its kept piece's boxes show where that
                // lays them out (against the line's baseline); the line's
                // others do not.
                if let Some(c) = cut_line
                    && let Some(k) = c.prefix.and_then(|k| self.shaped.get(&k.hash))
                {
                    if let (Some(kl), Some(kr)) = (k.layout.get(0), k.rows.first()) {
                        let r = Row {
                            parley_top: kr.parley_top,
                            parley_baseline: kr.parley_baseline,
                            parley_height: kr.parley_height,
                            ..*row
                        };
                        for item in kl.items() {
                            if let PositionedLayoutItem::InlineBox(b) = item
                                && padding_of(b.id).is_none()
                            {
                                out.push(PlacedBox {
                                    id: b.id,
                                    bounds: Bounds {
                                        x: b.x + dx,
                                        y: top + r.place(b.y),
                                        width: b.width,
                                        height: b.height,
                                    },
                                    shown: line_shown,
                                });
                            }
                        }
                    }
                    for item in line.items() {
                        if let PositionedLayoutItem::InlineBox(b) = item
                            && padding_of(b.id).is_none()
                            && !c.keeps(b.x, b.width, box_range(s, b.id))
                        {
                            out.push(PlacedBox {
                                id: b.id,
                                bounds: Bounds {
                                    x: b.x + dx,
                                    y: top + row.place(b.y),
                                    width: b.width,
                                    height: b.height,
                                },
                                shown: false,
                            });
                        }
                    }
                    continue;
                }
                let mut packer = Packer::new(cut_line, &self.trims);
                let mut clusters = None;
                for item in line.items() {
                    match item {
                        PositionedLayoutItem::GlyphRun(run) if cut_line.is_some() => {
                            let mut cx = run.offset();
                            for c in run_clusters(&mut clusters, &run, || run.run().visual_clusters()) {
                                let r = c.text_range();
                                packer.place(cx, c.advance(), (r.start, r.end));
                                cx += c.advance();
                            }
                        }
                        PositionedLayoutItem::GlyphRun(_) => {}
                        PositionedLayoutItem::InlineBox(b) => {
                            let removed = packer.removed;
                            let placed = packer.place(b.x, b.width, box_range(s, b.id));
                            if padding_of(b.id).is_none() {
                                out.push(PlacedBox {
                                    id: b.id,
                                    bounds: Bounds {
                                        x: placed.unwrap_or(b.x - removed) + dx,
                                        y: top + row.place(b.y),
                                        width: b.width,
                                        height: b.height,
                                    },
                                    shown: line_shown && placed.is_some(),
                                });
                            }
                        }
                    }
                }
            }
        }
        // The boxes on lines the clamp hides, never laid out or broken
        // ([`Self::lay_out_into`]): hidden, below what shows.
        if let Some((last, top)) = laid.last() {
            let below = top + self.shaped.get(&last.hash).map_or(t.line_height, |s| height(s, t));
            let from = self.laid_end(last);
            out.extend(
                p.boxes
                    .iter()
                    .filter(|b| b.at >= from && p.content.get(b.at..).is_some_and(|r| r.starts_with(OBJECT)))
                    .map(|b| PlacedBox {
                        id: b.id,
                        bounds: Bounds {
                            x: 0.0,
                            y: below,
                            width: b.width,
                            height: b.height,
                        },
                        shown: false,
                    }),
            );
        }
        out
    }

    /// Where the caret at byte `at` goes by `step` (in a field of `width`),
    /// by the grapheme clusters, words and lines of the laid out text.
    pub fn step(&mut self, content: &str, t: &ResolvedText, width: Option<f32>, at: usize, step: Step) -> usize {
        let laid = self.laid_out(content.into(), t, width);
        let Some(pi) = laid.iter().position(|(p, _)| at <= p.end).or(laid.len().checked_sub(1)) else {
            return at;
        };
        let Some((piece, top)) = laid.get(pi).copied() else {
            return at;
        };
        let Some(s) = self.shaped.get(&piece.hash) else {
            return at;
        };
        let layout = &s.layout;
        let local = at.saturating_sub(piece.start).min(s.content.len());
        let here = Selection::from_byte_index(layout, local, Affinity::Downstream);
        let len = s.content.len();
        let rtl = layout.is_rtl();
        let prev_piece = || pi.checked_sub(1).and_then(|i| laid.get(i)).map(|(p, _)| p.end);
        let next_piece = || laid.get(pi.saturating_add(1)).map(|(p, _)| p.start);
        let moved = |sel: Selection| piece.start.saturating_add(sel.focus().index());
        // Past a piece's first or last byte, a step lands in the one beside it.
        let back = |to: usize| if to == at { prev_piece().unwrap_or(at) } else { to };
        let forward = |to: usize| if to == at { next_piece().unwrap_or(at) } else { to };
        match step {
            Step::Left if rtl => forward(moved(here.previous_visual(layout, false))),
            Step::Right if rtl => back(moved(here.next_visual(layout, false))),
            Step::Left => back(moved(here.previous_visual(layout, false))),
            Step::Right => forward(moved(here.next_visual(layout, false))),
            Step::WordLeft => back(moved(here.previous_visual_word(layout, false))),
            Step::WordRight => forward(moved(here.next_visual_word(layout, false))),
            Step::LineStart => moved(here.line_start(layout, false)),
            Step::LineEnd => moved(here.line_end(layout, false)),
            Step::Up | Step::Down => {
                let up = step == Step::Up;
                let moved_in = moved(here.move_lines(layout, if up { -1 } else { 1 }, false));
                let line = ParleyCursor::from_byte_index(layout, local, Affinity::Downstream).geometry(layout, 1.0);
                let first = layout
                    .get(0)
                    .is_none_or(|l| (crate::convert::f64_to_f32(line.y0) - l.metrics().block_min_coord).abs() < 0.5);
                let last = layout
                    .lines()
                    .next_back()
                    .is_none_or(|l| (crate::convert::f64_to_f32(line.y1) - l.metrics().block_max_coord).abs() < 0.5);
                let leaves = if up { first } else { last };
                if !leaves {
                    return moved_in;
                }
                // Into the piece above or below, at the same x.
                let x = crate::convert::f64_to_f32(line.x0) + s.shift;
                let y = if up { top - 0.5 } else { top + height(s, t) + 0.5 };
                if (up && pi == 0) || (!up && pi.saturating_add(1) >= laid.len()) {
                    return if up { 0 } else { content.len() };
                }
                self.hit(content, t, width, x, y)
            }
            Step::DeleteBack => {
                if local == 0 {
                    return prev_piece().unwrap_or(0);
                }
                let [before, _] = here.focus().logical_clusters(layout);
                match before {
                    Some(c) if c.is_emoji() || c.is_hard_line_break() => {
                        piece.start.saturating_add(c.text_range().start)
                    }
                    _ => s
                        .content
                        .get(..local)
                        .and_then(|b| b.char_indices().next_back())
                        .map_or(at, |(i, _)| piece.start.saturating_add(i)),
                }
            }
            Step::DeleteForward => {
                if local >= len {
                    return next_piece().unwrap_or(at);
                }
                let [_, after] = here.focus().logical_clusters(layout);
                after.map_or(at, |c| piece.start.saturating_add(c.text_range().end))
            }
            Step::DeleteWordBack => {
                if local == 0 {
                    return prev_piece().unwrap_or(0);
                }
                piece
                    .start
                    .saturating_add(here.focus().previous_logical_word(layout).index())
            }
            Step::DeleteWordForward => {
                if local >= len {
                    return next_piece().unwrap_or(at);
                }
                piece
                    .start
                    .saturating_add(here.focus().next_logical_word(layout).index())
            }
        }
    }

    /// Keep `p` shaped as [`TextSystem::draw`] would at `width`, drawing
    /// nothing: a paragraph laid out just outside its clip (a virtual list's
    /// prefetched rows) is in use, so it is not shaped again when it
    /// scrolls in (ADR-0008's two-frame rule, ADR-0014).
    pub fn keep<'p>(&mut self, p: impl Into<Paragraph<'p>>, t: &ResolvedText, width: Option<f32>) {
        let mut laid = std::mem::take(&mut self.scratch.laid);
        self.lay_out_into(p.into(), t, width, laid.reuse());
        self.scratch.laid = laid;
    }

    /// Draw `p` in style `t` with its box's top-left at `origin` (logical
    /// pixels), every colour's alpha times `opacity`.
    ///
    /// Drawn again (a hover, a scroll) it allocates nothing once warm: the
    /// glyphs go straight into the target's items, and what it works in
    /// (the pieces, a line's runs, the glyphs an underline skips, the
    /// gaps, the lines and backgrounds held until the glyphs are down) is
    /// scratch the system keeps ([`DrawScratch`]).
    pub fn draw<'p>(
        &mut self,
        target: &mut Target<'_>,
        p: impl Into<Paragraph<'p>>,
        t: &ResolvedText,
        width: Option<f32>,
        origin: (f32, f32),
        opacity: f32,
    ) {
        let p = p.into();
        let mut sc = std::mem::take(&mut self.scratch);
        self.lay_out_into(p, t, width, sc.laid.reuse());
        let laid = &sc.laid.buf;
        let cut = self.cut(p, laid, t, width);
        let trims = std::mem::take(&mut self.trims);
        let ellipsis = cut.map(|_| self.piece(Paragraph::from(ELLIPSIS), t, 0, ELLIPSIS.len()));
        self.drawn.buf.extend(
            laid.iter()
                .map(|(piece, _)| piece.hash)
                .chain(ellipsis)
                .chain(cut.and_then(|c| c.probe))
                .chain(cut.and_then(|c| c.prefix.map(|k| k.hash))),
        );
        if let Some(e) = ellipsis {
            self.break_piece(e, Breaking::ONE_LINE);
        }
        let runs = p.runs;
        let style_of = |brush: u32| brush.checked_sub(1).and_then(|i| runs.get(usize::try_from(i).ok()?));
        let scale = target.scale;
        // Hinted below 2x, as Chrome's FreeType and DirectWrite text is;
        // on macOS Chrome's CoreText text is not hinted, and so not ours.
        let hinted = scale < 2.0 && !cfg!(target_os = "macos");
        // ADR-0020: on Linux at 1× glyph origins round to whole pixels, as Chrome does there.
        let whole = self.whole_origins && (scale - 1.0).abs() < f32::EPSILON;
        // Where a glyph's origin goes, device x: a whole pixel and the
        // quarter past it, the ellipsis's as the text's.
        let snap = |x: f32| if whole { (x.round(), 0) } else { quarter(x) };
        let visible = target.cull;
        let clamp = if t.truncate {
            Some(1)
        } else {
            t.line_clamp.map(usize::from)
        };
        // Whether any line has a background or a decoration to find.
        let decorates = !t.decorations.is_empty()
            || runs
                .iter()
                .any(|r| r.background.is_some() || !r.text.decorations.is_empty());
        // The ascent of each paragraph decorating box's primary font, where
        // it is not the paragraph's own (a struck serif heading around a
        // monospace paragraph): where its line-through sits (Blink's
        // decorating box), read from a probe shaped in that font.
        let boxes = sc.boxes.reuse();
        t.decorations.collect(boxes);
        let paragraph_underline = boxes.iter().any(|d| d.lines.underline());
        let through_ascents = sc.ascents.reuse();
        for d in boxes.iter() {
            let mut ascent = None;
            let own_font = (d.size, d.family, d.weight, d.italic) == (t.size, t.family, t.weight, t.italic);
            if d.lines.line_through() && !own_font {
                let font = d.font_in(t);
                let probe = self.piece(Paragraph::from(PROBE), &font, 0, PROBE.len());
                self.break_piece(probe, Breaking::ONE_LINE);
                self.drawn.buf.push(probe);
                ascent = self
                    .shaped
                    .get(&probe)
                    .and_then(|s| s.layout.root_span_metrics())
                    .map(|m| m.ascent);
            }
            through_ascents.push(ascent);
        }
        let mut shown = 0_usize;
        'pieces: for (pi, (piece, top)) in laid.iter().enumerate() {
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
            let layout = &s.layout;
            if layout.is_empty() {
                shown = shown.saturating_add(1);
                continue;
            }
            for (li, (line, row)) in layout.lines().zip(&s.rows).enumerate() {
                if clamp.is_some_and(|c| shown >= c) {
                    break 'pieces;
                }
                shown = shown.saturating_add(1);
                let cut_line = cut.filter(|c| c.piece == pi && c.line == li);
                // A clamp's line is drawn from what it keeps, laid out as a
                // line of its own ([`Apart::Kept`]), on the line's baseline;
                // a truncated line is closed up as it is cut ([`Packer`]).
                let kept = cut_line.and_then(|c| c.prefix).and_then(|k| {
                    let ks = self.shaped.get(&k.hash)?;
                    Some((ks, ks.layout.get(0)?, k))
                });
                let (s, line, piece, pack) = match kept {
                    Some((ks, kl, k)) => (ks, kl, k, None),
                    None => (s, line, *piece, cut_line),
                };
                let m = line.metrics();
                let line_top = top + row.top;
                // A long text in a scroll container: only the lines in view
                // cost anything; a line's ink may reach past its box, so it
                // counts as shown a line's height either side.
                if let Some(v) = visible {
                    let above = origin.1 + line_top - m.line_height;
                    let below = origin.1 + line_top + m.line_height * 2.0;
                    if below < v.y || above > v.bottom() {
                        continue;
                    }
                }
                // Blink lays decorations out on the unrounded baseline.
                let line_base = origin.1 + line_top + row.baseline;
                let device = baseline(line_base, scale);
                let base = device / scale;
                // A cut line is placed with its ellipsis (css-overflow-4).
                let x0 = origin.0 + cut_line.map_or(s.shift, |c| c.dx);
                // Chrome paints a line's run backgrounds, then its
                // underlines, under its glyphs, and its line-throughs over
                // them: the backgrounds and lines are found first, from the
                // glyphs' places, and put down before any glyph, which so
                // go into the target once and are never moved.
                if decorates {
                    let groups = sc.groups.reuse();
                    // The glyphs an underline skips the ink of, by their group
                    // (in the order of the groups).
                    let inked = sc.inked.reuse();
                    // Whether the next run's stretch may join the one
                    // before: not across an atomic inline, which CSS
                    // decorates no line through.
                    let mut joins = true;
                    let mut clusters = None;
                    let mut packer = Packer::new(pack, &trims);
                    for item in line.items() {
                        match item {
                            PositionedLayoutItem::GlyphRun(run) => {
                                let brush = run.style().brush;
                                let own = style_of(brush);
                                let face = self.fonts.face(run.run().font());
                                let axes = self
                                    .fonts
                                    .axes(run.run().normalized_coords().iter().map(|c| c.to_bits()));
                                let mut end = run.offset() - packer.removed;
                                let mut whole_run = true;
                                // Under a line: the paragraph's (which no run
                                // takes off) or the run's own.
                                let skips = paragraph_underline || own.is_some_and(|r| r.text.decorations.underline());
                                let skew = run.run().synthesis().skew().map_or(0.0, |d| d.to_radians().tan());
                                let ink_from = inked.len();
                                // The left edge of its first cluster kept.
                                let mut first = None;
                                let mut any = false;
                                // Where each cluster starts, and where each
                                // glyph's pen is (a ligature's glyphs are its
                                // first cluster's).
                                let (mut cx, mut gx) = (run.offset(), run.offset());
                                for c in run_clusters(&mut clusters, &run, || run.run().visual_clusters()) {
                                    any = true;
                                    let advance = c.advance();
                                    let r = c.text_range();
                                    let shift = packer.removed;
                                    let placed = packer.place(cx, advance, (r.start, r.end));
                                    let kept = placed.is_some();
                                    let trim = pack.map_or(0.0, |_| trim_of(&trims, r.end));
                                    // Skip-ink leaves CJK alone.
                                    let cjk = skips
                                        && s.content
                                            .get(c.text_range())
                                            .and_then(|t| t.chars().next())
                                            .is_some_and(ink::cjk);
                                    for g in c.glyphs() {
                                        if kept && skips && !cjk && !self.fonts.is_color(face, g.id) {
                                            // Its group is known once the run's is pushed.
                                            inked.push((
                                                usize::MAX,
                                                Inked {
                                                    font: face,
                                                    glyph: g.id,
                                                    axes,
                                                    size: run.run().font_size(),
                                                    skew,
                                                    x: x0 + gx - shift + g.x,
                                                    dy: (g.y + run.baseline()) - m.baseline,
                                                },
                                            ));
                                        }
                                        gx += g.advance;
                                    }
                                    if let Some(px) = placed {
                                        first.get_or_insert(px);
                                        end = px + advance - trim;
                                    } else {
                                        whole_run = false;
                                    }
                                    cx += advance;
                                }
                                // A run the cut left nothing of has no box:
                                // no background or line behind the ellipsis.
                                if any && first.is_none() {
                                    continue;
                                }
                                let start = first.unwrap_or(end);
                                let span = run.span_metrics();
                                let mut group = Group {
                                    brush,
                                    x: x0 + start,
                                    width: end - start,
                                    ink: (x0 + start, end - start),
                                    ascent: span.ascent,
                                    descent: span.descent,
                                };
                                // A run's padding is letter spacing after the
                                // character before it and after its last one
                                // (`Self::shape`): its box, not its ink, and not
                                // the box of the text before it.
                                if let Some(r) = own {
                                    let range = run.run().text_range();
                                    let local = |i: usize| i.saturating_sub(piece.start).saturating_add(s.key.lead);
                                    let (left, right) = (r.padding.left, r.padding.right);
                                    if right > 0.0
                                        && whole_run
                                        && r.range.end <= piece.end
                                        && range.end == local(r.range.end)
                                    {
                                        group.ink.1 = (group.ink.1 - right).max(0.0);
                                    }
                                    if left > 0.0
                                        && r.range.start > piece.start
                                        && range.start == local(r.range.start)
                                        && let Some(before) = groups.last_mut()
                                    {
                                        before.width = (before.width - left).max(0.0);
                                        before.ink.1 = (before.ink.1 - left).max(0.0);
                                        group.x -= left;
                                        group.width += left;
                                    }
                                }
                                push_group(groups, group, joins);
                                joins = true;
                                let at = groups.len().saturating_sub(1);
                                for i in inked.get_mut(ink_from..).unwrap_or_default() {
                                    i.0 = at;
                                }
                            }
                            PositionedLayoutItem::InlineBox(b) => {
                                match (padding_of(b.id), packer.place(b.x, b.width, box_range(s, b.id))) {
                                    // A run's padding, where the cut keeps it.
                                    (Some(index), Some(bx)) => {
                                        let r = runs.get(usize::try_from(index).unwrap_or(usize::MAX));
                                        let size = r.map_or(t.size, |r| r.text.size);
                                        push_group(
                                            groups,
                                            Group {
                                                brush: index.saturating_add(1),
                                                x: x0 + bx,
                                                width: b.width,
                                                ink: (x0 + bx, 0.0),
                                                ascent: size * 0.97,
                                                descent: size * 0.24,
                                            },
                                            true,
                                        );
                                    }
                                    (Some(_), None) => {}
                                    (None, _) => joins = false,
                                }
                            }
                        }
                    }
                    // CSS decorates no hanging space at a line's end.
                    if let Some(last) = groups.last_mut() {
                        last.width = (last.width - m.hanging_advance).max(0.0);
                        last.ink.1 = (last.ink.1 - m.hanging_advance).max(0.0);
                    }
                    // The paragraph's primary font's ascent: a line-through
                    // declared by a box of its font is placed by it.
                    let root_ascent = layout.root_span_metrics().map(|m| m.ascent);
                    let backgrounds = sc.backgrounds.reuse();
                    let mut lines = Decorator {
                        fonts: &mut self.fonts,
                        inked,
                        cuts: &mut sc.cuts,
                        pieces: &mut sc.segments,
                        under: sc.under.reuse(),
                        through: sc.through.reuse(),
                        base: line_base,
                        scale,
                    };
                    for (gi, g) in groups.iter().enumerate() {
                        let own = style_of(g.brush);
                        let ink = g.ink;
                        if ink.1 > 0.0 {
                            // CSS draws a decoration by the box that declares it
                            // (Blink's decorating box), over all it holds: the
                            // paragraph's (its own or an enclosing box's) in
                            // that box's colour and by its font under every
                            // run, which cannot take it off; a run's own,
                            // declared besides, by the run's font over the run.
                            // Each box's lines, outermost first.
                            for (di, d) in sc.boxes.buf.iter().enumerate() {
                                if d.lines.underline() {
                                    lines.underline(gi, d, opacity, ink);
                                }
                                if d.lines.line_through() {
                                    // The declaring box's primary font's.
                                    let ascent = sc
                                        .ascents
                                        .buf
                                        .get(di)
                                        .copied()
                                        .flatten()
                                        .or(root_ascent)
                                        .unwrap_or(g.ascent);
                                    lines.line_through(g.ascent, ascent, d, opacity, ink);
                                }
                            }
                            if let Some(r) = own {
                                r.text.decorations.collect(sc.own.reuse());
                            }
                            for d in own.iter().flat_map(|_| sc.own.buf.iter()) {
                                if d.lines.underline() {
                                    lines.underline(gi, d, opacity, ink);
                                }
                                if d.lines.line_through() {
                                    lines.line_through(g.ascent, g.ascent, d, opacity, ink);
                                }
                            }
                        } else if cut_line.is_some() {
                            // Padding alone, its run cut away.
                            continue;
                        }
                        if let Some(run) = own
                            && let Some(bg) = run.background
                        {
                            let top = base - g.ascent - run.padding.top;
                            backgrounds.push((
                                Bounds {
                                    x: g.x,
                                    y: top,
                                    width: g.width,
                                    height: base + g.descent + run.padding.bottom - top,
                                },
                                run.radius,
                                bg.opacity(opacity),
                            ));
                        }
                    }
                    for &(b, radius, color) in backgrounds.iter() {
                        target.fill_rounded(b, radius, color);
                    }
                    for &(b, color) in lines.under.iter() {
                        target.fill_rect(b, color);
                    }
                }
                let mut clusters = None;
                let mut packer = Packer::new(pack, &trims);
                for item in line.items() {
                    let run = match item {
                        PositionedLayoutItem::GlyphRun(run) => run,
                        PositionedLayoutItem::InlineBox(b) => {
                            packer.place(b.x, b.width, box_range(s, b.id));
                            continue;
                        }
                    };
                    let style = style_of(run.style().brush).map_or(t, |r| &r.text);
                    let color = style.color.opacity(opacity);
                    let face = self.fonts.face(run.run().font());
                    let axes = self
                        .fonts
                        .axes(run.run().normalized_coords().iter().map(|c| c.to_bits()));
                    let synthesis = run.run().synthesis();
                    let ppem = run.run().font_size() * scale;
                    let skew = synthesis
                        .skew()
                        .map_or(0, |d| i8::try_from(f32_to_i32(d.round())).unwrap_or(0));
                    // Whole clusters kept or dropped; a ligature's
                    // glyphs are its first cluster's.
                    let (mut cx, mut x) = (run.offset(), run.offset());
                    for c in run_clusters(&mut clusters, &run, || run.run().visual_clusters()) {
                        let advance = c.advance();
                        let r = c.text_range();
                        let shift = packer.removed;
                        let kept = packer.place(cx, advance, (r.start, r.end)).is_some();
                        cx += advance;
                        for g in c.glyphs() {
                            if !kept {
                                x += g.advance;
                                continue;
                            }
                            let gx = (x0 + x - shift + g.x) * scale;
                            let gy = device + ((g.y + run.baseline()) - m.baseline) * scale;
                            let (px, bin) = snap(gx);
                            let mut flags = 0;
                            if hinted {
                                flags |= GlyphKey::HINTED;
                            }
                            if synthesis.embolden() {
                                flags |= GlyphKey::EMBOLDEN;
                            }
                            if self.fonts.is_color(face, g.id) {
                                flags |= GlyphKey::COLOR;
                            }
                            target.items.push(Item::Glyph {
                                key: GlyphKey {
                                    font: face,
                                    glyph: g.id,
                                    ppem: ppem.to_bits(),
                                    axes,
                                    bin,
                                    flags,
                                    skew,
                                },
                                x: f32_to_i32(px),
                                y: f32_to_i32(gy.round()),
                                color,
                            });
                            x += g.advance;
                        }
                    }
                }
                // Chrome decorates no ellipsis, not even under a line the
                // block declares (measured, Chrome 145).
                if let (Some(c), Some(e)) = (cut_line, ellipsis) {
                    let ex = origin.0 + c.ellipsis_at;
                    if let Some(es) = self.shaped.get(&e) {
                        for eline in es.layout.lines() {
                            for item in eline.items() {
                                let PositionedLayoutItem::GlyphRun(run) = item else {
                                    continue;
                                };
                                let face = self.fonts.face(run.run().font());
                                let axes = self
                                    .fonts
                                    .axes(run.run().normalized_coords().iter().map(|c| c.to_bits()));
                                let ppem = run.run().font_size() * scale;
                                let mut x = run.offset();
                                for g in run.glyphs() {
                                    let (px, bin) = snap((ex + x + g.x) * scale);
                                    target.items.push(Item::Glyph {
                                        key: GlyphKey {
                                            font: face,
                                            glyph: g.id,
                                            ppem: ppem.to_bits(),
                                            axes,
                                            bin,
                                            flags: if hinted { GlyphKey::HINTED } else { 0 },
                                            skew: 0,
                                        },
                                        x: f32_to_i32(px),
                                        y: f32_to_i32(device),
                                        color: t.color.opacity(opacity),
                                    });
                                    x += g.advance;
                                }
                            }
                        }
                    }
                }
                if decorates {
                    for &(b, color) in &sc.through.buf {
                        target.fill_rect(b, color);
                    }
                }
            }
        }
        self.trims = trims;
        self.scratch = sc;
    }

    /// Forget the shaped text the last frames did not use; call once per frame.
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        let pinned = &self.pinned;
        self.shaped
            .retain(|hash, s| frame.saturating_sub(s.used) < 2 || pinned.contains_key(hash));
        self.fits.retain(|_, f| frame.saturating_sub(f.1) < 2);
        self.bound_apart();
        self.bound_hidden();
        self.frame = frame.saturating_add(1);
        self.drawn.clear();
        self.drawn.trim();
        self.scratch.trim();
        TextDecorations::sweep();
    }

    /// Keep the pieces shaped apart ([`Apart`]) to a bound that follows
    /// the working set (what this frame drew, and what retained chunks
    /// show: pinned): at most [`APART_MAX`], or twice the working set if
    /// more. Past it, the least recently used of the rest go; the working
    /// set never does, so what shows is never shaped again (a frame that
    /// draws nothing new, its chunks retained, keeps them too).
    fn bound_apart(&mut self) {
        let frame = self.frame;
        let pinned = &self.pinned;
        let (mut apart, mut used) = (0_usize, 0_usize);
        for (h, s) in self.shaped.iter().filter(|(_, s)| s.key.apart != Apart::No) {
            apart = apart.saturating_add(1);
            if s.used >= frame || pinned.contains_key(h) {
                used = used.saturating_add(1);
            }
        }
        let bound = APART_MAX.max(used.saturating_mul(2));
        let Some(over) = apart.checked_sub(bound).filter(|&n| n > 0) else {
            return;
        };
        let mut old = std::mem::take(&mut self.apart_order);
        old.clear();
        old.extend(
            self.shaped
                .iter()
                .filter(|(h, s)| s.key.apart != Apart::No && s.used < frame && !pinned.contains_key(*h))
                .map(|(h, s)| (s.used, *h)),
        );
        old.sort_unstable();
        for (_, h) in old.iter().take(over) {
            self.shaped.remove(h);
        }
        self.apart_order = old;
    }

    /// Keep the texts whose hidden lines are kept ([`access::HiddenLines`])
    /// to a bound that follows the working set (the texts this frame
    /// read): at most [`HIDDEN_MAX`], or twice the working set if more.
    /// Past it, the least recently read of the rest go; the working set
    /// never does, so fields read by turns are never cut again.
    fn bound_hidden(&mut self) {
        let frame = self.frame;
        let used = self.hidden_lines.values().filter(|h| h.used >= frame).count();
        let bound = HIDDEN_MAX.max(used.saturating_mul(2));
        let Some(over) = self.hidden_lines.len().checked_sub(bound).filter(|&n| n > 0) else {
            return;
        };
        let mut old = std::mem::take(&mut self.hidden_order);
        old.clear();
        old.extend(
            self.hidden_lines
                .iter()
                .filter(|(_, h)| h.used < frame)
                .map(|(k, h)| (h.used, *k)),
        );
        old.sort_unstable();
        for (_, k) in old.iter().take(over) {
            self.hidden_lines.remove(k);
        }
        self.hidden_order = old;
    }

    /// The pieces [`TextSystem::draw`] drew since last asked: what a paint
    /// chunk just recorded shows (a new chunk's; one recorded again takes
    /// them with [`TextSystem::swap_drawn`]). Copied out at their length:
    /// the system's room, sized by the most any chunk drew, stays here.
    pub fn take_drawn(&mut self) -> Vec<u64> {
        let pieces = self.drawn.buf.clone();
        self.drawn.clear();
        pieces
    }

    /// Forget the pieces drawn since last asked, keeping their room.
    pub fn clear_drawn(&mut self) {
        self.drawn.clear();
    }

    /// Keep shaped the pieces drawn since last asked ([`TextSystem::pin`]).
    pub fn pin_drawn(&mut self) {
        for p in &self.drawn.buf {
            let n = self.pinned.entry(*p).or_insert(0);
            *n = n.saturating_add(1);
        }
    }

    /// The pieces drawn since last asked, copied into `kept` (a chunk
    /// recorded again: what it showed before), whose room is reused: a
    /// chunk's room is the most it ever drew, never another chunk's (a
    /// hovered row recorded after a 10k-piece textarea keeps a row's).
    pub fn swap_drawn(&mut self, kept: &mut Vec<u64>) {
        kept.clear();
        kept.extend_from_slice(&self.drawn.buf);
        self.drawn.clear();
    }

    /// Keep `pieces` shaped while a retained chunk shows them: a chunk
    /// recorded again (a hover) shapes nothing again (ADR-0016).
    pub fn pin(&mut self, pieces: &[u64]) {
        for p in pieces {
            let n = self.pinned.entry(*p).or_insert(0);
            *n = n.saturating_add(1);
        }
    }

    /// Let go of `pieces` pinned by [`TextSystem::pin`].
    pub fn unpin(&mut self, pieces: &[u64]) {
        for p in pieces {
            if let Some(n) = self.pinned.get_mut(p) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    self.pinned.remove(p);
                }
            }
        }
    }

    /// Forget every piece it shaped (a layout pool thread's, at a new pass:
    /// the layout tree keeps the answers they gave).
    pub(crate) fn forget_shaped(&mut self) {
        // Its room too: a pool thread that shaped a big table once does
        // not keep a table's worth of slots while idle.
        self.shaped = HashMap::default();
        // And where clamps kept their lines to: a worker never ends a
        // frame, so nothing else would let these go.
        self.fits = HashMap::default();
        self.hidden_lines = HashMap::default();
        self.frame = self.frame.saturating_add(1);
    }

    /// Pieces of text held shaped.
    pub(crate) fn footprint(&self) -> usize {
        self.shaped.len()
    }

    /// Pieces shaped since this was last asked: a resize or a re-layout
    /// reshapes nothing, a keystroke reshapes the piece it changed.
    pub fn take_shaped(&mut self) -> u64 {
        std::mem::take(&mut self.shapes)
    }

    /// Count `n` pieces another thread's text system shaped for this frame.
    pub(crate) fn add_shaped(&mut self, n: u64) {
        self.shapes = self.shapes.saturating_add(n);
    }

    /// What a renderer's [`Glyphs`] lacks to draw what this system laid out
    /// since the last update: the faces and coordinate sets interned since,
    /// and the settings when they changed (everything when `all`, for a
    /// renderer made anew).
    pub fn glyph_update(&mut self, all: bool) -> GlyphUpdate {
        let (faces, axes) = self.fonts.glyph_tables(all);
        let settings = (all || self.sent_generation != Some(self.generation)).then(|| GlyphSettings {
            settings: self.settings,
            gamma: self.gamma,
            table: std::sync::Arc::clone(&self.table),
            generation: self.generation,
        });
        self.sent_generation = Some(self.generation);
        GlyphUpdate { faces, axes, settings }
    }

    /// A renderer's glyph tables, up to everything this system has interned
    /// (a renderer of its own: tests, a capture aside). Updates sent before
    /// are unaffected.
    #[must_use]
    pub fn glyphs(&mut self) -> Glyphs {
        let sent = self.sent_generation;
        let saved = self.fonts.sent_tables();
        let mut glyphs = Glyphs::new(
            self.settings,
            self.gamma,
            std::sync::Arc::clone(&self.table),
            self.generation,
        );
        glyphs.apply(self.glyph_update(true));
        self.fonts.restore_sent(saved);
        self.sent_generation = sent;
        glyphs
    }
}

/// What a renderer's [`Glyphs`] must take in to draw a commit's glyphs
/// (ADR-0015: sent with it, behind `Arc`s; font files are shared, never copied).
#[derive(Clone, Default)]
pub struct GlyphUpdate {
    pub(crate) faces: Vec<parley::FontData>,
    pub(crate) axes: Vec<Vec<i16>>,
    pub(crate) settings: Option<GlyphSettings>,
}

impl std::fmt::Debug for GlyphUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphUpdate")
            .field("faces", &self.faces.len())
            .field("axes", &self.axes.len())
            .field("settings", &self.settings.as_ref().map(|s| s.generation))
            .finish()
    }
}

impl GlyphUpdate {
    /// Nothing to take in.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty() && self.axes.is_empty() && self.settings.is_none()
    }
}

/// The settings glyphs are drawn with, as of a generation.
#[derive(Clone)]
pub(crate) struct GlyphSettings {
    pub(crate) settings: TextSettings,
    pub(crate) gamma: GammaParams,
    pub(crate) table: std::sync::Arc<MaskGamma>,
    pub(crate) generation: u64,
}

/// The last shown line of a paragraph cut short: which piece and line,
/// what of it is kept, the kept text's width (closed up by [`Packer`], or
/// laid out alone), where the line goes in the box (its x in the box is
/// Parley's, closed up, plus `dx`, in place of the piece's alignment
/// shift), where the ellipsis starts in the box, its width, and the pieces
/// a line clamp shapes apart ([`Apart`]), kept shaped while the paragraph
/// is drawn.
#[derive(Clone, Copy, Debug)]
struct Cut {
    piece: usize,
    line: usize,
    keep: Keep,
    kept: f32,
    dx: f32,
    ellipsis_at: f32,
    ellipsis: f32,
    probe: Option<u64>,
    /// What a clamp keeps, shaped apart as a line of its own and drawn in
    /// the line's place ([`Apart::Kept`]): its bytes in the paragraph.
    prefix: Option<Piece>,
}

/// The padding of a dropped run the kept unit ending at byte `end` holds
/// in its advance, taken out of the kept text (else 0), of `trims`: what
/// a truncation drops of the runs kept units hold padding for
/// ([`TextSystem::trims`]), sorted by byte.
fn trim_of(trims: &[(usize, f32)], end: usize) -> f32 {
    let from = trims.partition_point(|t| t.0 < end);
    trims
        .get(from..)
        .map_or(0.0, |rest| rest.iter().take_while(|t| t.0 == end).map(|t| t.1).sum())
}

/// One unit of a cut line ([`units`]): its x and advance, whether it is
/// whitespace, its bytes, the left padding it holds of the runs starting
/// where it ends, the first unit of those runs, and the unit holding its
/// run's padding (indices in visual order).
#[derive(Clone, Copy, Debug)]
struct Unit {
    x: f32,
    advance: f32,
    space: bool,
    range: (usize, usize),
    pad: f32,
    next: Option<usize>,
    owner: Option<usize>,
}

/// A cut line's units in visual order, and their order by bytes, in
/// buffers a [`TextSystem`] keeps.
#[derive(Default)]
struct Units {
    units: Vec<Unit>,
    /// (first byte, index), sorted by byte.
    order: Vec<(usize, usize)>,
}

impl Units {
    /// Fill with line `l` of `s`'s units, each with the padding it holds.
    fn fill(&mut self, s: &Shaped, l: &parley::Line<'_, u32>) {
        self.units.clear();
        self.order.clear();
        let lead = s.key.lead;
        let out = &mut self.units;
        units(s, l, |x, advance, space, range| {
            // The left padding of the runs that start where it ends, which
            // Parley adds after its character (`TextSystem::shape`), or
            // which the lead box before a leading run is.
            let pad = s
                .key
                .runs
                .iter()
                .filter(|r| r.starts && (r.range.0 > 0 || lead > 0) && r.range.0.saturating_add(lead) == range.1)
                .map(|r| f32::from_bits(r.padding.0))
                .sum();
            out.push(Unit {
                x,
                advance,
                space,
                range,
                pad,
                next: None,
                owner: None,
            });
        });
        self.order
            .extend(self.units.iter().enumerate().map(|(i, u)| (u.range.0, i)));
        self.order.sort_unstable_by_key(|o| o.0);
        for i in 0..self.units.len() {
            let Some(&Unit { pad, range, .. }) = self.units.get(i) else {
                continue;
            };
            if pad <= 0.0 {
                continue;
            }
            // The run's first unit that takes room: one that draws nothing
            // (a bidi isolate's mark) shows none of it.
            let mut next = None;
            let mut at = range.1;
            for _ in 0..self.units.len() {
                let Some(j) = self
                    .order
                    .binary_search_by_key(&at, |o| o.0)
                    .ok()
                    .and_then(|k| self.order.get(k))
                    .map(|o| o.1)
                else {
                    break;
                };
                let Some(u) = self.units.get(j) else { break };
                next = Some(j);
                if u.advance > 0.0 || u.range.1 <= at {
                    break;
                }
                at = u.range.1;
            }
            if let Some(u) = self.units.get_mut(i) {
                u.next = next;
            }
            if let Some(u) = next.and_then(|j| self.units.get_mut(j)) {
                u.owner = Some(i);
            }
        }
    }
}

/// What a cut line keeps.
#[derive(Clone, Copy, Debug)]
enum Keep {
    /// `text-overflow` (css-overflow-3 §5.1): the units visually between
    /// these x (Parley's), from the line's start edge.
    Visual { from: f32, to: f32 },
    /// css-overflow-4 `block-ellipsis`: the logical prefix of the line
    /// ending at this byte of its layout's text.
    Logical { end: usize },
}

impl Cut {
    /// Whether the cluster (or box) at `x`, `advance` wide, over bytes
    /// `range` of its layout's text, is kept. Its x is compared to x found
    /// by the same walk ([`units`]), so the slack is only float noise.
    fn keeps(&self, x: f32, advance: f32, range: (usize, usize)) -> bool {
        match self.keep {
            Keep::Visual { from, to } => x >= from - noise(from) && x + advance <= to + noise(to),
            Keep::Logical { end } => range.1 <= end,
        }
    }
}

/// The units truncated line `c` of `s` keeps, in visual order, as drawn:
/// each's x in the box and width shown (its dropped runs' padding out),
/// and its x in Parley's layout, advance, and bytes.
fn placed_units(s: &Shaped, c: Cut, trims: &[(usize, f32)], mut f: impl FnMut(f32, f32, f32, f32, (usize, usize))) {
    let Some(l) = s.layout.get(c.line) else { return };
    let mut packer = Packer::new(Some(c), trims);
    units(s, &l, |x, advance, _, range| {
        if let Some(at) = packer.place(x, advance, range) {
            f(at + c.dx, advance - trim_of(trims, range.1), x, advance, range);
        }
    });
}

/// Where the kept units of a line go: each moved toward the line's left by
/// the dropped units visually before it, so what a cut keeps closes up as
/// if laid out alone (none dropped on a line not cut).
struct Packer<'t> {
    cut: Option<Cut>,
    /// The cut's padding taken out ([`trim_of`]).
    trims: &'t [(usize, f32)],
    /// The advance of the units dropped so far.
    removed: f32,
}

impl<'t> Packer<'t> {
    fn new(cut: Option<Cut>, trims: &'t [(usize, f32)]) -> Self {
        Self {
            cut,
            trims,
            removed: 0.0,
        }
    }

    /// Where the unit at `x`, `advance` wide, over bytes `range`, is drawn
    /// (in Parley's x), or none when the cut drops it. Called for each
    /// cluster and box of the line, in visual order.
    /// The padding of a dropped run a kept unit holds closes up too: the
    /// units after it move by it.
    fn place(&mut self, x: f32, advance: f32, range: (usize, usize)) -> Option<f32> {
        match self.cut {
            Some(c) if !c.keeps(x, advance, range) => {
                self.removed += advance;
                None
            }
            Some(_) => {
                let at = x - self.removed;
                self.removed += trim_of(self.trims, range.1);
                Some(at)
            }
            None => Some(x),
        }
    }
}

/// A prefix a clamp may keep of a line: where it ends (a unit's end, a
/// byte of the paragraph), its width in the line, and whether it ends a
/// word.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Prefix {
    end: usize,
    inline: f32,
    closes: bool,
}

/// The most prefixes [`word_fit`] shapes alone, however long the line.
pub(super) const FIT_SHAPES: usize = 3;

/// How many of `prefixes` a clamp keeps (0 to their count), by the line's
/// own cluster advances, walked once: the longest whose width in the line
/// fits `room`. Shaping does not reach across a word boundary, so one
/// ending a word is as wide alone; one cut inside a word is shaped alone
/// (`alone(k)`, its width), as it can shape otherwise there (Arabic's last
/// letter kept turns final). Wider so, the cut steps back a cluster at a
/// time by the advances it has, by what shaping alone added, and is shaped
/// again; after [`FIT_SHAPES`] shapes, the last whole word that fits, or,
/// no whole word fitting, the first unit when that fits alone (one shape
/// more), not nothing. It never overflows: what it keeps fits shaped
/// alone, or ends a word.
fn word_fit(prefixes: &[Prefix], room: f32, mut alone: impl FnMut(usize) -> f32) -> usize {
    // The prefix of the first `k` units.
    let nth = |k: usize| k.checked_sub(1).and_then(|i| prefixes.get(i));
    let fit = room + noise(room);
    let top = prefixes.partition_point(|p| p.inline <= fit);
    let Some(p) = nth(top) else {
        // None fits by its width in the line: its first unit may alone
        // (it can shape narrower there).
        return usize::from(!prefixes.is_empty() && alone(1) <= fit);
    };
    if p.closes {
        return top;
    }
    let words = (1..=top).rev().find(|&k| nth(k).is_some_and(|q| q.closes)).unwrap_or(0);
    let mut k = top;
    let mut first_tried = false;
    for _ in 0..FIT_SHAPES {
        if k <= words {
            break;
        }
        first_tried |= k == 1;
        let shaped = alone(k);
        if shaped <= fit {
            return k;
        }
        let grew = shaped - nth(k).map_or(0.0, |q| q.inline);
        k = k.saturating_sub(1);
        while k > words && nth(k).is_some_and(|q| q.inline + grew > fit) {
            k = k.saturating_sub(1);
        }
    }
    // No whole word fits: its first unit, when that fits alone, rather
    // than nothing.
    if words == 0 && !first_tried && alone(1) <= fit {
        return 1;
    }
    words
}

/// Float noise in comparing sums of advances (or x found by the same
/// walk) near `w`: four of `w`'s ULPs, never more than 1/1024 px however
/// wide (f32's ULPs reach whole pixels past 2^24). The one tolerance cuts
/// and what they keep use.
fn noise(w: f32) -> f32 {
    let w = w.abs();
    (4.0 * (w.next_up() - w)).min(1.0 / 1024.0)
}

/// The bytes of the [`OBJECT`] inline box `id` stands on, in `s`'s text:
/// found by its index, at once.
fn box_range(s: &Shaped, id: u64) -> (usize, usize) {
    #[cfg(test)]
    tests::BOX_LOOKS.with(|n| n.set(n.get().saturating_add(1)));
    s.boxes
        .get(&id)
        .map_or((0, 0), |&i| (i, i.saturating_add(OBJECT.len_utf8())))
}

/// The units of `line` a cut keeps or drops whole, in visual order, each
/// as `f(x, advance, whether it is whitespace, its bytes)`: its grapheme
/// clusters, a ligature's clusters (one shaped cluster over several)
/// together, and its inline boxes, each at its own width.
fn units(s: &Shaped, line: &parley::Line<'_, u32>, mut f: impl FnMut(f32, f32, bool, (usize, usize))) {
    // The unit being gathered: its x, advance, whether all whitespace, its
    // bytes, and whether its last cluster (visually) continues a ligature.
    type Open = (f32, f32, bool, (usize, usize), bool);
    let mut open: Option<Open> = None;
    let mut clusters = None;
    for item in line.items() {
        match item {
            PositionedLayoutItem::GlyphRun(run) => {
                let mut x = run.offset();
                for c in run_clusters(&mut clusters, &run, || run.run().visual_clusters()) {
                    let continues = c.is_ligature_continuation();
                    let r = c.text_range();
                    // A ligature's continuations follow its start in LTR
                    // and come before it in RTL (visually); a ligature is
                    // one atom, in one glyph run.
                    let joins = open.is_some_and(|u| if c.is_rtl() { u.4 } else { continues });
                    match open.as_mut() {
                        Some(u) if joins => {
                            u.1 += c.advance();
                            u.2 &= c.is_space_or_nbsp();
                            u.3 = (u.3.0.min(r.start), u.3.1.max(r.end));
                            u.4 = continues;
                        }
                        _ => {
                            if let Some((ux, advance, space, range, _)) = open.take() {
                                f(ux, advance, space, range);
                            }
                            open = Some((x, c.advance(), c.is_space_or_nbsp(), (r.start, r.end), continues));
                        }
                    }
                    x += c.advance();
                }
            }
            PositionedLayoutItem::InlineBox(b) => {
                if let Some((ux, advance, space, range, _)) = open.take() {
                    f(ux, advance, space, range);
                }
                f(b.x, b.width, false, box_range(s, b.id));
            }
        }
    }
    if let Some((ux, advance, space, range, _)) = open {
        f(ux, advance, space, range);
    }
}

/// A line's extent by its units: its left and right edges, and its
/// content's: from its start edge (the left in LTR, the right in RTL: a
/// space it starts with is content) to its last unit that is not
/// whitespace (what hangs at its end edge is not).
fn extent(s: &Shaped, line: &parley::Line<'_, u32>) -> (f32, f32, f32, f32) {
    // The edges of the units that are not whitespace, visually.
    let mut ink: Option<(f32, f32)> = None;
    let mut left = None;
    let mut right = line.metrics().offset;
    units(s, line, |x, advance, space, _| {
        left.get_or_insert(x);
        right = right.max(x + advance);
        if !space {
            ink = Some(ink.map_or((x, x + advance), |(a, _)| (a, x + advance)));
        }
    });
    let left = left.unwrap_or(right);
    let (from, to) = match ink {
        None => (right, right),
        Some((a, _)) if s.key.rtl => (a, right),
        Some((_, b)) => (left, b),
    };
    (left, right, from, to)
}

/// The grapheme clusters of glyph run `run`, in visual order, from
/// `clusters`, the visual clusters of the run it is part of (made with
/// `fresh` when it is another run's). A run with several styles, or with
/// inline boxes in it, is several glyph runs one after another; each takes
/// the clusters that hold its glyphs, by count (Parley groups a ligature
/// under its first character's style, so a cluster's own style does not
/// tell its glyph run): a ligature's continuations, which hold none, go
/// with it (after it in LTR, before it in RTL), and a cluster of no glyph
/// that continues nothing (an inline box's) is in no glyph run.
fn run_clusters<'a, 'b, I: Iterator<Item = parley::Cluster<'a, u32>>, F: FnOnce() -> I>(
    clusters: &'b mut Option<(usize, std::iter::Peekable<I>)>,
    run: &parley::GlyphRun<'a, u32>,
    fresh: F,
) -> impl Iterator<Item = parley::Cluster<'a, u32>> + use<'a, 'b, I, F> {
    let index = run.run().index();
    if clusters.as_ref().is_none_or(|(i, _)| *i != index) {
        *clusters = Some((index, fresh().peekable()));
    }
    let rtl = run.run().is_rtl();
    let mut left = run.glyphs().count();
    clusters.as_mut().map(|(_, c)| c).into_iter().flat_map(move |c| {
        std::iter::from_fn(move || {
            loop {
                let next = c.peek()?;
                let glyphs = next.glyphs().count();
                let continues = next.is_ligature_continuation();
                if glyphs == 0 && !continues {
                    if left == 0 {
                        return None;
                    }
                    c.next();
                    continue;
                }
                if left == 0 && (rtl || !continues) {
                    return None;
                }
                left = left.saturating_sub(glyphs);
                return c.next();
            }
        })
    })
}

/// A run's stretch of one line: its brush, its box (with padding), its ink
/// (without), and its primary font's ascent and descent.
#[derive(Clone, Copy, Debug)]
struct Group {
    brush: u32,
    x: f32,
    width: f32,
    ink: (f32, f32),
    ascent: f32,
    descent: f32,
}

/// A glyph an underline skips the ink of: its outline's key and where its
/// pen sits, CSS pixels across from the window's left and below the line's
/// baseline.
struct Inked {
    font: u32,
    glyph: u32,
    axes: u16,
    size: f32,
    skew: f32,
    x: f32,
    dy: f32,
}

/// A scratch buffer [`TextSystem::draw`] fills again on each use, with the
/// most it held since it was last trimmed and for how many frames its room
/// was large and past twice that.
struct Scratch<T> {
    buf: Vec<T>,
    peak: usize,
    wide: u32,
}

impl<T> Default for Scratch<T> {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            peak: 0,
            wide: 0,
        }
    }
}

/// Bytes past which a scratch buffer's room is large: cut back when it
/// stays well past what draws use ([`Scratch::trim`]).
const SCRATCH_BYTES: usize = 64 << 10;

/// The room past twice the most a scratch buffer held that it may keep.
const SCRATCH_SLACK: usize = 16;

/// Frames a large scratch buffer may stay past twice what it held before
/// it is cut to that: a burst (one huge paragraph drawn once) is let go,
/// a paragraph drawn every frame keeps its room.
const SCRATCH_AGE: u32 = 8;

impl<T> Scratch<T> {
    /// The buffer, emptied, with what it held counted toward its peak.
    fn reuse(&mut self) -> &mut Vec<T> {
        self.clear();
        &mut self.buf
    }

    /// Empty the buffer, counting what it held toward its peak.
    fn clear(&mut self) {
        self.peak = self.peak.max(self.buf.len());
        self.buf.clear();
    }

    /// One frame passed: a large room past twice the most held for
    /// [`SCRATCH_AGE`] frames is cut to that (as the painter's scratch is).
    fn trim(&mut self) {
        // What it holds is this frame's, and let go: a draw that used it
        // once does not keep it counted in the frames after.
        self.clear();
        let peak = std::mem::take(&mut self.peak);
        let room = self.buf.capacity();
        let large = room.saturating_mul(std::mem::size_of::<T>()) > SCRATCH_BYTES;
        if large && room > peak.saturating_mul(2).saturating_add(SCRATCH_SLACK) {
            self.wide = self.wide.saturating_add(1);
            if self.wide >= SCRATCH_AGE {
                self.buf.clear();
                self.buf.shrink_to(peak);
                self.wide = 0;
            }
        } else {
            self.wide = 0;
        }
    }
}

/// What [`TextSystem::draw`] works in, kept so text drawn again allocates
/// nothing: the paragraph's pieces, a line's runs and the glyphs its
/// underlines skip (with whether each is CJK), the intercepts and the
/// pieces of one line, and the decorations and backgrounds held until the
/// glyphs are down.
#[derive(Default)]
struct DrawScratch {
    laid: Scratch<(Piece, f32)>,
    groups: Scratch<Group>,
    inked: Scratch<(usize, Inked)>,
    boxes: Scratch<TextDecoration>,
    own: Scratch<TextDecoration>,
    ascents: Scratch<Option<f32>>,
    cuts: Scratch<(f32, f32)>,
    segments: Scratch<(f32, f32)>,
    under: Scratch<(Bounds, Color)>,
    through: Scratch<(Bounds, Color)>,
    backgrounds: Scratch<(Bounds, f32, Color)>,
}

impl DrawScratch {
    fn trim(&mut self) {
        self.laid.trim();
        self.groups.trim();
        self.inked.trim();
        self.boxes.trim();
        self.own.trim();
        self.ascents.trim();
        self.cuts.trim();
        self.segments.trim();
        self.under.trim();
        self.through.trim();
        self.backgrounds.trim();
    }
}

/// The decoration lines of one line of text, into `out`: an underline cut
/// around the ink of the glyphs of its group in `inked` (sorted by group),
/// with `cuts` and `pieces` as scratch, and a line-through; on the line's
/// baseline `base` (CSS pixels, unrounded) at `scale`.
struct Decorator<'a> {
    fonts: &'a mut Fonts,
    inked: &'a [(usize, Inked)],
    cuts: &'a mut Scratch<(f32, f32)>,
    pieces: &'a mut Scratch<(f32, f32)>,
    under: &'a mut Vec<(Bounds, Color)>,
    through: &'a mut Vec<(Bounds, Color)>,
    base: f32,
    scale: f32,
}

impl Decorator<'_> {
    /// The underline `by`, its colour's alpha times `opacity`, over the
    /// ink `(x, width)` of group `group`, skipping its glyphs' ink.
    fn underline(&mut self, group: usize, by: &TextDecoration, opacity: f32, (x, width): (f32, f32)) {
        let color = by.color.opacity(opacity);
        let (below, thickness) = underline(by.size, by.offset);
        let (top, bottom) = ink::band(below, thickness);
        let from = self.inked.partition_point(|(at, _)| *at < group);
        let to = self.inked.partition_point(|(at, _)| *at <= group);
        let fonts = &mut *self.fonts;
        // Through the scratch, so each use counts toward what it keeps.
        let cuts = self.cuts.reuse();
        cuts.extend(
            self.inked
                .get(from..to)
                .unwrap_or_default()
                .iter()
                .filter_map(|(_, i)| {
                    let (l, r) = fonts.intercept(InkKey {
                        font: i.font,
                        glyph: i.glyph,
                        axes: i.axes,
                        size: i.size.to_bits(),
                        skew: i.skew.to_bits(),
                        band: ((top - i.dy).to_bits(), (bottom - i.dy).to_bits()),
                    })?;
                    Some((i.x + l, i.x + r))
                }),
        );
        let pieces = self.pieces.reuse();
        ink::segments(x, x + width, cuts, ink::dilation(thickness), pieces);
        for &(a, b) in pieces.iter() {
            if let Some(rect) = line_rect(a, b, self.base + below, thickness, self.scale) {
                self.under.push((rect, color));
            }
        }
    }

    /// The line-through `by`, whose font's ascent is `decorating_ascent`,
    /// its colour's alpha times `opacity`, over the ink `(x, width)` of
    /// text whose own ascent is `ascent`.
    fn line_through(
        &mut self,
        ascent: f32,
        decorating_ascent: f32,
        by: &TextDecoration,
        opacity: f32,
        (x, width): (f32, f32),
    ) {
        let color = by.color.opacity(opacity);
        let (above, thickness) = line_through(ascent, decorating_ascent, by.size);
        if let Some(rect) = line_rect(x, x + width, self.base + above, thickness, self.scale) {
            self.through.push((rect, color));
        }
    }
}

/// Add `g` to the line's groups, joined with the last when the same run
/// and `joins` (nothing between them that splits a decoration).
fn push_group(groups: &mut Vec<Group>, g: Group, joins: bool) {
    if joins
        && let Some(last) = groups.last_mut()
        && last.brush == g.brush
    {
        let right = (g.x + g.width).max(last.x + last.width);
        last.x = last.x.min(g.x);
        last.width = right - last.x;
        if g.ink.1 > 0.0 {
            let ink_right = (g.ink.0 + g.ink.1).max(last.ink.0 + last.ink.1);
            let ink_left = if last.ink.1 > 0.0 {
                last.ink.0.min(g.ink.0)
            } else {
                g.ink.0
            };
            last.ink = (ink_left, ink_right - ink_left);
        }
        return;
    }
    groups.push(g);
}

/// A device x position as a whole pixel and the quarter of a pixel past it
/// (four horizontal subpixel positions, as Skia keeps for text).
fn quarter(x: f32) -> (f32, u8) {
    let q = (x * 4.0).round();
    let whole = (q / 4.0).floor();
    let bin = f32_to_u32(q - whole * 4.0);
    (whole, u8::try_from(bin).unwrap_or(0))
}

/// The height of a piece's lines; an empty piece is one line high.
fn height(s: &Shaped, t: &ResolvedText) -> f32 {
    s.rows.last().map_or(t.line_height, |r| r.top + r.height)
}

/// One line of a piece as Blink boxes it: where it starts below the
/// piece's top, its height and its baseline below its top; and where
/// Parley put its top and baseline, and its height there, which what is in
/// the line (glyphs, inline boxes, carets) is placed against.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Row {
    top: f32,
    height: f32,
    baseline: f32,
    parley_top: f32,
    parley_baseline: f32,
    parley_height: f32,
}

impl Row {
    /// Where Parley's `y` in this line lies in ours: the line's content
    /// keeps its place against the baseline.
    fn place(&self, y: f32) -> f32 {
        self.top + self.baseline + (y - self.parley_baseline)
    }

    /// Where our `y` in this line lies in Parley's lines.
    fn parley(&self, y: f32) -> f32 {
        (self.parley_baseline + (y - self.top - self.baseline))
            .clamp(self.parley_top, self.parley_top + self.parley_height)
    }
}

/// The span box of a font whose primary font has `m`, on a line `line_height`
/// tall, as Blink sizes it: its ascent and descent each rounded to whole
/// pixels (`SimpleFontData`) and the half of the leading above them floored
/// (`CalculateLeadingSpace`); how far it reaches above and below its
/// baseline. Its baseline so lies a whole number of pixels below its top
/// (measured in Chrome: Geist Mono at 14px on a 22.75px line has its 18px
/// content box 2px below each line's top, not 2.375px).
fn blink_box(m: parley::SpanMetrics, line_height: f32) -> (f32, f32) {
    let (ascent, descent) = (m.ascent.round(), m.descent.round());
    let over = ((line_height - (ascent + descent)) / 2.0).floor() + ascent;
    (over, line_height - over)
}

/// The boxes of `layout`'s lines as broken, by Blink's rules (ADR-0005):
/// the line box holds the paragraph's strut and every span box and inline
/// box on the line, each aligned on its baseline where Parley aligned it,
/// the strut and span boxes sized by [`blink_box`]. Parley sizes them from
/// unrounded metrics, so a line an inline box grows (a 24px button on a
/// 14px/22.75 line) would be 24.43 tall where Chrome's is 24 (16 above
/// the baseline, the box's, and 6.75 below, the strut's); a line nothing
/// grows is the paragraph's line height with Blink's baseline. Written
/// over `out`, whose room is kept: a piece broken again (the ellipsis, by
/// paragraphs that break differently) allocates nothing.
fn rows(layout: &Layout<u32>, key: &ShapeKey, out: &mut Vec<Row>) {
    let line_height_of = |brush: u32| {
        let style = match brush.checked_sub(1) {
            Some(i) => key.runs.iter().find(|r| r.index == i).map_or(&key.base, |r| &r.style),
            None => &key.base,
        };
        f32::from_bits(style.line_height)
    };
    let primary = layout.root_span_metrics();
    let mut top = 0.0;
    out.clear();
    for line in layout.lines() {
        let m = line.metrics();
        let (mut over, mut under) = match primary {
            Some(p) => blink_box(p, line_height_of(0)),
            None => (m.baseline - m.block_min_coord, m.block_max_coord - m.baseline),
        };
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(run) => {
                    let (o, u) = blink_box(run.span_metrics(), line_height_of(run.style().brush));
                    // How far its baseline is above the line's.
                    let shift = m.baseline - run.baseline();
                    over = over.max(o + shift);
                    under = under.max(u - shift);
                }
                PositionedLayoutItem::InlineBox(b) => {
                    over = over.max(m.baseline - b.y);
                    under = under.max(b.y + b.height - m.baseline);
                }
            }
        }
        let height = over + under;
        out.push(Row {
            top,
            height,
            baseline: over,
            parley_top: m.block_min_coord,
            parley_baseline: m.baseline,
            parley_height: m.block_max_coord - m.block_min_coord,
        });
        top += height;
    }
}

/// The row of `rows` holding Parley's `y`.
fn row_at(rows: &[Row], y: f32) -> Option<&Row> {
    rows.iter().rev().find(|r| r.parley_top <= y + 0.01).or(rows.first())
}

/// A text decoration's thickness for `text-decoration-thickness: auto`
/// (Blink's `TextDecorationThickness::Resolve`): a tenth of the size of the
/// decorating box's font, at least one pixel, in CSS pixels. It is rounded
/// only where the line is drawn ([`line_rect`]); the offsets use it whole.
fn thickness(size: f32) -> f32 {
    (size / 10.0).max(1.0)
}

/// An underline's top below the baseline and its thickness, in CSS pixels
/// before the line snaps ([`line_rect`]), as Chrome lays out
/// `text-underline-position: auto` (Blink's `ComputeUnderlineOffsetAuto`):
/// a gap of half the thickness rounded up, at least a pixel, or exactly
/// `offset` rounded, with no gap. `size` is the decorating box's font size.
/// Chrome reads no font metric for `auto`: the post table's underline
/// position and thickness are only `from-font`'s.
///
/// Measured: Chrome's own (verdict reference, macOS, 2x) underline under
/// 16px text (TypographyDemo's link) fills two device rows, not the three
/// that 3.2 device pixels rounded down would; Chrome lays the line out and
/// rounds it in CSS pixels, then scales it.
///
/// Assumed, not measured: that every platform's Chrome does so. Only
/// macOS Chrome was measured (16px at 2x: two device rows); Linux and
/// Windows Chrome may lay decorations out in device pixels (zoom for the
/// device scale factor), which would need a branch per platform here and
/// in [`line_rect`].
fn underline(size: f32, offset: Option<f32>) -> (f32, f32) {
    let thickness = thickness(size);
    let below = match offset {
        Some(offset) => offset.round(),
        None => (thickness / 2.0).ceil().max(1.0),
    };
    (below, thickness)
}

/// A line-through's top relative to the baseline (negative above) and its
/// thickness, in CSS pixels before the line snaps, as Blink's
/// `ComputeLineThroughLineData` places it: centred two thirds of the
/// decorating box's font ascent (`decorating_ascent`, unrounded) below the
/// top of the text's own box, which is its font's `ascent`, rounded as
/// Blink rounds it, above the baseline. One font: a third of the ascent
/// above the baseline.
fn line_through(ascent: f32, decorating_ascent: f32, size: f32) -> (f32, f32) {
    let thickness = thickness(size);
    (
        -ascent.round() + 2.0 * decorating_ascent / 3.0 - thickness / 2.0,
        thickness,
    )
}

/// A decoration line from `x0` to `x1` with its top at `top`, `thickness`
/// thick (CSS pixels, unsnapped), as Chrome paints a solid one: snapped on
/// the y axis (`SnapYAxis`: the top to the nearest pixel, the thickness
/// rounded down, at least a pixel), then filled without anti-aliasing at
/// `scale`, so each edge falls on the device pixel nearest it, as Skia
/// fills a non-anti-aliased rectangle (the line's ends and skip-ink's
/// clipped gaps alike). Each edge is given as device pixels over `scale`,
/// which at the scales Chrome ships (1.25, 1.5, 2, 2.5, 3) multiplies back
/// to the whole device pixel exactly. None where it covers no pixel.
///
/// The snapping is in CSS pixels on every platform, as measured on macOS
/// Chrome alone (see [`underline`]): whether Linux and Windows Chrome snap
/// in device pixels instead is not verified.
fn line_rect(x0: f32, x1: f32, top: f32, thickness: f32, scale: f32) -> Option<Bounds> {
    let y = (top + 0.5).floor();
    let h = thickness.floor().max(1.0);
    let px = |v: f32| (v * scale + 0.5).floor();
    let (l, r, t, b) = (px(x0), px(x1), px(y), px(y + h));
    (r > l && b > t).then(|| Bounds {
        x: l / scale,
        y: t / scale,
        width: (r - l) / scale,
        height: (b - t) / scale,
    })
}

/// A line's baseline at window `y` ([`Row::baseline`] below its top,
/// Blink's), in device pixels: on the nearest one, a half rounding up as
/// Skia draws horizontal text.
fn baseline(y: f32, scale: f32) -> f32 {
    (y * scale + 0.5).floor()
}

/// The baseline of an empty piece's one line: its primary font's, by
/// Blink's rounding ([`blink_box`]), else at the line's bottom.
fn empty_baseline(layout: &Layout<u32>, t: &ResolvedText) -> f32 {
    layout
        .root_span_metrics()
        .map_or(t.line_height, |m| blink_box(m, t.line_height).0)
}

/// How many lines `layout` breaks into at `width`, counted to `cap` and
/// one past at most: what follows is not broken (Parley breaks line by
/// line), so a long paragraph is not broken whole to learn it is long.
/// Its lines are all broken when it returns `cap` or fewer.
fn lines_upto(layout: &mut Layout<u32>, width: f32, fit: Fit, cap: usize, broken: &mut u64) -> usize {
    let n = break_fit(layout, width, fit, cap.saturating_add(1));
    *broken = broken.saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
    n
}

/// Where a wrapped piece's lines break: those from line `from` on at most
/// `at` wide, those before it at the box's width. `pretty` narrows a
/// paragraph's last lines only, and `balance` all of them.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    from: usize,
    at: f32,
}

impl Fit {
    /// Every line at the box's width `width`, as text wraps by default.
    const fn whole(width: f32) -> Self {
        Self { from: 0, at: width }
    }
}

/// Break `layout`'s lines in a box `width` wide as `fit` says, line by
/// line and up to `most` of them: what follows is never broken. A
/// narrowed line is still aligned in the whole box, as Blink places the
/// lines its `ScoreLineBreaker` chose. How many lines it broke.
fn break_fit(layout: &mut Layout<u32>, width: f32, fit: Fit, most: usize) -> usize {
    let mut lines = layout.break_lines();
    lines.state_mut().set_layout_max_advance(width);
    let mut n = 0_usize;
    while n < most {
        let narrowed = n >= fit.from && fit.at < width;
        lines
            .state_mut()
            .set_line_max_advance(if narrowed { fit.at } else { width });
        // Only a line break yields: its boxes are in flow, and it has no
        // height to exceed.
        if lines.break_next().is_none() {
            break;
        }
        if narrowed {
            lines.set_prior_line_width(width);
        }
        n = n.saturating_add(1);
    }
    lines.finish();
    n
}

/// Where `layout`'s last lines break so the last is not one short word
/// alone, as Chromium's `text-wrap: pretty` (`ScoreLineBreaker`) re-breaks
/// them: its last [`PRETTY_MAX_LINES`] lines (all of a shorter paragraph)
/// at the narrowest width that keeps their count and sends a word down,
/// the lines before them wrapping as they would. Only when the greedy last
/// line is one word shorter than a third of the box (Blink's
/// `ShouldOptimize`), and only a paragraph whose lines are all shown
/// (`shown`: a clamp's, else all): a clamp that hides its last line leaves
/// its lines as they wrap, its hidden ones never broken.
fn pretty(layout: &mut Layout<u32>, content: &str, width: f32, shown: Option<u32>, broken: &mut u64) -> Fit {
    let whole = Fit::whole(width);
    let cap = shown.map_or(usize::MAX, |s| usize::try_from(s).unwrap_or(usize::MAX));
    let count = lines_upto(layout, width, whole, cap, broken);
    if !(2..=cap).contains(&count) || !orphan(layout, content) || !last_short(layout, width) {
        return whole;
    }
    let from = count.saturating_sub(PRETTY_MAX_LINES);
    let fit = |at| Fit { from, at };
    // Find the widest width that sends a word down to the last line.
    let (mut wide, mut narrow) = (width, 0.0_f32);
    while wide - narrow > 1.0 / LAYOUT_UNITS {
        let mid = (wide + narrow) / 2.0;
        if lines_upto(layout, width, fit(mid), count, broken) == count && orphan(layout, content) {
            wide = mid;
        } else {
            narrow = mid;
        }
    }
    if lines_upto(layout, width, fit(narrow), count, broken) != count || orphan(layout, content) {
        return whole;
    }
    fit(narrow)
}

/// Most lines `text-wrap: pretty` re-breaks: a paragraph's last four, as
/// Chromium's (`kMaxLinesForOptimal`).
const PRETTY_MAX_LINES: usize = 4;

/// Whether the last line of `layout` is shorter than a third of `width`,
/// Blink's measure of a last line worth re-breaking for.
fn last_short(layout: &Layout<u32>, width: f32) -> bool {
    layout.lines().next_back().is_some_and(|line| {
        let m = line.metrics();
        (m.advance - m.hanging_advance) * 3.0 < width
    })
}

/// Whether the last line of `layout` holds a single word.
fn orphan(layout: &Layout<u32>, content: &str) -> bool {
    layout.lines().next_back().is_some_and(|line| {
        content
            .get(line.text_range())
            .map_or(0, |s| s.split_whitespace().count())
            < 2
    })
}

/// Most lines Chromium balances; longer text wraps as usual.
const BALANCE_MAX_LINES: usize = 6;

/// The least width that keeps the line count `layout` has at `width`, as
/// Chromium balances (`text-wrap: balance`): only a paragraph of at most
/// [`BALANCE_MAX_LINES`] lines, all of them shown (`shown`: a clamp's,
/// else all); a longer one, or one a clamp cuts, is never broken whole.
fn balance(layout: &mut Layout<u32>, width: f32, shown: Option<u32>, broken: &mut u64) -> Fit {
    let cap = shown.map_or(BALANCE_MAX_LINES, |s| {
        BALANCE_MAX_LINES.min(usize::try_from(s).unwrap_or(usize::MAX))
    });
    let count = lines_upto(layout, width, Fit::whole(width), cap, broken);
    if !(2..=cap).contains(&count) {
        return Fit::whole(width);
    }
    let (mut fits, mut wraps) = (width, 0.0_f32);
    while fits - wraps > 1.0 / LAYOUT_UNITS {
        let mid = (fits + wraps) / 2.0;
        if lines_upto(layout, width, Fit::whole(mid), count, broken) == count {
            fits = mid;
        } else {
            wraps = mid;
        }
    }
    Fit::whole(fits)
}

/// Where a frame's primitives are recorded: the scene's items, its scale
/// and the clip that applies.
pub struct Target<'a> {
    pub items: &'a mut Vec<Item>,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// The physical size painted.
    pub size: (u32, u32),
    /// What is recorded at all, in the target's space (Chromium's cull
    /// rect): text lines outside it are left out.
    pub cull: Option<Bounds>,
    /// Whether box edges snap to device pixels: only where the transform to
    /// the screen keeps the pixel grid (ADR-0020).
    pub snap: bool,
}

impl Target<'_> {
    /// `b` as painted: snapped to device pixels where edges snap.
    #[must_use]
    pub fn snapped(&self, b: Bounds) -> Bounds {
        if self.snap { b.snapped(self.scale) } else { b }
    }

    /// Fill a logical-pixel rectangle with corners of radius `radius`.
    pub fn fill_rounded(&mut self, b: Bounds, radius: f32, color: Color) {
        if radius <= 0.0 {
            self.fill_rect(b, color);
            return;
        }
        self.items.push(Item::RoundedRect {
            bounds: b,
            radius: crate::style::Corners::all(radius),
            color,
        });
    }

    /// Fill a logical-pixel rectangle with a solid colour.
    pub fn fill_rect(&mut self, b: Bounds, color: Color) {
        self.items.push(Item::Rect { bounds: b, color });
    }
}

/// Physical pixel size of a logical length, at least 1.
#[must_use]
pub fn physical(len: f32, scale: f32) -> u32 {
    f32_to_u32((len * scale).round()).max(1)
}

/// A logical position in whole physical pixels.
#[must_use]
pub fn physical_pos(v: f32, scale: f32) -> i32 {
    crate::convert::f32_to_i32((v * scale).round())
}

/// Logical pixels from a physical integer.
#[must_use]
pub fn logical(v: i32, scale: f32) -> f32 {
    i32_to_f32(v) / scale
}

#[cfg(test)]
mod tests;
