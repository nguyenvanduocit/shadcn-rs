//! Curve text (ADR-0017, P9), the CPU's part: a glyph's quadratic curves
//! encoded once into a blob of bands, as HarfBuzz's `hb_gpu_draw_encode`
//! encodes them (`references/harfbuzz/src/hb-gpu-draw.cc`), kept in a
//! storage buffer keyed by font, glyph, variation and synthesis (not by
//! size, unless the glyph is thickened by its size), and let go when unused
//! for a few frames. `slug.wgsl` computes coverage from it.
//!
//! The blob differs from hb-gpu's in being `i32` texels (so coordinates
//! keep 1/16 of a curve pixel and offsets are not biased) and in holding a
//! list of every curve, for the full-scan variant.

use std::collections::HashMap;

use super::arena::{Arena, Range};
use crate::text::GlyphKey;

/// Quantization steps per curve pixel (`UNITS` in the shader).
pub(crate) const UNITS: f32 = 16.0;
/// vec4s per glyph instance (`SLUG`).
pub(crate) const SLUG: usize = 5;
/// Bands at most, each way (as the Slug paper caps them).
const MAX_BANDS: usize = 16;
/// Widen each curve's band range by a hair so the shader's `f32` band index
/// and this `f64` one cannot disagree at a boundary (hb-gpu's `BAND_EPSILON`).
const BAND_EPSILON: f64 = 1.0 / 1024.0;

/// A glyph's curves as the blob holds them: what keys a blob.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SlugKey {
    font: u32,
    glyph: u32,
    axes: u16,
    skew: i8,
    /// The size's bits when the outline was thickened for it, else 0.
    ppem: u32,
}

impl SlugKey {
    pub(crate) fn of(key: &GlyphKey, darkened: bool) -> Self {
        let sized = key.flags & GlyphKey::EMBOLDEN != 0 || darkened;
        Self {
            font: key.font,
            glyph: key.glyph,
            axes: key.axes,
            skew: key.skew,
            ppem: if sized { key.ppem } else { 0 },
        }
    }
}

/// A blob in the buffer: where, and the glyph's extents (curve pixels, y up).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Blob {
    pub(crate) range: Range,
    pub(crate) extents: [f32; 4],
    used: u64,
}

pub(crate) struct Slugs {
    pub(crate) blobs: Arena,
    cache: HashMap<SlugKey, Option<Blob>>,
    frame: u64,
    /// Blobs encoded since made, for tests and devtools.
    pub(crate) encoded: u64,
}

impl Slugs {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self {
            blobs: Arena::new(device, "gui slug blobs", 16, 1 << 14),
            cache: HashMap::new(),
            frame: 0,
            encoded: 0,
        }
    }

    /// The blob of `key`, encoded from `curves` (asked for only the first time).
    pub(crate) fn blob(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: SlugKey,
        curves: impl FnOnce() -> Option<Vec<Vec<[[f32; 2]; 3]>>>,
    ) -> Option<Blob> {
        let frame = self.frame;
        if let Some(b) = self.cache.get_mut(&key) {
            if let Some(b) = b {
                b.used = frame;
            }
            return *b;
        }
        let blob = curves().and_then(|c| encode(&c)).map(|(texels, extents)| {
            let range = self
                .blobs
                .alloc(device, queue, u32::try_from(texels.len()).unwrap_or(u32::MAX));
            let bytes: Vec<u8> = texels.iter().flatten().flat_map(|v| v.to_ne_bytes()).collect();
            queue.write_buffer(&self.blobs.buffer, self.blobs.offset(range.start), &bytes);
            self.encoded = self.encoded.saturating_add(1);
            Blob {
                range,
                extents,
                used: frame,
            }
        });
        self.cache.insert(key, blob);
        blob
    }

    pub(crate) fn bytes(&self) -> usize {
        usize::try_from(self.blobs.bytes()).unwrap_or(usize::MAX)
    }

    pub(crate) fn held(&self) -> usize {
        self.cache.values().flatten().count()
    }

    /// The frame is drawn: let go of blobs not drawn for a few frames.
    pub(crate) fn end_frame(&mut self) {
        let frame = self.frame;
        let mut freed = Vec::new();
        self.cache.retain(|_, b| match b {
            Some(b) if frame.saturating_sub(b.used) >= 60 => {
                freed.push(b.range);
                false
            }
            _ => true,
        });
        for r in freed {
            self.blobs.release(r);
        }
        self.frame = self.frame.saturating_add(1);
    }
}

#[derive(Clone, Copy, Debug)]
struct Info {
    min: [f64; 2],
    max: [f64; 2],
    horizontal: bool,
    vertical: bool,
    bands: [(usize, usize); 2],
}

fn quantize(v: f32) -> i32 {
    crate::convert::f32_to_i32((v * UNITS).round())
}

/// Encode contours of quadratics (curve pixels, y down from the pen) into
/// a blob, flipped to y up as hb-gpu's shader expects; with its extents
/// (y up). None when there is nothing to draw.
#[expect(clippy::too_many_lines, reason = "hb_gpu_draw_encode, step for step")]
#[expect(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::needless_range_loop,
    reason = "hb_gpu_draw_encode's index arithmetic, kept as it is to be read beside it: every index is \
              below a length counted just before (`total`, `n`, the band counts)"
)]
pub(crate) fn encode(contours: &[Vec<[[f32; 2]; 3]>]) -> Option<(Vec<[i32; 4]>, [f32; 4])> {
    // Each curve as quantized points (y up), and whether it starts a contour.
    let mut curves: Vec<([[i32; 2]; 3], bool)> = Vec::new();
    for c in contours {
        for (i, q) in c.iter().enumerate() {
            let p = q.map(|p| [quantize(p[0]), quantize(-p[1])]);
            curves.push((p, i == 0));
        }
    }
    if curves.is_empty() {
        return None;
    }
    let de = |v: i32| f64::from(v) / f64::from(UNITS);
    let mut lo = [i32::MAX; 2];
    let mut hi = [i32::MIN; 2];
    for (p, _) in &curves {
        for q in p {
            for k in 0..2 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
    }
    let (min, max) = ([de(lo[0]), de(lo[1])], [de(hi[0]), de(hi[1])]);
    let n = curves.len();
    let mut infos: Vec<Info> = curves
        .iter()
        .map(|(p, _)| Info {
            min: [
                de(p[0][0].min(p[1][0]).min(p[2][0])),
                de(p[0][1].min(p[1][1]).min(p[2][1])),
            ],
            max: [
                de(p[0][0].max(p[1][0]).max(p[2][0])),
                de(p[0][1].max(p[1][1]).max(p[2][1])),
            ],
            horizontal: p[0][1] == p[1][1] && p[1][1] == p[2][1],
            vertical: p[0][0] == p[1][0] && p[1][0] == p[2][0],
            bands: [(1, 0), (1, 0)],
        })
        .collect();
    // Axis 0: horizontal bands (split along y), holding curves that are not
    // horizontal; axis 1: vertical bands (along x).
    let mut counts = [n.clamp(1, MAX_BANDS); 2];
    let extent = [max[1] - min[1], max[0] - min[0]];
    for a in 0..2 {
        if extent[a] <= 0.0 {
            counts[a] = 1;
        }
    }
    let mut lists: [Vec<Vec<usize>>; 2] = [vec![Vec::new(); counts[0]], vec![Vec::new(); counts[1]]];
    for (i, info) in infos.iter_mut().enumerate() {
        for a in 0..2 {
            let skip = if a == 0 { info.horizontal } else { info.vertical };
            if skip {
                continue;
            }
            // Band axis 0 splits y (coordinate 1); axis 1 splits x (coordinate 0).
            let k = 1 - a;
            let (l, h) = if extent[a] > 0.0 {
                let size = extent[a] / crate::convert::u64_to_f64(u64::try_from(counts[a]).unwrap_or(1));
                let l = ((info.min[k] - min[k]) / size - BAND_EPSILON).floor().max(0.0);
                let h = ((info.max[k] - min[k]) / size + BAND_EPSILON).floor();
                let last = counts[a].saturating_sub(1);
                (
                    usize::try_from(crate::convert::f64_to_u64(l)).unwrap_or(0).min(last),
                    usize::try_from(crate::convert::f64_to_u64(h.max(0.0)))
                        .unwrap_or(0)
                        .min(last),
                )
            } else {
                (0, 0)
            };
            info.bands[a] = (l, h);
            for b in l..=h {
                if let Some(list) = lists[a].get_mut(b) {
                    list.push(i);
                }
            }
        }
    }
    let header = 2;
    let band_headers = counts[0] + counts[1];
    let indices: usize = lists.iter().flatten().map(|l| 2 * l.len()).sum();
    let breaks = curves.iter().skip(1).filter(|(_, s)| *s).count();
    let curve_data = n + breaks + 1;
    let total = header + band_headers + indices + n + curve_data;
    if i32::try_from(total).is_err() {
        return None;
    }
    let at = |v: usize| i32::try_from(v).unwrap_or(i32::MAX);
    let mut buf = vec![[0_i32; 4]; total];
    let curve_start = header + band_headers + indices + n;
    // Curves, sharing end points within a contour.
    let mut offset = vec![0_usize; n];
    let mut t = curve_start;
    for i in 0..n {
        let (p, start) = curves[i];
        if start {
            offset[i] = t;
            buf[t] = [p[0][0], p[0][1], p[1][0], p[1][1]];
            t += 1;
        } else {
            offset[i] = t - 1;
        }
        let next = curves.get(i + 1).filter(|(_, s)| !*s).map(|(q, _)| q[1]);
        buf[t] = [p[2][0], p[2][1], next.map_or(0, |q| q[0]), next.map_or(0, |q| q[1])];
        t += 1;
    }
    buf[0] = [lo[0], lo[1], hi[0], hi[1]];
    let full_list = header + band_headers + indices;
    buf[1] = [at(counts[0]), at(counts[1]), at(n), at(full_list)];
    for (i, o) in offset.iter().enumerate() {
        buf[full_list + i] = [at(*o), 0, 0, 0];
    }
    let mut index = header + band_headers;
    for a in 0..2 {
        let k = 1 - a;
        for (b, list) in lists[a].iter().enumerate() {
            let mut desc = list.clone();
            desc.sort_by(|x, y| infos[*y].max[k].total_cmp(&infos[*x].max[k]));
            let mut asc = list.clone();
            asc.sort_by(|x, y| infos[*x].min[k].total_cmp(&infos[*y].min[k]));
            // The split that leaves the fewest curves to the worse side.
            let count = list.len();
            let mut best_worst = count;
            let mut best_split = (min[k] + max[k]) * 0.5;
            let mut left = count;
            for (ci, c) in desc.iter().enumerate() {
                let split = infos[*c].max[k];
                while left > 0 && infos[asc[left - 1]].min[k] > split {
                    left -= 1;
                }
                let worst = (ci + 1).max(left);
                if worst < best_worst {
                    best_worst = worst;
                    best_split = split;
                }
            }
            let desc_at = index;
            for c in &desc {
                buf[index] = [at(offset[*c]), 0, 0, 0];
                index += 1;
            }
            let asc_at = index;
            for c in &asc {
                buf[index] = [at(offset[*c]), 0, 0, 0];
                index += 1;
            }
            let h = header + if a == 0 { b } else { counts[0] + b };
            buf[h] = [
                at(count),
                at(desc_at),
                at(asc_at),
                crate::convert::f32_to_i32((crate::convert::f64_to_f32(best_split) * UNITS).round()),
            ];
        }
    }
    let ext = [lo[0], lo[1], hi[0], hi[1]].map(|v| crate::convert::i32_to_f32(v) / UNITS);
    Some((buf, ext))
}

/// The coverage the shader computes, on the CPU in `f64`: the exact winding
/// number of the curves around `p` (curve pixels, y up), for tests.
#[cfg(test)]
pub(crate) fn winding(contours: &[Vec<[[f32; 2]; 3]>], p: [f64; 2]) -> i32 {
    let mut w: i32 = 0;
    for c in contours {
        for q in c {
            let q = q.map(|v| [f64::from(v[0]), -f64::from(v[1])]);
            // Crossings of the ray toward +x.
            let a = q[0][1] - 2.0 * q[1][1] + q[2][1];
            let b = 2.0 * (q[1][1] - q[0][1]);
            let cc = q[0][1] - p[1];
            let mut ts = Vec::new();
            if a.abs() < 1e-12 {
                if b.abs() > 1e-12 {
                    ts.push(-cc / b);
                }
            } else {
                let d = b * b - 4.0 * a * cc;
                if d >= 0.0 {
                    let r = d.sqrt();
                    ts.push((-b - r) / (2.0 * a));
                    ts.push((-b + r) / (2.0 * a));
                }
            }
            for t in ts {
                if !(0.0..1.0).contains(&t) {
                    continue;
                }
                let mt = 1.0 - t;
                let x = mt * mt * q[0][0] + 2.0 * mt * t * q[1][0] + t * t * q[2][0];
                if x <= p[0] {
                    continue;
                }
                let dy = 2.0 * mt * (q[1][1] - q[0][1]) + 2.0 * t * (q[2][1] - q[1][1]);
                w = if dy > 0.0 {
                    w.saturating_add(1)
                } else {
                    w.saturating_sub(1)
                };
            }
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Vec<[[f32; 2]; 3]>> {
        let pts = [[0.0, 0.0], [10.0, 0.0], [10.0, -10.0], [0.0, -10.0]];
        let c = (0..4)
            .map(|i| {
                let a: [f32; 2] = pts[i];
                let b: [f32; 2] = pts[i.saturating_add(1) % 4];
                [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b]
            })
            .collect();
        vec![c]
    }

    #[test]
    fn a_blob_holds_its_curves_in_bands_and_in_full() {
        let (buf, ext) = encode(&square()).expect("a blob");
        assert_eq!(ext, [0.0, 0.0, 10.0, 10.0], "y up");
        let [nh, nv, n, full] = buf[1];
        assert_eq!(n, 4);
        assert!(nh >= 1 && nv >= 1);
        // Every horizontal band holds the two vertical sides only.
        for b in 0..usize::try_from(nh).unwrap_or(0) {
            assert_eq!(buf[2 + b][0], 2, "band {b}");
        }
        let full = usize::try_from(full).unwrap_or(0);
        let first = usize::try_from(buf[full][0]).unwrap_or(0);
        assert_eq!(buf[first][..2], [0, 0], "the first curve starts at the origin");
    }

    #[test]
    fn the_reference_winding_sees_inside_and_out() {
        assert_eq!(winding(&square(), [5.0, 5.0]).abs(), 1);
        assert_eq!(winding(&square(), [15.0, 5.0]), 0);
    }
}
