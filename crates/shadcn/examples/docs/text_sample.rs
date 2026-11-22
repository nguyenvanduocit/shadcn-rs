//! The text sample (ADR-0019): the rows of `verdict/text-sample.tsv` drawn
//! by `gui` (`--text-sample <dir>`) as `verdict/text-sample.html` has Chrome
//! draw them, and both measured line by line (`--text-compare <dir>`) for
//! ink weight, stem width, colour fringing, OKLab difference and glyph
//! positions. `crates/shadcn/verdict/text.sh` runs the three steps.

use std::fmt::Write as _;
use std::path::Path;

use gui::Pixels;
use gui::prelude::*;
use gui::{Color, Element, div, text};

/// The page's width, logical px (the browser's viewport).
pub const WIDTH: u32 = 1280;
/// Where the rows start and the room between them, logical px.
const MARGIN: f32 = 16.0;
const GAP: f32 = 8.0;
/// The scales both sides are drawn at.
const SCALES: [u32; 2] = [1, 2];

const SAMPLE: &str = include_str!("../../verdict/text-sample.tsv");

/// One line of the sample and its line box.
struct Row {
    size: f32,
    weight: u16,
    text: &'static str,
    top: f32,
    height: f32,
}

/// The rows, placed as the HTML page places them.
fn rows() -> Vec<Row> {
    let mut y = MARGIN;
    let mut out = Vec::new();
    for line in SAMPLE.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let mut cells = line.splitn(3, '\t');
        let (Some(size), Some(weight), Some(text)) = (cells.next(), cells.next(), cells.next()) else {
            continue;
        };
        let (Ok(size), Ok(weight)) = (size.parse::<f32>(), weight.parse::<u16>()) else {
            continue;
        };
        let height = (size * 1.25).ceil();
        out.push(Row {
            size,
            weight,
            text,
            top: y,
            height,
        });
        y += height + GAP;
    }
    out
}

/// The page's height, logical px.
fn page_height(rows: &[Row]) -> u32 {
    let end = rows.last().map_or(MARGIN, |r| r.top + r.height + GAP);
    whole(end + GAP)
}

/// `v` (positive, below 2^16) as a whole number of px, rounded up.
fn whole(v: f32) -> u32 {
    (0..=1 << 16).find(|&n: &u32| f64::from(n) >= f64::from(v)).unwrap_or(0)
}

fn view(rows: &[Row], height: u32) -> Element {
    div()
        .position(Position::Relative)
        .width(f32::from(u16::try_from(WIDTH).unwrap_or(u16::MAX)))
        .height(f32::from(u16::try_from(height).unwrap_or(u16::MAX)))
        .background(Color::WHITE)
        .children(rows.iter().map(|r| {
            let line = div()
                .position(Position::Absolute)
                .left(MARGIN)
                .top(r.top)
                .text_size(r.size)
                .line_height(r.height)
                .text_color(Color::BLACK)
                .nowrap(true);
            let line = match r.weight {
                500 => line.font_weight(500),
                600 => line.font_weight(600),
                700 => line.font_weight(700),
                _ => line.font_weight(400),
            };
            line.child(text(r.text)).into_element()
        }))
        .into_element()
}

/// Draw the sample into `<dir>/ours-<scale>x.png`.
pub fn render(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let rows = rows();
    let height = page_height(&rows);
    for scale in SCALES {
        let path = dir.join(format!("ours-{scale}x.png"));
        let s = f32::from(u16::try_from(scale).unwrap_or(1));
        gui::snapshot(view(&rows, height), &path, WIDTH, height, s).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A picture's pixels as linear-light RGB and their sRGB bytes.
struct Picture {
    width: usize,
    height: usize,
    srgb: Vec<[u8; 3]>,
}

impl Picture {
    fn load(path: &Path) -> Result<Self, String> {
        let p = Pixels::load_png(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let srgb = p
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                // Both pages are opaque (white behind the text).
                let [r, g, b, _] = gui::demultiply(*c);
                [r, g, b]
            })
            .collect();
        Ok(Self {
            width: usize::try_from(p.width()).unwrap_or(0),
            height: usize::try_from(p.height()).unwrap_or(0),
            srgb,
        })
    }

    fn at(&self, x: usize, y: usize) -> [u8; 3] {
        y.checked_mul(self.width)
            .and_then(|i| i.checked_add(x))
            .and_then(|i| self.srgb.get(i))
            .copied()
            .unwrap_or([255; 3])
    }
}

fn linear(v: u8) -> f64 {
    let c = f64::from(v) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// How dark a pixel is: one minus its linear luminance (the coverage of
/// black ink on white).
fn ink(c: [u8; 3]) -> f64 {
    let [r, g, b] = c.map(linear);
    1.0 - (0.2126 * r + 0.7152 * g + 0.0722 * b)
}

fn oklab(c: [u8; 3]) -> [f64; 3] {
    let [r, g, b] = c.map(linear);
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

/// What one side's line measures.
#[derive(Default, Clone, Copy)]
struct Measure {
    /// Total ink (sum of coverage), device px².
    ink: f64,
    /// Mean width of the horizontal runs at least half covered, logical px.
    stem: f64,
    /// Mean chroma (max minus min of R, G, B, 0..1) over inked pixels.
    fringe: f64,
    /// Where ink starts and ends, logical px from the line's left.
    left: f64,
    right: f64,
}

fn measure(p: &Picture, x0: usize, y0: usize, w: usize, h: usize, scale: f64) -> (Measure, Vec<f64>) {
    let mut m = Measure::default();
    let mut runs = 0_u32;
    let mut run_px = 0_u32;
    let mut inked = 0_u32;
    let mut columns = vec![0.0; w];
    for y in y0..y0.saturating_add(h) {
        let mut run = 0_u32;
        for (x, column) in columns.iter_mut().enumerate() {
            let c = p.at(x0.saturating_add(x), y);
            let d = ink(c);
            m.ink += d;
            *column += d;
            if d > 0.05 {
                inked = inked.saturating_add(1);
                let (hi, lo) = (
                    c.iter().max().copied().unwrap_or(0),
                    c.iter().min().copied().unwrap_or(0),
                );
                m.fringe += f64::from(hi.saturating_sub(lo)) / 255.0;
            }
            if d >= 0.5 {
                run = run.saturating_add(1);
            } else if run > 0 {
                runs = runs.saturating_add(1);
                run_px = run_px.saturating_add(run);
                run = 0;
            }
        }
        if run > 0 {
            runs = runs.saturating_add(1);
            run_px = run_px.saturating_add(run);
        }
    }
    m.stem = if runs > 0 {
        f64::from(run_px) / f64::from(runs) / scale
    } else {
        0.0
    };
    m.fringe = if inked > 0 { m.fringe / f64::from(inked) } else { 0.0 };
    let peak = columns.iter().copied().fold(0.0, f64::max);
    let edge = |i: Option<usize>| {
        i.and_then(|i| u32::try_from(i).ok())
            .map_or(0.0, |i| f64::from(i) / scale)
    };
    m.left = edge(columns.iter().position(|&c| c > peak * 0.02));
    m.right = edge(columns.iter().rposition(|&c| c > peak * 0.02));
    (m, columns)
}

/// The shift (logical px, ours minus theirs) that best lines up two column
/// ink profiles, searched to a quarter of a device pixel.
fn best_shift(ours: &[f64], theirs: &[f64], scale: f64) -> f64 {
    let at = |x: usize, k: isize| {
        x.checked_add_signed(k)
            .and_then(|i| ours.get(i))
            .copied()
            .unwrap_or(0.0)
    };
    let mut best = (0.0, f64::MIN);
    for q in -64_i32..=64 {
        // q quarters of a device pixel: whole pixels `k` and a fraction `t`.
        let (k, t) = (q.div_euclid(4), f64::from(q.rem_euclid(4)) / 4.0);
        let k = isize::try_from(k).unwrap_or(0);
        let score: f64 = theirs
            .iter()
            .enumerate()
            .map(|(x, &v)| v * (at(x, k) * (1.0 - t) + at(x, k.saturating_add(1)) * t))
            .sum();
        if score > best.1 {
            best = (f64::from(q) / 4.0, score);
        }
    }
    best.0 / scale
}

/// Measure ours against Chrome's in `dir` at each scale, line by line, and
/// write `<dir>/text-sample.tsv`; the report's text is returned.
pub fn compare(dir: &Path) -> Result<String, String> {
    let rows = rows();
    let mut report = String::new();
    let mut tsv = String::from(
        "scale\tsize\tweight\tink_ours\tink_chrome\tink_ratio\tstem_ours\tstem_chrome\tfringe_ours\tfringe_chrome\toklab_mean\toklab_p95\tleft_delta\tright_delta\tshift\n",
    );
    for scale in SCALES {
        let ours = Picture::load(&dir.join(format!("ours-{scale}x.png")))?;
        let theirs = Picture::load(&dir.join(format!("chrome-{scale}x.png")))?;
        let s = f64::from(scale);
        writeln!(
            report,
            "{scale}x   size wt   ink ours/chrome   stem ours chrome   fringe ours chrome   ΔOK mean p95   left Δ  right Δ  shift"
        ).map_err(|e| e.to_string())?;
        let width = ours.width.min(theirs.width);
        for r in &rows {
            let px = |v: f32| usize::try_from(whole(v * f32::from(u16::try_from(scale).unwrap_or(1)))).unwrap_or(0);
            let (y0, h) = (px(r.top), px(r.height));
            if y0.saturating_add(h) > ours.height.min(theirs.height) {
                continue;
            }
            let (a, ca) = measure(&ours, 0, y0, width, h, s);
            let (b, cb) = measure(&theirs, 0, y0, width, h, s);
            // OKLab difference over pixels either side inked.
            let mut diffs = Vec::new();
            for y in y0..y0.saturating_add(h) {
                for x in 0..width {
                    let (p, q) = (ours.at(x, y), theirs.at(x, y));
                    if ink(p) > 0.02 || ink(q) > 0.02 {
                        let (lp, lq) = (oklab(p), oklab(q));
                        let d = lp
                            .iter()
                            .zip(lq.iter())
                            .map(|(u, v)| (u - v) * (u - v))
                            .sum::<f64>()
                            .sqrt();
                        diffs.push(d);
                    }
                }
            }
            diffs.sort_by(f64::total_cmp);
            let count = u32::try_from(diffs.len()).map_or(1.0, f64::from).max(1.0);
            let mean = diffs.iter().sum::<f64>() / count;
            let p95 = diffs
                .get(diffs.len().saturating_mul(95) / 100)
                .or(diffs.last())
                .copied()
                .unwrap_or(0.0);
            let shift = best_shift(&ca, &cb, s);
            let ratio = if b.ink > 0.0 { a.ink / b.ink } else { 0.0 };
            writeln!(
                report,
                "     {:>4} {:>3}   {:>6.3}           {:>5.2} {:>5.2}        {:>5.3} {:>5.3}          {:>5.3} {:>5.3}    {:>+6.2}  {:>+6.2}   {:>+6.2}",
                r.size,
                r.weight,
                ratio,
                a.stem,
                b.stem,
                a.fringe,
                b.fringe,
                mean,
                p95,
                a.left - b.left,
                a.right - b.right,
                shift
            ).map_err(|e| e.to_string())?;
            writeln!(
                tsv,
                "{scale}\t{}\t{}\t{:.1}\t{:.1}\t{ratio:.4}\t{:.3}\t{:.3}\t{:.4}\t{:.4}\t{mean:.4}\t{p95:.4}\t{:.3}\t{:.3}\t{shift:.3}",
                r.size,
                r.weight,
                a.ink,
                b.ink,
                a.stem,
                b.stem,
                a.fringe,
                b.fringe,
                a.left - b.left,
                a.right - b.right
            ).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(dir.join("text-sample.tsv"), tsv).map_err(|e| e.to_string())?;
    Ok(report)
}
