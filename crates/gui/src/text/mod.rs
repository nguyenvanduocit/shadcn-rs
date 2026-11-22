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
use self::fonts::Fonts;
pub use self::fonts::{Coverage, GlyphContent, GlyphImage, GlyphKey, Glyphs};
pub(crate) use self::fonts::{MONO, SANS};
use self::gamma::{GammaParams, MaskGamma};
use self::settings::TextSettings;
use crate::color::Color;
use crate::convert::{f32_to_i32, f32_to_u32, i32_to_f32};
use crate::geometry::Bounds;
use crate::scene::Item;
use crate::style::{Direction, Edges, FontFamily, GenericFamily, ResolvedText, TextAlign, WrapStyle};

/// Subdivisions of a pixel lengths are kept in (Blink's `LayoutUnit`).
const LAYOUT_UNITS: f32 = 64.0;

/// What CSS draws where text overflows its line (`text-overflow: ellipsis`).
const ELLIPSIS: &str = "\u{2026}";

/// U+FFFC OBJECT REPLACEMENT CHARACTER: the text an inline box stands on.
pub const OBJECT: char = '\u{FFFC}';

/// Bytes of a paragraph in an inline style of their own (`<a>`, `<code>`,
/// `<strong>`, a Shiki token).
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The bytes of the paragraph's content this run covers.
    pub range: Range<usize>,
    /// The paragraph's text style with the run's own applied on top. Its line
    /// height is not used: a run takes the paragraph's lines.
    pub text: ResolvedText,
    /// Painted behind the run's glyphs, over the font's height.
    pub background: Option<Color>,
    /// Corner radius of that background.
    pub radius: f32,
    /// Left and right add space in the line before and after the run; top
    /// and bottom only grow its background, as CSS pads an inline box.
    pub padding: Edges<f32>,
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
    /// Names it in [`TextSystem::inline_boxes`].
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
}

/// How a piece's lines are broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Breaking {
    /// The width lines break at; none for one unbroken line per piece.
    width: Option<u32>,
    nowrap: bool,
    wrap_style: WrapStyle,
    align: TextAlign,
}

impl Breaking {
    fn of(t: &ResolvedText, width: Option<f32>) -> Self {
        Self {
            width: width.map(f32::to_bits),
            nowrap: t.nowrap || t.truncate,
            wrap_style: if t.line_clamp.is_some() {
                WrapStyle::Auto
            } else {
                t.wrap_style
            },
            align: t.physical_align(),
        }
    }

    fn width(&self) -> Option<f32> {
        self.width.map(f32::from_bits)
    }
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
    /// The widths `pretty` and `balance` settled on, by how they were asked.
    settled: Vec<(Breaking, f32)>,
    /// Each line's size and baseline under the last few breakings, so
    /// layout's probes at alternating widths measure without breaking
    /// the lines again.
    lines: Vec<(Breaking, Vec<LineSize>)>,
    /// Its min-content width.
    min: Option<f32>,
    used: u64,
}

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
    drawn: Vec<u64>,
    /// Pieces kept shaped while a retained paint chunk shows them (ADR-0016),
    /// however long since a frame drew them; by how many chunks.
    pinned: HashMap<u64, u32>,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
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
            drawn: Vec::new(),
            pinned: HashMap::default(),
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
            let end = content
                .get(start..)
                .and_then(|s| s.find('\n'))
                .map_or(content.len(), |i| start.saturating_add(i));
            let hash = self.piece(p, t, start, end);
            pieces.push(Piece { start, end, hash });
            if end >= content.len() {
                break;
            }
            start = end.saturating_add(1);
        }
        pieces
    }

    /// Shape bytes `start..end` of `p` (no line break among them), or find them shaped.
    fn piece(&mut self, p: Paragraph<'_>, t: &ResolvedText, start: usize, end: usize) -> u64 {
        let text = p.content.get(start..end).unwrap_or("");
        let base = StyleKey::of(t, t.line_height);
        let runs: Vec<PieceRun> = p
            .runs
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
            })
            .collect();
        // An in-flow inline box sits on the U+FFFC its caller put in the
        // text for it (Parley's contract); one elsewhere is left out.
        let boxes = p
            .boxes
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
            })
            .collect();
        // A run padded at its start that starts the piece has no character
        // before it to carry its padding: an inline box on a U+FFFC put
        // before the text carries it.
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
        };
        let hash = hash_of(text, &key);
        let frame = self.frame;
        if let Some(s) = self.shaped.get_mut(&hash)
            && s.content.get(lead..) == Some(text)
            && s.key == key
        {
            s.used = frame;
            return hash;
        }
        let mut content = String::with_capacity(text.len().saturating_add(lead));
        if lead > 0 {
            content.push(OBJECT);
        }
        content.push_str(text);
        let layout = self.shape(&content, &key);
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
                s.shift = ((w - s.layout.width()) * factor).max(0.0);
            }
        } else if let Some(w) = width {
            let settled = s.settled.iter().find(|(b, _)| *b == how).map(|(_, w)| *w);
            let at = match settled {
                Some(at) => at,
                None => {
                    let at = match how.wrap_style {
                        WrapStyle::Auto => w,
                        WrapStyle::Pretty => pretty(&mut s.layout, &s.content, w),
                        WrapStyle::Balance => balance(&mut s.layout, w),
                    };
                    if s.settled.len() >= 8 {
                        s.settled.clear();
                    }
                    s.settled.push((how, at));
                    at
                }
            };
            s.layout.break_all_lines(Some(at));
            s.shift = (w - at) * factor;
        }
        s.rows = rows(&s.layout, &s.key);
        s.broken = Some(how);
    }

    /// The pieces of `p`, shaped and broken for `width`, with the top of each.
    fn laid_out(&mut self, p: Paragraph<'_>, t: &ResolvedText, width: Option<f32>) -> Vec<(Piece, f32)> {
        let how = Breaking::of(t, width);
        let pieces = self.pieces(p, t);
        let mut y = 0.0;
        let mut out = Vec::with_capacity(pieces.len());
        for piece in pieces {
            self.break_piece(piece.hash, how);
            out.push((piece, y));
            y += self.shaped.get(&piece.hash).map_or(t.line_height, |s| height(s, t));
        }
        out
    }

    /// The width `p` takes and the line it is cut on, when its last shown
    /// line is cut short with an ellipsis: (piece index, line index, where
    /// the kept text ends, the ellipsis's width).
    fn cut(&mut self, laid: &[(Piece, f32)], t: &ResolvedText, width: Option<f32>) -> Option<Cut> {
        let width = width?;
        let clamp = if t.truncate {
            Some(1)
        } else {
            t.line_clamp.map(usize::from)
        };
        let clamp = clamp?;
        // The last line shown, and whether more follow it.
        let mut seen = 0_usize;
        let mut last = None;
        let mut more = false;
        for (i, (piece, _)) in laid.iter().enumerate() {
            let lines = self.shaped.get(&piece.hash).map_or(1, |s| s.layout.len().max(1));
            if last.is_some() {
                more = true;
                break;
            }
            if seen.saturating_add(lines) >= clamp {
                let line = clamp.saturating_sub(seen).saturating_sub(1);
                last = Some((i, line));
                more = line.saturating_add(1) < lines;
            }
            seen = seen.saturating_add(lines);
        }
        let (pi, line) = last?;
        let (piece, _) = laid.get(pi)?;
        let s = self.shaped.get(&piece.hash)?;
        let l = s.layout.get(line)?;
        let m = l.metrics();
        let overflows = m.offset + m.advance - m.hanging_advance > width + 0.01;
        if !more && !overflows {
            return None;
        }
        let ellipsis = self.ellipsis_width(t);
        let s = self.shaped.get(&piece.hash)?;
        let l = s.layout.get(line)?;
        let limit = width - ellipsis - s.shift;
        let mut end = l.metrics().offset;
        for item in l.items() {
            if let PositionedLayoutItem::GlyphRun(run) = item {
                let mut x = run.offset();
                for g in run.glyphs() {
                    if x + g.advance > limit + 0.01 {
                        return Some(Cut {
                            piece: pi,
                            line,
                            end,
                            ellipsis,
                        });
                    }
                    x += g.advance;
                    end = x;
                }
            }
        }
        Some(Cut {
            piece: pi,
            line,
            end,
            ellipsis,
        })
    }

    fn ellipsis_width(&mut self, t: &ResolvedText) -> f32 {
        let hash = self.piece(Paragraph::from(ELLIPSIS), t, 0, ELLIPSIS.len());
        let how = Breaking::of(t, None);
        self.break_piece(hash, how);
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
        let cut = self.cut(&laid, t, width);
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
                    Some(c) if c.piece == i && c.line == j => c.end - m.offset + c.ellipsis,
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

    /// The piece holding byte `at` and its top, of `laid`.
    fn piece_at(laid: &[(Piece, f32)], at: usize) -> Option<&(Piece, f32)> {
        laid.iter().find(|(p, _)| at <= p.end).or(laid.last())
    }

    /// The byte offset in `content` nearest to `(x, y)`, relative to the text box.
    pub fn hit(&mut self, content: &str, t: &ResolvedText, width: Option<f32>, x: f32, y: f32) -> usize {
        let laid = self.laid_out(content.into(), t, width);
        let index = laid.iter().rposition(|(_, top)| *top <= y).unwrap_or(0);
        let Some((piece, top)) = laid.get(index) else {
            return content.len();
        };
        let Some(s) = self.shaped.get(&piece.hash) else {
            return piece.start;
        };
        let local = y - top;
        let local = s
            .rows
            .iter()
            .rev()
            .find(|r| r.top <= local)
            .or(s.rows.first())
            .map_or(local, |r| r.parley(local));
        let cursor = ParleyCursor::from_point(&s.layout, x - s.shift, local);
        piece.start.saturating_add(cursor.index()).min(piece.end)
    }

    /// Where the caret for byte offset `at` sits, relative to the text box.
    pub fn caret(&mut self, content: &str, t: &ResolvedText, width: Option<f32>, at: usize) -> Caret {
        let laid = self.laid_out(content.into(), t, width);
        let Some((piece, top)) = Self::piece_at(&laid, at) else {
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

    /// The boxes covering bytes `from..to`, relative to the text box.
    pub fn selection<'p>(
        &mut self,
        p: impl Into<Paragraph<'p>>,
        t: &ResolvedText,
        width: Option<f32>,
        from: usize,
        to: usize,
    ) -> Vec<Bounds> {
        let laid = self.laid_out(p.into(), t, width);
        let mut boxes = Vec::new();
        for (piece, top) in &laid {
            if to < piece.start || from > piece.end || (from == to && from != piece.start) {
                continue;
            }
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
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
                // Our line's box where Parley covers its whole line.
                let (y0, y1) = match row_at(&s.rows, y0) {
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
        }
        boxes
    }

    /// Where `p`'s inline boxes ([`Paragraph::boxes`]) sit, by id, relative to the text box.
    pub fn inline_boxes<'p>(
        &mut self,
        p: impl Into<Paragraph<'p>>,
        t: &ResolvedText,
        width: Option<f32>,
    ) -> Vec<(u64, Bounds)> {
        let laid = self.laid_out(p.into(), t, width);
        let mut out = Vec::new();
        for (piece, top) in &laid {
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
            for (line, row) in s.layout.lines().zip(&s.rows) {
                for item in line.items() {
                    if let PositionedLayoutItem::InlineBox(b) = item
                        && padding_of(b.id).is_none()
                    {
                        out.push((
                            b.id,
                            Bounds {
                                x: b.x + s.shift,
                                y: top + row.place(b.y),
                                width: b.width,
                                height: b.height,
                            },
                        ));
                    }
                }
            }
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
        self.laid_out(p.into(), t, width);
    }

    /// Draw `p` in style `t` with its box's top-left at `origin` (logical
    /// pixels), every colour's alpha times `opacity`.
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
        let laid = self.laid_out(p, t, width);
        let cut = self.cut(&laid, t, width);
        let ellipsis = cut.map(|_| self.piece(Paragraph::from(ELLIPSIS), t, 0, ELLIPSIS.len()));
        self.drawn
            .extend(laid.iter().map(|(piece, _)| piece.hash).chain(ellipsis));
        if let Some(e) = ellipsis {
            self.break_piece(e, Breaking::of(t, None));
        }
        let runs = p.runs;
        let style_of = |brush: u32| brush.checked_sub(1).and_then(|i| runs.get(usize::try_from(i).ok()?));
        let scale = target.scale;
        // Hinted below 2x, as Chrome's FreeType and DirectWrite text is;
        // on macOS Chrome's CoreText text is not hinted, and so not ours.
        let hinted = scale < 2.0 && !cfg!(target_os = "macos");
        // ADR-0020: on Linux at 1× glyph origins round to whole pixels, as Chrome does there.
        let whole = cfg!(all(unix, not(target_os = "macos"))) && (scale - 1.0).abs() < f32::EPSILON;
        let visible = target.cull;
        let clamp = if t.truncate {
            Some(1)
        } else {
            t.line_clamp.map(usize::from)
        };
        let mut placed: Vec<Item> = Vec::new();
        let mut decorations: Vec<(Bounds, Color)> = Vec::new();
        let mut backgrounds: Vec<(Bounds, f32, Color)> = Vec::new();
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
                let device = baseline(origin.1 + line_top + row.baseline, scale);
                let base = device / scale;
                let cut_end = cut.filter(|c| c.piece == pi && c.line == li).map(|c| c.end);
                let x0 = origin.0 + s.shift;
                let mut groups: Vec<Group> = Vec::new();
                for item in line.items() {
                    match item {
                        PositionedLayoutItem::GlyphRun(run) => {
                            let brush = run.style().brush;
                            let style = style_of(brush).map_or(t, |r| &r.text);
                            let color = style.color.opacity(opacity);
                            let font = run.run().font();
                            let face = self.fonts.face(font);
                            let axes = self
                                .fonts
                                .axes(run.run().normalized_coords().iter().map(|c| c.to_bits()));
                            let synthesis = run.run().synthesis();
                            let ppem = run.run().font_size() * scale;
                            let mut x = run.offset();
                            let mut end = x;
                            let mut whole_run = true;
                            for g in run.glyphs() {
                                if cut_end.is_some_and(|e| x + g.advance > e + 0.01) {
                                    whole_run = false;
                                    break;
                                }
                                let gx = (x0 + x + g.x) * scale;
                                let gy = device + ((g.y + run.baseline()) - m.baseline) * scale;
                                let (px, bin) = if whole { (gx.round(), 0) } else { quarter(gx) };
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
                                placed.push(Item::Glyph {
                                    key: GlyphKey {
                                        font: face,
                                        glyph: g.id,
                                        ppem: ppem.to_bits(),
                                        axes,
                                        bin,
                                        flags,
                                        skew: synthesis
                                            .skew()
                                            .map_or(0, |d| i8::try_from(f32_to_i32(d.round())).unwrap_or(0)),
                                    },
                                    x: f32_to_i32(px),
                                    y: f32_to_i32(gy.round()),
                                    color,
                                });
                                x += g.advance;
                                end = x;
                            }
                            let span = run.span_metrics();
                            let mut group = Group {
                                brush,
                                x: x0 + run.offset(),
                                width: end - run.offset(),
                                ink: (x0 + run.offset(), end - run.offset()),
                                ascent: span.ascent,
                                descent: span.descent,
                            };
                            // A run's padding is letter spacing after the
                            // character before it and after its last one
                            // (`Self::shape`): its box, not its ink, and not
                            // the box of the text before it.
                            if let Some(r) = style_of(brush) {
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
                            push_group(&mut groups, group);
                        }
                        PositionedLayoutItem::InlineBox(b) => {
                            if let Some(index) = padding_of(b.id) {
                                let r = runs.get(usize::try_from(index).unwrap_or(usize::MAX));
                                let size = r.map_or(t.size, |r| r.text.size);
                                push_group(
                                    &mut groups,
                                    Group {
                                        brush: index.saturating_add(1),
                                        x: x0 + b.x,
                                        width: b.width,
                                        ink: (x0 + b.x, 0.0),
                                        ascent: size * 0.97,
                                        descent: size * 0.24,
                                    },
                                );
                            }
                        }
                    }
                }
                if let (Some(c), Some(e)) = (cut.filter(|c| c.piece == pi && c.line == li), ellipsis) {
                    let ex = x0 + c.end;
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
                                    let (px, bin) = quarter((ex + x + g.x) * scale);
                                    placed.push(Item::Glyph {
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
                // CSS decorates no hanging space at a line's end.
                if let Some(last) = groups.last_mut() {
                    last.width = (last.width - m.hanging_advance).max(0.0);
                    last.ink.1 = (last.ink.1 - m.hanging_advance).max(0.0);
                }
                for g in &groups {
                    let style = style_of(g.brush).map_or(t, |r| &r.text);
                    let color = style.color.opacity(opacity);
                    let thickness = (style.size / 14.0).max(1.0);
                    let (ink_x, ink_w) = g.ink;
                    if style.underline && ink_w > 0.0 {
                        let below = style.underline_offset.unwrap_or(style.size * 0.12);
                        decorations.push((
                            Bounds {
                                x: ink_x,
                                y: base + below,
                                width: ink_w,
                                height: thickness,
                            },
                            color,
                        ));
                    }
                    if style.line_through && ink_w > 0.0 {
                        decorations.push((
                            Bounds {
                                x: ink_x,
                                y: base - style.size * 0.3,
                                width: ink_w,
                                height: thickness,
                            },
                            color,
                        ));
                    }
                    if let Some(run) = style_of(g.brush)
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
            }
        }
        for (b, radius, color) in backgrounds {
            target.fill_rounded(b, radius, color);
        }
        target.items.extend(placed);
        for (b, color) in decorations {
            target.fill_rect(b, color);
        }
    }

    /// Forget the shaped text the last frames did not use; call once per frame.
    pub fn end_frame(&mut self) {
        let frame = self.frame;
        let pinned = &self.pinned;
        self.shaped
            .retain(|hash, s| frame.saturating_sub(s.used) < 2 || pinned.contains_key(hash));
        self.frame = frame.saturating_add(1);
        self.drawn.clear();
    }

    /// The pieces [`TextSystem::draw`] drew since last asked: what a paint
    /// chunk just recorded shows.
    pub fn take_drawn(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.drawn)
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

/// The last shown line of a paragraph cut short: which piece and line, where
/// the kept glyphs end (from the line's start, as Parley places them) and the
/// ellipsis's width.
#[derive(Clone, Copy, Debug)]
struct Cut {
    piece: usize,
    line: usize,
    end: f32,
    ellipsis: f32,
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

/// Add `g` to the line's groups, joined with the last when the same run.
fn push_group(groups: &mut Vec<Group>, g: Group) {
    if let Some(last) = groups.last_mut()
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
/// grows is the paragraph's line height with Blink's baseline.
fn rows(layout: &Layout<u32>, key: &ShapeKey) -> Vec<Row> {
    let line_height_of = |brush: u32| {
        let style = match brush.checked_sub(1) {
            Some(i) => key.runs.iter().find(|r| r.index == i).map_or(&key.base, |r| &r.style),
            None => &key.base,
        };
        f32::from_bits(style.line_height)
    };
    let primary = layout.root_span_metrics();
    let mut top = 0.0;
    let mut out = Vec::with_capacity(layout.len());
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
    out
}

/// The row of `rows` holding Parley's `y`.
fn row_at(rows: &[Row], y: f32) -> Option<&Row> {
    rows.iter().rev().find(|r| r.parley_top <= y + 0.01).or(rows.first())
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

/// The narrowest width at which `layout` keeps the line count it has at
/// `width` and its last line is not one word alone, as Chromium's
/// `text-wrap: pretty` re-breaks short paragraphs; `width` itself when no
/// width does.
fn pretty(layout: &mut Layout<u32>, content: &str, width: f32) -> f32 {
    layout.break_all_lines(Some(width));
    let count = layout.len();
    if !(2..=PRETTY_MAX_LINES).contains(&count) || !orphan(layout, content) {
        return width;
    }
    // Find the widest width that sends a word down to the last line.
    let (mut wide, mut narrow) = (width, 0.0_f32);
    while wide - narrow > 1.0 / LAYOUT_UNITS {
        let mid = (wide + narrow) / 2.0;
        layout.break_all_lines(Some(mid));
        if layout.len() == count && orphan(layout, content) {
            wide = mid;
        } else {
            narrow = mid;
        }
    }
    layout.break_all_lines(Some(narrow));
    if layout.len() != count || orphan(layout, content) {
        return width;
    }
    narrow
}

/// Most lines `text-wrap: pretty` re-breaks; Chromium looks at a paragraph's last four.
const PRETTY_MAX_LINES: usize = 4;

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
/// Chromium balances (`text-wrap: balance`).
fn balance(layout: &mut Layout<u32>, width: f32) -> f32 {
    layout.break_all_lines(Some(width));
    let count = layout.len();
    if !(2..=BALANCE_MAX_LINES).contains(&count) {
        return width;
    }
    let (mut fits, mut wraps) = (width, 0.0_f32);
    while fits - wraps > 1.0 / LAYOUT_UNITS {
        let mid = (fits + wraps) / 2.0;
        layout.break_all_lines(Some(mid));
        if layout.len() == count {
            fits = mid;
        } else {
            wraps = mid;
        }
    }
    fits
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
