//! A canvas on the render thread (ADR-0017, `crate::canvas`): its objects
//! drawn straight through camera-relative matrices, or, past the tiling
//! bound, from a tile cache scheduled as Chromium's tile manager schedules
//! raster (`cc/tiles/tile_manager.cc`, `cc/raster/`).
//!
//! - Tiles are 256 device pixels square in a *tiling*: a raster scale and
//!   turn, world aligned. The raster scale follows Chromium's pinch rules
//!   (`cc/layers/picture_layer_impl.cc`): while the camera moves it steps
//!   at most 2× toward the zoom and snaps to a tiling within 1.2×, and the
//!   turn holds; settled, both are the camera's exactly.
//! - Each frame a canvas states what it wants, most urgent first: the
//!   whole-world tiling (the fallback), then its target tiling's visible
//!   tiles nearest the centre first, then a ring around them stretched
//!   along the pan (Chromium's skewport). Raster workers plan wanted tiles
//!   ([`crate::canvas::planner`]); the render thread rasterizes planned
//!   steps at the start of a frame within [`RASTER_BUDGET`], a tile split
//!   across frames when it is large, so no frame waits for raster.
//! - What shows changes atomically, as Chromium activates a pending tree:
//!   the last complete tiling keeps showing (scaled, turned) until every
//!   visible tile of the target is ready, then the target shows. Where the
//!   shown tiling lacks a tile, the whole-world tiling shows under it, so
//!   nothing is blank; only a canvas with nothing ready at all rasterizes
//!   its whole-world tiles in the frame that first shows it.
//! - A tile becoming ready, or a tiling becoming shown, moves the canvas'
//!   revision, which its chunk's signature holds: that redraws the canvas'
//!   damage without the UI.

use std::sync::Arc;

use super::{GpuRenderer, Op};
use crate::canvas::aabb::{self, Aabb};
use crate::canvas::plan::{Step, View, cut_about, plan_live};
use crate::canvas::planner::{Planner, Request, TileKey, plan_tile};
use crate::canvas::{Camera, WorldSnapshot, WorldView, cos_sin, turn};
use crate::chunk::{DataItem, TextureId};
use crate::figure::Path;
use crate::geometry::Bounds;
use crate::scene::Clip;
use crate::text::Glyphs;

/// Device pixels of a tile's side.
pub(super) const TILE: u32 = 256;
const TILE_F: f64 = 256.0;
const TILE_F32: f32 = 256.0;
/// What a frame may spend rasterizing tiles, in [`Step::cost`] units
/// (eighths of a small path's job): on the stress `canvas` scene a raster
/// frame's CPU p50 is 2.4 ms and its GPU p50 0.2 ms at this budget, where
/// twice it gave 4 ms and a p95 past the 8.3 ms of a 120 Hz frame.
pub const RASTER_BUDGET: u32 = 2048;
/// Tiles not drawn or wanted for this many frames let go.
const TILE_FRAMES: u64 = 240;
/// Bytes of tiles kept, the least recently used let go past it.
const TILE_BUDGET: usize = 512 << 20;
/// Bytes of one tile's texture.
const TILE_BYTES: usize = 4 * 256 * 256;
/// Tiles of prefetch around what is visible.
const PREFETCH: i64 = 1;
/// Frames of pan the prefetch looks ahead.
const SKEW_FRAMES: f64 = 8.0;
/// How long a headless frame waits for the plans the last frame asked for.
const HEADLESS_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Where a tile's raster is.
enum Work {
    /// Asked of the workers.
    Asked,
    /// Planned: its steps, rasterized up to `next`.
    Planned { steps: Vec<Step>, next: usize },
    /// Rastered, waiting to take the shown texture's place.
    Done,
}

/// A raster of a tile under way: the world version it draws, how far it
/// is, the texture it draws into (not the one shown, so nothing half drawn
/// or of two versions ever shows).
struct Raster {
    version: u64,
    work: Work,
    tex: Option<TextureId>,
}

struct Tile {
    /// What shows, and the world version it is of: kept while no change
    /// reaches the tile (Chromium's invalidation).
    tex: Option<TextureId>,
    version: u64,
    raster: Option<Raster>,
    used: u64,
}

impl Tile {
    /// It shows what version `v` of the world shows.
    fn current(&self, v: u64) -> bool {
        self.tex.is_some() && self.version == v
    }
}

/// A raster scale and turn, and where its grid falls within a device
/// pixel: tiles share them. The grid's phase is chosen so that, settled,
/// tile corners land on whole device pixels and a tile's pixels are
/// exactly the pixels drawing straight gives (Chromium's raster
/// translation); a moving camera keeps the phase it has and its tiles are
/// placed on whole pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tiling {
    scale: f64,
    turn: f64,
    phase: (f64, f64),
}

impl Tiling {
    fn of(key: &TileKey) -> Self {
        Self {
            scale: f64::from_bits(key.scale),
            turn: f64::from_bits(key.turn),
            phase: (f64::from_bits(key.phase[0]), f64::from_bits(key.phase[1])),
        }
    }

    fn key(&self, world: u64, (x, y): (i64, i64)) -> TileKey {
        TileKey {
            world,
            scale: self.scale.to_bits(),
            turn: self.turn.to_bits(),
            phase: [self.phase.0.to_bits(), self.phase.1.to_bits()],
            x,
            y,
        }
    }

    /// The view tile `(x, y)` shows: its corner at the target's origin.
    fn view(&self, (x, y): (i64, i64)) -> View {
        let cs = cos_sin(self.turn);
        let u = (
            crate::convert::i64_to_f64(x).mul_add(TILE_F, self.phase.0),
            crate::convert::i64_to_f64(y).mul_add(TILE_F, self.phase.1),
        );
        let w = turn(u, (cs.0, -cs.1));
        View {
            center: (w.0 / self.scale, w.1 / self.scale),
            k: self.scale,
            cs,
            origin: (0.0, 0.0),
            cut: [0.0, 0.0, TILE_F32, TILE_F32],
        }
    }

    /// The world a tile's pixels can be inked from: its area, grown by two
    /// of its pixels (anti-aliasing reaches one past an edge).
    fn reach(&self, (x, y): (i64, i64)) -> Aabb {
        let a = self.view((x, y)).area();
        let m = 2.0 / self.scale.max(f64::MIN_POSITIVE);
        [a[0] - m, a[1] - m, a[2] + m, a[3] + m]
    }

    /// The tile range meeting world polygon `corners`.
    fn range(&self, corners: &[(f64, f64)]) -> (i64, i64, i64, i64) {
        let cs = cos_sin(self.turn);
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for c in corners {
            let u = turn((c.0 * self.scale, c.1 * self.scale), cs);
            let u = (u.0 - self.phase.0, u.1 - self.phase.1);
            b = [b[0].min(u.0), b[1].min(u.1), b[2].max(u.0), b[3].max(u.1)];
        }
        let i = |v: f64| crate::convert::f64_to_i64((v / TILE_F).floor());
        (i(b[0]), i(b[1]), i(b[2]), i(b[3]))
    }

    /// The tiles meeting world polygon `corners`, grown by `margin` tiles,
    /// within those meeting the world's `extent` (the rest are empty).
    fn covering(&self, corners: &[(f64, f64)], margin: i64, extent: &[f64; 4]) -> (i64, i64, i64, i64) {
        let r = self.range(corners);
        let e = self.range(&[
            (extent[0], extent[1]),
            (extent[2], extent[1]),
            (extent[0], extent[3]),
            (extent[2], extent[3]),
        ]);
        (
            r.0.saturating_sub(margin).max(e.0),
            r.1.saturating_sub(margin).max(e.1),
            r.2.saturating_add(margin).min(e.2),
            r.3.saturating_add(margin).min(e.3),
        )
    }
}

/// Each canvas' tiling state.
struct Canvas {
    /// The tiling it rasterizes toward, and the one it shows (the last
    /// whose visible tiles were all current).
    target: Option<Tiling>,
    shown: Option<Tiling>,
    /// The target's visible tiles.
    visible: Vec<TileKey>,
    /// Every tile it wants, most urgent first.
    wants: Vec<TileKey>,
    /// The tiles that show once ready (the whole-world tiling's and the
    /// shown tiling's in view): one finishing redraws the canvas.
    shows: Vec<TileKey>,
    world: Option<Arc<WorldSnapshot>>,
    /// Moves when what it would draw changes.
    revision: u64,
    last: Option<Camera>,
    used: u64,
}

struct Anchored {
    /// The object it was cut from: a moved object is cut again.
    object: Arc<crate::canvas::Placed>,
    cell: (i64, i64),
    zoom: u64,
    path: Path,
    used: u64,
}

/// What the renderer keeps for canvases.
#[derive(Default)]
pub(super) struct WorldTiles {
    tiles: foldhash::HashMap<TileKey, Tile>,
    canvases: std::collections::BTreeMap<u64, Canvas>,
    /// Paths of huge objects anchored near the camera: by world, object.
    anchored: foldhash::HashMap<(u64, u32), Anchored>,
    planner: Option<Planner>,
    /// Tile raster passes for this frame: the texture, its draws, and
    /// whether the pass clears it (its first part).
    pub(super) jobs: Vec<(TextureId, Vec<Op>, bool)>,
    frame: u64,
    /// Tiles finished this frame, tiles drawn from a coarser tiling, and
    /// the raster cost spent.
    rastered: usize,
    filled: usize,
    spent: u32,
    /// Objects drawn live this frame (lifted, or where tiles are being
    /// rastered again), and device pixels drawn live.
    live: (usize, u64),
    /// Both counts and the cost, for the last frame ended.
    pub(super) last: (usize, usize, u32),
    pub(super) last_live: (usize, u64),
    /// Headless: a frame first waits for the plans asked before it.
    pub(super) blocking: bool,
    /// The frame raster last ran in.
    pumped: Option<u64>,
    /// A frame's raster budget other than [`RASTER_BUDGET`] (tests).
    budget: Option<u32>,
    /// Lists a frame fills and empties, kept so a steady frame allocates
    /// nothing for them.
    scratch: Scratch,
}

#[derive(Default)]
struct Scratch {
    held: foldhash::HashSet<TileKey>,
    keys: Vec<TileKey>,
    by_age: Vec<(u64, TileKey)>,
}

impl WorldTiles {
    pub(super) fn end_frame(&mut self, store: &mut crate::chunk::store::Store) {
        self.frame = self.frame.saturating_add(1);
        self.last = (
            std::mem::take(&mut self.rastered),
            std::mem::take(&mut self.filled),
            std::mem::take(&mut self.spent),
        );
        self.last_live = std::mem::take(&mut self.live);
        let frame = self.frame;
        self.canvases.retain(|_, c| frame.saturating_sub(c.used) <= TILE_FRAMES);
        // What a canvas wants or shows stays.
        let Scratch {
            held,
            keys: drop,
            by_age,
        } = &mut self.scratch;
        held.clear();
        drop.clear();
        for c in self.canvases.values() {
            held.extend(c.wants.iter().copied());
        }
        let mut bytes = 0_usize;
        for (k, t) in &self.tiles {
            let wanted = held.contains(k);
            let stale = frame.saturating_sub(t.used) > TILE_FRAMES;
            if !wanted && (stale || t.tex.is_none()) {
                drop.push(*k);
            } else {
                let held = usize::from(t.tex.is_some())
                    .saturating_add(usize::from(t.raster.as_ref().is_some_and(|r| r.tex.is_some())));
                bytes = bytes.saturating_add(TILE_BYTES.saturating_mul(held));
            }
        }
        if bytes > TILE_BUDGET {
            by_age.clear();
            by_age.extend(
                self.tiles
                    .iter()
                    .filter(|(k, _)| !held.contains(k) && !drop.contains(k))
                    .map(|(k, t)| (t.used, *k)),
            );
            by_age.sort_unstable_by_key(|(u, k)| (*u, k.x, k.y, k.scale));
            for &(_, k) in by_age.iter() {
                if bytes <= TILE_BUDGET {
                    break;
                }
                bytes = bytes.saturating_sub(TILE_BYTES);
                drop.push(k);
            }
        }
        for k in drop.drain(..) {
            if let Some(t) = self.tiles.remove(&k) {
                for tex in t.tex.into_iter().chain(t.raster.and_then(|r| r.tex)) {
                    store.release_texture(tex);
                }
            }
        }
        self.anchored.retain(|_, a| frame.saturating_sub(a.used) <= TILE_FRAMES);
    }

    /// The raster cost spent this frame so far, and the tiles finished.
    pub(super) fn spent(&self) -> u32 {
        self.spent
    }

    pub(super) fn done(&self) -> usize {
        self.rastered
    }

    /// Canvas `key`'s revision: what its chunk's signature holds.
    pub(super) fn revision(&self, key: u64) -> u64 {
        self.canvases.get(&key).map_or(0, |c| c.revision)
    }

    /// Whether a canvas wants a tile not current, or shows a tiling that is
    /// not its target: frames must go on to finish raster.
    pub(super) fn pending(&self) -> bool {
        self.canvases.values().any(|c| {
            let v = c.world.as_ref().map_or(0, |w| w.version);
            c.target != c.shown || c.wants.iter().any(|k| !self.tiles.get(k).is_some_and(|t| t.current(v)))
        })
    }

    fn planner(&mut self) -> &mut Planner {
        self.planner.get_or_insert_with(Planner::sized_for_machine)
    }

    /// Tile `key` brought up to date with `world`: kept when no change
    /// since its version reached it, else (and when it has nothing) a
    /// raster of this version asked for; whether one waits to be planned.
    fn freshen(&mut self, key: TileKey, world: &WorldSnapshot, store: &mut crate::chunk::store::Store) -> bool {
        let frame = self.frame;
        let v = world.version;
        let tile = self.tiles.entry(key).or_insert(Tile {
            tex: None,
            version: 0,
            raster: None,
            used: frame,
        });
        tile.used = frame;
        let reach = || Tiling::of(&key).reach((key.x, key.y));
        if tile.tex.is_some() && tile.version != v && world.clean(&reach(), tile.version) {
            tile.version = v;
        }
        if tile.current(v) {
            // A raster of an older version is of no use now.
            if let Some(tex) = tile.raster.take().and_then(|r| r.tex) {
                store.release_texture(tex);
            }
            return false;
        }
        match &mut tile.raster {
            Some(r) if r.version == v => {}
            Some(r) if !matches!(r.work, Work::Asked) && world.clean(&reach(), r.version) => r.version = v,
            _ => {
                if let Some(tex) = tile.raster.take().and_then(|r| r.tex) {
                    store.release_texture(tex);
                }
                tile.raster = Some(Raster {
                    version: v,
                    work: Work::Asked,
                    tex: None,
                });
            }
        }
        tile.raster.as_ref().is_some_and(|r| matches!(r.work, Work::Asked))
    }

    /// Tile `key`'s finished raster takes the shown texture's place.
    fn promote(&mut self, key: TileKey, store: &mut crate::chunk::store::Store) {
        let Some(t) = self.tiles.get_mut(&key) else { return };
        if !t.raster.as_ref().is_some_and(|r| matches!(r.work, Work::Done)) {
            return;
        }
        let Some(r) = t.raster.take() else { return };
        if let Some(old) = std::mem::replace(&mut t.tex, r.tex) {
            store.release_texture(old);
        }
        t.version = r.version;
        for c in self.canvases.values_mut() {
            if c.shows.contains(&key) {
                c.revision = c.revision.wrapping_add(1);
            }
        }
    }

    fn done_raster(&self, key: &TileKey, v: u64) -> bool {
        self.tiles
            .get(key)
            .and_then(|t| t.raster.as_ref())
            .is_some_and(|r| r.version == v && matches!(r.work, Work::Done))
    }
}

/// The phase putting a tiling's grid (scale `k`, turned `angle`) on whole
/// device pixels for camera `cam` over a canvas `size` (logical) whose top
/// left is at device `dev0`, `s` device pixels a logical.
fn phase_of(cam: Camera, k: f64, angle: f64, s: f64, size: (f64, f64), dev0: (f64, f64)) -> (f64, f64) {
    // A world point at tiling coordinate u is at device u + d, the camera
    // turned as the tiling and zoomed `k` (as the tiles are drawn exact).
    let cs = cos_sin(angle);
    let c = turn((cam.center.0 * k, cam.center.1 * k), cs);
    let d = (dev0.0 + size.0 / 2.0 * s - c.0, dev0.1 + size.1 / 2.0 * s - c.1);
    ((-d.0).rem_euclid(1.0), (-d.1).rem_euclid(1.0))
}

/// The scale of the whole-world tiling: the world's extent in one tile.
fn whole_scale(extent: [f64; 4]) -> f64 {
    let side = (extent[2] - extent[0]).max(extent[3] - extent[1]);
    if side.is_finite() && side > 0.0 {
        // A power of two, so it stays the same while the world grows a little.
        (TILE_F / side).log2().floor().exp2()
    } else {
        1.0
    }
}

/// The raster scale for a camera at `ideal` device pixels a unit, the last
/// one `last`, the tilings that exist `have`.
pub(super) fn raster_scale(ideal: f64, last: Option<f64>, settled: bool, have: &[f64]) -> f64 {
    let snap = |k: f64| {
        have.iter()
            .copied()
            .filter(|h| (h / k).max(k / h) <= 1.2)
            .min_by(|a, b| (a / k).max(k / a).total_cmp(&(b / k).max(k / b)))
    };
    if settled {
        return ideal;
    }
    let Some(last) = last else {
        return snap(ideal).unwrap_or(ideal);
    };
    let ratio = ideal / last;
    let step = if ratio > 2.0 {
        last * 2.0
    } else if ratio < 0.5 {
        last / 2.0
    } else {
        return last;
    };
    snap(step).unwrap_or(step)
}

impl GpuRenderer {
    /// Canvas tiles finished and tiles filled from a coarser tiling in the
    /// last frame ended, and the raster cost it spent.
    #[must_use]
    pub fn canvas_raster(&self) -> (usize, usize, u32) {
        self.world.last
    }

    /// Whether canvas raster, or a chunk's readback, is still under way:
    /// a frame drawn now would show (or deliver) more of it.
    #[must_use]
    pub fn raster_pending(&self) -> bool {
        self.world.pending() || self.data.reads_out()
    }

    /// Objects drawn live over tiles in the last frame ended (lifted ones
    /// and changes tiles were rastered again for), and the device pixels
    /// drawn live.
    #[must_use]
    pub fn canvas_live(&self) -> (usize, u64) {
        self.world.last_live
    }

    /// Wait for the raster workers before each frame's raster (headless
    /// runs, tests): a frame then sees every plan the last one asked for.
    pub fn set_raster_blocking(&mut self, on: bool) {
        self.world.blocking = on;
    }

    /// Spend at most `budget` a frame on raster (tests: split tiles finely).
    #[cfg(test)]
    pub(crate) fn set_raster_budget(&mut self, budget: u32) {
        self.world.budget = Some(budget);
    }

    /// Raster what canvases want, most urgent first, within the frame's
    /// budget: plans taken from the workers, steps drawn into tile passes.
    pub(super) fn pump_raster(&mut self, text: &mut Glyphs) {
        // Once a frame: a capture of the frame drawn spends no second budget.
        if self.world.canvases.is_empty() || self.world.pumped == Some(self.world.frame) {
            return;
        }
        self.world.pumped = Some(self.world.frame);
        if self.world.blocking
            && let Some(p) = &self.world.planner
        {
            let t0 = std::time::Instant::now();
            p.wait_idle(HEADLESS_WAIT);
            self.drawn.raster_wait = self.drawn.raster_wait.saturating_add(t0.elapsed());
        }
        let done = self.world.planner.as_ref().map(Planner::take).unwrap_or_default();
        for (key, version, steps) in done {
            if let Some(r) = self.world.tiles.get_mut(&key).and_then(|t| t.raster.as_mut())
                && r.version == version
                && matches!(r.work, Work::Asked)
            {
                r.work = Work::Planned { steps, next: 0 };
            }
        }
        let mut order = std::mem::take(&mut self.world.scratch.keys);
        order.clear();
        order.extend(self.world.canvases.values().flat_map(|c| c.wants.iter().copied()));
        let total = self.world.budget.unwrap_or(RASTER_BUDGET);
        for &key in &order {
            if self.world.spent >= total {
                break;
            }
            let planned = self
                .world
                .tiles
                .get(&key)
                .and_then(|t| t.raster.as_ref())
                .is_some_and(|r| matches!(r.work, Work::Planned { .. }));
            if planned {
                let budget = total.saturating_sub(self.world.spent);
                self.raster_part(key, budget, text);
            }
        }
        self.world.scratch.keys = order;
        self.activate();
    }

    /// Show each canvas' target once its visible tiles are all current.
    fn activate(&mut self) {
        let tiles = &self.world.tiles;
        for c in self.world.canvases.values_mut() {
            let v = c.world.as_ref().map_or(0, |w| w.version);
            if c.target != c.shown
                && c.target.is_some()
                && c.visible.iter().all(|k| tiles.get(k).is_some_and(|t| t.current(v)))
            {
                c.shown = c.target;
                c.revision = c.revision.wrapping_add(1);
            }
        }
    }

    /// Rasterize planned tile `key`'s next steps worth `budget` (one at
    /// least), as one pass into its raster's texture; a tile with nothing
    /// to show yet shows it once done.
    fn raster_part(&mut self, key: TileKey, budget: u32, text: &mut Glyphs) {
        let Some(r) = self.world.tiles.get_mut(&key).and_then(|t| t.raster.as_mut()) else {
            return;
        };
        let Work::Planned { steps, next } = std::mem::replace(&mut r.work, Work::Asked) else {
            return;
        };
        let mut slot = r.tex;
        let Some(tex) = self.data.store.ensure_texture(
            &self.device,
            &mut slot,
            (TILE, TILE),
            self.format,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ) else {
            return;
        };
        let first = next == 0;
        let mut at = next;
        let mut spent = 0_u32;
        let mut ops = Vec::new();
        let tile_bounds = Bounds {
            x: 0.0,
            y: 0.0,
            width: TILE_F32,
            height: TILE_F32,
        };
        let full = [0.0, 0.0, TILE_F32, TILE_F32];
        let seed = foldhash::fast::FixedState::with_seed(0x711e).hash_one(key);
        self.paths.once = true;
        while let Some(step) = steps.get(at) {
            let cost = step.cost();
            if spent > 0 && spent.saturating_add(cost) > budget {
                break;
            }
            spent = spent.saturating_add(cost);
            let place = Stage {
                bounds: tile_bounds,
                s: 1.0,
                off: (0.0, 0.0),
                origin: (0.0, 0.0),
                visible: full,
                clip: 0,
                local: 0,
                alpha: 1.0,
                seed,
            };
            self.step_ops(&mut ops, step, &place, at, text);
            at = at.saturating_add(1);
        }
        self.paths.once = false;
        self.world.spent = self.world.spent.saturating_add(spent);
        self.world.jobs.push((tex, ops, first));
        let done = at >= steps.len();
        let frame = self.world.frame;
        let mut show_now = false;
        if let Some(t) = self.world.tiles.get_mut(&key) {
            t.used = frame;
            show_now = t.tex.is_none();
            if let Some(r) = t.raster.as_mut() {
                r.tex = Some(tex);
                r.work = if done {
                    Work::Done
                } else {
                    Work::Planned { steps, next: at }
                };
            }
        }
        if done {
            self.world.rastered = self.world.rastered.saturating_add(1);
            if show_now {
                self.world.promote(key, &mut self.data.store);
            }
        }
    }

    /// The draws of step `step` (the `nth` of its plan) placed by `place`.
    fn step_ops(&mut self, ops: &mut Vec<Op>, step: &Step, place: &Stage, nth: usize, text: &mut Glyphs) {
        let (ox, oy) = place.origin;
        match step {
            Step::Boxes(data) => {
                let count = u32::try_from(data.len() / crate::canvas::boxes::BOX).unwrap_or(u32::MAX);
                let key = foldhash::fast::FixedState::with_seed(0xb0c5).hash_one((place.seed, nth));
                let item = DataItem::new::<crate::canvas::boxes::Boxes>(
                    key,
                    place.bounds,
                    place.bounds,
                    &crate::canvas::boxes::words(count),
                    vec![crate::chunk::Source::Bytes(crate::chunk::Bytes::new(
                        bytemuck::cast_slice::<f32, u8>(data).to_vec(),
                    ))],
                );
                if let Some(at) = self.data.prepare(
                    &self.device,
                    &self.queue,
                    &item,
                    place.s,
                    place.off,
                    place.clip,
                    place.alpha,
                ) {
                    super::push_op(ops, Op::Data(at));
                }
            }
            Step::Path { path, m, .. } => {
                let Some(style) = step.style() else { return };
                let m = [m[0], m[1], m[2], m[3], m[4] + ox, m[5] + oy];
                let style = style.clone();
                self.push_path(
                    ops,
                    path,
                    &style,
                    m,
                    place.visible,
                    place.clip,
                    place.local,
                    place.alpha,
                );
            }
            Step::Glyph { key, m, color } => {
                let m = [m[0], m[1], m[2], m[3], m[4] + ox, m[5] + oy];
                self.push_slug(ops, text, key, m, *color, place.alpha, place.clip, place.local);
            }
        }
    }

    /// The draws of canvas `v` moved by `off` (device pixels), at `s`
    /// device pixels a logical one, under device clip `clip`.
    #[expect(clippy::too_many_arguments, reason = "an item and how it is drawn")]
    pub(super) fn world_ops(
        &mut self,
        ops: &mut Vec<Op>,
        v: &Arc<WorldView>,
        s: f32,
        off: (f32, f32),
        visible: [f32; 4],
        clip: u32,
        alpha: f32,
        text: &mut Glyphs,
    ) {
        let b = v.bounds;
        let dev = [
            b.x.mul_add(s, off.0),
            b.y.mul_add(s, off.1),
            b.right().mul_add(s, off.0),
            b.bottom().mul_add(s, off.1),
        ];
        let shown = [
            visible[0].max(dev[0]),
            visible[1].max(dev[1]),
            visible[2].min(dev[2]),
            visible[3].min(dev[3]),
        ];
        if shown[2] <= shown[0] || shown[3] <= shown[1] {
            return;
        }
        let at = Stage {
            bounds: b,
            s,
            off,
            origin: (dev[0], dev[1]),
            visible: shown,
            clip,
            local: 0,
            alpha,
            seed: v.key,
        };
        if v.tiled {
            self.tiled_ops(ops, v, &at, dev, text);
        } else {
            // Exactly the canvas' box: a path reaching past it is cut there.
            let whole = [dev[0], dev[1], dev[2], dev[3]].map(f64::from);
            self.live_ops(ops, v, &at, dev, &[whole], text);
        }
    }

    /// What of canvas `v` falls in each of device rectangles `areas`
    /// (disjoint), drawn straight: every object meeting it, lifted ones
    /// included, each through its own camera-relative matrix and cut to
    /// the rectangle.
    fn live_ops(
        &mut self,
        ops: &mut Vec<Op>,
        v: &WorldView,
        place: &Stage,
        dev: [f32; 4],
        areas: &[[f64; 4]],
        text: &mut Glyphs,
    ) {
        let cam = v.camera;
        let s = f64::from(place.s);
        let size = (f64::from(v.bounds.width), f64::from(v.bounds.height));
        let d0 = (f64::from(dev[0]), f64::from(dev[1]));
        let full = View {
            center: cam.center,
            k: cam.zoom * s,
            cs: cam.turn(),
            origin: (size.0 / 2.0 * s, size.1 / 2.0 * s),
            cut: [0.0, 0.0, dev[2] - dev[0], dev[3] - dev[1]],
        };
        let world = Arc::clone(&v.world);
        let frame = self.world.frame;
        let f = crate::convert::f64_to_f32;
        let mut steps = Vec::new();
        for (n, area) in areas.iter().enumerate() {
            let view = View {
                cut: [
                    f(area[0] - d0.0),
                    f(area[1] - d0.1),
                    f(area[2] - d0.0),
                    f(area[3] - d0.1),
                ],
                ..full
            };
            steps.clear();
            let anchored = &mut self.world.anchored;
            // Huge objects are cut about the whole view (and kept), whatever
            // part of it this is.
            plan_live(
                &world,
                &view,
                &mut |i, o| reanchored(anchored, frame, world.id, i, o, &cam, size, &full),
                &mut steps,
            );
            let r = [area[0], area[1], area[2], area[3]].map(f);
            let local = clip_entry_logical(&mut self.fr.pclips, r);
            let visible = [
                r[0].max(place.visible[0]),
                r[1].max(place.visible[1]),
                r[2].min(place.visible[2]),
                r[3].min(place.visible[3]),
            ];
            let at = Stage {
                origin: (dev[0], dev[1]),
                visible,
                local,
                seed: foldhash::fast::FixedState::with_seed(0x11fe).hash_one((place.seed, n)),
                ..*place
            };
            for (nth, step) in steps.iter().enumerate() {
                // A moving camera's paths, and a lifted object's, are this
                // frame's only.
                let lifted = match step {
                    Step::Path { object, .. } => world.live.iter().any(|(_, o)| Arc::ptr_eq(o, object)),
                    Step::Boxes(_) | Step::Glyph { .. } => false,
                };
                self.paths.once = !cam.settled || lifted;
                if matches!(step, Step::Path { .. }) {
                    self.world.live.0 = self.world.live.0.saturating_add(1);
                }
                self.step_ops(ops, step, &at, nth, text);
            }
            self.paths.once = false;
            let px = (area[2] - area[0]).max(0.0) * (area[3] - area[1]).max(0.0);
            self.world.live.1 = self.world.live.1.saturating_add(crate::convert::f64_to_u64(px));
        }
    }

    /// The canvas from tiles: say what it wants, show what is ready, and
    /// draw live what tiles cannot show yet (lifted objects, changes being
    /// rastered).
    #[expect(clippy::too_many_lines, reason = "a tiled frame's decisions, in order")]
    fn tiled_ops(&mut self, ops: &mut Vec<Op>, v: &Arc<WorldView>, place: &Stage, dev: [f32; 4], text: &mut Glyphs) {
        let cam = v.camera;
        let world = Arc::clone(&v.world);
        let ver = world.version;
        let extent = world.extent();
        let s = f64::from(place.s);
        let size = (f64::from(v.bounds.width), f64::from(v.bounds.height));
        let frame = self.world.frame;
        let canvas = self.world.canvases.entry(v.key).or_insert_with(|| Canvas {
            target: None,
            shown: None,
            visible: Vec::new(),
            wants: Vec::new(),
            shows: Vec::new(),
            world: None,
            revision: 0,
            last: None,
            used: frame,
        });
        canvas.used = frame;
        let last_cam = canvas.last.replace(cam);
        let last_target = canvas.target;
        if canvas.world.as_ref().is_some_and(|w| w.id != world.id) {
            canvas.target = None;
            canvas.shown = None;
        }
        canvas.world = Some(Arc::clone(&world));
        // The target tiling.
        let turn_now = match last_target {
            Some(t) if !cam.settled => t.turn,
            _ => cam.rotation,
        };
        // The tilings there are of this world and turn: scale and phase.
        let have: Vec<(f64, (f64, f64))> = {
            let mut h: Vec<(f64, (f64, f64))> = self
                .world
                .tiles
                .keys()
                .filter(|k| k.world == world.id && k.turn == turn_now.to_bits())
                .map(|k| {
                    (
                        f64::from_bits(k.scale),
                        (f64::from_bits(k.phase[0]), f64::from_bits(k.phase[1])),
                    )
                })
                .collect();
            h.sort_by(|a, b| {
                a.0.total_cmp(&b.0)
                    .then(a.1.0.total_cmp(&b.1.0))
                    .then(a.1.1.total_cmp(&b.1.1))
            });
            h.dedup();
            h
        };
        let scales: Vec<f64> = have.iter().map(|h| h.0).collect();
        let last_tiling = last_target.filter(|t| t.turn.to_bits() == turn_now.to_bits());
        let k = raster_scale(cam.zoom * s, last_tiling.map(|t| t.scale), cam.settled, &scales);
        // Settled, the grid falls on whole pixels as the camera has it now;
        // moving, a tiling keeps the phase it has.
        let exact = phase_of(cam, k, turn_now, s, size, (f64::from(dev[0]), f64::from(dev[1])));
        let phase = if cam.settled {
            exact
        } else {
            last_tiling
                .filter(|t| t.scale.to_bits() == k.to_bits())
                .map(|t| t.phase)
                .or_else(|| have.iter().find(|h| h.0.to_bits() == k.to_bits()).map(|h| h.1))
                .unwrap_or(exact)
        };
        let target = Tiling {
            scale: k,
            turn: turn_now,
            phase,
        };
        let whole = Tiling {
            scale: whole_scale(extent),
            turn: 0.0,
            phase: (0.0, 0.0),
        };
        // The view's corners in the world, and ahead along the pan.
        let corners = [(0.0, 0.0), (size.0, 0.0), (0.0, size.1), size].map(|p| cam.to_world(p, size));
        let pan = last_cam.map_or((0.0, 0.0), |l| {
            (
                (cam.center.0 - l.center.0) * SKEW_FRAMES,
                (cam.center.1 - l.center.1) * SKEW_FRAMES,
            )
        });
        let ahead: Vec<(f64, f64)> = corners.iter().flat_map(|c| [*c, (c.0 + pan.0, c.1 + pan.1)]).collect();
        let centre = cam.to_world((size.0 / 2.0, size.1 / 2.0), size);
        let by_distance = |t: &Tiling, r: (i64, i64, i64, i64)| {
            let cs = cos_sin(t.turn);
            let u = turn((centre.0 * t.scale, centre.1 * t.scale), cs);
            let u = (u.0 - t.phase.0, u.1 - t.phase.1);
            let mut list: Vec<(i64, i64)> = (r.1..=r.3).flat_map(|y| (r.0..=r.2).map(move |x| (x, y))).collect();
            let d = |p: &(i64, i64)| {
                let cx = (crate::convert::i64_to_f64(p.0) + 0.5) * TILE_F - u.0;
                let cy = (crate::convert::i64_to_f64(p.1) + 0.5) * TILE_F - u.1;
                cx.hypot(cy)
            };
            list.sort_by(|a, b| d(a).total_cmp(&d(b)).then(a.cmp(b)));
            list
        };
        let id = world.id;
        let whole_tiles: Vec<TileKey> = by_distance(&whole, whole.covering(&corners, 0, &extent))
            .into_iter()
            .map(|p| whole.key(id, p))
            .collect();
        let visible: Vec<TileKey> = by_distance(&target, target.covering(&corners, 0, &extent))
            .into_iter()
            .map(|p| target.key(id, p))
            .collect();
        let mut wants = whole_tiles.clone();
        for key in &visible {
            if !wants.contains(key) {
                wants.push(*key);
            }
        }
        let seen: foldhash::HashSet<TileKey> = wants.iter().copied().collect();
        let ring: Vec<TileKey> = by_distance(&target, target.covering(&ahead, PREFETCH, &extent))
            .into_iter()
            .map(|p| target.key(id, p))
            .filter(|k| !seen.contains(k))
            .collect();
        wants.extend(ring);
        // Each wanted tile kept where no change reached it; a raster of
        // this version asked for the rest.
        let mut asked = Vec::new();
        for key in &wants {
            if self.world.freshen(*key, &world, &mut self.data.store) {
                let tiling = if whole_tiles.contains(key) { whole } else { target };
                asked.push(Request {
                    key: *key,
                    world: Arc::clone(&world),
                    view: tiling.view((key.x, key.y)),
                });
            }
        }
        let mut shown = None;
        if let Some(canvas) = self.world.canvases.get_mut(&v.key) {
            canvas.target = Some(target);
            canvas.visible.clone_from(&visible);
            canvas.wants.clone_from(&wants);
            shown = canvas.shown;
        }
        let has = |tiles: &foldhash::HashMap<TileKey, Tile>, k: &TileKey| tiles.get(k).is_some_and(|t| t.tex.is_some());
        // Nothing to show at all: the whole-world tiles now, whatever the cost.
        let any_ready = whole_tiles.iter().chain(&visible).any(|k| has(&self.world.tiles, k))
            || shown.is_some_and(|t| {
                by_distance(&t, t.covering(&corners, 0, &extent))
                    .into_iter()
                    .any(|p| has(&self.world.tiles, &t.key(id, p)))
            });
        if !any_ready {
            for key in &whole_tiles {
                let steps = plan_tile(&world, &whole.view((key.x, key.y)));
                if let Some(r) = self.world.tiles.get_mut(key).and_then(|t| t.raster.as_mut()) {
                    r.work = Work::Planned { steps, next: 0 };
                }
                self.raster_part(*key, u32::MAX, text);
            }
        }
        asked.retain(|r| {
            self.world
                .tiles
                .get(&r.key)
                .and_then(|t| t.raster.as_ref())
                .is_some_and(|x| matches!(x.work, Work::Asked))
        });
        self.world.planner().want(asked);
        self.activate();
        let shown = self.world.canvases.get(&v.key).and_then(|c| c.shown);
        // What shows: the shown tiling, else the target's tiles that have
        // something; the whole-world tiling under it where it lacks a tile.
        let top = shown.unwrap_or(target);
        let top_keys: Vec<TileKey> = by_distance(&top, top.covering(&corners, 0, &extent))
            .into_iter()
            .map(|p| top.key(id, p))
            .collect();
        // The shown tiling's tiles a change did not reach are current too.
        for key in &top_keys {
            if let Some(t) = self.world.tiles.get_mut(key)
                && t.tex.is_some()
                && t.version != ver
                && world.clean(&Tiling::of(key).reach((key.x, key.y)), t.version)
            {
                t.version = ver;
            }
        }
        if let Some(c) = self.world.canvases.get_mut(&v.key) {
            c.shows = whole_tiles.iter().chain(&top_keys).copied().collect();
        }
        let gaps = top_keys.iter().any(|k| !has(&self.world.tiles, k));
        let mut drawn: Vec<TileKey> = Vec::new();
        if gaps && top != whole {
            drawn.extend(whole_tiles.iter().copied().filter(|k| has(&self.world.tiles, k)));
        }
        let under = drawn.len();
        drawn.extend(top_keys.iter().copied().filter(|k| has(&self.world.tiles, k)));
        // Finished rasters of tiles not drawn take their place at once.
        let off_screen: Vec<TileKey> = wants.iter().copied().filter(|k| !drawn.contains(k)).collect();
        for key in off_screen {
            self.world.promote(key, &mut self.data.store);
        }
        // A change being rastered again shows drawn live, over the tiles
        // still of the version before, as long as that is little to draw;
        // else the tiles of before show until every one is rastered anew,
        // then all at once (Chromium's activation).
        let (mut holes, small) = self.live_holes(v, place, dev, &drawn);
        let stale: Vec<TileKey> = drawn
            .iter()
            .copied()
            .filter(|k| !self.world.tiles.get(k).is_some_and(|t| t.current(ver)))
            .collect();
        if !stale.is_empty() && (small || stale.iter().all(|k| self.world.done_raster(k, ver))) {
            for key in stale {
                self.world.promote(key, &mut self.data.store);
            }
            holes = self.live_holes(v, place, dev, &drawn).0;
        }
        self.world.filled = self.world.filled.saturating_add(under);
        for (n, key) in drawn.into_iter().enumerate() {
            if let Some(t) = self.world.tiles.get_mut(&key) {
                t.used = frame;
            }
            let tiling = if n < under { whole } else { top };
            self.draw_tile(ops, v, key, &tiling, place, dev, &holes);
        }
        if !holes.is_empty() {
            self.live_ops(ops, v, place, dev, &holes, text);
        }
    }

    /// Where canvas `v` draws live over its tiles (device rectangles,
    /// disjoint, whole pixels): its lifted objects, and the changes tiles
    /// `drawn` do not show yet when they are little to draw; and whether
    /// those changes are (else they are left to the tiles).
    fn live_holes(&self, v: &WorldView, place: &Stage, dev: [f32; 4], drawn: &[TileKey]) -> (Vec<[f64; 4]>, bool) {
        let world = &v.world;
        let cam = v.camera;
        let s = f64::from(place.s);
        let size = (f64::from(v.bounds.width), f64::from(v.bounds.height));
        let d0 = (f64::from(dev[0]), f64::from(dev[1]));
        let shown = place.visible.map(f64::from);
        let area = cam.visible(size);
        // A world box on the device, grown by a pixel and snapped out,
        // cut to what shows.
        let to_dev = |b: &Aabb| -> Option<[f64; 4]> {
            let mut r = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for c in [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])] {
                let p = cam.to_screen(c, size);
                let p = (d0.0 + p.0 * s, d0.1 + p.1 * s);
                r = [r[0].min(p.0), r[1].min(p.1), r[2].max(p.0), r[3].max(p.1)];
            }
            let r = [
                (r[0] - 1.0).floor().max(shown[0].floor()),
                (r[1] - 1.0).floor().max(shown[1].floor()),
                (r[2] + 1.0).ceil().min(shown[2].ceil()),
                (r[3] + 1.0).ceil().min(shown[3].ceil()),
            ];
            (r[2] > r[0] && r[3] > r[1]).then_some(r)
        };
        let live: Vec<[f64; 4]> = world
            .live
            .iter()
            .map(|(_, o)| o.ink())
            .filter(|b| aabb::overlaps(b, &area))
            .filter_map(|b| to_dev(&b))
            .collect();
        let stale_from = drawn
            .iter()
            .filter_map(|k| self.world.tiles.get(k))
            .filter(|t| !t.current(world.version))
            .map(|t| t.version)
            .min();
        let Some(from) = stale_from else {
            return (disjoint(live), true);
        };
        if from < world.damage_from {
            return (disjoint(live), false);
        }
        let mut all = live.clone();
        all.extend(
            world
                .damaged_since(from)
                .filter(|b| aabb::overlaps(b, &area))
                .filter_map(to_dev),
        );
        let all = disjoint(all);
        let px: f64 = all.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum();
        let canvas_px = (shown[2] - shown[0]).max(0.0) * (shown[3] - shown[1]).max(0.0);
        let mut objects = 0_usize;
        for r in &all {
            let corners = [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])]
                .map(|p| cam.to_world(((p.0 - d0.0) / s, (p.1 - d0.1) / s), size));
            let b = corners.iter().fold(
                [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                |b, p| [b[0].min(p.0), b[1].min(p.1), b[2].max(p.0), b[3].max(p.1)],
            );
            objects = objects.saturating_add(world.base.count(&b, LIVE_OBJECTS.saturating_add(1)));
        }
        if px <= canvas_px * LIVE_SHARE && objects <= LIVE_OBJECTS {
            (all, true)
        } else {
            (disjoint(live), false)
        }
    }

    /// Tile `key` of `tiling` where it lands in the canvas: its corners
    /// on whole device pixels when the tiling is turned as the camera is,
    /// so neighbours meet without a seam; nothing of it inside `holes`
    /// (device rectangles drawn live).
    #[expect(clippy::too_many_arguments, reason = "a tile and where it is drawn")]
    fn draw_tile(
        &mut self,
        ops: &mut Vec<Op>,
        v: &WorldView,
        key: TileKey,
        tiling: &Tiling,
        place: &Stage,
        dev: [f32; 4],
        holes: &[[f64; 4]],
    ) {
        let Some(tex) = self.world.tiles.get(&key).and_then(|t| t.tex) else {
            return;
        };
        let cam = v.camera;
        let s = f64::from(place.s);
        let size = (f64::from(v.bounds.width), f64::from(v.bounds.height));
        let view = tiling.view((key.x, key.y));
        let straight = tiling.turn.to_bits() == cam.rotation.to_bits();
        let to_dev = |t: (f64, f64)| {
            let p = cam.to_screen(view.to_world(t), size);
            let d = (f64::from(dev[0]) + p.0 * s, f64::from(dev[1]) + p.1 * s);
            if straight { (d.0.round(), d.1.round()) } else { d }
        };
        let quad = [(0.0, 0.0), (TILE_F, 0.0), (0.0, TILE_F), (TILE_F, TILE_F)].map(to_dev);
        // Cut to the canvas' whole pixels.
        let cut = [
            f64::from(dev[0]).floor(),
            f64::from(dev[1]).floor(),
            f64::from(dev[2]).ceil(),
            f64::from(dev[3]).ceil(),
        ];
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for q in &quad {
            b = [b[0].min(q.0), b[1].min(q.1), b[2].max(q.0), b[3].max(q.1)];
        }
        let b = [b[0].max(cut[0]), b[1].max(cut[1]), b[2].min(cut[2]), b[3].min(cut[3])];
        if b[2] <= b[0] || b[3] <= b[1] {
            return;
        }
        let f = crate::convert::f64_to_f32;
        for (n, piece) in subtract(b, holes).into_iter().enumerate() {
            let rel = |p: (f64, f64)| [f(p.0 - piece[0]), f(p.1 - piece[1])];
            let corners = quad.map(rel);
            let cut_rel = [0.0, 0.0, f(piece[2] - piece[0]), f(piece[3] - piece[1])];
            let bounds = Bounds {
                x: (f(piece[0]) - place.off.0) / place.s,
                y: (f(piece[1]) - place.off.1) / place.s,
                width: f(piece[2] - piece[0]) / place.s,
                height: f(piece[3] - piece[1]) / place.s,
            };
            let hash =
                foldhash::fast::FixedState::with_seed(0x7e1e).hash_one((v.key, key.scale, key.turn, key.x, key.y, n));
            let item = DataItem::new::<crate::canvas::tile::Tiles>(
                hash,
                bounds,
                bounds,
                &crate::canvas::tile::words(tex, corners, cut_rel),
                Vec::new(),
            );
            if let Some(at) = self.data.prepare(
                &self.device,
                &self.queue,
                &item,
                place.s,
                place.off,
                place.clip,
                place.alpha,
            ) {
                super::push_op(ops, Op::Data(at));
            }
        }
    }
}

/// Objects past which what changed is left to the tiles rather than drawn
/// live while they are rastered again.
const LIVE_OBJECTS: usize = 2048;
/// The share of a canvas past which the same holds.
const LIVE_SHARE: f64 = 0.25;
/// Rectangles past which live areas are drawn as the one box around them.
const LIVE_AREAS: usize = 16;

/// Rectangles `rects` (x0, y0, x1, y1) made disjoint: those that meet
/// joined into the box around them, until none meet.
fn disjoint(mut rects: Vec<[f64; 4]>) -> Vec<[f64; 4]> {
    let meet = |a: &[f64; 4], b: &[f64; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
    'again: loop {
        for i in 0..rects.len() {
            for j in i.saturating_add(1)..rects.len() {
                if let (Some(a), Some(b)) = (rects.get(i).copied(), rects.get(j).copied())
                    && meet(&a, &b)
                {
                    rects.swap_remove(j);
                    if let Some(r) = rects.get_mut(i) {
                        *r = [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
                    }
                    continue 'again;
                }
            }
        }
        break;
    }
    if rects.len() > LIVE_AREAS {
        let all = rects.iter().fold(
            [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
            |b, r| [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])],
        );
        return vec![all];
    }
    // In reading order, so a frame's draws are the same every time.
    rects.sort_by(|a, b| a[1].total_cmp(&b[1]).then(a[0].total_cmp(&b[0])));
    rects
}

/// Rectangle `r` less `holes` (disjoint), as rectangles: each hole cuts
/// what is left into the bands above and below it and the parts beside it.
fn subtract(r: [f64; 4], holes: &[[f64; 4]]) -> Vec<[f64; 4]> {
    let mut parts = vec![r];
    for h in holes {
        let mut next = Vec::with_capacity(parts.len().saturating_add(3));
        for p in parts {
            if !(h[0] < p[2] && p[0] < h[2] && h[1] < p[3] && p[1] < h[3]) {
                next.push(p);
                continue;
            }
            let (y0, y1) = (h[1].max(p[1]), h[3].min(p[3]));
            for piece in [
                [p[0], p[1], p[2], y0],
                [p[0], y1, p[2], p[3]],
                [p[0], y0, h[0].max(p[0]), y1],
                [h[2].min(p[2]), y0, p[2], y1],
            ] {
                if piece[2] > piece[0] && piece[3] > piece[1] {
                    next.push(piece);
                }
            }
        }
        parts = next;
    }
    parts
}

/// Where a plan's steps are drawn: the item's bounds and how they map
/// (for boxes), the device origin the steps' coordinates start from, what
/// is visible, the clips and fade, a seed for the boxes' keys.
#[derive(Clone, Copy)]
struct Stage {
    bounds: Bounds,
    s: f32,
    off: (f32, f32),
    origin: (f32, f32),
    visible: [f32; 4],
    clip: u32,
    local: u32,
    alpha: f32,
    seed: u64,
}

/// Huge object `i` cut to around the view, about a point near the camera:
/// on a grid of a quarter of the view, so it is made again only when the
/// camera has moved that far or zoomed, or the object changed.
#[expect(clippy::too_many_arguments, reason = "an object, the camera and the cache")]
fn reanchored(
    anchored: &mut foldhash::HashMap<(u64, u32), Anchored>,
    frame: u64,
    world: u64,
    i: u32,
    o: &Arc<crate::canvas::Placed>,
    cam: &Camera,
    size: (f64, f64),
    view: &View,
) -> Option<(Path, (f64, f64))> {
    let zoom = cam.zoom.max(f64::MIN_POSITIVE);
    let side = size.0.max(size.1).max(1.0) / zoom;
    let grid = side / 4.0;
    let cell = (
        crate::convert::f64_to_i64((cam.center.0 / grid).floor()),
        crate::convert::f64_to_i64((cam.center.1 / grid).floor()),
    );
    let anchor = (
        crate::convert::i64_to_f64(cell.0) * grid,
        crate::convert::i64_to_f64(cell.1) * grid,
    );
    if let Some(a) = anchored.get_mut(&(world, i))
        && a.cell == cell
        && a.zoom == zoom.to_bits()
        && Arc::ptr_eq(&a.object, o)
    {
        a.used = frame;
        return Some((a.path.clone(), anchor));
    }
    // The view from anywhere in the cell: grown by a view each way.
    let grown = View {
        cut: [
            view.cut[0] - view.cut[2],
            view.cut[1] - view.cut[3],
            view.cut[2] * 2.0,
            view.cut[3] * 2.0,
        ],
        ..*view
    };
    let (path, anchor) = cut_about(o, &grown, anchor)?;
    anchored.insert(
        (world, i),
        Anchored {
            object: Arc::clone(o),
            cell,
            zoom: zoom.to_bits(),
            path: path.clone(),
            used: frame,
        },
    );
    Some((path, anchor))
}

/// A clip of device rectangle `r`, as a path's local clip.
fn clip_entry_logical(clips: &mut Vec<f32>, r: [f32; 4]) -> u32 {
    super::frame::clip_entry(
        clips,
        Clip {
            bounds: Bounds {
                x: r[0],
                y: r[1],
                width: r[2] - r[0],
                height: r[3] - r[1],
            },
            radius: crate::style::Corners::default(),
            curve: crate::shape::ROUND,
        },
        1.0,
        None,
    )
}

use std::hash::BuildHasher;

#[cfg(test)]
mod tests {
    use super::raster_scale;

    #[test]
    fn a_pinch_steps_at_most_twice_and_snaps_within_a_fifth() {
        // Zooming in fast from 1: 2, 4, 8, … never more than twice the last.
        let mut have = vec![1.0];
        let mut last = 1.0;
        for _ in 0..5 {
            let k = raster_scale(100.0, Some(last), false, &have);
            assert!(k <= last * 2.0 + 1e-9 && k > last);
            have.push(k);
            last = k;
        }
        assert_eq!(last, 32.0);
        // Within 2× the last it is kept; an existing tiling within 1.2× is taken.
        assert_eq!(raster_scale(45.0, Some(32.0), false, &have), 32.0);
        assert_eq!(raster_scale(70.0, Some(32.0), false, &[32.0, 60.0]), 60.0);
        // Settled, the exact zoom: sharp, whatever tilings there are.
        assert_eq!(raster_scale(45.0, Some(32.0), true, &have), 45.0);
        assert_eq!(raster_scale(35.0, Some(32.0), true, &have), 35.0);
    }
}
