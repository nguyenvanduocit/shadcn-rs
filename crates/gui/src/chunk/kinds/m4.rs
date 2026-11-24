//! M4 on the GPU (ADR-0018), shared by lines, areas and bars: a pass over
//! gangs of pages bound side by side (one dispatch a gang, not a page) that
//! finds each pixel column's points by binary search on x and sums them
//! through the pyramid, one thread a column a page meets, a merge of the
//! pages' partials in page order, an ordered prefix sum and a compaction
//! into the list of points the marks draw. The CPU computes only the column bounds (one `i64` a
//! column), which pages meet the view, and the two points just outside it.

use super::super::runtime::{Bind, BindKind, ChunkCompute, ChunkUpload, ComputeId, PipelineBuilder};
use super::super::series::{NONE, column_count};
use super::super::store::{BufferId, ResidentPage, split};
use super::MarkView;
use crate::gpu::contract::{Const, Value, bytes_of, prelude};

/// The most device pixel columns a chunk reduces to.
pub const MAX_COLUMNS: u32 = 16_384;
/// Bytes between uniform blocks (wgpu's largest `min_uniform_buffer_offset_alignment`).
pub const BLOCK: u64 = 256;
/// Words of a partial, of an extended column: prepended to every M4 shader
/// (`m4_shader`), so the passes cannot differ from what is allocated here.
const PARTIAL: u64 = 24;
const COLUMN: u64 = 20;
const M4_CONSTS: &[Const] = &[("PARTIAL", Value::Wide(PARTIAL)), ("COLUMN", Value::Wide(COLUMN))];

/// What every M4 pass reads (`M4` in `m4_common.wgsl`), at the start of the
/// chunk's uniform buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct M4Params {
    cols: u32,
    npages: u32,
    before: u32,
    after: u32,
    /// The view's first x, as (high, low) words.
    x0: [u32; 2],
    kx: f32,
    bx: f32,
    /// The view's top y (high, low parts).
    y1: [f32; 2],
    ky: f32,
    by: f32,
    base: f32,
    floor_on: u32,
    n: u32,
    pad: u32,
}

/// A page's parameters (`PageP` in `m4_page.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct PageP {
    len: u32,
    cap: u32,
    at: u32,
    y: u32,
    levels: u32,
    lo: u32,
    col0: u32,
    col1: u32,
    out: u32,
    fy: u32,
    flo: u32,
    pad: u32,
}

/// A gang's uniform block (`Gang` in `m4_page.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GangP {
    first: u32,
    count: u32,
    threads: u32,
    pad: u32,
}

/// Bytes of the uniforms the passes bind a range of.
const M4_BYTES: u64 = bytes_of::<M4Params>();
const GANG_BYTES: u64 = bytes_of::<GangP>();
/// Bytes of a page's parameters.
const PAGE_P: u64 = bytes_of::<PageP>();

/// An M4 shader: the constants, then `parts`.
fn m4_shader(parts: &[&str]) -> String {
    let mut s = prelude(&[M4_CONSTS]);
    s.push_str(&wgsl(parts, &[]));
    s
}

/// A shader of `parts` reading the pages bound as `pages` (one binding
/// each, declared by `parts`).
pub(crate) fn wgsl(parts: &[&str], pages: &[&str]) -> String {
    let mut s = String::from(include_str!("node.wgsl"));
    s.push_str(include_str!("x64.wgsl"));
    for p in pages {
        s.push_str(&format!("fn {p}_w(i: u32) -> u32 {{\n    return {p}[i];\n}}\n"));
        s.push_str(&include_str!("series.wgsl").replace("PAGE", p));
    }
    for p in parts {
        s.push_str(p);
    }
    s
}

/// The page pass's shader for gangs of `gang` pages: bindings from `first`
/// on are the pages (`pg0`…), then their floor pages (`fpg0`…);
/// `page_w` and `fpage_w` read the one the thread's `cur` picks.
fn gang_wgsl(parts: &[&str], gang: u32, first: u32) -> String {
    let mut s = prelude(&[M4_CONSTS]);
    s.push_str(include_str!("node.wgsl"));
    s.push_str(include_str!("x64.wgsl"));
    for p in parts {
        s.push_str(p);
    }
    for (name, at) in [("pg", first), ("fpg", first.saturating_add(gang))] {
        for k in 0..gang {
            let binding = at.saturating_add(k);
            s.push_str(&format!(
                "@group(0) @binding({binding}) var<storage, read> {name}{k}: array<u32>;\n"
            ));
        }
    }
    for (f, name) in [("page_w", "pg"), ("fpage_w", "fpg")] {
        s.push_str(&format!("fn {f}(i: u32) -> u32 {{\n    switch cur {{\n"));
        for k in 0..gang {
            s.push_str(&format!("        case {k}u: {{ return {name}{k}[i]; }}\n"));
        }
        s.push_str("        default: { return 0u; }\n    }\n}\n");
    }
    s.push_str(&include_str!("series.wgsl").replace("PAGE", "page"));
    s
}

/// The page pass's bindings for gangs of `gang` pages: the pages'
/// parameters, the bounds, the partials (written), then the pages and their
/// floors; two uniforms before them (`gang_wgsl`).
fn page_binds(gang: u32) -> Vec<BindKind> {
    use BindKind::{Storage, StorageWrite, Uniform};
    let mut binds = vec![Uniform, Uniform, Storage, Storage, StorageWrite];
    binds.extend(std::iter::repeat_n(
        Storage,
        usize::try_from(gang.saturating_mul(2)).unwrap_or(2),
    ));
    binds
}

/// Bindings of the page pass before its pages.
const GANG_FIRST: u32 = 5;
/// The most pages a gang binds.
const MAX_GANG: u32 = 16;
/// One pass of the M4 reduction: its pipeline's label, shader, entry point
/// and bindings, as `M4Pipes::new` makes it and the tests check it.
struct PassSpec {
    label: &'static str,
    source: String,
    entry: &'static str,
    binds: Vec<BindKind>,
}

/// The page pass (for gangs of `gang` pages), merge, scan and compact.
fn pass_specs(gang: u32) -> [PassSpec; 4] {
    use BindKind::{Storage, StorageWrite, Uniform};
    let common = include_str!("m4_common.wgsl");
    [
        PassSpec {
            label: "gui m4 page",
            source: gang_wgsl(&[common, include_str!("m4_page.wgsl")], gang, GANG_FIRST),
            entry: "page_pass",
            binds: page_binds(gang),
        },
        PassSpec {
            label: "gui m4 merge",
            source: m4_shader(&[common, include_str!("m4_merge.wgsl")]),
            entry: "merge",
            binds: vec![Uniform, Storage, Storage, StorageWrite],
        },
        PassSpec {
            label: "gui m4 scan",
            source: m4_shader(&[common, include_str!("m4_scan.wgsl"), include_str!("scan.wgsl")]),
            entry: "scan",
            binds: vec![Uniform, Storage, StorageWrite],
        },
        PassSpec {
            label: "gui m4 compact",
            source: m4_shader(&[common, include_str!("m4_compact.wgsl")]),
            entry: "compact",
            binds: vec![Uniform, Storage, Storage, StorageWrite],
        },
    ]
}

pub(crate) struct M4Pipes {
    page: ComputeId,
    /// Pages a gang binds: as many as the device's storage bindings allow.
    gang: u32,
    merge: ComputeId,
    scan: ComputeId,
    compact: ComputeId,
}

impl M4Pipes {
    pub(crate) fn new(ctx: &mut PipelineBuilder<'_>) -> Self {
        // Storage bindings: the pages' parameters, the bounds, the
        // partials, then the pages and their floors; two uniforms besides.
        let limits = ctx.limits();
        let storage = limits.max_storage_buffers_per_shader_stage.saturating_sub(3);
        let buffers = limits
            .max_buffers_and_acceleration_structures_per_shader_stage
            .saturating_sub(5);
        let gang = (storage.min(buffers) / 2).clamp(1, MAX_GANG);
        let [page, merge, scan, compact] = pass_specs(gang).map(|p| ctx.compute(p.label, &p.source, p.entry, &p.binds));
        Self {
            page,
            gang,
            merge,
            scan,
            compact,
        }
    }
}

/// A page the pass reads: its seq, its floor page's (its own when none),
/// and the partials it writes.
#[derive(Clone, Copy, Debug)]
struct PageRun {
    seq: u64,
    floor: u64,
    columns: u32,
}

/// A chunk's M4 buffers and this frame's dispatches.
#[derive(Default)]
pub(crate) struct M4State {
    params: Option<BufferId>,
    pps: Option<BufferId>,
    bounds: Option<BufferId>,
    partials: Option<BufferId>,
    table: Option<BufferId>,
    pub(crate) columns: Option<BufferId>,
    pub offsets: Option<BufferId>,
    pub points: Option<BufferId>,
    runs: Vec<PageRun>,
    /// Pages a gang binds, as the pipes were built.
    gang: u32,
    /// Extended columns (`cols + 2`).
    pub n: u32,
    pub cols: u32,
    /// x and y to pixels: (kx, ky).
    pub scale: (f32, f32),
}

fn words(v: &[u32]) -> &[u8] {
    bytemuck::cast_slice(v)
}

/// x high and low words and y's high and low parts of an `f64`.
fn split_f(v: f64) -> (f32, f32) {
    let hi = crate::convert::f64_to_f32(v);
    (hi, crate::convert::f64_to_f32(v - f64::from(hi)))
}

/// The extended columns page `p` meets, given the column bounds.
fn columns_of(p: &ResidentPage, bounds: &[i64], before: Option<u32>, after: Option<u32>) -> Option<(u32, u32)> {
    let cols = u32::try_from(bounds.len().saturating_sub(1)).ok()?;
    let first = *bounds.first()?;
    let last = *bounds.last()?;
    let end = p.at.saturating_add(p.len);
    let mut lo = u32::MAX;
    let mut hi = 0_u32;
    if p.x_last >= first && p.x_first < last {
        let c0 = u32::try_from(bounds.partition_point(|b| *b <= p.x_first).saturating_sub(1)).ok()?;
        let c1 = u32::try_from(bounds.partition_point(|b| *b <= p.x_last).saturating_sub(1))
            .ok()?
            .min(cols.saturating_sub(1));
        lo = c0.saturating_add(1);
        hi = c1.saturating_add(2);
    }
    if before.is_some_and(|b| b >= p.at && b < end) {
        lo = 0;
        hi = hi.max(1);
    }
    if after.is_some_and(|a| a >= p.at && a < end) {
        lo = lo.min(cols.saturating_add(1));
        hi = cols.saturating_add(2);
    }
    (lo < hi).then_some((lo, hi))
}

impl M4State {
    /// Bring the M4 buffers up to series `source` seen through `view` in
    /// `frame`'s bounds, an area or bar standing on series `floor` (same
    /// pages) or on `view.base`. False when there is nothing to reduce.
    pub(crate) fn update(
        &mut self,
        pipes: &M4Pipes,
        up: &mut ChunkUpload<'_>,
        view: &MarkView,
        source: usize,
        floor: Option<usize>,
        frame: &super::super::FrameInfo,
    ) -> bool {
        self.runs.clear();
        self.n = 0;
        let Some(series) = up.series(source) else { return false };
        if series.snapshot.is_empty() || view.x1 <= view.x0 {
            return false;
        }
        let [bx, by, bw, bh] = frame.bounds;
        let first_col = bx.floor();
        let frac = bx - first_col;
        let cols = column_count(bw, frac).min(MAX_COLUMNS);
        let columns = series.snapshot.columns(view.x0, view.x1, bw, frac);
        let span = i128::from(view.x1).saturating_sub(i128::from(view.x0)).max(1);
        let kx = crate::convert::f64_to_f32(f64::from(bw) / super::i128_f64(span));
        let dy = (view.y1 - view.y0).abs().max(f64::MIN_POSITIVE);
        let ky = crate::convert::f64_to_f32(f64::from(bh) / dy);
        self.scale = (kx, ky);
        let y1 = split_f(view.y1);
        let base = view.base.map_or(by + bh, |b| {
            by + crate::convert::f64_to_f32((view.y1 - b) * f64::from(ky))
        });
        let floor_pages = floor.and_then(|f| up.series(f)).map(|f| f.pages.to_vec());
        let floor_ok = floor_pages.as_ref().is_some_and(|f| {
            f.len() == series.pages.len()
                && f.iter()
                    .zip(series.pages)
                    .all(|(a, b)| a.len == b.len && a.layout.cap == b.layout.cap && a.at == b.at)
        });
        let pages: Vec<ResidentPage> = series.pages.to_vec();
        let mut runs = Vec::new();
        let mut table: Vec<u32> = Vec::new();
        let mut out = 0_u32;
        for (k, p) in pages.iter().enumerate() {
            let Some((c0, c1)) = columns_of(p, &columns.bounds, columns.before, columns.after) else {
                continue;
            };
            let f = floor_pages.as_ref().filter(|_| floor_ok).and_then(|f| f.get(k));
            let lay = p.layout;
            let pp = PageP {
                len: p.len,
                cap: lay.cap,
                at: p.at,
                y: lay.y,
                levels: lay.levels,
                lo: if lay.lo == lay.y { NONE } else { lay.lo },
                col0: c0,
                col1: c1,
                out,
                fy: f.map_or(lay.y, |f| f.layout.y),
                flo: f.map_or(NONE, |f| if f.layout.lo == f.layout.y { NONE } else { f.layout.lo }),
                pad: 0,
            };
            runs.push((
                PageRun {
                    seq: p.seq,
                    floor: f.map_or(p.seq, |f| f.seq),
                    columns: c1.saturating_sub(c0),
                },
                pp,
            ));
            table.extend([c0, c1, out, 0]);
            out = out.saturating_add(c1.saturating_sub(c0));
        }
        if runs.is_empty() {
            return false;
        }
        let n = cols.saturating_add(2);
        let npages = u32::try_from(runs.len()).unwrap_or(0);
        let x0 = split(view.x0);
        let m4 = M4Params {
            cols,
            npages,
            before: columns.before.unwrap_or(NONE),
            after: columns.after.unwrap_or(NONE),
            x0: [x0.0, x0.1],
            kx,
            bx,
            y1: [y1.0, y1.1],
            ky,
            by,
            base,
            floor_on: u32::from(floor_ok),
            n,
            pad: 0,
        };
        // A gang's block: its first page, how many, its threads.
        let gang = usize::try_from(pipes.gang).unwrap_or(1).max(1);
        let gangs: Vec<GangP> = runs
            .chunks(gang)
            .enumerate()
            .map(|(i, g)| {
                let threads = g.iter().map(|(r, _)| r.columns).fold(0_u32, u32::saturating_add);
                let count = u32::try_from(g.len()).unwrap_or(0);
                GangP {
                    first: u32::try_from(i.saturating_mul(gang)).unwrap_or(0),
                    count,
                    threads,
                    pad: 0,
                }
            })
            .collect();
        let blocks = u64::try_from(gangs.len()).unwrap_or(0).saturating_add(1);
        let pps: Vec<PageP> = runs.iter().map(|(_, pp)| *pp).collect();
        let (Some(params), Some(pps_buf), Some(bounds_buf), Some(_), Some(table_buf), Some(_), Some(_), Some(_)) = (
            up.buffer(&mut self.params, BLOCK.saturating_mul(blocks)),
            up.buffer(&mut self.pps, u64::from(npages).saturating_mul(PAGE_P)),
            up.buffer(
                &mut self.bounds,
                u64::try_from(columns.bounds.len()).unwrap_or(0).saturating_mul(8),
            ),
            up.buffer(&mut self.partials, u64::from(out).max(1).saturating_mul(PARTIAL * 4)),
            up.buffer(&mut self.table, u64::from(npages).saturating_mul(16)),
            up.buffer(&mut self.columns, u64::from(n).saturating_mul(COLUMN * 4)),
            up.buffer(&mut self.offsets, u64::from(n).saturating_add(1).saturating_mul(4)),
            up.buffer(&mut self.points, u64::from(n).saturating_mul(4 * 32)),
        ) else {
            return false;
        };
        up.write(params, 0, bytemuck::bytes_of(&m4));
        for (i, g) in gangs.iter().enumerate() {
            let block = BLOCK.saturating_mul(u64::try_from(i).unwrap_or(0).saturating_add(1));
            up.write(params, block, bytemuck::bytes_of(g));
        }
        up.write(pps_buf, 0, bytemuck::cast_slice(&pps));
        let b: Vec<u32> = columns
            .bounds
            .iter()
            .flat_map(|x| {
                let (hi, lo) = split(*x);
                [hi, lo]
            })
            .collect();
        up.write(bounds_buf, 0, words(&b));
        up.write(table_buf, 0, words(&table));
        self.runs = runs.into_iter().map(|(r, _)| r).collect();
        self.gang = pipes.gang;
        self.n = n;
        self.cols = cols;
        true
    }

    /// The passes, in order.
    pub(crate) fn compute(&self, pipes: &M4Pipes, pass: &mut ChunkCompute<'_>) {
        let (Some(params), Some(bounds), Some(partials), Some(table), Some(columns), Some(offsets), Some(points)) = (
            self.params,
            self.bounds,
            self.partials,
            self.table,
            self.columns,
            self.offsets,
            self.points,
        ) else {
            return;
        };
        if self.n == 0 {
            return;
        }
        let m4 = Bind::Range(params, 0, M4_BYTES);
        let gang = usize::try_from(self.gang).unwrap_or(1).max(1);
        for (i, g) in self.runs.chunks(gang).enumerate() {
            let (Some(last), Some(pps)) = (g.last(), self.pps) else {
                continue;
            };
            let threads = g.iter().map(|r| r.columns).fold(0_u32, u32::saturating_add);
            let block = BLOCK.saturating_mul(u64::try_from(i).unwrap_or(0).saturating_add(1));
            let mut binds = vec![
                m4,
                Bind::Range(params, block, GANG_BYTES),
                Bind::Buffer(pps),
                Bind::Buffer(bounds),
                Bind::Buffer(partials),
            ];
            // Slots past the gang's pages repeat its last: never read.
            binds.extend((0..gang).map(|k| Bind::Page(g.get(k).unwrap_or(last).seq)));
            binds.extend((0..gang).map(|k| Bind::Page(g.get(k).unwrap_or(last).floor)));
            pass.dispatch(pipes.page, &binds, [threads.div_ceil(64), 1, 1]);
        }
        let groups = self.n.div_ceil(64);
        pass.dispatch(
            pipes.merge,
            &[m4, Bind::Buffer(table), Bind::Buffer(partials), Bind::Buffer(columns)],
            [groups, 1, 1],
        );
        pass.dispatch(
            pipes.scan,
            &[m4, Bind::Buffer(columns), Bind::Buffer(offsets)],
            [1, 1, 1],
        );
        pass.dispatch(
            pipes.compact,
            &[m4, Bind::Buffer(columns), Bind::Buffer(offsets), Bind::Buffer(points)],
            [groups, 1, 1],
        );
    }

    /// The compact points' capacity: what a mark draws instances up to.
    pub(crate) fn capacity(&self) -> u32 {
        self.n.saturating_mul(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::contract::reflect::{Kind, assert_layout, reflect};
    use naga::ShaderStage;

    /// Every pass of the M4 reduction (page pass for every gang size it can
    /// be built with, merge, scan, compact), as `M4Pipes::new` builds them
    /// (V3): naga validates each; its bindings, and the uniform structs it
    /// reads (size, member names and offsets) against the `#[repr(C)]` types
    /// the writers fill; and the sizes the passes bind.
    #[test]
    fn passes_agree_with_rust_for_every_gang() {
        for gang in 1..=MAX_GANG {
            for spec in pass_specs(gang) {
                let label = format!("{}, gang {gang}", spec.label);
                let r = reflect(&label, &spec.source, false);
                assert_eq!(r.bindings.len(), spec.binds.len(), "{label}: bindings");
                for (i, bind) in spec.binds.iter().enumerate() {
                    let at = u32::try_from(i).unwrap();
                    let found = r
                        .bindings
                        .get(&(0, at))
                        .unwrap_or_else(|| panic!("{label}: Rust binds {at}, the shader declares none"));
                    let same = match (bind, found.kind) {
                        (BindKind::Uniform, Kind::Uniform { .. }) => true,
                        (BindKind::Storage, Kind::Storage { read_only, .. }) => read_only,
                        (BindKind::StorageWrite, Kind::Storage { read_only, .. }) => !read_only,
                        _ => false,
                    };
                    assert!(
                        same,
                        "{label}: binding {at} (`{}`): Rust {bind:?}, shader {:?}",
                        found.name, found.kind
                    );
                    assert_eq!(
                        found.written_by.is_empty(),
                        !matches!(bind, BindKind::StorageWrite),
                        "{label}: binding {at} written"
                    );
                    // The bytes a dispatch binds of a uniform (`compute`).
                    if let Kind::Uniform { size } = found.kind {
                        let bound = if at == 1 { GANG_BYTES } else { M4_BYTES };
                        assert_eq!(u64::from(size), bound, "{label}: binding {at} bytes bound");
                    }
                }
                assert!(
                    r.entry_points.contains(&(spec.entry.to_owned(), ShaderStage::Compute)),
                    "{label}: no compute entry point `{}`",
                    spec.entry
                );
                assert_layout!(
                    r,
                    &label,
                    "M4",
                    M4Params,
                    [
                        cols, npages, before, after, x0, kx, bx, y1, ky, by, base, floor_on, n, pad
                    ]
                );
                if spec.entry == "page_pass" {
                    assert_layout!(
                        r,
                        &label,
                        "PageP",
                        PageP,
                        [len, cap, at, y, levels, lo, col0, col1, out, fy, flo, pad]
                    );
                    assert_layout!(r, &label, "Gang", GangP, [first, count, threads, pad]);
                }
            }
        }
    }

    /// The words of a partial and of a column are Rust's: no M4 shader
    /// declares them.
    #[test]
    fn m4_shaders_do_not_declare_the_sizes_rust_owns() {
        for (file, text) in [
            ("m4_common.wgsl", include_str!("m4_common.wgsl")),
            ("m4_page.wgsl", include_str!("m4_page.wgsl")),
            ("m4_merge.wgsl", include_str!("m4_merge.wgsl")),
            ("m4_scan.wgsl", include_str!("m4_scan.wgsl")),
            ("m4_compact.wgsl", include_str!("m4_compact.wgsl")),
        ] {
            for (name, _) in M4_CONSTS {
                let declared = text
                    .lines()
                    .any(|l| l.trim_start().starts_with(&format!("const {name}:")));
                assert!(!declared, "{file} declares `const {name}`: m4.rs owns it");
            }
        }
    }
}
