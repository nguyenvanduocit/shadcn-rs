//! Visual verdict: each demo on a page against the same shadcn-vue demo,
//! which `crates/shadcn/verdict` renders in a browser. Both are cropped to the
//! demo's box; ours is laid over theirs and the pixels that differ are scored.
//!
//! Both crops are trimmed to the ink either side paints (plus a margin), from
//! the demo box's top-left, so a shift shows as a shift. Per demo, under
//! `<dir>/<page>/<Demo>/`: `ours.png`, `ref.png`, `overlay.png` (ours at half
//! opacity over theirs), `diff.png` (theirs faded, differing pixels magenta)
//! and `side.png` (those four, top to bottom).

use std::fmt;
use std::path::Path;

use gui::Bounds;
use gui::Pixels;
use gui::layout::Frame;
use gui::text::{logical, physical, physical_pos};
use shadcn::CalendarDate;
use shadcn::theme::ThemeSpec;

use crate::{Page, Shot};

/// The instant the docs are at, in ms since the Unix epoch: noon UTC on
/// 2026-11-20, so the calendars draw the same month on every run. The
/// verdict pins Chrome's clock and time zone to it (`docs --clock`, read by
/// `run.sh` and handed to `shoot.mjs`), so shadcn-vue's demos, which take
/// `today(getLocalTimeZone())`, see the same day ours do. Noon, so the
/// seconds a run takes never cross into another day.
pub const CLOCK_MS: u64 = 1_795_176_000_000;
/// The IANA time zone [`CLOCK_MS`] is read in.
pub const CLOCK_ZONE: &str = "UTC";

/// The day it is at [`CLOCK_MS`] in [`CLOCK_ZONE`]: the docs' today.
pub fn today() -> CalendarDate {
    std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::from_millis(CLOCK_MS))
        .and_then(|at| CalendarDate::at(at, CLOCK_ZONE))
        .unwrap_or_else(CalendarDate::today)
}

/// The smallest channel difference from the background that counts as ink:
/// low, so hairline borders (`border-input`, a few steps off white) count.
const INK: u8 = 8;
/// Largest channel distance from a blend of two neighbouring colours at which
/// a pixel still counts as that blend (see [`Tolerance::coverage`]).
const BLEND: f64 = 12.0;

/// What the comparison forgives: how the two rasterisers differ, not the port.
/// Each is a flag of `--verdict` (and of `run.sh`), the defaults calibrated
/// on demos that match by eye.
#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    /// `--color=N`: the largest channel difference at which two pixels match.
    pub color: u8,
    /// `--shift=N`: physical pixels a pixel may move and still match, for
    /// sub-pixel glyph placement.
    pub shift: u8,
    /// `--size=N`: logical pixels the ink box may be off per axis, free.
    pub size: f32,
    /// On unless `--no-coverage`: an edge pixel matches at any coverage, a
    /// pixel being a blend of two colours the other side has within `shift`
    /// of it. Chrome's text gamma darkens glyph edges tiny-skia leaves light
    /// (light-on-dark most), which is not the port's to fix; a flat fill has
    /// no two colours to blend, so a wrong one still differs.
    pub coverage: bool,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            color: 48,
            shift: 1,
            size: 1.0,
            coverage: true,
        }
    }
}

impl Tolerance {
    /// The tolerance flags in `args` applied to the defaults, and the other args.
    pub fn parse(args: &[String]) -> Result<(Self, Vec<String>), String> {
        let mut t = Self::default();
        let mut rest = Vec::new();
        for a in args {
            let bad = |e: &dyn fmt::Display| format!("{a}: {e}");
            match a.split_once('=') {
                Some(("--color", v)) => t.color = v.parse().map_err(|e| bad(&e))?,
                Some(("--shift", v)) => t.shift = v.parse().map_err(|e| bad(&e))?,
                Some(("--size", v)) => t.size = v.parse().map_err(|e| bad(&e))?,
                _ if a == "--no-coverage" => t.coverage = false,
                _ => rest.push(a.clone()),
            }
        }
        Ok((t, rest))
    }
}

impl fmt::Display for Tolerance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "--color={} --shift={} --size={}{}",
            self.color,
            self.shift,
            self.size,
            if self.coverage { "" } else { " --no-coverage" }
        )
    }
}
/// Room between the panels of `side.png`, in physical pixels.
const GAP: u32 = 24;
/// Margin kept around the ink, in physical pixels.
const MARGIN: u32 = 16;

/// The width a demo is laid out in, on both sides (`shoot.mjs` uses the same).
pub const SPECIMEN_WIDTH: f32 = 768.0;

/// The demos on the page's first frame: nodes named after a shadcn-vue demo file.
pub fn specimens(frame: &Frame) -> Vec<(&'static str, Bounds)> {
    frame
        .nodes
        .iter()
        .filter_map(|n| {
            n.name
                .filter(|name| name.ends_with(".vue"))
                .map(|name| (name, n.bounds))
        })
        .collect()
}

/// How long a page may take to come to rest before it is judged: the
/// reference waits as long for its demo's mount timers and animations.
const REST: std::time::Duration = std::time::Duration::from_secs(3);

/// A canvas tall enough for the whole page, so no demo falls below the fold:
/// laid out once to measure, then again at the content's height; each at
/// rest, as the reference is shot ([`Shot::rest`]).
pub fn scenario(page: &Page, dir: &Path) -> Result<Shot, gui::ScenarioError> {
    const WIDTH: u32 = 1280;
    const PROBE: u32 = 2400;
    // The page starts with its own timers only, not a page's judged before it.
    gui::clock::cancel_all();
    let mut s = Shot::new((page.render)(), dir, WIDTH, PROBE, 2.0)?;
    s.rest(REST)?;
    let bottom = s.last().nodes.iter().map(|n| n.bounds.bottom()).fold(0.0_f32, f32::max);
    // The page's own padding below the last demo.
    let height = physical(bottom + 32.0, 1.0);
    if height <= PROBE {
        return Ok(s);
    }
    gui::clock::cancel_all();
    let mut s = Shot::new((page.render)(), dir, WIDTH, height, 2.0)?;
    s.rest(REST)?;
    Ok(s)
}

/// How one demo compares.
pub struct Verdict {
    pub demo: &'static str,
    /// Logical size of the ink each side paints.
    pub theirs: (f32, f32),
    pub ours: (f32, f32),
    /// Pixels that differ, and pixels either side paints (not background).
    pub differing: u32,
    pub ink: u32,
    /// Why ours is better, from `verdict/better.tsv`, if it says so.
    pub better: Option<String>,
    pub tolerance: Tolerance,
}

/// Score at or above which a demo needs no look.
pub const PASS: f64 = 90.0;
/// Score below which a demo is a port defect; between the two, look at `side.png`.
pub const FAIL: f64 = 75.0;
/// Points lost per logical pixel the ink box is off, past [`Tolerance::size`].
const SIZE_PENALTY: f64 = 5.0;

impl Verdict {
    /// Share of the ink that differs, in percent.
    pub fn differ_percent(&self) -> f64 {
        f64::from(self.differing) * 100.0 / f64::from(self.ink.max(1))
    }

    /// 0–100: the share of ink that matches, less [`SIZE_PENALTY`] per pixel of
    /// size drift past the tolerance.
    pub fn score(&self) -> f64 {
        let free = self.tolerance.size;
        let drift = |a: f32, b: f32| f64::from(((a - b).abs() - free).max(0.0));
        let size = drift(self.ours.0, self.theirs.0) + drift(self.ours.1, self.theirs.1);
        (100.0 - self.differ_percent() - SIZE_PENALTY * size).clamp(0.0, 100.0)
    }

    /// The grade, `Better` when `better` holds a reason and the score is short of PASS.
    pub fn grade(&self) -> Grade {
        let score = self.score();
        if self.better.is_some() && score < PASS {
            Grade::Better
        } else if score >= PASS {
            Grade::Pass
        } else if score >= FAIL {
            Grade::Warn
        } else {
            Grade::Fail
        }
    }

    /// One JSON object, for tools that read `verdict.json`.
    pub fn json(&self, page: &str) -> String {
        format!(
            r#"{{"page":"{page}","demo":"{}","score":{:.1},"grade":"{}","differ_percent":{:.2},"theirs":[{:.0},{:.0}],"ours":[{:.0},{:.0}],"better":{:?}}}"#,
            self.demo,
            self.score(),
            self.grade(),
            self.differ_percent(),
            self.theirs.0,
            self.theirs.1,
            self.ours.0,
            self.ours.1,
            self.better.as_deref().unwrap_or(""),
        )
    }
}

/// What a verdict judges in: the style, the theme's choices and the scheme
/// (the docs' `--style`, `--theme` and `--dark`; `run.sh` hands the browser
/// the same).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choices {
    pub style: &'static str,
    pub spec: ThemeSpec,
    pub dark: bool,
}

impl Choices {
    /// The value of a scope key of `better.tsv`: `style`, `scheme`, or a [`ThemeSpec`] key.
    fn get(&self, key: &str) -> Option<&'static str> {
        match key {
            "style" => Some(self.style),
            "scheme" => Some(if self.dark { "dark" } else { "light" }),
            _ => self.spec.get(key),
        }
    }

    /// Whether `scope` (`key=value,…`, or `*`) holds for these choices; an unknown key is an error.
    pub fn within(&self, scope: &str) -> Result<bool, String> {
        let mut holds = true;
        for pair in scope.split(',').map(str::trim).filter(|p| !p.is_empty() && *p != "*") {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("scope {pair:?} is not key=value"))?;
            let have = self.get(key.trim()).ok_or_else(|| {
                format!(
                    "scope key {key:?} is none of style, scheme, {}",
                    ThemeSpec::KEYS.join(", ")
                )
            })?;
            holds &= have == value.trim();
        }
        Ok(holds)
    }
}

impl fmt::Display for Choices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.dark { "dark" } else { "light" };
        write!(f, "style {} scheme {scheme} theme {}", self.style, self.spec)
    }
}

/// `verdict/better.tsv`'s entries that hold for `choices`: demo file → why our
/// port is better than shadcn-vue's. A line is `<Demo>.vue<TAB>why` (every
/// style and theme) or `<Demo>.vue<TAB><scope><TAB>why`, the scope as
/// [`Choices::within`] reads it (`iconLibrary=tabler`, `style=nova,scheme=dark`).
pub fn better_list(choices: &Choices) -> Result<Vec<(String, String)>, String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/verdict/better.tsv");
    better_entries(&std::fs::read_to_string(path).unwrap_or_default(), choices)
}

fn better_entries(tsv: &str, choices: &Choices) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    for (n, line) in (1_usize..).zip(tsv.lines()) {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        let (demo, scope, why) = match cols.as_slice() {
            [demo, why] => (*demo, "*", *why),
            [demo, scope, why] => (*demo, *scope, *why),
            [""] => continue,
            _ => return Err(format!("better.tsv:{n}: not <Demo>.vue<TAB>[scope<TAB>]why")),
        };
        if choices.within(scope).map_err(|e| format!("better.tsv:{n}: {e}"))? {
            entries.push((demo.to_owned(), why.to_owned()));
        }
    }
    Ok(entries)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    Pass,
    Warn,
    Fail,
    /// Differs on purpose: `verdict/better.tsv` says why ours is better.
    Better,
}

impl fmt::Display for Grade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
            Self::Better => "BETTER",
        })
    }
}

impl Grade {
    /// The grade whose name (as it displays) is `name`.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Pass, Self::Warn, Self::Fail, Self::Better]
            .into_iter()
            .find(|g| g.to_string() == name)
    }
}

/// One demo as a `verdict.json` holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Judged {
    pub page: String,
    pub demo: String,
    pub score: f64,
    pub grade: Grade,
}

/// The demos the `verdict.json` at `path` judged, read back from the one
/// object per line [`Verdict::json`] writes; `None` when there is no file.
pub fn read_json(path: &Path) -> Option<Vec<Judged>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().filter_map(judged).collect())
}

/// The value of `key` in a one-line JSON object of flat fields, as the
/// verdict and the bench write them: a string's text, or a number as written.
pub fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.split_once(&format!("\"{key}\":"))?.1;
    Some(match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or(""),
        None => rest.split([',', '}']).next().unwrap_or(""),
    })
}

fn judged(line: &str) -> Option<Judged> {
    let field = |key: &str| field(line, key);
    Some(Judged {
        page: field("page")?.to_owned(),
        demo: field("demo")?.to_owned(),
        score: field("score")?.parse().ok()?,
        grade: Grade::parse(field("grade")?)?,
    })
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:<6} {:>3.0}  {:<36} theirs {:>4.0}×{:<4.0} ours {:>4.0}×{:<4.0} differ {:>5.1}%{}",
            self.grade(),
            self.score(),
            self.demo,
            self.theirs.0,
            self.theirs.1,
            self.ours.0,
            self.ours.1,
            self.differ_percent(),
            match (&self.better, self.grade()) {
                (Some(why), Grade::Better) => format!("  ours better: {why}"),
                (Some(_), _) => "  (better.tsv entry no longer needed: it passes)".to_owned(),
                (None, _) => String::new(),
            },
        )
    }
}

/// What went wrong comparing one demo.
#[derive(Debug)]
pub enum VerdictError {
    /// `shoot.mjs` has not rendered this demo (or could not).
    NoReference(&'static str),
    Bad(String),
}

impl fmt::Display for VerdictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoReference(d) => write!(f, "NOREF    -  {d:<36} no reference shot (shoot.mjs printed why)"),
            Self::Bad(e) => write!(f, "ERROR {e}"),
        }
    }
}

fn bad(e: impl fmt::Display) -> VerdictError {
    VerdictError::Bad(e.to_string())
}

/// Crop ours from the scenario, theirs from `ref-full.png` + `ref.rect`, and judge.
pub fn judge(
    s: &Shot,
    demo: &'static str,
    bounds: Bounds,
    dir: &Path,
    better: Option<String>,
    tolerance: Tolerance,
) -> Result<Verdict, VerdictError> {
    let scale = s.scale();
    let ours = crop(s.pixmap(), bounds, scale).ok_or_else(|| bad(format!("{demo}: our box {bounds} is off canvas")))?;
    ours.save_png(dir.join("ours.png")).map_err(bad)?;
    let (Ok(full), Ok(rect)) = (
        Pixels::load_png(dir.join("ref-full.png")),
        std::fs::read_to_string(dir.join("ref.rect")),
    ) else {
        return Err(VerdictError::NoReference(demo));
    };
    let theirs_box = parse_rect(&rect).ok_or_else(|| bad(format!("{demo}: ref.rect {rect:?}")))?;
    let theirs = crop(&full, theirs_box, scale).ok_or_else(|| bad(format!("{demo}: their box is off the shot")))?;
    theirs.save_png(dir.join("ref.png")).map_err(bad)?;

    let bg = at(&theirs, 0, 0, [255, 255, 255, 255]);
    let (Some(ink_t), Some(ink_o)) = (ink_box(&theirs, bg), ink_box(&ours, bg)) else {
        return Err(bad(format!("{demo}: one side paints nothing")));
    };
    let area = ink_t.union(ink_o).expand(MARGIN);
    let (theirs, ours) = (cut(&theirs, area, bg), cut(&ours, area, bg));
    let (Some(theirs), Some(ours)) = (theirs, ours) else {
        return Err(bad(format!("{demo}: empty ink area")));
    };
    ours.save_png(dir.join("ours.png")).map_err(bad)?;
    theirs.save_png(dir.join("ref.png")).map_err(bad)?;

    let cmp = Comparison::new(&theirs, &ours, bg, tolerance).ok_or_else(|| bad("empty crop"))?;
    let overlay = cmp.overlay().ok_or_else(|| bad("overlay size"))?;
    let diff = cmp.diff().ok_or_else(|| bad("diff size"))?;
    overlay.save_png(dir.join("overlay.png")).map_err(bad)?;
    diff.save_png(dir.join("diff.png")).map_err(bad)?;
    stacked(&[&theirs, &ours, &overlay, &diff], bg)
        .ok_or_else(|| bad("side size"))?
        .save_png(dir.join("side.png"))
        .map_err(bad)?;
    Ok(Verdict {
        demo,
        theirs: ink_t.logical_size(scale),
        ours: ink_o.logical_size(scale),
        differing: cmp.differing,
        ink: cmp.ink,
        better,
        tolerance,
    })
}

/// A box of physical pixels, `x1`/`y1` exclusive.
#[derive(Clone, Copy)]
struct Ink {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Ink {
    fn union(self, o: Self) -> Self {
        Self {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    fn expand(self, m: u32) -> Self {
        Self {
            x0: self.x0.saturating_sub(m),
            y0: self.y0.saturating_sub(m),
            x1: self.x1.saturating_add(m),
            y1: self.y1.saturating_add(m),
        }
    }

    fn logical_size(self, scale: f32) -> (f32, f32) {
        let len = |a: u32, b: u32| logical(i32::try_from(b.saturating_sub(a)).unwrap_or(i32::MAX), scale);
        (len(self.x0, self.x1), len(self.y0, self.y1))
    }
}

/// The smallest box holding every pixel of `p` that is not background.
fn ink_box(p: &Pixels, bg: Rgba) -> Option<Ink> {
    let mut found: Option<Ink> = None;
    for y in 0..p.height() {
        for x in 0..p.width() {
            if distance(at(p, x, y, bg), bg) > INK {
                let (x1, y1) = (x.saturating_add(1), y.saturating_add(1));
                let here = Ink { x0: x, y0: y, x1, y1 };
                found = Some(found.map_or(here, |f| f.union(here)));
            }
        }
    }
    found
}

/// `area` of `p`, background where `p` ends.
fn cut(p: &Pixels, area: Ink, bg: Rgba) -> Option<Pixels> {
    let mut out = Pixels::new(area.x1.saturating_sub(area.x0), area.y1.saturating_sub(area.y0))?;
    out.fill(bg);
    let (dx, dy) = (i32::try_from(area.x0).ok()?, i32::try_from(area.y0).ok()?);
    out.draw(p, dx.checked_neg()?, dy.checked_neg()?, 1.0);
    Some(out)
}

fn parse_rect(s: &str) -> Option<Bounds> {
    let v: Vec<f32> = s.split_whitespace().map(str::parse).collect::<Result<_, _>>().ok()?;
    match v.as_slice() {
        [x, y, width, height] => Some(Bounds {
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        }),
        _ => None,
    }
}

fn crop(p: &Pixels, b: Bounds, scale: f32) -> Option<Pixels> {
    p.crop(
        physical_pos(b.x, scale),
        physical_pos(b.y, scale),
        physical(b.width, scale),
        physical(b.height, scale),
    )
}

type Rgba = [u8; 4];

fn at(p: &Pixels, x: u32, y: u32, bg: Rgba) -> Rgba {
    p.straight(x, y).unwrap_or(bg)
}

fn distance(a: Rgba, b: Rgba) -> u8 {
    a.iter().zip(b.iter()).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

/// Whether `c` matches some pixel of `p` within `tol.shift` of `(x, y)`, or
/// (with `tol.coverage`) a blend of two of them.
fn near(p: &Pixels, x: u32, y: u32, c: Rgba, bg: Rgba, tol: Tolerance) -> bool {
    let shift = i32::from(tol.shift);
    let reach = shift.saturating_neg()..=shift;
    let window: Vec<Rgba> = reach
        .clone()
        .flat_map(|dy| reach.clone().map(move |dx| (dx, dy)))
        .filter_map(|(dx, dy)| Some(at(p, x.checked_add_signed(dx)?, y.checked_add_signed(dy)?, bg)))
        .collect();
    window.iter().any(|&w| distance(w, c) <= tol.color)
        || (tol.coverage
            && window
                .iter()
                .enumerate()
                .any(|(i, &a)| window.iter().skip(i.saturating_add(1)).any(|&b| blend_of(c, a, b))))
}

/// Whether `c` is `a` and `b` mixed, as an edge pixel between them is.
fn blend_of(c: Rgba, a: Rgba, b: Rgba) -> bool {
    let v = |[r, g, b, _]: Rgba| [r, g, b].map(f64::from);
    let (c, a, b) = (v(c), v(a), v(b));
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let dot = |p: [f64; 3], q: [f64; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let len2 = dot(ab, ab);
    if len2 < 1.0 {
        return false;
    }
    let t = (dot(ac, ab) / len2).clamp(0.0, 1.0);
    ac.iter().zip(&ab).all(|(c, d)| (c - t * d).abs() <= BLEND)
}

struct Comparison<'a> {
    theirs: &'a Pixels,
    ours: &'a Pixels,
    size: (u32, u32),
    /// The page background, taken from theirs' top-left pixel.
    bg: Rgba,
    /// 255 where the two differ, row-major over `size`.
    mask: Vec<u8>,
    differing: u32,
    ink: u32,
}

impl<'a> Comparison<'a> {
    fn new(theirs: &'a Pixels, ours: &'a Pixels, bg: Rgba, tol: Tolerance) -> Option<Self> {
        let (w, h) = (theirs.width().max(ours.width()), theirs.height().max(ours.height()));
        let mut mask = Vec::new();
        let (mut differing, mut ink) = (0_u32, 0_u32);
        for y in 0..h {
            for x in 0..w {
                let (t, o) = (at(theirs, x, y, bg), at(ours, x, y, bg));
                if distance(t, bg) > INK || distance(o, bg) > INK {
                    ink = ink.saturating_add(1);
                }
                let differs = !near(ours, x, y, t, bg, tol) || !near(theirs, x, y, o, bg, tol);
                if differs {
                    differing = differing.saturating_add(1);
                }
                mask.push(if differs { 255 } else { 0 });
            }
        }
        Some(Self {
            theirs,
            ours,
            size: (w, h),
            bg,
            mask,
            differing,
            ink,
        })
    }

    fn canvas(&self) -> Option<Pixels> {
        let mut p = Pixels::new(self.size.0, self.size.1)?;
        p.fill(self.bg);
        Some(p)
    }

    /// Theirs, with ours at half opacity on top.
    fn overlay(&self) -> Option<Pixels> {
        let mut p = self.canvas()?;
        p.draw(self.theirs, 0, 0, 1.0);
        p.draw(self.ours, 0, 0, 0.5);
        Some(p)
    }

    /// Theirs faded, the differing pixels magenta.
    fn diff(&self) -> Option<Pixels> {
        let mut p = self.canvas()?;
        p.draw(self.theirs, 0, 0, 0.25);
        let magenta = [255, 0, 170, 255];
        for (px, m) in p.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(&self.mask) {
            if *m != 0 {
                *px = magenta;
            }
        }
        Some(p)
    }
}

fn stacked(panels: &[&Pixels], bg: Rgba) -> Option<Pixels> {
    let h = panels
        .iter()
        .try_fold(0_u32, |acc, p| acc.checked_add(p.height())?.checked_add(GAP))?;
    let w = panels.iter().map(|p| p.width()).max()?;
    let mut out = Pixels::new(w, h)?;
    out.fill(bg);
    let mut y = 0_i32;
    for p in panels {
        out.draw(p, 0, y, 1.0);
        y = y.checked_add(i32::try_from(p.height().checked_add(GAP)?).ok()?)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices(style: &'static str, spec: &str, dark: bool) -> Choices {
        Choices {
            style,
            spec: ThemeSpec::parse(spec).unwrap(),
            dark,
        }
    }

    #[test]
    fn better_entries_hold_in_their_scope_only() {
        let tsv = "# comment\nA.vue\tevery run\nB.vue\ticonLibrary=lucide\tlucide only\n\
                   C.vue\tstyle=nova,scheme=dark\tnova dark\nD.vue\t*\tstar\n";
        let names =
            |c: &Choices| -> Vec<String> { better_entries(tsv, c).unwrap().into_iter().map(|(d, _)| d).collect() };
        assert_eq!(names(&choices("vega", "", false)), ["A.vue", "B.vue", "D.vue"]);
        assert_eq!(names(&choices("nova", "", true)), ["A.vue", "B.vue", "C.vue", "D.vue"]);
        assert_eq!(names(&choices("nova", "", false)), ["A.vue", "B.vue", "D.vue"]);
    }

    #[test]
    fn better_entries_refuse_what_they_cannot_read() {
        let c = choices("vega", "", false);
        assert!(
            better_entries("A.vue\tcolour=red\twhy", &c)
                .unwrap_err()
                .contains("better.tsv:1: scope key")
        );
        assert!(
            better_entries("A.vue\ta\tb\tc", &c)
                .unwrap_err()
                .contains("better.tsv:1")
        );
        assert!(
            better_entries("A.vue\tradius\twhy", &c)
                .unwrap_err()
                .contains("not key=value")
        );
    }

    #[test]
    fn the_repository_better_list_reads() {
        for style in ["vega", "nova"] {
            for dark in [false, true] {
                better_list(&choices(style, "", dark)).unwrap();
            }
        }
    }
}
