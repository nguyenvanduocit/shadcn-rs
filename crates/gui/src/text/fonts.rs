//! Fonts and glyphs: the faces `gui` embeds and the system's (fontique),
//! the small ids a glyph is keyed by (a face, a set of variation
//! coordinates), and each glyph rasterized from them (ADR-0019).

use foldhash::HashMap;
use std::sync::Arc;

use parley::fontique::{Blob, Collection, CollectionOptions, FontInfoOverride, SourceCache};
use parley::{FontContext, FontData, LayoutContext};
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, Engine, GlyphStyles, HintingInstance, HintingOptions, SmoothMode, Target};
use skrifa::{GlyphId, MetadataProvider};

use super::color_glyph;
use super::raster::{self, Affine, Outline};
use super::settings::{Subpixel, TextSettings};

const INTER: &[u8] = include_bytes!("../../assets/Inter.ttf");
const GEIST_MONO: &[u8] = include_bytes!("../../assets/GeistMono.ttf");

/// The family names the embedded faces are registered under: names of
/// their own, so a copy of Inter installed on the machine (perhaps static,
/// perhaps another version) never stands in for the one `gui` ships.
pub(crate) const SANS: &str = "gui Inter";
pub(crate) const MONO: &str = "gui Geist Mono";

/// The pixels per em glyph curves are drawn at, for curve text.
pub(crate) const CURVE_EM: f32 = 1024.0;

/// One glyph as the atlas keeps it: everything that changes its pixels
/// except its colour, which the shader applies (with the gamma table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// The face, as [`Fonts::face`] interned it.
    pub font: u32,
    pub glyph: u32,
    /// Device pixels per em, as `f32` bits.
    pub ppem: u32,
    /// The variation coordinates (`opsz`, `wght`, …), interned.
    pub axes: u16,
    /// Where the pen falls within a device pixel, in quarters.
    pub bin: u8,
    /// [`GlyphKey::HINTED`], [`GlyphKey::EMBOLDEN`], [`GlyphKey::COLOR`].
    pub flags: u8,
    /// A synthetic slant, in degrees (fontique's `Synthesis::skew`).
    pub skew: i8,
}

impl GlyphKey {
    /// Hinted by the autohinter (drawn below 2×).
    pub const HINTED: u8 = 1;
    /// A synthetic bold: the face has no weight this heavy.
    pub const EMBOLDEN: u8 = 2;
    /// A colour glyph (COLR, sbix, CBDT): drawn into an RGBA page.
    pub const COLOR: u8 = 4;

    #[must_use]
    pub fn is_color(&self) -> bool {
        self.flags & Self::COLOR != 0
    }
}

/// What a glyph's pixels hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphContent {
    /// Coverage, one byte a pixel.
    Mask,
    /// Coverage per colour stripe, RGBA (alpha the most of the three).
    Lcd,
    /// Premultiplied colour, RGBA.
    Color,
}

/// A rasterized glyph: its pixels and where they sit from its pen on the baseline.
#[derive(Clone, Debug)]
pub struct GlyphImage {
    pub left: i32,
    /// Rows above the baseline.
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub content: GlyphContent,
    pub data: Vec<u8>,
}

/// How a glyph's coverage is wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Coverage {
    Gray,
    Lcd(Subpixel),
}

/// Fonts, the contexts Parley shapes in, and what rasterizing reuses.
pub(crate) struct Fonts {
    pub(crate) fcx: FontContext,
    pub(crate) lcx: LayoutContext<u32>,
    faces: Vec<FontData>,
    /// Whether each face has a colour table (COLR, sbix or CBDT) at all.
    colored: Vec<bool>,
    face_ids: HashMap<(u64, u32), u32>,
    axes: Vec<Vec<i16>>,
    axes_ids: HashMap<Vec<i16>, u16>,
    color: HashMap<(u32, u32), bool>,
    /// How many faces and coordinate sets the renderer's [`Glyphs`] was
    /// given ([`Fonts::glyph_tables`]).
    sent: (usize, usize),
}

/// Hinting instances kept before the oldest are dropped.
const MAX_HINTING: usize = 256;

/// The process's font collection and loaded font files, shared by every
/// thread's text system (the layout pool's included): the system's fonts
/// are listed once, and a file is read once however many threads shape
/// with it.
fn shared_fonts() -> (Collection, SourceCache) {
    static SHARED: std::sync::OnceLock<std::sync::Mutex<(Collection, SourceCache)>> = std::sync::OnceLock::new();
    let shared = SHARED.get_or_init(|| {
        let mut collection = Collection::new(CollectionOptions {
            shared: true,
            system_fonts: true,
        });
        for (data, name) in [(INTER, SANS), (GEIST_MONO, MONO)] {
            collection.register_fonts(
                Blob::new(Arc::new(data)),
                Some(FontInfoOverride {
                    family_name: Some(name),
                    ..FontInfoOverride::default()
                }),
            );
        }
        std::sync::Mutex::new((collection, SourceCache::new_shared()))
    });
    let guard = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    (guard.0.clone(), guard.1.clone())
}

impl Fonts {
    pub(crate) fn new() -> Self {
        let (collection, source_cache) = shared_fonts();
        Self {
            fcx: FontContext {
                collection,
                source_cache,
            },
            lcx: LayoutContext::new(),
            faces: Vec::new(),
            colored: Vec::new(),
            face_ids: HashMap::default(),
            axes: Vec::new(),
            axes_ids: HashMap::default(),
            color: HashMap::default(),
            sent: (0, 0),
        }
    }

    /// The id of face `font`, given the first time it is seen.
    pub(crate) fn face(&mut self, font: &FontData) -> u32 {
        let key = (font.data.id(), font.index);
        if let Some(id) = self.face_ids.get(&key) {
            return *id;
        }
        let id = u32::try_from(self.faces.len()).unwrap_or(u32::MAX);
        let colored = skrifa::FontRef::from_index(font.data.data(), font.index).is_ok_and(|f| {
            use skrifa::raw::TableProvider;
            f.colr().is_ok() || f.sbix().is_ok() || f.cbdt().is_ok()
        });
        self.faces.push(font.clone());
        self.colored.push(colored);
        self.face_ids.insert(key, id);
        id
    }

    /// The id of a set of normalized variation coordinates.
    pub(crate) fn axes(&mut self, coords: impl Iterator<Item = i16>) -> u16 {
        let bits: Vec<i16> = coords.collect();
        if let Some(id) = self.axes_ids.get(&bits) {
            return *id;
        }
        let id = u16::try_from(self.axes.len()).unwrap_or(u16::MAX);
        self.axes.push(bits.clone());
        self.axes_ids.insert(bits, id);
        id
    }

    fn font_ref(&self, font: u32) -> Option<skrifa::FontRef<'_>> {
        let data = self.faces.get(usize::try_from(font).ok()?)?;
        skrifa::FontRef::from_index(data.data.data(), data.index).ok()
    }

    /// Whether glyph `glyph` of face `font` is drawn in colour.
    pub(crate) fn is_color(&mut self, font: u32, glyph: u32) -> bool {
        // Most faces have no colour table: no glyph of theirs is looked up.
        if !usize::try_from(font)
            .ok()
            .and_then(|i| self.colored.get(i))
            .copied()
            .unwrap_or(false)
        {
            return false;
        }
        if let Some(c) = self.color.get(&(font, glyph)) {
            return *c;
        }
        let c = self
            .font_ref(font)
            .is_some_and(|f| color_glyph::has_color(&f, GlyphId::new(glyph)));
        self.color.insert((font, glyph), c);
        c
    }

    /// How far [`Self::glyph_tables`] has sent.
    pub(crate) fn sent_tables(&self) -> (usize, usize) {
        self.sent
    }

    pub(crate) fn restore_sent(&mut self, sent: (usize, usize)) {
        self.sent = sent;
    }

    /// The faces and coordinate sets interned since the last call (all of
    /// them when `all`): what a renderer's [`Glyphs`] must add to key the
    /// glyphs painted since by the same ids.
    pub(crate) fn glyph_tables(&mut self, all: bool) -> (Vec<FontData>, Vec<Vec<i16>>) {
        let (faces, axes) = if all { (0, 0) } else { self.sent };
        self.sent = (self.faces.len(), self.axes.len());
        (
            self.faces.get(faces..).unwrap_or_default().to_vec(),
            self.axes.get(axes..).unwrap_or_default().to_vec(),
        )
    }
}

/// The face [`Glyphs::test_glyph`] keys: the shipped sans, outside the
/// shared table.
const TEST_FACE: u32 = u32::MAX;

/// What a renderer rasterizes glyphs from (ADR-0015: the render thread owns
/// it, apart from the UI thread's shaping): the faces and variation
/// coordinates glyph keys name, in the ids the UI's text system gave them,
/// the text settings and gamma glyphs are drawn with, and the hinting each
/// face and size reuses. The UI sends what it interned since in each commit
/// ([`super::GlyphUpdate`]).
pub struct Glyphs {
    faces: Vec<FontData>,
    axes: Vec<Vec<i16>>,
    /// The autohinter's analysis of each face's scripts, done once a face.
    styles: HashMap<u32, GlyphStyles>,
    hinting: HashMap<(u32, u16, u32), Option<HintingInstance>>,
    pub(crate) settings: TextSettings,
    pub(crate) gamma: super::gamma::GammaParams,
    pub(crate) table: Arc<super::gamma::MaskGamma>,
    generation: u64,
}

impl Glyphs {
    pub(crate) fn new(
        settings: TextSettings,
        gamma: super::gamma::GammaParams,
        table: Arc<super::gamma::MaskGamma>,
        generation: u64,
    ) -> Self {
        Self {
            faces: Vec::new(),
            axes: Vec::new(),
            styles: HashMap::default(),
            hinting: HashMap::default(),
            settings,
            gamma,
            table,
            generation,
        }
    }

    /// Glyph tables with nothing in them yet, drawing as this OS does until
    /// the first update brings the UI's settings.
    #[must_use]
    pub fn for_os() -> Self {
        let settings = TextSettings::default_for_os();
        let gamma = super::gamma_for(&settings);
        let table = Arc::new(super::gamma::MaskGamma::new(&gamma));
        Self::new(settings, gamma, table, u64::MAX)
    }

    /// Take in what the UI's text system interned or changed since its last update.
    pub fn apply(&mut self, update: super::GlyphUpdate) {
        let super::GlyphUpdate { faces, axes, settings } = update;
        self.faces.extend(faces);
        self.axes.extend(axes);
        if let Some(s) = settings {
            self.settings = s.settings;
            self.gamma = s.gamma;
            self.table = s.table;
            self.generation = s.generation;
            self.hinting.clear();
        }
    }

    /// Changes each time the settings change how glyphs look.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The settings glyphs rasterize with.
    #[must_use]
    pub fn settings(&self) -> &TextSettings {
        &self.settings
    }

    /// The gamma and contrast glyph coverage is corrected with, and its table.
    #[must_use]
    pub fn gamma(&self) -> (&super::gamma::GammaParams, &super::gamma::MaskGamma) {
        (&self.gamma, &self.table)
    }

    fn data(&self, font: u32) -> Option<FontData> {
        if font == TEST_FACE {
            static BLOB: std::sync::OnceLock<Blob<u8>> = std::sync::OnceLock::new();
            return Some(FontData::new(
                BLOB.get_or_init(|| Blob::new(Arc::new(INTER))).clone(),
                0,
            ));
        }
        self.faces.get(usize::try_from(font).ok()?).cloned()
    }

    fn coords(&self, axes: u16) -> Vec<NormalizedCoord> {
        self.axes
            .get(usize::from(axes))
            .map(|v| v.iter().map(|b| NormalizedCoord::from_bits(*b)).collect())
            .unwrap_or_default()
    }

    /// Whether the colour glyph `key` paints with the text's colour, so a
    /// renderer keys it by that colour too.
    #[must_use]
    pub fn uses_foreground(&self, key: &GlyphKey) -> bool {
        key.is_color()
            && self.data(key.font).is_some_and(|d| {
                skrifa::FontRef::from_index(d.data.data(), d.index)
                    .is_ok_and(|f| color_glyph::uses_foreground(&f, GlyphId::new(key.glyph)))
            })
    }

    /// Glyph `key`'s outline as quadratic curves, in pixels at
    /// [`CURVE_EM`] per em from its pen, y down: what curve text
    /// (Slug) draws at any size and under any transform. None for a colour
    /// glyph or one with no ink.
    pub(crate) fn curves(&mut self, key: &GlyphKey) -> Option<Vec<Vec<[[f32; 2]; 3]>>> {
        let settings = self.settings;
        self.outline_curves(key, &settings)
    }

    /// The pixels of glyph `key` as `coverage` asks, rasterized by `gui`;
    /// `foreground` is the text's colour, for a colour glyph that paints
    /// with it. A renderer caches what it makes of it.
    pub fn raster(
        &mut self,
        key: &GlyphKey,
        coverage: Coverage,
        foreground: crate::color::Color,
    ) -> Option<GlyphImage> {
        let settings = self.settings;
        self.render(key, coverage, &settings, foreground.to_rgba8())
    }

    /// The hinting instance for a face at a size and variation, made once.
    fn hinter(&mut self, key: &GlyphKey) -> Option<&HintingInstance> {
        let id = (key.font, key.axes, key.ppem);
        if !self.hinting.contains_key(&id) {
            if self.hinting.len() >= MAX_HINTING {
                self.hinting.clear();
            }
            let coords = self.coords(key.axes);
            let data = self.data(key.font)?;
            let font = skrifa::FontRef::from_index(data.data.data(), data.index).ok()?;
            let outlines = font.outline_glyphs();
            let styles = self
                .styles
                .entry(key.font)
                .or_insert_with(|| GlyphStyles::new(&outlines))
                .clone();
            // Forced on (not `AutoFallback`, which picks the TrueType
            // interpreter for a font with `prep` but no `fpgm`, as Inter
            // has, and so does nothing): light, vertical only, advances
            // untouched, as FreeType's light autohinting.
            let options = HintingOptions {
                engine: Engine::Auto(Some(styles)),
                target: Target::Smooth {
                    mode: SmoothMode::Light,
                    symmetric_rendering: true,
                    preserve_linear_metrics: true,
                },
            };
            let instance = HintingInstance::new(
                &outlines,
                Size::new(f32::from_bits(key.ppem)),
                LocationRef::new(&coords),
                options,
            )
            .ok();
            self.hinting.insert(id, instance);
        }
        self.hinting.get(&id).and_then(Option::as_ref)
    }

    /// A key for the shipped sans face's "g" at `ppem`: a glyph with curves,
    /// a counter and a descender, for a renderer to test itself with.
    #[must_use]
    pub fn test_glyph(&self, ppem: f32) -> Option<GlyphKey> {
        let glyph = skrifa::FontRef::from_index(INTER, 0).ok()?.charmap().map('g')?;
        Some(GlyphKey {
            font: TEST_FACE,
            glyph: glyph.to_u32(),
            ppem: ppem.to_bits(),
            // No coordinates: the face's defaults.
            axes: u16::MAX,
            bin: 0,
            flags: 0,
            skew: 0,
        })
    }

    /// The outline of `key` as quadratic curves for curve text (Slug,
    /// ADR-0017), in pixels at [`CURVE_EM`] pixels per em, y down, from its
    /// pen: unhinted, slanted as the key says, and thickened as the atlas
    /// would thicken it at the key's size (only then does the size matter).
    fn outline_curves(&mut self, key: &GlyphKey, settings: &TextSettings) -> Option<Vec<Vec<[[f32; 2]; 3]>>> {
        if key.is_color() {
            return None;
        }
        let ppem = f32::from_bits(key.ppem).max(1e-3);
        let skew = if key.skew == 0 {
            0.0
        } else {
            f32::from(key.skew).to_radians().tan()
        };
        let mut outline = Outline::new(Affine {
            xx: 1.0,
            xy: skew,
            yy: -1.0,
            dx: 0.0,
            dy: 0.0,
        });
        let coords = self.coords(key.axes);
        let data = self.data(key.font)?;
        let font = skrifa::FontRef::from_index(data.data.data(), data.index).ok()?;
        let glyph = font.outline_glyphs().get(GlyphId::new(key.glyph))?;
        glyph
            .draw(
                DrawSettings::unhinted(Size::new(CURVE_EM), LocationRef::new(&coords)),
                &mut outline,
            )
            .ok()?;
        if outline.is_empty() {
            return None;
        }
        let mut b = 0.0;
        if key.flags & GlyphKey::EMBOLDEN != 0 {
            b += raster::fake_bold(ppem);
        }
        let (mut bx, mut by) = (b, b);
        if settings.stem_darkening {
            let (dx, dy) = raster::stem_darkening(ppem);
            bx += dx;
            by += dy;
        }
        let k = CURVE_EM / ppem;
        outline.embolden(bx * k, by * k);
        let quads = outline.quadratics(CURVE_EM / 512.0);
        Some(
            quads
                .into_iter()
                .map(|c| c.into_iter().map(|q| q.map(|p| [p.x, p.y])).collect())
                .collect(),
        )
    }

    /// Rasterize `key` as `coverage` asks; `foreground` is the text's colour
    /// for a colour glyph that paints with it.
    fn render(
        &mut self,
        key: &GlyphKey,
        coverage: Coverage,
        settings: &TextSettings,
        foreground: [u8; 4],
    ) -> Option<GlyphImage> {
        let ppem = f32::from_bits(key.ppem);
        let x_offset = f32::from(key.bin) / 4.0;
        if key.is_color() {
            let coords = self.coords(key.axes);
            let data = self.data(key.font)?;
            let font = skrifa::FontRef::from_index(data.data.data(), data.index).ok()?;
            let image = color_glyph::render(&font, GlyphId::new(key.glyph), ppem, &coords, x_offset, foreground)?;
            return Some(GlyphImage {
                left: image.left,
                top: image.top,
                width: image.width,
                height: image.height,
                content: GlyphContent::Color,
                data: image.rgba,
            });
        }
        let (sx, sy) = match coverage {
            Coverage::Lcd(Subpixel::VRgb | Subpixel::VBgr) => (1.0, 3.0),
            Coverage::Lcd(_) => (3.0, 1.0),
            Coverage::Gray => (1.0, 1.0),
        };
        let skew = if key.skew == 0 {
            0.0
        } else {
            f32::from(key.skew).to_radians().tan()
        };
        let mut outline = Outline::new(Affine {
            xx: sx,
            xy: sx * skew,
            yy: -sy,
            dx: x_offset * sx,
            dy: 0.0,
        });
        let hinted = key.flags & GlyphKey::HINTED != 0;
        let coords = self.coords(key.axes);
        // The face's bytes, held apart from `self` so the hinting instance
        // (kept in `self`) and the outline can be borrowed together.
        let data = self.data(key.font)?;
        let font = skrifa::FontRef::from_index(data.data.data(), data.index).ok()?;
        let glyph = font.outline_glyphs().get(GlyphId::new(key.glyph))?;
        if hinted {
            let instance = self.hinter(key)?;
            glyph.draw(DrawSettings::hinted(instance, false), &mut outline).ok()?;
        } else {
            glyph
                .draw(
                    DrawSettings::unhinted(Size::new(ppem), LocationRef::new(&coords)),
                    &mut outline,
                )
                .ok()?;
        }
        if outline.is_empty() {
            return None;
        }
        let (mut bx, mut by) = (0.0, 0.0);
        if key.flags & GlyphKey::EMBOLDEN != 0 {
            let b = raster::fake_bold(ppem);
            bx += b;
            by += b;
        }
        if settings.stem_darkening {
            let (dx, dy) = raster::stem_darkening(ppem);
            bx += dx;
            by += dy;
        }
        outline.embolden(bx * sx, by * sy);
        let (lo, hi) = outline.bounds()?;
        match coverage {
            Coverage::Gray => {
                let (left, top, w, h) = raster::pixel_box(lo, hi);
                outline.translate(-raster::offset(left), -raster::offset(top));
                let data = raster::fill(&outline, w, h);
                Some(GlyphImage {
                    left,
                    top: top.saturating_neg(),
                    width: w,
                    height: h,
                    content: GlyphContent::Mask,
                    data,
                })
            }
            Coverage::Lcd(stripes) => {
                // Whole pixels around the tripled outline, with a pixel more
                // either side for the filter's spread.
                let px = |v: f32, s: f32| crate::convert::f32_to_i32((v / s).floor());
                let (vertical, s) = if sy > sx { (true, sy) } else { (false, sx) };
                let (left, top, right, bottom) = if vertical {
                    (
                        px(lo.x, 1.0),
                        px(lo.y, s).saturating_sub(1),
                        px(hi.x, 1.0).saturating_add(2),
                        px(hi.y, s).saturating_add(2),
                    )
                } else {
                    (
                        px(lo.x, s).saturating_sub(1),
                        px(lo.y, 1.0),
                        px(hi.x, s).saturating_add(2),
                        px(hi.y, 1.0).saturating_add(1),
                    )
                };
                let w = u32::try_from(right.saturating_sub(left)).ok()?;
                let h = u32::try_from(bottom.saturating_sub(top)).ok()?;
                let (w3, h3, dx, dy) = if vertical {
                    (w, h.saturating_mul(3), left, top.saturating_mul(3))
                } else {
                    (w.saturating_mul(3), h, left.saturating_mul(3), top)
                };
                outline.translate(-raster::offset(dx), -raster::offset(dy));
                let cov = raster::fill(&outline, w3, h3);
                let (width, height, data) = raster::lcd_pack(&cov, w3, h3, stripes);
                Some(GlyphImage {
                    left,
                    top: top.saturating_neg(),
                    width,
                    height,
                    content: GlyphContent::Lcd,
                    data,
                })
            }
        }
    }
}
