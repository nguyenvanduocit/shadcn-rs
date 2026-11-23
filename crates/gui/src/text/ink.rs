//! Where glyph ink crosses an underline: Chrome's `text-decoration-skip-ink:
//! auto`, its default. Blink (`TextPainter::ClipDecorationLine`) asks Skia
//! for each glyph's intercept with the line's band (`SkFont::getIntercepts`,
//! `SkGlyph::ensureIntercepts`), widens each by a dilation and clips those
//! spans out of the line; this is that, read from skrifa's outlines.
//!
//! An intercept is Skia's `calculate_path_gap`: one span per glyph, from the
//! leftmost to the rightmost x at which the outline is in the band, read
//! from where each segment crosses the band's top or bottom and from the
//! segment's points (control points too) inside it. A glyph whose ink stays
//! out of the band has none.

use skrifa::outline::OutlinePen;

/// Most a gap is widened past the ink on each side, in CSS pixels
/// (Blink's `kDecorationClipMaxDilation`).
const MAX_DILATION: f32 = 13.0;

/// How far a gap reaches past a glyph's intercept on each side for a line
/// `thickness` thick (resolved, before it is rounded): the thickness, at
/// most [`MAX_DILATION`].
pub(super) fn dilation(thickness: f32) -> f32 {
    thickness.min(MAX_DILATION)
}

/// The band skip-ink measures glyphs against for a line from `top` to
/// `top + thickness` below the baseline (unsnapped): half a pixel in from
/// each edge, so ink only grazing the line leaves no gap (Blink insets the
/// line's bounds by 0.5 vertically).
pub(super) fn band(top: f32, thickness: f32) -> (f32, f32) {
    (top + 0.5, top + thickness - 0.5)
}

/// The leftmost and rightmost x at which an outline is inside a horizontal
/// band, pen-relative, y down; Skia's `calculate_path_gap`, fed by skrifa.
pub(super) struct Gap {
    top: f32,
    bottom: f32,
    /// Shear of a synthetic italic: x grows by this times the height above
    /// the baseline.
    skew: f32,
    left: f32,
    right: f32,
    start: (f32, f32),
    at: (f32, f32),
}

impl Gap {
    /// A gap over the band from `top` to `bottom` below the baseline.
    pub(super) fn new(top: f32, bottom: f32, skew: f32) -> Self {
        Self {
            top,
            bottom,
            skew,
            left: f32::MAX,
            right: f32::MIN,
            start: (0.0, 0.0),
            at: (0.0, 0.0),
        }
    }

    /// The span found, if the outline entered the band at all.
    pub(super) fn span(&self) -> Option<(f32, f32)> {
        (self.left < self.right).then_some((self.left, self.right))
    }

    /// A pen point (font space scaled, y up) in band space (y down).
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (x + self.skew * y, -y)
    }

    fn expand(&mut self, x: f32) {
        self.left = self.left.min(x);
        self.right = self.right.max(x);
    }

    /// One segment given by its points: where it crosses the band's edges,
    /// and its points inside the band.
    fn segment(&mut self, pts: &[(f32, f32)]) {
        let (lo, hi) = pts
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
        if self.top > hi || lo > self.bottom {
            return;
        }
        for edge in [self.top, self.bottom] {
            crossings(pts, edge, |x| self.expand(x));
        }
        for p in pts {
            if self.top < p.1 && p.1 < self.bottom {
                self.expand(p.0);
            }
        }
    }
}

impl OutlinePen for Gap {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = self.point(x, y);
        self.at = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.segment(&[self.at, p]);
        self.at = p;
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (c, p) = (self.point(cx0, cy0), self.point(x, y));
        self.segment(&[self.at, c, p]);
        self.at = p;
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (c0, c1, p) = (self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y));
        self.segment(&[self.at, c0, c1, p]);
        self.at = p;
    }

    /// A contour's last edge back to its start, which FreeType's outlines
    /// (and so Skia's paths) draw as a line.
    fn close(&mut self) {
        if self.at != self.start {
            self.segment(&[self.at, self.start]);
            self.at = self.start;
        }
    }
}

/// The value at `t` of the Bézier curve of degree `pts.len() - 1` over one
/// coordinate of `pts`.
fn bezier(pts: &[(f32, f32)], t: f32, coord: impl Fn(&(f32, f32)) -> f32) -> f32 {
    let u = 1.0 - t;
    match pts {
        [a, b] => u * coord(a) + t * coord(b),
        [a, b, c] => u * u * coord(a) + 2.0 * u * t * coord(b) + t * t * coord(c),
        [a, b, c, d] => {
            u * u * u * coord(a) + 3.0 * u * u * t * coord(b) + 3.0 * u * t * t * coord(c) + t * t * t * coord(d)
        }
        _ => 0.0,
    }
}

/// The parameters in `0..1` where a segment's y turns (its derivative's
/// roots), sorted, with 0 and 1 around them: the curve is monotonic in y
/// between two neighbours. A cubic turns at most twice: the first `n` of
/// the array returned.
fn monotonic(pts: &[(f32, f32)]) -> ([f32; 4], usize) {
    let mut roots = [0.0_f32; 2];
    let mut found = 0_usize;
    let mut root = |t: f32| {
        if t > 0.0
            && t < 1.0
            && let Some(slot) = roots.get_mut(found)
        {
            *slot = t;
            found = found.saturating_add(1);
        }
    };
    match pts {
        // Quadratic: y' = 2((b - a)(1 - t) + (c - b)t).
        [(_, a), (_, b), (_, c)] => {
            let den = a - 2.0 * b + c;
            if den != 0.0 {
                root((a - b) / den);
            }
        }
        // Cubic: y' / 3 = A t² + B t + C.
        [(_, a), (_, b), (_, c), (_, d)] => {
            let qa = -a + 3.0 * b - 3.0 * c + d;
            let qb = 2.0 * (a - 2.0 * b + c);
            let qc = b - a;
            if qa.abs() < f32::EPSILON {
                if qb != 0.0 {
                    root(-qc / qb);
                }
            } else {
                let disc = qb * qb - 4.0 * qa * qc;
                if disc >= 0.0 {
                    let s = disc.sqrt();
                    root((-qb - s) / (2.0 * qa));
                    root((-qb + s) / (2.0 * qa));
                }
            }
        }
        _ => {}
    }
    let [r0, r1] = roots;
    match found {
        0 => ([0.0, 1.0, 1.0, 1.0], 2),
        1 => ([0.0, r0, 1.0, 1.0], 3),
        _ => ([0.0, r0.min(r1), r0.max(r1), 1.0], 4),
    }
}

/// Each x at which the segment `pts` (a line, quadratic or cubic) meets
/// the horizontal line at `y`: a root on each monotonic piece that spans
/// `y`, found by bisection to within float precision.
fn crossings(pts: &[(f32, f32)], y: f32, mut out: impl FnMut(f32)) {
    let at = |t: f32| bezier(pts, t, |p| p.1);
    let (ts, n) = monotonic(pts);
    for pair in ts.get(..n).unwrap_or_default().windows(2) {
        let [lo, hi] = pair else { continue };
        let (y0, y1) = (at(*lo), at(*hi));
        if y0 == y1 || y < y0.min(y1) || y > y0.max(y1) {
            continue;
        }
        let rising = y1 > y0;
        let (mut a, mut b) = (*lo, *hi);
        for _ in 0..32 {
            let mid = (a + b) / 2.0;
            if (at(mid) < y) == rising {
                a = mid;
            } else {
                b = mid;
            }
        }
        out(bezier(pts, (a + b) / 2.0, |p| p.0));
    }
}

/// Skip-ink's gaps cut from the line `x0..x1`: what is left of it once
/// each intercept, widened by `dilation` either side, is taken out, as
/// sorted, disjoint pieces written over `out` (kept by the caller, so a
/// line drawn again allocates nothing).
pub(super) fn segments(x0: f32, x1: f32, intercepts: &mut [(f32, f32)], dilation: f32, out: &mut Vec<(f32, f32)>) {
    intercepts.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    out.clear();
    let mut from = x0;
    for (l, r) in intercepts.iter() {
        let (l, r) = (l - dilation, r + dilation);
        let to = l.min(x1);
        if to > from {
            out.push((from, to));
        }
        from = from.max(r);
        if from >= x1 {
            break;
        }
    }
    if from < x1 {
        out.push((from, x1));
    }
}

/// Whether skip-ink leaves `c` alone: CJK ideographs and symbols, which
/// Blink's `auto` does not skip (`InkSkipCjkHandling::kExcludeCjk`, by
/// `Character::IsCJKIdeographOrSymbol`'s main blocks).
pub(super) fn cjk(c: char) -> bool {
    matches!(u32::from(c),
        0x2E80..=0x2FDF // CJK radicals, Kangxi radicals
        | 0x2FF0..=0x303F // ideographic description, CJK symbols and punctuation
        | 0x3040..=0x30FF // hiragana, katakana
        | 0x3100..=0x31FF // bopomofo, Hangul compatibility jamo, kanbun, katakana extensions
        | 0x3200..=0x4DBF // enclosed CJK, compatibility, extension A
        | 0x4E00..=0x9FFF // unified ideographs
        | 0xA960..=0xA97F // Hangul jamo extended A
        | 0xAC00..=0xD7FF // Hangul syllables and jamo extended B
        | 0xF900..=0xFAFF // compatibility ideographs
        | 0xFE30..=0xFE4F // CJK compatibility forms
        | 0xFF00..=0xFFEF // half- and full-width forms
        | 0x1F200..=0x1F2FF // enclosed ideographic supplement
        | 0x20000..=0x3FFFF // extensions B onward
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_crossing_the_band_gaps_from_its_left_to_its_right() {
        // A 4-wide box from 2 above the baseline to 6 below, drawn as a
        // font would (y up), closed implicitly.
        let mut g = Gap::new(3.0, 4.0, 0.0);
        g.move_to(1.0, 2.0);
        g.line_to(5.0, 2.0);
        g.line_to(5.0, -6.0);
        g.line_to(1.0, -6.0);
        g.close();
        assert_eq!(g.span(), Some((1.0, 5.0)));
        // A box above the band leaves no gap.
        let mut above = Gap::new(3.0, 4.0, 0.0);
        above.move_to(0.0, 8.0);
        above.line_to(4.0, 8.0);
        above.line_to(4.0, 0.0);
        above.line_to(0.0, 0.0);
        above.close();
        assert_eq!(above.span(), None);
    }

    #[test]
    fn a_curve_crosses_where_it_meets_the_band() {
        // A quadratic bowl from (0,0) down through (5, -10) back to (10, 0):
        // y = -20 t (1 - t) below, meeting y = 4 (band space) at
        // t(1 - t) = 0.2, t = 0.2763..: x = 10 t.
        let mut g = Gap::new(4.0, 4.5, 0.0);
        g.move_to(0.0, 0.0);
        g.quad_to(5.0, -10.0, 10.0, 0.0);
        let (l, r) = g.span().unwrap_or_default();
        let t = (1.0 - (1.0_f32 - 0.8).sqrt()) / 2.0;
        assert!((l - 10.0 * t).abs() < 1e-3, "{l}");
        assert!((r - 10.0 * (1.0 - t)).abs() < 1e-3, "{r}");
    }

    #[test]
    fn dilated_gaps_split_the_line_and_merge_when_they_meet() {
        let cut = |intercepts: &mut [(f32, f32)], dilation: f32| {
            let mut out = vec![(-1.0, -1.0)];
            segments(0.0, 30.0, intercepts, dilation, &mut out);
            out
        };
        assert_eq!(cut(&mut [(10.0, 12.0)], 1.5), vec![(0.0, 8.5), (13.5, 30.0)]);
        assert_eq!(
            cut(&mut [(20.0, 22.0), (10.0, 12.0), (13.0, 14.0)], 1.0),
            vec![(0.0, 9.0), (15.0, 19.0), (23.0, 30.0)]
        );
        assert_eq!(cut(&mut [], 1.0), vec![(0.0, 30.0)]);
        assert_eq!(cut(&mut [(-5.0, 40.0)], 1.0), Vec::<(f32, f32)>::new());
    }

    #[test]
    fn a_curve_turns_where_its_derivative_is_zero() {
        // A line never turns; a bowl turns once, at its bottom.
        assert_eq!(monotonic(&[(0.0, 0.0), (1.0, 1.0)]), ([0.0, 1.0, 1.0, 1.0], 2));
        assert_eq!(
            monotonic(&[(0.0, 0.0), (5.0, -10.0), (10.0, 0.0)]),
            ([0.0, 0.5, 1.0, 1.0], 3)
        );
        // An S-curve turns twice, sorted.
        let (ts, n) = monotonic(&[(0.0, 0.0), (1.0, 3.0), (2.0, -3.0), (3.0, 0.0)]);
        assert_eq!(n, 4);
        assert!(ts[0] < ts[1] && ts[1] < ts[2] && ts[2] < ts[3], "{ts:?}");
    }
}
