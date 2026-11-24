//! Paths rasterized on the GPU (ADR-0017), the CPU's part: each path drawn
//! is a *job* (its curves, a transform, a tile rectangle, a style); the CPU
//! bounds how many lines, tile entries and instances the job may need (from
//! each curve's Wang constant and length and the transform's largest
//! scale), reserves those ranges, and writes one job record and one work
//! item per curve. The compute passes of `path_compute.wgsl` then flatten,
//! stroke and bin it, and the main pass draws its tiles
//! (`path_render.wgsl`). Nothing is read back and nothing can overflow.
//!
//! A job is kept while its path, style, linear transform and fractional
//! translation stay the same and what is visible stays inside the
//! rectangle it was binned for: a whole-pixel scroll or pan moves it by an
//! offset, and the GPU redoes nothing. Curves are uploaded once per path and
//! kept while a job uses them.
//!
//! [`reference`] is the same algorithm in plain Rust, pass for pass.

#[cfg(test)]
pub(crate) mod reference;

use std::collections::HashMap;
use std::f32::consts::PI;

use super::arena::{Arena, Range, Scratch};
use crate::figure::{Cap, Draw, FillRule, Join, Path, PathStyle, Stroke};
use crate::scene::Gradient;

/// Words per job record (`JOB` in the shaders).
pub(crate) const JOB: usize = 32;
/// Words per job's draw record (`DRAW`).
pub(crate) const DRAW: usize = 32;
/// Words per resident curve (`CURVE`).
pub(crate) const CURVE: usize = 16;
/// Words per curve item (`ITEM`).
pub(crate) const ITEM: usize = 8;
/// Floats of scratch per curve item (`HEAD`).
pub(crate) const HEAD: usize = 20;
/// Device pixels per tile side.
pub(crate) const TILE: f32 = 16.0;
/// How far a flattened curve may stray from the true one, in device pixels.
pub(crate) const TOLERANCE: f32 = 0.125;
/// Device pixels kept binned around what is visible, so a small scroll
/// needs no new binning.
const MARGIN: f32 = 256.0;
/// Frames a job outlives its last draw.
const KEEP: u64 = 60;
/// The most segments one curve is cut into.
const MAX_SEGMENTS: u32 = 4096;

pub(crate) const C_START_CAP: u32 = 1;
pub(crate) const C_END_CAP: u32 = 2;
pub(crate) const C_JOIN_IN: u32 = 4;
pub(crate) const C_DOT: u32 = 8;
/// A job's flags word (word 16): stroked, filled even-odd, then the cap and
/// the join in two bits each.
pub(crate) const JOB_FLAGS: usize = 16;
pub(crate) const CAP_MASK: u32 = 3;
pub(crate) const JOIN_MASK: u32 = 3;
/// The grid, in steps per pixel, the compute passes snap points to.
pub(crate) const FIX: f32 = 256.0;
pub(crate) const F_STROKE: u32 = 1;
pub(crate) const F_EVEN_ODD: u32 = 2;
pub(crate) const CAP_SHIFT: u32 = 4;
pub(crate) const JOIN_SHIFT: u32 = 6;
pub(crate) const CAP_BUTT: u32 = 0;
pub(crate) const CAP_ROUND: u32 = 1;
pub(crate) const CAP_SQUARE: u32 = 2;
pub(crate) const JOIN_MITER: u32 = 0;
pub(crate) const JOIN_ROUND: u32 = 1;
pub(crate) const JOIN_BEVEL: u32 = 2;
const NONE: u32 = u32::MAX;

/// A curve as the CPU bounds it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CurveInfo {
    pub order: u8,
    pub wang: f32,
    pub length: f32,
}

/// One resident curve: what the GPU reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Record {
    pub points: [[f32; 2]; 4],
    pub order: u32,
    pub flags: u32,
    /// The curve whose end this one joins (an index within the set).
    pub prev: u32,
}

/// A path's curves on the GPU, for filling or for stroking.
struct CurveSet {
    range: Range,
    info: Vec<CurveInfo>,
    used: u64,
}

/// The curves of `path` as fills read them (every subpath closed) or as
/// strokes read them (zero-length curves dropped, caps, joins and dots
/// marked).
pub(crate) fn records(path: &Path, stroke: bool) -> Vec<(Record, CurveInfo)> {
    let mut out: Vec<(Record, CurveInfo)> = Vec::new();
    for sub in path.subpaths() {
        let mut curves = sub.curves.clone();
        if !stroke {
            if let (Some(first), Some(last)) = (curves.first(), curves.last())
                && first.start() != last.end()
            {
                let close = crate::figure::PathBuilder::default()
                    .move_to(last.end()[0], last.end()[1])
                    .line_to(first.start()[0], first.start()[1])
                    .build();
                curves.extend(close.subpaths().into_iter().flat_map(|s| s.curves));
            }
        } else {
            let had = !curves.is_empty();
            curves.retain(|c| c.points.iter().take(usize::from(c.order)).any(|p| *p != c.start()));
            if curves.is_empty() {
                if had || sub.closed {
                    let p = sub.start;
                    out.push((
                        Record {
                            points: [p, p, p, p],
                            order: 2,
                            flags: C_DOT,
                            prev: NONE,
                        },
                        CurveInfo {
                            order: 2,
                            wang: 0.0,
                            length: 0.0,
                        },
                    ));
                }
                continue;
            }
        }
        let first = u32::try_from(out.len()).unwrap_or(u32::MAX);
        let n = u32::try_from(curves.len()).unwrap_or(u32::MAX);
        for (i, c) in (0_u32..).zip(&curves) {
            let mut flags = 0;
            let mut prev = NONE;
            if stroke {
                if i > 0 {
                    flags |= C_JOIN_IN;
                    prev = first.saturating_add(i).saturating_sub(1);
                } else if sub.closed {
                    flags |= C_JOIN_IN;
                    prev = first.saturating_add(n).saturating_sub(1);
                } else {
                    flags |= C_START_CAP;
                }
                if !sub.closed && i.saturating_add(1) == n {
                    flags |= C_END_CAP;
                }
            }
            out.push((
                Record {
                    points: c.points,
                    order: u32::from(c.order),
                    flags,
                    prev,
                },
                CurveInfo {
                    order: c.order,
                    wang: c.wang,
                    length: c.length,
                },
            ));
        }
    }
    out
}

/// The largest and least scale of a transform's linear part (its singular values).
pub(crate) fn scales(m: [f32; 6]) -> (f32, f32) {
    let [a, b, c, d, ..] = m;
    let p = a * a + b * b + c * c + d * d;
    let q = a * d - b * c;
    let root = (p * p - 4.0 * q * q).max(0.0).sqrt();
    (((p + root) / 2.0).max(0.0).sqrt(), ((p - root) / 2.0).max(0.0).sqrt())
}

/// The inverse of `m`, or none when it flattens everything.
pub(crate) fn invert(m: [f32; 6]) -> Option<[f32; 6]> {
    let [a, b, c, d, e, f] = m;
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return None;
    }
    let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
    Some([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)])
}

/// `m` then `n` (`n · m`).
pub(crate) fn then(m: [f32; 6], n: [f32; 6]) -> [f32; 6] {
    let [a, b, c, d, e, f] = m;
    let [p, q, r, s, t, u] = n;
    [
        p * a + r * b,
        q * a + s * b,
        p * c + r * d,
        q * c + s * d,
        p * e + r * f + t,
        q * e + s * f + u,
    ]
}

pub(crate) fn apply(m: [f32; 6], p: [f32; 2]) -> [f32; 2] {
    [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
}

/// How a job strokes: the numbers the shaders read, in the path's space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StrokeParams {
    /// Half the width, in the path's units.
    pub half: f32,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f32,
    /// The angle one segment of a round join or cap may turn.
    pub theta: f32,
    /// The most segments of one round join or cap.
    pub arc_bound: u32,
    /// Inside a curve, the cosine of the largest turn the two sides may take
    /// as one miter point (within the tolerance).
    pub smooth: f32,
    /// What the alpha is multiplied by (a hairline's width).
    pub alpha: f32,
}

impl StrokeParams {
    /// `stroke` under a transform of largest scale `s`.
    pub(crate) fn new(stroke: &Stroke, s: f32) -> Self {
        let mut half = stroke.width.max(0.0) / 2.0;
        let mut alpha = 1.0;
        // Thinner than a device pixel: a hairline a pixel wide, as faint as
        // its width (Skia's hairlines).
        if stroke.width * s < 1.0 && s > 0.0 {
            alpha = (stroke.width * s).max(0.0);
            half = 0.5 / s;
        }
        let h = half * s;
        let theta = if h > TOLERANCE {
            2.0 * (1.0 - TOLERANCE / h).clamp(-1.0, 1.0).acos()
        } else {
            PI
        }
        .max(1e-3);
        let arc_bound = crate::convert::f32_to_u32((PI / theta).ceil())
            .saturating_add(1)
            .clamp(2, 256);
        let k = h / (h + TOLERANCE);
        Self {
            half,
            cap: stroke.cap,
            join: stroke.join,
            miter_limit: stroke.miter_limit.max(1.0),
            theta,
            arc_bound,
            smooth: 2.0 * k * k - 1.0,
            alpha,
        }
    }

    /// How far outside the path's points the stroke may reach, device pixels.
    pub(crate) fn reach(&self, s: f32) -> f32 {
        let k = match (self.join, self.cap) {
            (Join::Miter, _) => self.miter_limit.max(std::f32::consts::SQRT_2),
            (_, Cap::Square) => std::f32::consts::SQRT_2,
            _ => 1.0,
        };
        self.half * s * k
    }
}

/// The segments one curve is cut into at most under scale `s`.
pub(crate) fn segment_bound(c: &CurveInfo, s: f32) -> u32 {
    let k = match c.order {
        3 => 0.25,
        4 => 0.75,
        _ => return 1,
    };
    let n = (k * s * c.wang / TOLERANCE).sqrt().ceil();
    crate::convert::f32_to_u32(n).saturating_add(1).clamp(1, MAX_SEGMENTS)
}

/// Line slots one curve may fill: in `flatten`, and in `joins` after.
pub(crate) fn line_bounds(c: &CurveInfo, s: f32, stroke: Option<&StrokeParams>) -> (u32, u32) {
    let n = segment_bound(c, s);
    let len = c.length * s;
    let lines = |raw: f32, total: f32| crate::convert::f32_to_u32(2.0 * raw + (2.0 * total / 64.0).ceil() + 2.0);
    match stroke {
        None => (lines(2.0 * f32_of(n), len), 0),
        Some(p) => {
            let h = p.half * s;
            let j = f32_of(p.arc_bound);
            let raw = 2.0 * f32_of(n) + f32_of(n) * (j + 2.0);
            let total = 2.0 * len + f32_of(n) * (8.0 + PI + 2.0 * p.miter_limit) * h;
            let raw_j = (j + 2.0) + 2.0 * (j + 3.0) + 2.0 * j + 4.0;
            let total_j = (4.0 * PI + 4.0 * p.miter_limit + 16.0) * h;
            (lines(raw, total), lines(raw_j, total_j))
        }
    }
}

fn f32_of(v: u32) -> f32 {
    crate::convert::u32_to_f32(v)
}

/// Entries a job's lines may make in its tiles' lists: each piece at most
/// 64 px, passing at most its length over 16 plus three tiles.
pub(crate) fn entry_bound(lines: u32, total_length: f32) -> u32 {
    crate::convert::f32_to_u32(
        3.0 * f32_of(lines) + (std::f32::consts::SQRT_2 * 2.0 * total_length / TILE).ceil() + 4.0,
    )
}

/// A rectangle of tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TileRect {
    pub x0: i32,
    pub y0: i32,
    pub w: u32,
    pub h: u32,
}

impl TileRect {
    /// The tiles covering `r` (x0, y0, x1, y1, device pixels).
    pub(crate) fn covering(r: [f32; 4]) -> Option<Self> {
        let x0 = (r[0] / TILE).floor();
        let y0 = (r[1] / TILE).floor();
        let x1 = (r[2] / TILE).ceil();
        let y1 = (r[3] / TILE).ceil();
        if !(x1 > x0 && y1 > y0) || !x0.is_finite() || !y0.is_finite() {
            return None;
        }
        Some(Self {
            x0: crate::convert::f32_to_i32(x0),
            y0: crate::convert::f32_to_i32(y0),
            w: crate::convert::f32_to_u32(x1 - x0),
            h: crate::convert::f32_to_u32(y1 - y0),
        })
    }

    pub(crate) fn count(&self) -> u32 {
        self.w.saturating_mul(self.h)
    }

    /// Whether `other` lies inside.
    pub(crate) fn contains(&self, other: &Self) -> bool {
        let r = |t: &Self| {
            (
                t.x0,
                t.y0,
                t.x0.saturating_add_unsigned(t.w),
                t.y0.saturating_add_unsigned(t.h),
            )
        };
        let (a, b) = (r(self), r(other));
        b.0 >= a.0 && b.1 >= a.1 && b.2 <= a.2 && b.3 <= a.3
    }
}

/// What is binned: everything but the whole-pixel translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct JobKey {
    path: u64,
    /// The stroke's numbers' bits, or none for a fill.
    stroke: Option<[u32; 5]>,
    fill_rule: FillRule,
    /// The linear part's bits and the translation's fraction's.
    linear: [u32; 4],
    fraction: [u32; 2],
    /// Into an atlas page, not the window.
    atlas: bool,
    /// The n-th use of the same key this frame.
    nth: u32,
}

struct Job {
    slot: u32,
    curves: u64,
    stroke: bool,
    lines: Range,
    /// Line slots the job's curves may fill (its range may be larger).
    line_count: u32,
    tiles: Range,
    tile_count: u32,
    instances: Range,
    entries: Range,
    rect: TileRect,
    used: u64,
    /// Drawn for this frame only (a canvas tile, a moving camera's
    /// paths): let go when the frame ends unless drawn kept since.
    once: bool,
}

/// Where a job is drawn from: its instances.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Drawn {
    pub instances: std::ops::Range<u32>,
}

/// What a job is painted with this frame.
#[derive(Clone, Debug)]
pub(crate) struct Paint {
    pub color: [f32; 4],
    /// A linear gradient: its stops' first index and count, its line in the
    /// path's space.
    pub gradient: Option<LinearPaint>,
    pub alpha: f32,
    pub device_clip: u32,
    pub local_clip: u32,
}

/// Everything about the paths drawn: their buffers, jobs and caches.
pub(crate) struct Paths {
    pub(crate) curves: Arena,
    pub(crate) lines: Arena,
    pub(crate) tiles: Arena,
    pub(crate) entries: Arena,
    pub(crate) instances: Arena,
    /// Job records, draw records and per-job state, by slot.
    pub(crate) jobs: Scratch,
    pub(crate) draws: Scratch,
    pub(crate) state: Scratch,
    pub(crate) work: Scratch,
    pub(crate) scratch: Scratch,
    pub(crate) params: Scratch,
    job_words: Vec<u32>,
    draw_words: Vec<u32>,
    /// Slots whose draw record changed since the last upload.
    draws_dirty: Vec<u32>,
    /// The draw and job buffers were made anew (their contents not kept):
    /// written whole next time.
    draws_whole: bool,
    jobs_whole: bool,
    /// Writes waiting for this frame's encoder, coalesced, copied through
    /// one staging belt: a frame that bins thousands of paths makes a few
    /// copies, not a staging buffer per write.
    staged: Staged,
    belt: wgpu::util::StagingBelt,
    /// What `end_frame` lets go, kept so a frame allocates nothing for it.
    gone: Vec<(u32, Range, Range, Range, Range)>,
    freed: Vec<Range>,
    slots: u32,
    free_slots: Vec<u32>,
    sets: HashMap<(u64, bool), CurveSet>,
    cache: HashMap<JobKey, Job>,
    seen: HashMap<JobKey, u32>,
    /// Dashed paths, cut once: by the path's key and the dash's bits, with
    /// the frame they were last drawn in.
    dashed: HashMap<(u64, Vec<u32>), (Path, u64)>,
    /// Jobs to flatten and bin this frame.
    dirty: Vec<JobKey>,
    frame: u64,
    /// Work prepared for this frame's compute passes.
    pub(crate) pending: Option<Pending>,
    /// Jobs binned since made: what tests and devtools count.
    pub(crate) binned: u64,
    /// Jobs drawn while it is set are this frame's only ([`Job::once`]).
    pub(crate) once: bool,
    /// How draws treat the contained-quad clip skip ([`ClipSkip`]).
    pub(crate) clip_skip: ClipSkip,
}

/// How paths and curve glyphs skip a clip chain their quad lies wholly
/// inside (`clip_needed` in `path_render.wgsl`, `slug.wgsl`): as they draw,
/// or, for tests, marking each pixel drawn with the test skipped (pure
/// green), or never skipping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "tests mark or turn off the skip"))]
pub(crate) enum ClipSkip {
    #[default]
    Skip,
    Mark,
    Never,
}

impl ClipSkip {
    /// The word the shaders read.
    pub(crate) fn word(self) -> u32 {
        match self {
            Self::Skip => 0,
            Self::Mark => 1,
            Self::Never => 2,
        }
    }
}

/// The sizes of this frame's dispatches.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pending {
    pub points: u32,
    pub items: u32,
    pub lines: u32,
    pub rows: u32,
    /// The ranges to clear before binning.
    clears: usize,
}

impl Paths {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let storage = wgpu::BufferUsages::STORAGE;
        Self {
            curves: Arena::new(device, "gui path curves", 64, 1024),
            lines: Arena::new(device, "gui path lines", 16, 1 << 14),
            tiles: Arena::new(device, "gui path tiles", 16, 1 << 12),
            entries: Arena::new(device, "gui path entries", 32, 1 << 14),
            instances: Arena::new(device, "gui path instances", 16, 1 << 12),
            jobs: Scratch::new(device, "gui path jobs", storage, 64 * 4 * 32),
            draws: Scratch::new(device, "gui path draws", storage, 64 * 4 * 32),
            state: Scratch::new(device, "gui path state", storage, 64 * 4),
            work: Scratch::new(device, "gui path work", storage, 4096),
            scratch: Scratch::new(device, "gui path scratch", storage, 4096),
            params: Scratch::new(
                device,
                "gui path params",
                wgpu::BufferUsages::UNIFORM,
                super::contract::PASS_PARAMS,
            ),
            job_words: Vec::new(),
            draw_words: Vec::new(),
            draws_dirty: Vec::new(),
            draws_whole: true,
            jobs_whole: true,
            staged: Staged::default(),
            gone: Vec::new(),
            freed: Vec::new(),
            belt: wgpu::util::StagingBelt::new(device.clone(), 4 << 20),
            slots: 0,
            free_slots: Vec::new(),
            sets: HashMap::new(),
            cache: HashMap::new(),
            seen: HashMap::new(),
            dashed: HashMap::new(),
            dirty: Vec::new(),
            frame: 0,
            pending: None,
            binned: 0,
            once: false,
            clip_skip: ClipSkip::Skip,
        }
    }

    /// The bytes held on the GPU.
    pub(crate) fn bytes(&self) -> usize {
        let b = [&self.curves, &self.lines, &self.tiles, &self.entries, &self.instances]
            .iter()
            .map(|a| a.bytes())
            .sum::<u64>()
            .saturating_add(self.jobs.buffer.size())
            .saturating_add(self.draws.buffer.size());
        usize::try_from(b).unwrap_or(usize::MAX)
    }

    /// How many jobs are kept.
    pub(crate) fn jobs_held(&self) -> usize {
        self.cache.len()
    }

    /// The curves of `path` on the GPU, uploaded the first time.
    fn curve_set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, path: &Path, stroke: bool) -> u64 {
        let key = path.key() ^ u64::from(stroke).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let frame = self.frame;
        if let Some(set) = self.sets.get_mut(&(key, stroke)) {
            set.used = frame;
            return key;
        }
        let recs = records(path, stroke);
        let range = self
            .curves
            .alloc(device, queue, u32::try_from(recs.len()).unwrap_or(u32::MAX).max(1));
        let mut words: Vec<u32> = Vec::with_capacity(recs.len().saturating_mul(CURVE));
        for (r, info) in &recs {
            for p in r.points {
                words.push(p[0].to_bits());
                words.push(p[1].to_bits());
            }
            words.push(r.order);
            words.push(r.flags);
            words.push(if r.prev == NONE {
                NONE
            } else {
                r.prev.saturating_add(range.start)
            });
            words.push(info.wang.to_bits());
            words.push(info.length.to_bits());
            words.extend([0, 0, 0]);
        }
        self.staged.push(Dest::Curves, self.curves.offset(range.start), &words);
        self.sets.insert(
            (key, stroke),
            CurveSet {
                range,
                info: recs.into_iter().map(|(_, i)| i).collect(),
                used: frame,
            },
        );
        key
    }

    fn slot(&mut self) -> u32 {
        if let Some(s) = self.free_slots.pop() {
            return s;
        }
        let s = self.slots;
        self.slots = self.slots.saturating_add(1);
        let n = usize::try_from(self.slots).unwrap_or(0);
        self.job_words.resize(n.saturating_mul(JOB), 0);
        self.draw_words.resize(n.saturating_mul(DRAW), 0);
        s
    }

    /// Draw `path` with `style` through `m` (its space to window pixels),
    /// where `visible` (x0, y0, x1, y1, window pixels) shows; `None` when
    /// nothing of it shows. Into an atlas page (`atlas`), it is binned for
    /// `visible` exactly.
    #[expect(clippy::too_many_arguments, reason = "one draw's whole description")]
    pub(crate) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
        style: &PathStyle,
        m: [f32; 6],
        visible: [f32; 4],
        paint: &Paint,
        atlas: bool,
    ) -> Option<Drawn> {
        if path.is_empty() {
            return None;
        }
        let (s, _) = scales(m);
        if !(s.is_finite() && s > 0.0) {
            return None;
        }
        let stroke = match &style.draw {
            Draw::Fill => None,
            Draw::Stroke(st) => Some((StrokeParams::new(st, s), st)),
        };
        let frame = self.frame;
        let dashed = stroke.as_ref().and_then(|(_, st)| st.dash.as_ref()).map(|d| {
            let mut bits: Vec<u32> = d.array.iter().map(|v| v.to_bits()).collect();
            bits.push(d.offset.to_bits());
            let entry = self
                .dashed
                .entry((path.key(), bits))
                .or_insert_with(|| (path.dashed(d), frame));
            entry.1 = frame;
            entry.0.clone()
        });
        let path = dashed.as_ref().unwrap_or(path);
        // Where the path can reach on the window.
        let [bx0, by0, bx1, by1] = path.bounds();
        let corners = [[bx0, by0], [bx1, by0], [bx0, by1], [bx1, by1]].map(|p| apply(m, p));
        let reach = stroke.as_ref().map_or(0.0, |(p, _)| p.reach(s)) + 1.0;
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for c in corners {
            b = [b[0].min(c[0]), b[1].min(c[1]), b[2].max(c[0]), b[3].max(c[1])];
        }
        let b = [b[0] - reach, b[1] - reach, b[2] + reach, b[3] + reach];
        let shown = [
            b[0].max(visible[0]),
            b[1].max(visible[1]),
            b[2].min(visible[2]),
            b[3].min(visible[3]),
        ];
        if shown[2] <= shown[0] || shown[3] <= shown[1] {
            return None;
        }
        // The whole-pixel part of the translation moves a binned job: it is
        // binned in coordinates from there, and drawn offset by it.
        let (ex, fy) = (m[4].floor(), m[5].floor());
        let binned_m = [m[0], m[1], m[2], m[3], m[4] - ex, m[5] - fy];
        let local = |r: [f32; 4]| [r[0] - ex, r[1] - fy, r[2] - ex, r[3] - fy];
        let (shown, b) = (local(shown), local(b));
        let mut key = JobKey {
            path: path.key(),
            stroke: stroke.as_ref().map(|(p, st)| {
                [
                    p.half.to_bits(),
                    st.width.to_bits(),
                    p.miter_limit.to_bits(),
                    cap_word(p.cap) | (join_word(p.join) << 2),
                    p.alpha.to_bits(),
                ]
            }),
            fill_rule: path.fill_rule(),
            linear: [m[0], m[1], m[2], m[3]].map(f32::to_bits),
            fraction: [binned_m[4], binned_m[5]].map(f32::to_bits),
            atlas,
            nth: 0,
        };
        let nth = self.seen.entry(key).or_insert(0);
        key.nth = *nth;
        *nth = nth.saturating_add(1);
        let want = TileRect::covering(shown)?;
        let reuse = self.cache.get(&key).is_some_and(|j| j.rect.contains(&want));
        if !reuse {
            let rect = if atlas {
                want
            } else {
                let grown = [
                    shown[0] - MARGIN,
                    shown[1] - MARGIN,
                    shown[2] + MARGIN,
                    shown[3] + MARGIN,
                ];
                let r = [
                    grown[0].max(b[0]),
                    grown[1].max(b[1]),
                    grown[2].min(b[2]),
                    grown[3].min(b[3]),
                ];
                TileRect::covering(r).unwrap_or(want)
            };
            let curves = self.curve_set(device, queue, path, stroke.is_some());
            self.bin(
                device,
                queue,
                key,
                curves,
                rect,
                binned_m,
                stroke.as_ref().map(|(p, _)| *p),
            );
        }
        let job = self.cache.get_mut(&key)?;
        job.used = frame;
        job.once &= self.once;
        if let Some(set) = self.sets.get_mut(&(job.curves, job.stroke)) {
            set.used = frame;
        }
        let slot = job.slot;
        let instances = job.instances.start..job.instances.start.saturating_add(job.tile_count);
        let hairline = stroke.as_ref().map_or(1.0, |(p, _)| p.alpha);
        let inv = invert(m).unwrap_or([0.0; 6]);
        let mut w = [0_u32; DRAW];
        {
            w[0] = ex.to_bits();
            w[1] = fy.to_bits();
            for (d, v) in w.iter_mut().skip(2).zip(inv) {
                *d = v.to_bits();
            }
            for (d, v) in w.iter_mut().skip(8).zip(paint.color) {
                *d = v.to_bits();
            }
            if let Some((first, count, line)) = paint.gradient {
                w[12] = 1;
                w[13] = first;
                w[14] = count;
                for (d, v) in w.iter_mut().skip(16).zip(line) {
                    *d = v.to_bits();
                }
            }
            w[15] = (paint.alpha * hairline).to_bits();
            w[20] = paint.device_clip;
            w[21] = paint.local_clip;
            w[22] = self.clip_skip.word();
        }
        if let Some(d) = words_at::<DRAW>(&mut self.draw_words, slot)
            && *d != w
        {
            *d = w;
            self.draws_dirty.push(slot);
        }
        Some(Drawn { instances })
    }

    /// Reserve `key`'s ranges for `rect` and write its job record: it is
    /// binned in this frame's compute passes.
    #[expect(clippy::too_many_arguments, reason = "a job's whole description")]
    fn bin(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: JobKey,
        curves: u64,
        rect: TileRect,
        m: [f32; 6],
        stroke: Option<StrokeParams>,
    ) {
        let (s, _) = scales(m);
        let Some(set) = self.sets.get(&(curves, stroke.is_some())) else {
            return;
        };
        let mut total_lines = 0_u32;
        let mut total_length = 0.0_f32;
        for c in &set.info {
            let (a, b) = line_bounds(c, s, stroke.as_ref());
            total_lines = total_lines.saturating_add(a).saturating_add(b);
            let h = stroke.as_ref().map_or(0.0, |p| p.half * s);
            total_length += 2.0 * c.length * s + f32_of(segment_bound(c, s)) * 12.0 * h + 40.0 * h;
        }
        let tiles = rect.count();
        let entries = entry_bound(total_lines, total_length);
        // A job kept with ranges large enough is binned again in place.
        let old = self.cache.remove(&key);
        let slot = old.as_ref().map_or_else(|| self.slot(), |j| j.slot);
        let keep = |arena: &mut Arena, old: Option<Range>, n: u32| match old {
            Some(r) if r.len >= n => r,
            Some(r) => {
                arena.release(r);
                arena.alloc(device, queue, n)
            }
            None => arena.alloc(device, queue, n),
        };
        let lines_r = keep(&mut self.lines, old.as_ref().map(|j| j.lines), total_lines);
        let tiles_r = keep(&mut self.tiles, old.as_ref().map(|j| j.tiles), tiles);
        let inst_r = keep(&mut self.instances, old.as_ref().map(|j| j.instances), tiles);
        let entries_r = keep(&mut self.entries, old.as_ref().map(|j| j.entries), entries);
        let flags = if stroke.is_some() { F_STROKE } else { 0 }
            | if key.fill_rule == FillRule::EvenOdd {
                F_EVEN_ODD
            } else {
                0
            }
            | stroke.as_ref().map_or(0, |p| {
                (cap_word(p.cap) << CAP_SHIFT) | (join_word(p.join) << JOIN_SHIFT)
            });
        if let Some(w) = words_at::<JOB>(&mut self.job_words, slot) {
            w.fill(0);
            for (d, v) in w.iter_mut().zip(m) {
                *d = v.to_bits();
            }
            w[6] = lines_r.start;
            w[7] = total_lines;
            w[8] = tiles_r.start;
            w[9] = rect.x0.cast_unsigned();
            w[10] = rect.y0.cast_unsigned();
            w[11] = rect.w;
            w[12] = rect.h;
            w[13] = entries_r.start;
            w[14] = entries;
            w[15] = inst_r.start;
            w[JOB_FLAGS] = flags;
            if let Some(p) = stroke {
                w[17] = p.half.to_bits();
                w[18] = p.miter_limit.to_bits();
                w[20] = p.theta.to_bits();
                w[21] = p.arc_bound;
                w[23] = p.smooth.to_bits();
            }
            w[19] = TOLERANCE.to_bits();
            w[25] = slot;
        }
        self.cache.insert(
            key,
            Job {
                slot,
                curves,
                stroke: stroke.is_some(),
                lines: lines_r,
                line_count: total_lines,
                tiles: tiles_r,
                tile_count: tiles,
                instances: inst_r,
                entries: entries_r,
                rect,
                used: self.frame,
                once: self.once,
            },
        );
        self.dirty.push(key);
        self.binned = self.binned.saturating_add(1);
    }

    /// Write this frame's records and work for the GPU; the bytes written.
    /// Draw and job records are staged where they changed, for
    /// [`Self::flush`] to copy at the start of the frame's encoder.
    pub(crate) fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> u64 {
        let slots = u64::from(self.slots.max(1));
        let words = |n: usize| u64::try_from(n).unwrap_or(0).saturating_mul(4).saturating_mul(slots);
        let (draws, jobs) = (self.draws.generation, self.jobs.generation);
        self.jobs.ensure(device, words(JOB));
        self.draws.ensure(device, words(DRAW));
        self.state.ensure(device, words(1));
        self.draws_whole |= draws != self.draws.generation;
        self.jobs_whole |= jobs != self.jobs.generation;
        if std::mem::take(&mut self.draws_whole) {
            self.draws_dirty.clear();
            self.staged.push(Dest::Draws, 0, &self.draw_words);
        } else {
            stage_slots(
                &mut self.staged,
                Dest::Draws,
                &mut self.draws_dirty,
                &self.draw_words,
                DRAW,
            );
        }
        if self.dirty.is_empty() {
            self.pending = None;
            return self.staged.bytes();
        }
        if std::mem::take(&mut self.jobs_whole) {
            self.staged.push(Dest::Jobs, 0, &self.job_words);
        } else {
            let mut slots: Vec<u32> = self
                .dirty
                .iter()
                .filter_map(|k| self.cache.get(k).map(|j| j.slot))
                .collect();
            stage_slots(&mut self.staged, Dest::Jobs, &mut slots, &self.job_words, JOB);
        }
        let mut bytes = self.staged.bytes();
        let mut items: Vec<u32> = Vec::new();
        let mut line_ranges: Vec<u32> = Vec::new();
        let mut row_ranges: Vec<u32> = Vec::new();
        let (mut lines, mut rows) = (0_u32, 0_u32);
        for key in &self.dirty {
            let Some(job) = self.cache.get(key) else { continue };
            let Some(set) = self.sets.get(&(job.curves, job.stroke)) else {
                continue;
            };
            let j = usize::try_from(job.slot).unwrap_or(0).saturating_mul(JOB);
            let w = self
                .job_words
                .get(j..)
                .and_then(|w| w.first_chunk::<JOB>())
                .copied()
                .unwrap_or([0; JOB]);
            let m: [f32; 6] = std::array::from_fn(|i| f32::from_bits(w.get(i).copied().unwrap_or(0)));
            let (s, _) = scales(m);
            let stroke = job.stroke.then(|| stroke_from_words(&w));
            let mut base = job.lines.start;
            for (i, c) in (0_u32..).zip(&set.info) {
                let (a, b) = line_bounds(c, s, stroke.as_ref());
                items.extend([
                    set.range.start.saturating_add(i),
                    job.slot,
                    base,
                    segment_bound(c, s),
                    a,
                    b,
                    0,
                    0,
                ]);
                base = base.saturating_add(a).saturating_add(b);
            }
            line_ranges.extend([lines, job.slot]);
            lines = lines.saturating_add(job.line_count);
            row_ranges.extend([rows, job.slot]);
            rows = rows.saturating_add(job.rect.h);
        }
        let n_items = u32::try_from(items.len() / ITEM).unwrap_or(u32::MAX);
        let line_at = u32::try_from(items.len()).unwrap_or(0);
        let n_line_ranges = u32::try_from(line_ranges.len() / 2).unwrap_or(0);
        let row_at = line_at.saturating_add(u32::try_from(line_ranges.len()).unwrap_or(0));
        let n_row_ranges = u32::try_from(row_ranges.len() / 2).unwrap_or(0);
        let mut work = items;
        work.extend(line_ranges);
        work.extend(row_ranges);
        self.work.write(device, queue, &words_bytes(&work));
        bytes = bytes.saturating_add(u64::try_from(work.len()).unwrap_or(0).saturating_mul(4));
        let points = n_items.saturating_mul(4);
        self.scratch.ensure(
            device,
            u64::from(points)
                .saturating_mul(2)
                .saturating_add(u64::from(n_items).saturating_mul(u64::try_from(HEAD).unwrap_or(0)))
                .saturating_mul(4),
        );
        let params = super::contract::PassParams {
            n_points: points,
            n_items,
            n_lines: lines,
            n_rows: rows,
            line_ranges: line_at,
            n_line_ranges,
            row_ranges: row_at,
            n_row_ranges,
        };
        self.params.write(device, queue, bytemuck::bytes_of(&params));
        self.pending = Some(Pending {
            points,
            items: n_items,
            lines,
            rows,
            clears: self.dirty.len(),
        });
        bytes
    }

    /// Copy what was staged into the path buffers, first in `encoder`.
    pub(crate) fn flush(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let staged = std::mem::take(&mut self.staged);
        for (dest, offset, range) in &staged.spans {
            let buffer = match dest {
                Dest::Curves => &self.curves.buffer,
                Dest::Draws => &self.draws.buffer,
                Dest::Jobs => &self.jobs.buffer,
            };
            let Some(words) = staged.words.get(range.clone()) else {
                continue;
            };
            let size = u64::try_from(words.len()).unwrap_or(0).saturating_mul(4);
            let Some(size) = wgpu::BufferSize::new(size) else {
                continue;
            };
            if offset.saturating_add(size.get()) > buffer.size() {
                continue;
            }
            self.belt
                .write_buffer(encoder, buffer, *offset, size)
                .copy_from_slice(bytemuck::cast_slice(words));
        }
        self.belt.finish_and_recall_on_submit(encoder);
        // Keep the allocation for the next frame.
        let mut staged = staged;
        staged.words.clear();
        staged.spans.clear();
        self.staged = staged;
    }

    /// Clear the dirty jobs' lines, tiles and state, then run the compute
    /// passes (`pipelines` in pass order) with `bind`.
    pub(crate) fn encode(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &ComputePipelines,
        bind: &wgpu::BindGroup,
        stamp: Option<(&wgpu::QuerySet, u32)>,
    ) {
        let Some(p) = self.pending.take() else { return };
        // Each job's ranges cleared, neighbours merged into one clear: the
        // jobs binned in a frame were mostly given ranges side by side.
        let (mut lines, mut tiles, mut state) = (Vec::new(), Vec::new(), Vec::new());
        for key in self.dirty.drain(..).take(p.clears) {
            let Some(job) = self.cache.get(&key) else { continue };
            lines.push((job.lines.start, job.lines.len));
            tiles.push((job.tiles.start, job.tiles.len));
            state.push((job.slot, 1));
        }
        for (arena, ranges) in [(&self.lines, lines), (&self.tiles, tiles)] {
            for (start, len) in merged(ranges) {
                let r = Range { start, len };
                encoder.clear_buffer(&arena.buffer, arena.offset(r.start), Some(arena.size(r)));
            }
        }
        for (start, len) in merged(state) {
            encoder.clear_buffer(&self.state.buffer, u64::from(start) * 4, Some(u64::from(len) * 4));
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gui paths"),
            timestamp_writes: stamp.map(|(set, at)| wgpu::ComputePassTimestampWrites {
                query_set: set,
                beginning_of_pass_write_index: Some(at),
                end_of_pass_write_index: Some(at.saturating_add(1)),
            }),
        });
        pass.set_bind_group(0, bind, &[]);
        let groups = |n: u32| n.div_ceil(64).max(1);
        for (pipeline, n) in [
            (&pipelines.transform, p.points),
            (&pipelines.flatten, p.items),
            (&pipelines.joins, p.items),
            (&pipelines.count, p.lines),
            (&pipelines.scan, p.rows),
            (&pipelines.write, p.lines),
            (&pipelines.compact, p.rows),
        ] {
            if n == 0 {
                continue;
            }
            pass.set_pipeline(pipeline);
            dispatch(&mut pass, groups(n));
        }
    }

    /// The frame is drawn: forget jobs and curves the last [`KEEP`] frames
    /// did not use. A frame draws only the chunks in its damage, so a path
    /// that did not change is kept binned while it may be drawn again.
    pub(crate) fn end_frame(&mut self) {
        let frame = self.frame;
        let stale = |used: u64| frame.saturating_sub(used) >= KEEP;
        let mut gone = std::mem::take(&mut self.gone);
        self.cache.retain(|_, j| {
            if j.once || stale(j.used) {
                gone.push((j.slot, j.lines, j.tiles, j.instances, j.entries));
                false
            } else {
                true
            }
        });
        for (slot, lines, tiles, instances, entries) in gone.drain(..) {
            self.lines.release(lines);
            self.tiles.release(tiles);
            self.instances.release(instances);
            self.entries.release(entries);
            self.free_slots.push(slot);
            if let Some(w) = words_at::<DRAW>(&mut self.draw_words, slot) {
                w.fill(0);
                self.draws_dirty.push(slot);
            }
        }
        self.gone = gone;
        let mut freed = std::mem::take(&mut self.freed);
        self.sets.retain(|_, s| {
            if stale(s.used) {
                freed.push(s.range);
                false
            } else {
                true
            }
        });
        for r in freed.drain(..) {
            self.curves.release(r);
        }
        self.freed = freed;
        self.dashed.retain(|_, (_, used)| !stale(*used));
        self.seen.clear();
        self.frame = self.frame.saturating_add(1);
    }
}

/// Which path buffer a staged write goes to (resolved when copied, so a
/// buffer grown meanwhile gets it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dest {
    Curves,
    Draws,
    Jobs,
}

/// Writes for the next encoder: words, and where each run of them goes.
#[derive(Default)]
struct Staged {
    words: Vec<u32>,
    spans: Vec<(Dest, u64, std::ops::Range<usize>)>,
}

impl Staged {
    /// Write `words` at byte `offset` of `dest`, joined to the last write
    /// when it ends where this one starts.
    fn push(&mut self, dest: Dest, offset: u64, words: &[u32]) {
        if words.is_empty() {
            return;
        }
        let start = self.words.len();
        self.words.extend_from_slice(words);
        let end = self.words.len();
        if let Some((d, o, r)) = self.spans.last_mut()
            && *d == dest
            && r.end == start
            && o.saturating_add(u64::try_from(r.len()).unwrap_or(0).saturating_mul(4)) == offset
        {
            r.end = end;
            return;
        }
        self.spans.push((dest, offset, start..end));
    }

    fn bytes(&self) -> u64 {
        u64::try_from(self.words.len()).unwrap_or(0).saturating_mul(4)
    }
}

/// Stage the records of `slots` (each `n` words of `all`), runs of
/// neighbouring slots as one write.
fn stage_slots(staged: &mut Staged, dest: Dest, slots: &mut Vec<u32>, all: &[u32], n: usize) {
    let mut runs = std::mem::take(slots);
    runs.sort_unstable();
    runs.dedup();
    let pairs: Vec<(u32, u32)> = runs.iter().map(|s| (*s, 1)).collect();
    for (start, len) in merged(pairs) {
        let from = usize::try_from(start).unwrap_or(usize::MAX).saturating_mul(n);
        let to = from.saturating_add(usize::try_from(len).unwrap_or(0).saturating_mul(n));
        if let Some(words) = all.get(from..to) {
            let at = u64::try_from(from).unwrap_or(0).saturating_mul(4);
            staged.push(dest, at, words);
        }
    }
    runs.clear();
    *slots = runs;
}

/// Ranges (start, length) sorted and joined where they touch or overlap.
fn merged(mut ranges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    ranges.retain(|r| r.1 > 0);
    ranges.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (start, len) in ranges {
        let end = start.saturating_add(len);
        if let Some(last) = out.last_mut()
            && start <= last.0.saturating_add(last.1)
        {
            last.1 = end.max(last.0.saturating_add(last.1)).saturating_sub(last.0);
            continue;
        }
        out.push((start, len));
    }
    out
}

/// A large dispatch is split along y: at most 65535 groups per dimension.
fn dispatch(pass: &mut wgpu::ComputePass<'_>, groups: u32) {
    pass.dispatch_workgroups(groups.min(65_535), groups.div_ceil(65_535).max(1), 1);
}

fn stroke_from_words(w: &[u32]) -> StrokeParams {
    let f = |i: usize| f32::from_bits(w.get(i).copied().unwrap_or(0));
    let flags = w.get(JOB_FLAGS).copied().unwrap_or(0);
    StrokeParams {
        half: f(17),
        cap: match (flags >> CAP_SHIFT) & CAP_MASK {
            CAP_ROUND => Cap::Round,
            CAP_SQUARE => Cap::Square,
            _ => Cap::Butt,
        },
        join: match (flags >> JOIN_SHIFT) & JOIN_MASK {
            JOIN_ROUND => Join::Round,
            JOIN_BEVEL => Join::Bevel,
            _ => Join::Miter,
        },
        miter_limit: f(18),
        theta: f(20),
        arc_bound: w.get(21).copied().unwrap_or(2),
        smooth: f(23),
        alpha: 1.0,
    }
}

pub(crate) fn cap_word(c: Cap) -> u32 {
    match c {
        Cap::Butt => CAP_BUTT,
        Cap::Round => CAP_ROUND,
        Cap::Square => CAP_SQUARE,
    }
}

pub(crate) fn join_word(j: Join) -> u32 {
    match j {
        Join::Miter => JOIN_MITER,
        Join::Round => JOIN_ROUND,
        Join::Bevel => JOIN_BEVEL,
    }
}

/// The `N` words of record `slot`.
fn words_at<const N: usize>(words: &mut [u32], slot: u32) -> Option<&mut [u32; N]> {
    let at = usize::try_from(slot).ok()?.checked_mul(N)?;
    words.get_mut(at..)?.first_chunk_mut::<N>()
}

pub(crate) fn words_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_ne_bytes()).collect()
}

/// The paint of `g`, its stops put in `stops` (two vec4s each).
/// A linear gradient on the GPU: its stops' first index and count, its line.
pub(crate) type LinearPaint = (u32, u32, [f32; 4]);

pub(crate) fn paint_of(g: &Gradient, stops: &mut Vec<f32>) -> ([f32; 4], Option<LinearPaint>) {
    match g {
        Gradient::Solid(c) => (super::rgba(*c), None),
        Gradient::Linear { start, end, stops: s } => {
            let first = u32::try_from(stops.len() / 8).unwrap_or(0);
            for (offset, c) in s {
                stops.extend_from_slice(&[*offset, 0.0, 0.0, 0.0]);
                stops.extend_from_slice(&super::rgba(*c));
            }
            let count = u32::try_from(s.len()).unwrap_or(0);
            ([0.0; 4], Some((first, count, [start.0, start.1, end.0, end.1])))
        }
    }
}

/// The compute passes, in order.
pub(crate) struct ComputePipelines {
    pub transform: wgpu::ComputePipeline,
    pub flatten: wgpu::ComputePipeline,
    pub joins: wgpu::ComputePipeline,
    pub count: wgpu::ComputePipeline,
    pub scan: wgpu::ComputePipeline,
    pub write: wgpu::ComputePipeline,
    pub compact: wgpu::ComputePipeline,
}

impl ComputePipelines {
    pub(crate) fn new(device: &wgpu::Device, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule) -> Self {
        let make = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        Self {
            transform: make("transform"),
            flatten: make("flatten"),
            joins: make("joins"),
            count: make("count"),
            scan: make("scan"),
            write: make("write"),
            compact: make("compact"),
        }
    }
}
