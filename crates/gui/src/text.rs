//! Text: shaping, wrapping and measuring with cosmic-text, and drawing the
//! glyphs into a pixmap. Inter (sans) and Geist Mono are embedded so text
//! looks the same on every machine; system fonts fill in what they lack.
//!
//! A [`Paragraph`] is one string wrapped as a whole; its [`Run`]s give byte
//! ranges of it a style of their own (CSS inline elements: a link, a `<code>`,
//! a highlighted token), shaped together through cosmic-text's attribute
//! spans so they wrap as one paragraph (ADR-0001).

use std::ops::Range;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Cursor, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping,
    Style as FontStyle, SwashCache, SwashContent, Weight, Wrap,
};
use tiny_skia::{Mask, Pixmap, PixmapPaint, Transform};

use crate::cache::FrameCache;
use crate::color::Color;
use crate::convert::{f32_to_u32, i32_to_f32, u32_to_f32};
use crate::geometry::Bounds;
use crate::inspect::Raster;
use crate::style::{Edges, FontFamily, ResolvedText, TextAlign, WrapStyle};

/// Subdivisions of a pixel lengths are kept in (Blink's `LayoutUnit`).
const LAYOUT_UNITS: f32 = 64.0;

const INTER: &[u8] = include_bytes!("../assets/Inter.ttf");
/// Inter pinned at the optical sizes of Tailwind's type scale, each a family
/// of its own (`assets/inter-opsz/instance.py`). Chrome sets Inter's `opsz`
/// axis (14 to 32) to the font size (`font-optical-sizing: auto`), which
/// tightens large text; cosmic-text varies only `wght`, so a size takes the
/// face nearest to it, and up to 15px the default (`opsz` 14) one.
const INTER_OPTICAL: [(f32, &str, &[u8]); 6] = [
    (
        16.0,
        "Inter Opsz16",
        include_bytes!("../assets/inter-opsz/Inter-opsz16.ttf"),
    ),
    (
        18.0,
        "Inter Opsz18",
        include_bytes!("../assets/inter-opsz/Inter-opsz18.ttf"),
    ),
    (
        20.0,
        "Inter Opsz20",
        include_bytes!("../assets/inter-opsz/Inter-opsz20.ttf"),
    ),
    (
        24.0,
        "Inter Opsz24",
        include_bytes!("../assets/inter-opsz/Inter-opsz24.ttf"),
    ),
    (
        30.0,
        "Inter Opsz30",
        include_bytes!("../assets/inter-opsz/Inter-opsz30.ttf"),
    ),
    (
        32.0,
        "Inter Opsz32",
        include_bytes!("../assets/inter-opsz/Inter-opsz32.ttf"),
    ),
];
const GEIST_MONO: &[u8] = include_bytes!("../assets/GeistMono.ttf");

/// Where a paragraph's lines may break, besides at a `\n`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Breaks {
    /// Nowhere (`white-space: nowrap`).
    None,
    /// Between words only: how its min-content width is measured, which
    /// splits no word.
    Words,
    /// Between words, and inside a word wider than the line, as
    /// `overflow-wrap: break-word` lays out.
    WordsOrGlyphs,
}

impl Breaks {
    /// How text in style `t` breaks when laid out in a box.
    fn of(t: &ResolvedText) -> Self {
        if t.nowrap { Self::None } else { Self::WordsOrGlyphs }
    }

    fn wrap(self) -> Wrap {
        match self {
            Self::None => Wrap::None,
            Self::Words => Wrap::Word,
            Self::WordsOrGlyphs => Wrap::WordOrGlyph,
        }
    }
}

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

/// What text shapes and draws: `content`, with `runs` (sorted, not
/// overlapping, on char boundaries) styled on their own; the rest of it takes
/// the paragraph's style.
#[derive(Clone, Copy, Debug)]
pub struct Paragraph<'a> {
    pub content: &'a str,
    pub runs: &'a [Run],
}

impl<'a> From<&'a str> for Paragraph<'a> {
    fn from(content: &'a str) -> Self {
        Self { content, runs: &[] }
    }
}

impl<'a> From<&'a String> for Paragraph<'a> {
    fn from(content: &'a String) -> Self {
        Self::from(content.as_str())
    }
}

/// What of a run changes how it is shaped.
#[derive(Clone, PartialEq, Eq, Hash)]
struct RunKey {
    range: (usize, usize),
    size: u32,
    weight: u16,
    family: FontFamily,
    italic: bool,
    tracking: u32,
    tabular_nums: bool,
    padding: (u32, u32),
}

impl RunKey {
    fn of(run: &Run) -> Self {
        let t = &run.text;
        Self {
            range: (run.range.start, run.range.end),
            size: t.size.to_bits(),
            weight: t.weight,
            family: t.family,
            italic: t.italic,
            tracking: t.tracking.to_bits(),
            tabular_nums: t.tabular_nums,
            padding: (run.padding.left.to_bits(), run.padding.right.to_bits()),
        }
    }
}

/// Glyph metadata: which run a glyph belongs to, and whether it is the run's
/// first character (drawn after the run's left padding).
fn run_tag(run: usize, first: bool) -> usize {
    run.saturating_add(1)
        .saturating_mul(2)
        .saturating_add(usize::from(first))
}

/// The run a glyph's metadata names, and whether it is that run's first character.
fn untag(metadata: usize) -> (Option<usize>, bool) {
    ((metadata / 2).checked_sub(1), metadata % 2 == 1)
}

/// The sans face for text of `size` pixels: Inter at the optical size nearest to it.
fn sans(size: f32) -> Family<'static> {
    let opsz = size.clamp(14.0, 32.0);
    INTER_OPTICAL
        .iter()
        .filter(|(at, _, _)| (at - opsz).abs() < (14.0 - opsz).abs())
        .min_by(|a, b| (a.0 - opsz).abs().total_cmp(&(b.0 - opsz).abs()))
        .map_or(Family::SansSerif, |(_, name, _)| Family::Name(name))
}

/// cosmic-text attributes for text in style `t`.
fn attrs_of(t: &ResolvedText) -> Attrs<'static> {
    let family = match t.family {
        FontFamily::Sans => sans(t.size),
        FontFamily::Mono => Family::Monospace,
    };
    let mut attrs = Attrs::new().family(family).weight(Weight(t.weight));
    if t.italic {
        attrs = attrs.style(FontStyle::Italic);
    }
    if t.tracking != 0.0 {
        attrs = attrs.letter_spacing(t.tracking);
    }
    if t.tabular_nums {
        let mut features = cosmic_text::FontFeatures::new();
        features.enable(cosmic_text::FeatureTag::new(b"tnum"));
        attrs = attrs.font_features(features);
    }
    attrs
}

/// The attribute spans of `p` over base style `t`. A run's left and right
/// padding become extra letter spacing on its first and last character, so
/// lines wrap with the padding counted; the first glyph is then drawn shifted.
fn spans<'s>(p: Paragraph<'s>, t: &ResolvedText) -> Vec<(&'s str, Attrs<'static>)> {
    let content = p.content;
    let mut out = Vec::with_capacity(p.runs.len().saturating_mul(2).saturating_add(1));
    let mut at = 0;
    for (i, run) in p.runs.iter().enumerate() {
        if let Some(gap) = content.get(at..run.range.start).filter(|g| !g.is_empty()) {
            out.push((gap, attrs_of(t)));
        }
        at = run.range.end.max(at);
        let Some(piece) = content.get(run.range.clone()).filter(|p| !p.is_empty()) else {
            continue;
        };
        let mut attrs = attrs_of(&run.text);
        if run.text.size != t.size {
            attrs = attrs.metrics(Metrics::new(run.text.size, t.line_height));
        }
        let em = |px: f32| run.text.tracking + px / run.text.size.max(1.0);
        let (left, right) = (run.padding.left, run.padding.right);
        let first_len = piece.chars().next().map_or(0, char::len_utf8);
        let last_at = piece.char_indices().last().map_or(0, |(j, _)| j);
        let tagged = |a: Attrs<'static>, first: bool| a.metadata(run_tag(i, first));
        if left == 0.0 && right == 0.0 {
            out.push((piece, tagged(attrs, false)));
        } else if last_at == 0 {
            out.push((piece, tagged(attrs.letter_spacing(em(left + right)), true)));
        } else {
            let (first, rest) = piece.split_at(first_len);
            let (middle, last) = rest.split_at(last_at.saturating_sub(first_len));
            out.push((first, tagged(attrs.clone().letter_spacing(em(left)), true)));
            if !middle.is_empty() {
                out.push((middle, tagged(attrs.clone(), false)));
            }
            out.push((last, tagged(attrs.letter_spacing(em(right)), false)));
        }
    }
    if let Some(tail) = content.get(at..).filter(|g| !g.is_empty()) {
        out.push((tail, attrs_of(t)));
    }
    out
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ShapeKey {
    content: String,
    runs: Vec<RunKey>,
    size: u32,
    line_height: u32,
    weight: u16,
    family: FontFamily,
    italic: bool,
    breaks: Breaks,
    truncate: bool,
    clamp: Option<u16>,
    wrap_style: WrapStyle,
    width: Option<u32>,
    align: TextAlign,
    tracking: u32,
    tabular_nums: bool,
}

struct Shaped {
    buffer: Buffer,
    /// How far right the lines sit in the box: balanced lines are laid out
    /// narrower than it, and still aligned in all of it.
    shift: f32,
    /// The ascent and descent of the paragraph's primary font at its size,
    /// which place each line's baseline (see [`baseline`]).
    primary: Option<(f32, f32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    glyph: CacheKey,
    rgba: [u8; 4],
}

struct GlyphImage {
    pixmap: Pixmap,
    left: i32,
    top: i32,
}

/// Fonts, shaped text and rasterised glyphs, kept across frames.
pub struct TextSystem {
    fonts: FontSystem,
    swash: SwashCache,
    shaped: FrameCache<ShapeKey, Shaped>,
    glyphs: FrameCache<GlyphKey, Option<GlyphImage>>,
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a caret sits: its top-left and height, relative to the text box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub x: f32,
    pub y: f32,
    pub height: f32,
}

impl TextSystem {
    #[must_use]
    pub fn new() -> Self {
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_system_fonts();
        db.load_font_data(INTER.to_vec());
        db.load_font_data(GEIST_MONO.to_vec());
        for (_, _, data) in INTER_OPTICAL {
            db.load_font_data(data.to_vec());
        }
        db.set_sans_serif_family("Inter");
        db.set_monospace_family("Geist Mono");
        Self {
            fonts: FontSystem::new_with_locale_and_db("en-US".to_owned(), db),
            swash: SwashCache::new(),
            shaped: FrameCache::default(),
            glyphs: FrameCache::default(),
        }
    }

    fn key(p: Paragraph<'_>, t: &ResolvedText, width: Option<f32>, breaks: Breaks) -> ShapeKey {
        ShapeKey {
            content: p.content.to_owned(),
            runs: p.runs.iter().map(RunKey::of).collect(),
            size: t.size.to_bits(),
            line_height: t.line_height.to_bits(),
            weight: t.weight,
            family: t.family,
            italic: t.italic,
            breaks,
            truncate: t.truncate,
            clamp: t.line_clamp,
            wrap_style: t.wrap_style,
            width: width.map(f32::to_bits),
            align: t.physical_align(),
            tracking: t.tracking.to_bits(),
            tabular_nums: t.tabular_nums,
        }
    }

    /// `p` laid out in style `t` in a box `width` wide, and how far right its lines sit.
    fn shape(&mut self, p: Paragraph<'_>, t: &ResolvedText, width: Option<f32>) -> &Shaped {
        self.shape_breaking(p, t, width, Breaks::of(t))
    }

    /// `p` laid out in style `t`, its lines broken at `breaks` to fit `width`.
    fn shape_breaking(&mut self, p: Paragraph<'_>, t: &ResolvedText, width: Option<f32>, breaks: Breaks) -> &Shaped {
        let key = Self::key(p, t, width, breaks);
        let fonts = &mut self.fonts;
        self.shaped.get_or_insert_with(key, || {
            let mut buffer = Buffer::new(fonts, Metrics::new(t.size, t.line_height));
            buffer.set_wrap(breaks.wrap());
            // A min-content measure cuts nothing short: it finds the widest word.
            let cut = breaks != Breaks::Words;
            if cut && t.truncate {
                buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)));
            } else if cut && let Some(lines) = t.line_clamp {
                buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(usize::from(lines))));
            }
            buffer.set_size(width, None);
            let attrs = attrs_of(t);
            let align = match t.physical_align() {
                TextAlign::Left | TextAlign::Start | TextAlign::End => Some(cosmic_text::Align::Left),
                TextAlign::Center => Some(cosmic_text::Align::Center),
                TextAlign::Right => Some(cosmic_text::Align::Right),
            };
            if p.runs.is_empty() {
                buffer.set_text(p.content, &attrs, Shaping::Advanced, align);
            } else {
                buffer.set_rich_text(spans(p, t), &attrs, Shaping::Advanced, align);
            }
            buffer.shape_until_scroll(fonts, false);
            let rebreaks = breaks == Breaks::WordsOrGlyphs && t.line_clamp.is_none();
            let shift = match width {
                Some(width) if rebreaks && t.wrap_style == WrapStyle::Pretty => {
                    pretty(&mut buffer, fonts, width);
                    0.0
                }
                Some(width) if rebreaks && t.wrap_style == WrapStyle::Balance => {
                    let balanced = balance(&mut buffer, fonts, width);
                    match t.physical_align() {
                        TextAlign::Left | TextAlign::Start | TextAlign::End => 0.0,
                        TextAlign::Center => (width - balanced) / 2.0,
                        TextAlign::Right => width - balanced,
                    }
                }
                _ => 0.0,
            };
            let primary = primary_metrics(fonts, t);
            Shaped { buffer, shift, primary }
        })
    }

    /// The size `p` takes in style `t`, wrapped to `width` when given.
    pub fn measure<'p>(&mut self, p: impl Into<Paragraph<'p>>, t: &ResolvedText, width: Option<f32>) -> (f32, f32) {
        let line_height = t.line_height;
        let buffer = &self.shape(p.into(), t, width).buffer;
        let mut lines = 0_u32;
        for _ in buffer.layout_runs() {
            lines = lines.saturating_add(1);
        }
        (widest_line(buffer), u32_to_f32(lines.max(1)) * line_height)
    }

    /// The min-content width of `p` in style `t`: its widest word, or all of
    /// it when it does not wrap. Breaking inside a word
    /// (`overflow-wrap: break-word`) does not narrow it, as in CSS.
    pub fn min_content<'p>(&mut self, p: impl Into<Paragraph<'p>>, t: &ResolvedText) -> f32 {
        let p = p.into();
        if t.nowrap {
            return self.measure(p, t, None).0;
        }
        widest_line(&self.shape_breaking(p, t, Some(0.0), Breaks::Words).buffer)
    }

    /// The byte offset in `content` nearest to `(x, y)`, relative to the text box.
    pub fn hit(&mut self, content: &str, t: &ResolvedText, width: Option<f32>, x: f32, y: f32) -> usize {
        let Shaped { buffer, shift, .. } = self.shape(content.into(), t, width);
        let Some(cursor) = buffer.hit(x - shift, y) else {
            return content.len();
        };
        line_start(content, cursor.line)
            .saturating_add(cursor.index)
            .min(content.len())
    }

    /// Where the caret for byte offset `at` sits, relative to the text box.
    pub fn caret(&mut self, content: &str, t: &ResolvedText, width: Option<f32>, at: usize) -> Caret {
        let (line, index) = line_and_index(content, at);
        let line_height = t.line_height;
        let Shaped { buffer, shift, .. } = self.shape(content.into(), t, width);
        let (x, y) = buffer.cursor_position(&Cursor::new(line, index)).unwrap_or((0.0, 0.0));
        Caret {
            x: x + shift,
            y,
            height: line_height,
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
        let p = p.into();
        let (l0, i0) = line_and_index(p.content, from);
        let (l1, i1) = line_and_index(p.content, to);
        let (start, end) = (Cursor::new(l0, i0), Cursor::new(l1, i1));
        let Shaped { buffer, shift, .. } = self.shape(p, t, width);
        let mut boxes = Vec::new();
        for run in buffer.layout_runs() {
            for (x, w) in run.highlight(start, end) {
                boxes.push(Bounds {
                    x: x + shift,
                    y: run.line_top,
                    width: w.max(1.0),
                    height: run.line_height,
                });
            }
        }
        boxes
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
        let runs = p.runs;
        let run_of = |metadata: usize| untag(metadata).0.and_then(|i| runs.get(i));
        let scale = target.scale;
        let mut placed: Vec<(GlyphKey, i32, i32, Color)> = Vec::new();
        let mut lines: Vec<(Bounds, Color)> = Vec::new();
        let mut backgrounds: Vec<Background> = Vec::new();
        let Shaped {
            buffer, shift, primary, ..
        } = self.shape(p, t, width);
        let origin = (origin.0 + shift, origin.1);
        for line in buffer.layout_runs() {
            let device = baseline(origin.1, &line, *primary, scale);
            let base = device / scale;
            for glyph in line.glyphs {
                let run = run_of(glyph.metadata);
                let first = untag(glyph.metadata).1;
                let dx = run.filter(|_| first).map_or(0.0, |r| r.padding.left);
                let color = run.map_or(t.color, |r| r.text.color).opacity(opacity);
                let phys = glyph.physical(((origin.0 + dx) * scale, device), scale);
                placed.push((
                    GlyphKey {
                        glyph: phys.cache_key,
                        rgba: color.to_rgba8(),
                    },
                    phys.x,
                    phys.y,
                    color,
                ));
            }
            for group in groups(&line) {
                let style = run_of(group.metadata).map_or(t, |r| &r.text);
                let color = style.color.opacity(opacity);
                let thickness = (style.size / 14.0).max(1.0);
                let x = origin.0 + group.x;
                if style.underline {
                    let below = style.underline_offset.unwrap_or(style.size * 0.12);
                    lines.push((
                        Bounds {
                            x,
                            y: base + below,
                            width: group.width,
                            height: thickness,
                        },
                        color,
                    ));
                }
                if style.line_through {
                    lines.push((
                        Bounds {
                            x,
                            y: base - style.size * 0.3,
                            width: group.width,
                            height: thickness,
                        },
                        color,
                    ));
                }
                if let Some(run) = run_of(group.metadata)
                    && let Some(bg) = run.background
                {
                    backgrounds.push(Background {
                        x,
                        baseline: base,
                        width: group.width,
                        font: group.font,
                        size: run.text.size,
                        radius: run.radius,
                        padding: run.padding,
                        color: bg.opacity(opacity),
                    });
                }
            }
        }
        for bg in backgrounds {
            let (ascent, descent) = self.fonts.get_font(bg.font.0, bg.font.1).map_or((0.97, 0.24), |f| {
                let m = f.metrics();
                let em = f32::from(m.units_per_em).max(1.0);
                (m.ascent / em, -m.descent / em)
            });
            let top = bg.baseline - ascent * bg.size - bg.padding.top;
            let b = Bounds {
                x: bg.x,
                y: top,
                width: bg.width,
                height: bg.baseline + descent * bg.size + bg.padding.bottom - top,
            };
            target.fill_rounded(b, bg.radius, bg.color);
        }
        for (key, x, y, color) in placed {
            let fonts = &mut self.fonts;
            let swash = &mut self.swash;
            let image = self
                .glyphs
                .get_or_insert_with(key, || glyph_image(fonts, swash, key, color));
            if let Some(g) = image {
                target.pixmap.draw_pixmap(
                    x.saturating_add(g.left),
                    y.saturating_sub(g.top),
                    g.pixmap.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    target.clip,
                );
            }
        }
        for (b, color) in lines {
            target.fill_rect(b, color);
        }
    }

    /// Forget the shaped text and glyphs the last frames did not draw; call once per frame.
    pub fn end_frame(&mut self) {
        self.shaped.end_frame();
        self.glyphs.end_frame();
    }

    /// Paragraphs shaped and glyph images held.
    pub(crate) fn footprint(&self) -> (usize, Raster) {
        (
            self.shaped.len(),
            Raster::of(self.glyphs.values().flatten().map(|g| &g.pixmap)),
        )
    }
}

/// Where a frame is painted: the pixmap, its scale and the current clip.
pub struct Target<'a> {
    pub pixmap: &'a mut Pixmap,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    pub clip: Option<&'a Mask>,
}

impl Target<'_> {
    /// Fill a logical-pixel rectangle with corners of radius `radius`.
    pub fn fill_rounded(&mut self, b: Bounds, radius: f32, color: Color) {
        if radius <= 0.0 {
            self.fill_rect(b, color);
            return;
        }
        if let Some(path) = crate::paint::rrect(b, crate::style::Corners::all(radius)) {
            let mut paint = tiny_skia::Paint::default();
            paint.set_color(color.to_skia());
            paint.anti_alias = true;
            self.pixmap.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::from_scale(self.scale, self.scale),
                self.clip,
            );
        }
    }

    /// Fill a logical-pixel rectangle with a solid colour.
    pub fn fill_rect(&mut self, b: Bounds, color: Color) {
        if let Some(rect) = tiny_skia::Rect::from_xywh(b.x, b.y, b.width, b.height) {
            let mut paint = tiny_skia::Paint::default();
            paint.set_color(color.to_skia());
            self.pixmap
                .fill_rect(rect, &paint, Transform::from_scale(self.scale, self.scale), self.clip);
        }
    }
}

/// A run's background on one line, waiting for its font's metrics.
struct Background {
    x: f32,
    baseline: f32,
    width: f32,
    font: (cosmic_text::fontdb::ID, cosmic_text::fontdb::Weight),
    size: f32,
    radius: f32,
    padding: Edges<f32>,
    color: Color,
}

/// Glyphs of one line that belong to the same run (or to none), left to right.
struct Group {
    metadata: usize,
    /// From the line's start.
    x: f32,
    width: f32,
    font: (cosmic_text::fontdb::ID, cosmic_text::fontdb::Weight),
}

/// The line's glyphs grouped by run, trailing white space left out as CSS
/// decorates no hanging space.
fn groups(line: &cosmic_text::LayoutRun<'_>) -> Vec<Group> {
    let tag = |m: usize| untag(m).0;
    let inked = line
        .glyphs
        .iter()
        .rposition(|g| {
            !line
                .text
                .get(g.start..g.end)
                .is_some_and(|c| c.chars().all(char::is_whitespace))
        })
        .map_or(0, |i| i.saturating_add(1));
    let mut out: Vec<Group> = Vec::new();
    for g in line.glyphs.iter().take(inked) {
        match out.last_mut() {
            Some(last) if tag(last.metadata) == tag(g.metadata) => {
                let right = (g.x + g.w).max(last.x + last.width);
                last.x = last.x.min(g.x);
                last.width = right - last.x;
            }
            _ => out.push(Group {
                metadata: g.metadata,
                x: g.x,
                width: g.w,
                font: (g.font_id, g.font_weight),
            }),
        }
    }
    out
}

/// The ascent and descent of the primary font of text in style `t`, in
/// pixels at its size. The primary font is the style's own, as Blink takes
/// the first font of `font-family` for a line's metrics: glyphs another font
/// fills in (Persian digits, an emoji) sit on the baseline it sets.
fn primary_metrics(fonts: &mut FontSystem, t: &ResolvedText) -> Option<(f32, f32)> {
    let attrs = attrs_of(t);
    let query = cosmic_text::fontdb::Query {
        families: &[attrs.family],
        weight: attrs.weight,
        stretch: attrs.stretch,
        style: attrs.style,
    };
    let id = fonts.db().query(&query)?;
    let font = fonts.get_font(id, attrs.weight)?;
    let m = font.metrics();
    let em = f32::from(m.units_per_em).max(1.0);
    Some((m.ascent / em * t.size, -m.descent / em * t.size))
}

/// Where `line` of a paragraph whose box's top is at window `y` has its
/// baseline, in device pixels, as Blink places it: the primary font's ascent
/// and descent each rounded to whole pixels (`SimpleFontData`) and the half
/// of the leading above them floored (`CalculateLeadingSpace`), so the
/// baseline lies a whole number of pixels below the line's top (measured in
/// Chrome: Geist Mono at 14px on a 22.75px line has its 18px content box 2px
/// below each line's top, not 2.375px); then on the nearest device pixel, a
/// half rounding up as Skia draws horizontal text (a fractional baseline
/// blurs every glyph on it). A line at a fractional place (y = 25.25 at 2x)
/// is such a half.
fn baseline(y: f32, line: &cosmic_text::LayoutRun<'_>, primary: Option<(f32, f32)>, scale: f32) -> f32 {
    let offset = primary.map_or(line.line_y - line.line_top, |(ascent, descent)| {
        let (ascent, descent) = (ascent.round(), descent.round());
        ((line.line_height - (ascent + descent)) / 2.0).floor() + ascent
    });
    ((y + line.line_top + offset) * scale + 0.5).floor()
}

/// The width of `buffer`'s longest line, rounded up to the layout unit
/// (1/64 px) as Blink sizes a box to its text: the box is never narrower
/// than its text, so the text keeps its lines when laid out in it.
fn widest_line(buffer: &Buffer) -> f32 {
    let w = buffer.layout_runs().map(|run| run.line_w).fold(0.0_f32, f32::max);
    (w * LAYOUT_UNITS).ceil() / LAYOUT_UNITS
}

/// Most lines `text-wrap: pretty` re-breaks; Chromium looks at a paragraph's last four.
const PRETTY_MAX_LINES: usize = 4;

/// Whether the last line of `buffer` holds a single word.
fn orphan(buffer: &Buffer) -> bool {
    buffer.layout_runs().last().is_some_and(|run| {
        let words = run
            .glyphs
            .first()
            .zip(run.glyphs.last())
            .and_then(|(a, b)| run.text.get(a.start..b.end))
            .map_or(0, |s| s.split_whitespace().count());
        words < 2
    })
}

/// Narrow `buffer`, laid out at `width`, until its last line is not one word
/// alone, keeping its line count, as Chromium's `text-wrap: pretty` does for
/// short paragraphs; left as it is when no width does.
fn pretty(buffer: &mut Buffer, fonts: &mut FontSystem, width: f32) {
    let lines = |buffer: &Buffer| buffer.layout_runs().count();
    let count = lines(buffer);
    if !(2..=PRETTY_MAX_LINES).contains(&count) || !orphan(buffer) {
        return;
    }
    // Find the widest width that sends a word down to the last line.
    let (mut wide, mut narrow) = (width, 0.0_f32);
    while wide - narrow > 1.0 / LAYOUT_UNITS {
        let mid = (wide + narrow) / 2.0;
        buffer.set_size(Some(mid), None);
        buffer.shape_until_scroll(fonts, false);
        if lines(buffer) == count && orphan(buffer) {
            wide = mid;
        } else {
            narrow = mid;
        }
    }
    buffer.set_size(Some(narrow), None);
    buffer.shape_until_scroll(fonts, false);
    if lines(buffer) != count || orphan(buffer) {
        buffer.set_size(Some(width), None);
        buffer.shape_until_scroll(fonts, false);
    }
}

/// Most lines Chromium balances; longer text wraps as usual.
const BALANCE_MAX_LINES: usize = 6;

/// Narrow `buffer`, laid out at `width`, to the least width that keeps its
/// line count, as Chromium balances (`text-wrap: balance`); that width.
fn balance(buffer: &mut Buffer, fonts: &mut FontSystem, width: f32) -> f32 {
    let lines = |buffer: &Buffer| buffer.layout_runs().count();
    let count = lines(buffer);
    if !(2..=BALANCE_MAX_LINES).contains(&count) {
        return width;
    }
    let (mut fits, mut wraps) = (width, 0.0_f32);
    while fits - wraps > 1.0 / LAYOUT_UNITS {
        let mid = (fits + wraps) / 2.0;
        buffer.set_size(Some(mid), None);
        buffer.shape_until_scroll(fonts, false);
        if lines(buffer) == count {
            fits = mid;
        } else {
            wraps = mid;
        }
    }
    buffer.set_size(Some(fits), None);
    buffer.shape_until_scroll(fonts, false);
    fits
}

fn glyph_image(fonts: &mut FontSystem, swash: &mut SwashCache, key: GlyphKey, color: Color) -> Option<GlyphImage> {
    let image = swash.get_image_uncached(fonts, key.glyph)?;
    let (w, h) = (image.placement.width, image.placement.height);
    let mut pixmap = Pixmap::new(w.max(1), h.max(1))?;
    let [r, g, b, a] = color.to_rgba8();
    match image.content {
        SwashContent::Mask => {
            for (px, &coverage) in pixmap.pixels_mut().iter_mut().zip(image.data.iter()) {
                let alpha = mul_u8(coverage, a);
                *px = tiny_skia::ColorU8::from_rgba(r, g, b, alpha).premultiply();
            }
        }
        SwashContent::Color | SwashContent::SubpixelMask => {
            let (rgba, _) = image.data.as_chunks::<4>();
            for (px, &[cr, cg, cb, ca]) in pixmap.pixels_mut().iter_mut().zip(rgba) {
                *px = tiny_skia::ColorU8::from_rgba(cr, cg, cb, mul_u8(ca, a)).premultiply();
            }
        }
    }
    Some(GlyphImage {
        pixmap,
        left: image.placement.left,
        top: image.placement.top,
    })
}

/// `a * b / 255`, rounded.
fn mul_u8(a: u8, b: u8) -> u8 {
    let p = u16::from(a).saturating_mul(u16::from(b));
    u8::try_from(p.saturating_add(127) / 255).unwrap_or(u8::MAX)
}

/// The byte offset where line `line` (0-based, split on `\n`) starts.
fn line_start(content: &str, line: usize) -> usize {
    content
        .match_indices('\n')
        .nth(line.wrapping_sub(1))
        .filter(|_| line > 0)
        .map_or(0, |(i, _)| i.saturating_add(1))
}

/// Byte offset `at` as (line, byte index within the line).
fn line_and_index(content: &str, at: usize) -> (usize, usize) {
    let before = content.get(..at).unwrap_or(content);
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |i| i.saturating_add(1));
    (line, at.saturating_sub(start))
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
mod tests {
    use super::*;

    #[test]
    fn lines_map_to_byte_offsets_and_back() {
        let s = "ab\ncd\ne";
        assert_eq!(line_start(s, 0), 0);
        assert_eq!(line_start(s, 1), 3);
        assert_eq!(line_start(s, 2), 6);
        assert_eq!(line_and_index(s, 4), (1, 1));
        assert_eq!(line_and_index(s, 7), (2, 1));
    }

    #[test]
    fn wrapping_to_a_narrow_width_adds_lines() {
        let mut ts = TextSystem::new();
        let t = ResolvedText::root(Color::BLACK);
        let (w, h) = ts.measure("hello wide world", &t, None);
        assert!(w > 50.0 && (h - 24.0).abs() < 0.1, "one line: {w}x{h}");
        let (_, h2) = ts.measure("hello wide world", &t, Some(w / 2.0));
        assert!(h2 >= 48.0, "wraps to two or more lines: {h2}");
    }
}
