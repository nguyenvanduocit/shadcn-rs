//! The canvas: deep zoom without jitter, tiles never blank (ADR-0017).

#![expect(clippy::as_conversions, reason = "test values are small and known")]

use super::{Camera, WorldPath, WorldView, world};
use crate::color::Color;
use crate::figure::PathStyle;
use crate::geometry::Bounds;
use crate::gpu::GpuRenderer;
use crate::image::Pixels;
use crate::scene::{Gradient, Item, Renderer, Scene};
use crate::text::Glyphs;
use std::sync::Arc;

fn gpu() -> GpuRenderer {
    let mut g = GpuRenderer::headless().expect("a GPU (CI paints on lavapipe)");
    // Each frame sees the plans the last asked for: runs repeat.
    g.set_raster_blocking(true);
    g
}

/// The most a frame's raster may spend: its budget, and the one step that
/// starts past it (a step is never split).
fn frame_bound() -> u32 {
    crate::gpu::RASTER_BUDGET + 8 + 64
}

fn black() -> PathStyle {
    PathStyle::fill(Gradient::Solid(Color::BLACK))
}

fn shoot(gpu: &mut GpuRenderer, view: WorldView, size: (u32, u32)) -> Pixels {
    let mut scene = Scene::new(size, 1.0, Some(Color::WHITE));
    scene.push(vec![Item::World(Arc::new(view))]);
    let shot = gpu.capture(&scene, None, &mut Glyphs::for_os()).expect("read back");
    gpu.end_frame();
    shot
}

/// Ink (0 white to 1 black) of each pixel.
fn ink(p: &Pixels) -> Vec<Vec<f64>> {
    (0..p.height())
        .map(|y| {
            (0..p.width())
                .map(|x| 1.0 - f64::from(p.pixel(x, y).map_or(255, |c| c[0])) / 255.0)
                .collect()
        })
        .collect()
}

/// The ink's centre and its sum.
fn centroid(p: &Pixels) -> (f64, f64, f64) {
    let (mut sx, mut sy, mut sum) = (0.0, 0.0, 0.0);
    for (y, row) in ink(p).iter().enumerate() {
        for (x, v) in row.iter().enumerate() {
            sx += *v * (x as f64 + 0.5);
            sy += *v * (y as f64 + 0.5);
            sum += *v;
        }
    }
    (sx / sum, sy / sum, sum)
}

/// The ink in column band `x0..x1`, summed down and divided by the height:
/// where a vertical edge stands (its covered width).
fn covered(p: &Pixels, x0: usize, x1: usize) -> f64 {
    let rows = ink(p);
    let h = rows.len() as f64;
    rows.iter()
        .map(|r| r.get(x0..x1).map_or(0.0, |s| s.iter().sum::<f64>()))
        .sum::<f64>()
        / h
}

#[test]
fn a_billionfold_zoom_moves_by_the_subpixel_it_was_panned() {
    // A square of 10⁻⁷ units far from the origin, 100 px at zoom 10⁹, and
    // an object 10⁶ units wide whose edge crosses the same view.
    let w = world();
    let (x, y) = (1_000.000_000_03, -2_000.000_000_07);
    w.add(WorldPath::rect(x, y, 1e-7, 1e-7), black());
    w.add(WorldPath::rect(x + 1.5e-7, -1e6, 1e6, 2e6), black());
    let size = (400_u32, 200_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 200.0,
    };
    let mut gpu = gpu();
    let mut last: Option<(f64, f64, f64, f64)> = None;
    for step in 0..6_i32 {
        // Each step pans a quarter of a pixel: 2.5·10⁻¹⁰ units.
        let pan = f64::from(step) * 0.25e-9;
        let camera = Camera {
            center: (x + 0.5e-7 + 1e-7 - pan, y + 0.5e-7),
            zoom: 1e9,
            rotation: 0.0,
            settled: true,
        };
        let shot = shoot(
            &mut gpu,
            WorldView {
                key: 1,
                bounds,
                world: w.snapshot(),
                camera,
                tiled: false,
            },
            size,
        );
        // The square spans 50..150 px plus the pan; the huge object starts
        // at 200 px plus the pan and runs past the right edge.
        let (cx, cy, sum) = centroid(&shot.crop(0, 0, 180, 200).unwrap());
        let edge = 400.0 - covered(&shot, 180, 400);
        assert!((sum - 10_000.0).abs() < 50.0, "step {step}: the square's ink {sum}");
        assert!((cy - 100.0).abs() < 0.02, "step {step}: no vertical jitter: {cy}");
        assert!(
            (cx - (100.0 + pan * 1e9)).abs() < 0.02,
            "step {step}: the square at {cx}"
        );
        assert!(
            (edge - (200.0 + pan * 1e9)).abs() < 0.02,
            "step {step}: the edge at {edge}"
        );
        if let Some((lx, _, _, le)) = last {
            assert!(
                (cx - lx - 0.25).abs() < 0.01 && (edge - le - 0.25).abs() < 0.01,
                "step {step}: moved by a quarter pixel"
            );
        }
        last = Some((cx, cy, sum, edge));
    }
}

#[test]
fn a_fast_zoom_through_tiles_never_shows_one_blank() {
    // 40 000 squares of 0.8 units on a unit grid, filling the view from
    // zoom 6 on: up to zoom 500 any 128 px block holds ink.
    let w = world();
    w.extend(
        (0..200).flat_map(|i| (0..200).map(move |j| (WorldPath::rect(f64::from(i), f64::from(j), 0.8, 0.8), black()))),
    );
    let size = (1024_u32, 1024_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 1024.0,
    };
    let mut gpu = gpu();
    // A budget the settled raster cannot meet in one frame.
    gpu.set_raster_budget(256);
    let mut camera = Camera {
        center: (100.0, 100.0),
        zoom: 6.0,
        rotation: 0.0,
        settled: false,
    };
    let mut filled = 0;
    let check = |shot: &Pixels, what: &str| {
        let rows = ink(shot);
        for by in 0..8 {
            for bx in 0..8 {
                let block: f64 = rows
                    .iter()
                    .skip(by * 128)
                    .take(128)
                    .map(|r| r.iter().skip(bx * 128).take(128).sum::<f64>())
                    .sum();
                assert!(
                    block / 16384.0 > 0.05,
                    "{what}: block ({bx}, {by}) is blank: {}",
                    block / 16384.0
                );
            }
        }
    };
    for frame in 0..13 {
        camera.zoom *= 1.4;
        let shot = shoot(
            &mut gpu,
            WorldView {
                key: 7,
                bounds,
                world: w.snapshot(),
                camera,
                tiled: true,
            },
            size,
        );
        let (_, fill, spent) = gpu.canvas_raster();
        filled += fill;
        check(&shot, &format!("frame {frame} at zoom {}", camera.zoom));
        // The first frame shows the whole-world tiles at once; every
        // frame after spends no more than its budget.
        if frame > 0 {
            assert!(spent <= frame_bound(), "frame {frame} spent {spent}");
        }
    }
    assert!(filled > 0, "some frames needed coarser tiles");
    // Settled: the exact zoom. The tiles shown stay until the new ones are
    // all ready, frame by frame within the budget, never blank.
    camera.settled = true;
    let view = || WorldView {
        key: 7,
        bounds,
        world: w.snapshot(),
        camera,
        tiled: true,
    };
    let mut frames = 0;
    while {
        let shot = shoot(&mut gpu, view(), size);
        check(&shot, &format!("settling, frame {frames}"));
        assert!(gpu.canvas_raster().2 <= frame_bound(), "a settling frame over budget");
        gpu.raster_pending()
    } {
        frames += 1;
        assert!(frames < 600, "the settled tiles never finish");
    }
    assert!(frames > 1, "the settled raster took more than one frame: {frames}");
    let again = shoot(&mut gpu, view(), size);
    assert_eq!(gpu.canvas_raster().0, 0, "settled tiles are kept and complete");
    // Tiled and drawn straight agree once settled.
    let mut straight = view();
    straight.tiled = false;
    let direct = shoot(&mut gpu, straight, size);
    let diff = again
        .data()
        .iter()
        .zip(direct.data())
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum::<f64>()
        / f64::from(u32::try_from(again.data().len()).unwrap());
    assert!(diff < 1.0, "tiles and straight drawing differ by {diff} a channel");
}

#[test]
fn culling_and_order_follow_edits() {
    let w = world();
    let a = w.add(
        WorldPath::rect(0.0, 0.0, 10.0, 10.0),
        PathStyle::fill(Gradient::Solid(Color::hex(0x00ff_0000))),
    );
    let b = w.add(
        WorldPath::rect(5.0, 5.0, 10.0, 10.0),
        PathStyle::fill(Gradient::Solid(Color::hex(0x0000_00ff))),
    );
    let s = w.snapshot();
    assert_eq!(s.visible(&[6.0, 6.0, 7.0, 7.0]), vec![a.0, b.0], "in draw order");
    assert!(s.visible(&[20.0, 20.0, 30.0, 30.0]).is_empty());
    w.raise_above(a, Some(b));
    assert_eq!(w.snapshot().visible(&[6.0, 6.0, 7.0, 7.0]), vec![b.0, a.0], "raised");
    assert_eq!(w.hit((6.0, 6.0), 0.0), Some(a));
    w.remove(a);
    assert_eq!(w.hit((6.0, 6.0), 0.0), Some(b));
    assert_eq!(w.len(), 1);
}

#[test]
fn a_huge_fill_cut_to_the_view_keeps_its_winding() {
    // A ring (outer square, inner square the other way round) far larger
    // than the area: inside the area only the ring's band shows.
    let ring = WorldPath::rect(-1e6, -1e6, 2e6, 2e6)
        .move_to(-10.0, -10.0)
        .line_to(-10.0, 10.0)
        .line_to(10.0, 10.0)
        .line_to(10.0, -10.0)
        .close();
    let p = ring.near((-20.0, -20.0), [-20.0, -20.0, 20.0, 20.0], 0.01, false);
    let [x0, y0, x1, y1] = p.bounds();
    assert_eq!(
        [x0, y0, x1, y1],
        [0.0, 0.0, 40.0, 40.0],
        "cut to the area about its anchor"
    );
    // A curve far away is its chord; one through the area is flattened.
    let curve = WorldPath::new().move_to(-1e6, 0.0).quad_to(0.0, 1e6, 1e6, 0.0);
    let near = curve.near((0.0, 0.0), [-100.0, 400_000.0, 100.0, 600_000.0], 0.01, true);
    let [_, cy0, _, cy1] = near.bounds();
    assert!(cy0 >= 400_000.0 - 1e-3 && cy1 <= 600_000.0 + 1e-3, "{cy0} {cy1}");
}

#[test]
fn small_objects_draw_as_boxes_of_their_area() {
    // Squares of 0.8 units on a unit grid at zoom 3: 2.4 px each, under the
    // box size, so boxes; their ink is their area, 64 % of the view.
    let w = world();
    w.extend(
        (0..100).flat_map(|i| (0..100).map(move |j| (WorldPath::rect(f64::from(i), f64::from(j), 0.8, 0.8), black()))),
    );
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 240.0,
        height: 240.0,
    };
    let mut gpu = gpu();
    let view = |tiled| WorldView {
        key: 3,
        bounds,
        world: w.snapshot(),
        camera: Camera {
            center: (50.0, 50.0),
            zoom: 3.0,
            rotation: 0.0,
            settled: true,
        },
        tiled,
    };
    let shot = shoot(&mut gpu, view(false), (240, 240));
    let (_, _, sum) = centroid(&shot);
    let share = sum / (240.0 * 240.0);
    assert!((share - 0.64).abs() < 0.01, "boxes cover {share}");
    let tiled = shoot(&mut gpu, view(true), (240, 240));
    let (_, _, tsum) = centroid(&tiled);
    assert!((tsum - sum).abs() / sum < 0.01, "tiles cover {tsum} against {sum}");
}

/// Frames until no canvas rasters, the last one's pixels.
fn finish(gpu: &mut GpuRenderer, view: &dyn Fn() -> WorldView, size: (u32, u32)) -> Pixels {
    let mut shot = shoot(gpu, view(), size);
    for _ in 0..2000 {
        if !gpu.raster_pending() {
            break;
        }
        shot = shoot(gpu, view(), size);
    }
    assert!(!gpu.raster_pending(), "raster never finished");
    shot
}

/// Many kinds of object over a 600 unit square: fills, strokes, curves,
/// objects small enough to be boxes, overlapping in order.
fn mixed() -> super::World {
    let w = world();
    let mut seed = 7_u64;
    let mut unit = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as f64 / f64::from(1_u32 << 31)
    };
    for i in 0..3000_u32 {
        let (x, y) = (unit() * 600.0, unit() * 600.0);
        let size = unit() * 20.0 + 0.5;
        let color = Color::hex([0x00e1_1d48, 0x0025_63eb, 0x0016_a34a, 0x000f_172a][(i % 4) as usize]);
        match i % 3 {
            0 => {
                w.add(
                    WorldPath::rect(x, y, size, size * 0.6),
                    PathStyle::fill(Gradient::Solid(color.opacity(0.7))),
                );
            }
            1 => {
                let r = size / 2.0;
                let p = WorldPath::new()
                    .move_to(x + r, y)
                    .quad_to(x + r, y + r, x, y + r)
                    .quad_to(x - r, y + r, x - r, y)
                    .quad_to(x - r, y - r, x, y - r)
                    .quad_to(x + r, y - r, x + r, y)
                    .close();
                w.add(p, PathStyle::fill(Gradient::Solid(color)));
            }
            _ => {
                let p = WorldPath::new()
                    .move_to(x, y)
                    .line_to(x + size, y + size * 0.5)
                    .line_to(x + size * 1.5, y);
                w.add(
                    p,
                    PathStyle::stroke(Gradient::Solid(color), crate::figure::Stroke::round(1.0)),
                );
            }
        }
    }
    w
}

#[test]
fn tiles_rastered_across_frames_equal_tiles_rastered_at_once() {
    // Incremental equals cold: the same tiles rastered a few steps a frame
    // and all at once give the same pixels, bit for bit.
    let w = mixed();
    let size = (640_u32, 480_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 640.0,
        height: 480.0,
    };
    let view = || WorldView {
        key: 11,
        bounds,
        world: w.snapshot(),
        camera: Camera {
            center: (300.0, 300.0),
            zoom: 1.7,
            rotation: 0.0,
            settled: true,
        },
        tiled: true,
    };
    let mut slow = gpu();
    slow.set_raster_budget(24);
    let a = finish(&mut slow, &view, size);
    let mut fast = gpu();
    fast.set_raster_budget(u32::MAX);
    let b = finish(&mut fast, &view, size);
    assert_eq!(a.data(), b.data(), "split raster differs from one raster");
    // And settled tiles agree with drawing straight.
    let mut straight = view();
    straight.tiled = false;
    let c = shoot(&mut fast, straight, size);
    let diff = b
        .data()
        .iter()
        .zip(c.data())
        .map(|(x, y)| f64::from(x.abs_diff(*y)))
        .sum::<f64>()
        / b.data().len() as f64;
    assert!(diff < 1.0, "tiles and straight drawing differ by {diff} a channel");
}

#[test]
fn a_turned_camera_turns_the_world() {
    // A bar 100 × 10 units at zoom 1, turned a quarter: it stands upright.
    let w = world();
    w.add(WorldPath::rect(0.0, 0.0, 100.0, 10.0), black());
    let size = (200_u32, 200_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 200.0,
    };
    let camera = Camera {
        center: (50.0, 5.0),
        zoom: 1.0,
        rotation: std::f64::consts::FRAC_PI_2,
        settled: true,
    };
    let mut gpu = gpu();
    let shot = shoot(
        &mut gpu,
        WorldView {
            key: 2,
            bounds,
            world: w.snapshot(),
            camera,
            tiled: false,
        },
        size,
    );
    let rows = ink(&shot);
    let at = |x: usize, y: usize| rows[y][x];
    assert!(at(100, 60) > 0.99 && at(100, 140) > 0.99, "upright");
    assert!(at(60, 100) < 0.01 && at(140, 100) < 0.01, "no longer lying");
    let (cx, cy, sum) = centroid(&shot);
    assert!(
        (cx - 100.0).abs() < 0.05 && (cy - 100.0).abs() < 0.05,
        "about the centre: {cx} {cy}"
    );
    assert!((sum - 1000.0).abs() < 5.0, "its area: {sum}");
}

#[test]
fn turned_tiles_match_drawing_straight_once_settled() {
    let w = mixed();
    let size = (512_u32, 384_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 512.0,
        height: 384.0,
    };
    let mut camera = Camera {
        center: (300.0, 300.0),
        zoom: 1.3,
        rotation: 0.4,
        settled: true,
    };
    let view = |camera: Camera, tiled: bool| WorldView {
        key: 5,
        bounds,
        world: w.snapshot(),
        camera,
        tiled,
    };
    let mut gpu = gpu();
    let tiled = finish(&mut gpu, &|| view(camera, true), size);
    let straight = shoot(&mut gpu, view(camera, false), size);

    let diff = tiled
        .data()
        .iter()
        .zip(straight.data())
        .map(|(x, y)| f64::from(x.abs_diff(*y)))
        .sum::<f64>()
        / tiled.data().len() as f64;
    assert!(
        diff < 1.0,
        "turned tiles and straight drawing differ by {diff} a channel"
    );
    // Turning on: the tiles of the last turn show, turned, until settled.
    camera.rotation = 0.9;
    camera.settled = false;
    let moving = shoot(&mut gpu, view(camera, true), size);
    assert_eq!(gpu.canvas_raster().0, 0, "a moving turn rasters nothing new on screen");
    let (_, _, sum) = centroid(&moving);
    assert!(sum > 1000.0, "the old tiles show, turned");
}

#[test]
fn the_camera_keeps_the_point_under_the_pointer() {
    let size = (800.0, 600.0);
    let c = Camera {
        center: (12.5, -3.0),
        zoom: 2.5,
        rotation: 0.3,
        settled: true,
    };
    let p = (123.0, 456.0);
    let w = c.to_world(p, size);
    let back = c.to_screen(w, size);
    assert!((back.0 - p.0).abs() < 1e-9 && (back.1 - p.1).abs() < 1e-9);
    for next in [c.zoomed(1.7, p, size), c.rotated(1.1, p, size)] {
        let s = next.to_screen(w, size);
        assert!((s.0 - p.0).abs() < 1e-9 && (s.1 - p.1).abs() < 1e-9, "{s:?}");
    }
    // A pan moves the world with the pointer, turned or not.
    let panned = c.panned((10.0, -4.0));
    let s = panned.to_screen(w, size);
    assert!(
        (s.0 - p.0 - 10.0).abs() < 1e-9 && (s.1 - p.1 + 4.0).abs() < 1e-9,
        "{s:?}"
    );
}

#[test]
fn long_lines_are_simplified_per_level_of_detail() {
    // A line of 4 001 points wiggling by 0.01 units over 1 000, and one
    // real corner.
    let mut p = WorldPath::new().move_to(0.0, 0.0);
    for i in 1..=4000 {
        let x = f64::from(i) * 0.25;
        let wiggle = if i % 2 == 0 { 0.01 } else { -0.01 };
        let y = if i > 2000 { (x - 500.0) + wiggle } else { wiggle };
        p = p.line_to(x, y);
    }
    let simple = p.simplified(0.05);
    assert!(simple.points.len() <= 4, "kept {} points", simple.points.len());
    // The corner stays.
    assert!(
        simple
            .points
            .iter()
            .any(|q| (q[0] - 500.0).abs() < 0.5 && q[1].abs() < 0.1)
    );
    // Within tolerance: no simplification at all.
    assert_eq!(p.simplified(0.001).points.len(), p.points.len());
    // Per level: zoomed out it draws few points, zoomed in all of them.
    let w = world();
    w.add(
        p.clone(),
        PathStyle::stroke(Gradient::Solid(Color::BLACK), crate::figure::Stroke::round(1.0)),
    );
    let s = w.snapshot();
    let o = s.object(0).unwrap();
    let body = o.as_path().unwrap();
    assert!(body.at_level(1.0, o.anchor).points().len() < 10, "zoomed out");
    assert_eq!(
        body.at_level(1000.0, o.anchor).points().len(),
        p.points.len(),
        "zoomed in"
    );
    // And what it draws zoomed out looks the same.
    let size = (500_u32, 300_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 500.0,
        height: 300.0,
    };
    let camera = Camera {
        center: (500.0, 250.0),
        zoom: 0.5,
        rotation: 0.0,
        settled: true,
    };
    let mut gpu = gpu();
    let shot = shoot(
        &mut gpu,
        WorldView {
            key: 4,
            bounds,
            world: s.clone(),
            camera,
            tiled: false,
        },
        size,
    );
    let full = world();
    full.add(
        p,
        PathStyle::stroke(Gradient::Solid(Color::BLACK), crate::figure::Stroke::round(1.0)),
    );
    // Below the simplification bound, the same line drawn whole.
    let whole = shoot(
        &mut gpu,
        WorldView {
            key: 4,
            bounds,
            world: full.snapshot(),
            camera: Camera { zoom: 0.5, ..camera },
            tiled: false,
        },
        size,
    );
    let (_, _, a) = centroid(&shot);
    let (_, _, b) = centroid(&whole);
    assert!((a - b).abs() / b < 0.02, "simplified ink {a} against {b}");
}

#[test]
fn text_in_a_canvas_is_drawn_from_its_curves_and_as_bars_far_out() {
    let mut text = crate::text::TextSystem::new();
    let w = world();
    w.add_text(super::WorldText::new("Hello canvas", (0.0, 0.0), 10.0));
    // The paint shapes it.
    let picture = super::CanvasPicture {
        world: w.snapshot(),
        camera: Camera::default(),
    };
    crate::chunk::Picture::shape(&picture, &mut text);
    let s = picture.labelled();
    let o = s.object(0).unwrap();
    let super::Body::Text(t) = &o.body else { panic!("text") };
    let shaped = s.labels.get(t).expect("shaped");
    assert_eq!(shaped.glyphs.len(), 12, "a glyph a character");
    assert!(
        shaped.width > 4.0 && shaped.width < 8.0,
        "about half an em a letter: {}",
        shaped.width
    );
    let size = (400_u32, 100_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 100.0,
    };
    let mut glyphs = text.glyphs();
    let mut shoot_text = |gpu: &mut GpuRenderer, camera: Camera| {
        let mut scene = Scene::new(size, 1.0, Some(Color::WHITE));
        scene.push(vec![Item::World(Arc::new(WorldView {
            key: 9,
            bounds,
            world: Arc::clone(&s),
            camera,
            tiled: false,
        }))]);
        let shot = gpu.capture(&scene, None, &mut glyphs).expect("read back");
        gpu.end_frame();
        shot
    };
    let mut gpu = gpu();
    // 20 px an em: curves, the line short enough to stay inside the
    // canvas turned.
    let near = Camera {
        center: (30.0, 6.0),
        zoom: 2.0,
        rotation: 0.0,
        settled: true,
    };
    let (cx, _, sum) = centroid(&shoot_text(&mut gpu, near));
    assert!(sum > 200.0, "letters inked: {sum}");
    assert!((cx - 200.0).abs() < 40.0, "about the middle: {cx}");
    // Turned, about the same ink: edges no longer fall on pixel rows, and
    // a partly covered pixel, blended in linear light and stored as sRGB,
    // reads lighter than its coverage, so the sum drops a little.
    let (_, _, turned) = centroid(&shoot_text(&mut gpu, Camera { rotation: 0.5, ..near }));
    assert!((turned - sum).abs() / sum < 0.2, "turned ink {turned} against {sum}");
    // At 2 px an em: a grey bar, no glyphs.
    let far = Camera { zoom: 0.2, ..near };
    let view = super::plan::View {
        center: far.center,
        k: far.zoom,
        cs: far.turn(),
        origin: (200.0, 50.0),
        cut: [0.0, 0.0, 400.0, 100.0],
    };
    let mut steps = Vec::new();
    super::plan::plan(&s, &view, &mut |_, _| None, &mut steps);
    assert!(
        matches!(steps.as_slice(), [super::plan::Step::Boxes(_)]),
        "a bar: {steps:?}"
    );
    let (_, _, bar) = centroid(&shoot_text(&mut gpu, far));
    assert!(bar > 1.0, "the bar shows");
}

#[test]
fn raster_workers_plan_the_most_urgent_first() {
    use super::planner::{Planner, Request, TileKey};
    let w = mixed();
    let s = w.snapshot();
    let view = super::plan::View {
        center: (0.0, 0.0),
        k: 1.0,
        cs: (1.0, 0.0),
        origin: (0.0, 0.0),
        cut: [0.0, 0.0, 256.0, 256.0],
    };
    let key = |x: i64| TileKey {
        world: s.id,
        scale: 1.0_f64.to_bits(),
        turn: 0,
        phase: [0, 0],
        x,
        y: 0,
    };
    let request = |x: i64| Request {
        key: key(x),
        world: Arc::clone(&s),
        view: super::plan::View {
            center: (x as f64 * 256.0, 0.0),
            ..view
        },
    };
    let mut p = Planner::new(1);
    p.want((0..6).map(request).collect());
    assert!(p.wait_idle(std::time::Duration::from_secs(30)));
    let order: Vec<i64> = p.take().into_iter().map(|(k, _, _)| k.x).collect();
    assert_eq!(order, vec![0, 1, 2, 3, 4, 5], "in the order wanted");
    // What still waits is replaced by what is wanted now. The workers are
    // held while both wants come, so none has taken any of the first: a
    // loaded machine may otherwise run the worker through all 30 before
    // the second want, which is no replacement to test.
    p.hold(true);
    p.want((10..40).map(request).collect());
    p.want(vec![request(99), request(98)]);
    p.hold(false);
    assert!(p.wait_idle(std::time::Duration::from_secs(30)));
    let got: Vec<i64> = p.take().into_iter().map(|(k, _, _)| k.x).collect();
    assert_eq!(got, vec![99, 98], "only the latest want, in its order");
    // A plan is the same wherever it is made.
    let direct = super::planner::plan_tile(&s, &request(1).view);
    assert!(!direct.is_empty());
}

/// Mean channel difference of `a` and `b` over pixels `x0..x1`, `y0..y1`.
fn diff_in(a: &Pixels, b: &Pixels, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> f64 {
    let mut sum = 0.0;
    let mut n = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let (p, q) = (a.pixel(x, y).unwrap(), b.pixel(x, y).unwrap());
            for c in 0..4 {
                sum += f64::from(p[c].abs_diff(q[c]));
                n += 1.0;
            }
        }
    }
    sum / n
}

/// Frames until no canvas rasters: the last one's pixels and the tiles
/// rastered on the way.
fn finish_counting(gpu: &mut GpuRenderer, view: &dyn Fn() -> WorldView, size: (u32, u32)) -> (Pixels, usize) {
    let mut shot = shoot(gpu, view(), size);
    let mut tiles = gpu.canvas_raster().0;
    for _ in 0..2000 {
        if !gpu.raster_pending() {
            break;
        }
        shot = shoot(gpu, view(), size);
        tiles = tiles.saturating_add(gpu.canvas_raster().0);
    }
    assert!(!gpu.raster_pending(), "raster never finished");
    (shot, tiles)
}

#[test]
fn a_dragged_object_draws_live_and_rasters_only_the_tiles_it_touched() {
    // Among 3 000 objects, a red bar in the middle of the order with a
    // translucent blue square above it: lifted and dragged under the
    // square, it must show beneath it, as drawing straight shows it.
    let w = mixed();
    let red = w.add(
        WorldPath::rect(290.0, 290.0, 30.0, 20.0),
        PathStyle::fill(Gradient::Solid(Color::hex(0x00ff_0000))),
    );
    w.add(
        WorldPath::rect(318.0, 284.0, 40.0, 40.0),
        PathStyle::fill(Gradient::Solid(Color::hex(0x0000_00ff).opacity(0.5))),
    );
    let size = (1024_u32, 768_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 1024.0,
        height: 768.0,
    };
    let camera = Camera {
        center: (300.0, 300.0),
        zoom: 1.7,
        rotation: 0.0,
        settled: true,
    };
    let view = |key: u64, tiled: bool| WorldView {
        key,
        bounds,
        world: w.snapshot(),
        camera,
        tiled,
    };
    // Where the bar and the square are on screen, and around.
    let near = (512 - 40, 384 - 40, 512 + 140, 384 + 90);
    let mut gpu = gpu();
    let (_, all) = finish_counting(&mut gpu, &|| view(12, true), size);
    assert!(all >= 12, "the canvas took {all} tiles");
    let check = |gpu: &mut GpuRenderer, what: &str| {
        let tiled = shoot(gpu, view(12, true), size);
        let rastered = gpu.canvas_raster().0;
        let straight = shoot(gpu, view(13, false), size);
        let d = diff_in(&tiled, &straight, near);
        assert!(d < 1.0, "{what}: tiles and live differ from drawing straight by {d}");
        rastered
    };
    // Lifted: the very next frame shows it right (no ghost in the tiles
    // below, which are still of before), drawn live.
    w.lift(red);
    check(&mut gpu, "lifted");
    assert!(gpu.canvas_live().0 >= 1, "drawn live");
    let (_, again) = finish_counting(&mut gpu, &|| view(12, true), size);
    // At most the four tiles of its tiling it touched, and the whole-world tile.
    assert!((1..=5).contains(&again), "lifting rastered {again} tiles of {all}");
    // Dragged: nothing rastered, every frame as drawing straight.
    for step in 0..8 {
        w.translate(red, (5.0, 2.0));
        let rastered = check(&mut gpu, &format!("drag step {step}"));
        assert_eq!(rastered, 0, "a drag frame rasters no tile");
        assert!(!gpu.raster_pending(), "nothing waits for raster while dragging");
    }
    assert!(w.snapshot().live_object(red.0).is_some());
    assert_eq!(
        w.hit((330.0, 302.0), 0.0),
        w.hit((339.0, 305.0), 0.0),
        "hit testing sees it where it is"
    );
    // Landed: the tiles where it lands rastered, then all as before.
    w.land(red);
    check(&mut gpu, "landed");
    let (_, landed) = finish_counting(&mut gpu, &|| view(12, true), size);
    assert!((1..=5).contains(&landed), "landing rastered {landed} tiles");
    assert!(w.lifted().is_empty());
    let tiled = shoot(&mut gpu, view(12, true), size);
    let straight = shoot(&mut gpu, view(13, false), size);
    assert!(diff_in(&tiled, &straight, (0, 0, size.0, size.1)) < 1.0);
}

#[test]
fn a_large_change_shows_all_at_once() {
    // Thousands of objects added across the view: too much to draw live,
    // so the tiles of before show until every visible tile is rastered
    // anew, then the new ones, never a mix.
    let w = mixed();
    let size = (640_u32, 480_u32);
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 640.0,
        height: 480.0,
    };
    let view = || WorldView {
        key: 14,
        bounds,
        world: w.snapshot(),
        camera: Camera {
            center: (300.0, 300.0),
            zoom: 1.7,
            rotation: 0.0,
            settled: true,
        },
        tiled: true,
    };
    let mut gpu = gpu();
    gpu.set_raster_budget(200);
    let before = finish(&mut gpu, &view, size);
    w.extend((0..60).flat_map(|i| {
        (0..60).map(move |j| {
            (
                WorldPath::rect(f64::from(i) * 10.0, f64::from(j) * 10.0, 4.0, 4.0),
                PathStyle::fill(Gradient::Solid(Color::hex(0x0016_a34a))),
            )
        })
    }));
    let mut frames = Vec::new();
    for _ in 0..2000 {
        frames.push(shoot(&mut gpu, view(), size));
        if !gpu.raster_pending() {
            break;
        }
    }
    let after = frames.last().unwrap().clone();
    assert!(frames.len() > 2, "rastered over {} frames", frames.len());
    assert_ne!(before.data(), after.data());
    for (n, f) in frames.iter().enumerate() {
        assert!(
            f.data() == before.data() || f.data() == after.data(),
            "frame {n} mixes tiles of before and after"
        );
    }
}

#[test]
fn objects_too_small_to_draw_keep_their_tone() {
    // Squares of 0.4 units on a unit grid at zoom 1: 0.4 px each, under
    // the drop size. Not drawn they would leave the view white; gathered
    // per pixel they cover 16 % of it, as their area does.
    let w = world();
    w.extend(
        (0..100).flat_map(|i| (0..100).map(move |j| (WorldPath::rect(f64::from(i), f64::from(j), 0.4, 0.4), black()))),
    );
    let bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };
    let view = |tiled| WorldView {
        key: 15,
        bounds,
        world: w.snapshot(),
        camera: Camera {
            center: (50.0, 50.0),
            zoom: 1.0,
            rotation: 0.0,
            settled: true,
        },
        tiled,
    };
    let mut gpu = gpu();
    let shot = shoot(&mut gpu, view(false), (100, 100));
    // Each pixel 16 % black (boxes blend as the UI's rects do, in the
    // target's encoding, as Chrome blends).
    let want = 0.16;
    let rows = ink(&shot);
    let mean = rows.iter().flatten().sum::<f64>() / 10_000.0;
    assert!((mean - want).abs() < 0.01, "ink {mean}, want {want}");
    let spread = rows.iter().flatten().fold(0.0_f64, |m, v| m.max((v - mean).abs()));
    assert!(spread < 0.02, "an even tone: {spread}");
    let tiled = finish(&mut gpu, &|| view(true), (100, 100));
    let tmean = ink(&tiled).iter().flatten().sum::<f64>() / 10_000.0;
    assert!((tmean - mean).abs() < 0.01, "tiles {tmean} against {mean}");
}

#[test]
fn text_in_a_canvas_wraps_to_its_width() {
    let mut text = crate::text::TextSystem::new();
    let w = world();
    let content = "The quick brown fox jumps over the lazy dog, again and again and again";
    let para = w.add_text(super::WorldText::new(content, (0.0, 0.0), 10.0).width(120.0));
    let line = w.add_text(super::WorldText::new(content, (0.0, 200.0), 10.0));
    let picture = super::CanvasPicture {
        world: w.snapshot(),
        camera: Camera::default(),
    };
    crate::chunk::Picture::shape(&picture, &mut text);
    let s = picture.labelled();
    let shaped = |id: super::ObjectId| {
        let super::Body::Text(t) = &s.object(id.0).unwrap().body else {
            panic!("text")
        };
        s.labels.get(t).expect("shaped").clone()
    };
    let (p, l) = (shaped(para), shaped(line));
    assert_eq!(l.lines.len(), 1, "unwrapped, one line");
    assert!(p.lines.len() >= 4, "wrapped into {} lines", p.lines.len());
    assert_eq!(p.glyphs.len(), l.glyphs.len(), "the same glyphs, broken into lines");
    // Every line within the width (12 ems), one under the other.
    for (a, b) in p.lines.iter().zip(p.lines.iter().skip(1)) {
        assert!(b[1] >= a[3] - 1e-3, "lines stack: {a:?} {b:?}");
    }
    assert!(p.lines.iter().all(|l| l[2] <= 12.0 + 1e-3), "{:?}", p.lines);
    assert!(p.glyphs.iter().all(|(_, pen, _)| pen.0 < 12.0));
    // Its box holds its ink.
    let o = s.object(para.0).unwrap();
    assert!(
        o.bbox[3] >= p.height * 10.0 && o.bbox[2] >= p.width * 10.0,
        "{:?}",
        o.bbox
    );
    // Near: every glyph from its curves, turned or not; far out, a bar a line.
    for rotation in [0.0, 0.6] {
        let view = super::plan::View {
            center: (60.0, 30.0),
            k: 4.0,
            cs: super::cos_sin(rotation),
            origin: (300.0, 200.0),
            cut: [0.0, 0.0, 600.0, 400.0],
        };
        let mut steps = Vec::new();
        super::plan::plan(&s, &view, &mut |_, _| None, &mut steps);
        let glyphs = steps
            .iter()
            .filter(|s| matches!(s, super::plan::Step::Glyph { .. }))
            .count();
        assert_eq!(glyphs, p.glyphs.len(), "turned {rotation}");
    }
    let far = super::plan::View {
        center: (60.0, 30.0),
        k: 0.2,
        cs: (1.0, 0.0),
        origin: (50.0, 50.0),
        cut: [0.0, 0.0, 100.0, 100.0],
    };
    let mut steps = Vec::new();
    super::plan::plan(&s, &far, &mut |_, _| None, &mut steps);
    let [super::plan::Step::Boxes(bars)] = steps.as_slice() else {
        panic!("bars: {steps:?}")
    };
    // (The one-line copy below is in view too.)
    assert_eq!(bars.len() / super::boxes::BOX, p.lines.len() + 1, "a bar a line");
    // Drawn, it inks within its box.
    let size = (300_u32, 300_u32);
    let mut glyphs = text.glyphs();
    let mut scene = Scene::new(size, 1.0, Some(Color::WHITE));
    scene.push(vec![Item::World(Arc::new(WorldView {
        key: 16,
        bounds: Bounds {
            x: 0.0,
            y: 0.0,
            width: 300.0,
            height: 300.0,
        },
        world: s.clone(),
        camera: Camera {
            center: (60.0, 30.0),
            zoom: 2.0,
            rotation: 0.0,
            settled: true,
        },
        tiled: false,
    }))]);
    let mut gpu = gpu();
    let shot = gpu.capture(&scene, None, &mut glyphs).expect("read back");
    let (cx, cy, sum) = centroid(&shot);
    assert!(sum > 500.0, "inked {sum}");
    assert!((cx - 150.0).abs() < 40.0 && (cy - 150.0).abs() < 60.0, "{cx} {cy}");
}

#[test]
fn snapshots_find_what_a_scan_finds_through_edits() {
    // Adds, moves, raises, removals and lifts, past the point where the
    // tree is copied again: what a snapshot finds in an area, in order, is
    // what a scan of every object finds.
    let w = world();
    let mut rng = crate::testing::Rng::new(41);
    let mut ids: Vec<super::ObjectId> = Vec::new();
    let mut unit = || f64::from(rng.unit());
    for step in 0..3000 {
        let r = unit();
        if ids.len() < 50 || r < 0.35 {
            let (x, y) = (unit() * 1000.0, unit() * 1000.0);
            ids.push(w.add(WorldPath::rect(x, y, 5.0, 5.0), black()));
        } else {
            let at = ((unit() * ids.len() as f64) as usize).min(ids.len() - 1);
            let id = ids[at];
            if r < 0.7 {
                w.translate(id, (unit() * 40.0 - 20.0, unit() * 40.0 - 20.0));
            } else if r < 0.8 {
                let below = ids[((unit() * ids.len() as f64) as usize).min(ids.len() - 1)];
                if below != id {
                    w.raise_above(id, Some(below));
                }
            } else if r < 0.9 {
                w.remove(id);
                ids.swap_remove(at);
            } else if w.lifted().contains(&id) {
                w.land(id);
            } else {
                w.lift(id);
            }
        }
        if step % 250 == 0 || step == 2999 {
            let s = w.snapshot();
            let area = [200.0, 300.0, 700.0, 650.0];
            let found: Vec<u32> = s.drawn(&area).iter().map(|(i, _)| *i).collect();
            let mut scan: Vec<(super::zkey::ZKey, u32)> = ids
                .iter()
                .filter_map(|id| {
                    let o = s.live_object(id.0).or_else(|| s.object(id.0))?;
                    super::aabb::overlaps(&o.bbox, &area).then(|| (o.z.clone(), id.0))
                })
                .collect();
            scan.sort();
            let scan: Vec<u32> = scan.into_iter().map(|(_, i)| i).collect();
            assert_eq!(found, scan, "step {step}");
        }
    }
}

/// Text a cold frame's paint shapes is its own: the live paint after it
/// still finds the text unshaped, and the labels it had are as they were.
#[cfg(debug_assertions)]
#[test]
fn a_cold_paint_does_not_shape_text_for_the_live_one() {
    let mut text = crate::text::TextSystem::new();
    let w = world();
    let seen = w.add_text(super::WorldText::new("Seen live", (0.0, 0.0), 10.0));
    let picture = || super::CanvasPicture {
        world: w.snapshot(),
        camera: Camera::default(),
    };
    let body = |s: &super::WorldSnapshot, id: super::ObjectId| {
        let super::Body::Text(t) = &s.object(id.0).unwrap().body else {
            panic!("text")
        };
        Arc::clone(t)
    };
    crate::chunk::Picture::shape(&picture(), &mut text);
    let live = picture().labelled();
    assert!(
        live.labels.get(&body(&live, seen)).is_some(),
        "the live paint shaped it"
    );
    let later = w.add_text(super::WorldText::new("Added since", (0.0, 40.0), 10.0));
    {
        let _cold = crate::engine::ColdFrame::begin();
        let p = picture();
        crate::chunk::Picture::shape(&p, &mut text);
        let cold = p.labelled();
        assert!(
            cold.labels.get(&body(&cold, later)).is_some(),
            "the cold paint shaped it"
        );
    }
    let after = picture().labelled();
    assert!(
        after.labels.get(&body(&after, later)).is_none(),
        "the cold shaping is gone with the cold frame"
    );
    assert!(
        after.labels.get(&body(&after, seen)).is_some(),
        "the live labels are kept"
    );
    assert!(Arc::ptr_eq(&after.labels, &live.labels), "the live labels unchanged");
}

/// A cold paint shapes every text it draws, not only what the live
/// snapshots left unshaped: text the live paint shaped before an edit is
/// in the cold scene, drawn with the glyphs the live one has.
#[cfg(debug_assertions)]
#[test]
fn a_cold_paint_draws_text_the_live_paint_shaped_before_an_edit() {
    let mut text = crate::text::TextSystem::new();
    let w = world();
    let a = w.add_text(super::WorldText::new("Shaped live", (0.0, 0.0), 10.0));
    let picture = || super::CanvasPicture {
        world: w.snapshot(),
        camera: Camera::default(),
    };
    let view = |p: &super::CanvasPicture| {
        let mut out = Vec::new();
        crate::chunk::Picture::draw(
            p,
            Bounds {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
            1.0,
            7,
            &mut out,
        );
        let Some(Item::World(v)) = out.pop() else {
            panic!("a world")
        };
        v
    };
    crate::chunk::Picture::shape(&picture(), &mut text);
    // An edit, and the live paint again: A no longer waits to be shaped.
    w.add_text(super::WorldText::new("Added since", (0.0, 40.0), 10.0));
    let p = picture();
    crate::chunk::Picture::shape(&p, &mut text);
    let live = view(&p);
    let cold = crate::engine::in_cold_frame(|| {
        let p = picture();
        crate::chunk::Picture::shape(&p, &mut text);
        view(&p)
    });
    let super::Body::Text(t) = &cold.world.object(a.0).unwrap().body else {
        panic!("text")
    };
    assert!(cold.world.labels.get(t).is_some(), "A is in the cold scene");
    assert!(*cold == *live, "drawn with the same glyphs");
    let bare = super::WorldView {
        world: w.snapshot(),
        ..(*live).clone()
    };
    assert!(bare != *live, "a view is told apart by its text's glyphs");
}

/// A world's snapshot made first in a cold frame is the frame's: the
/// world's cache of it stays empty, its unshaped text as it was.
#[cfg(debug_assertions)]
#[test]
fn a_cold_snapshot_leaves_the_worlds_caches_empty() {
    let w = world();
    w.add_text(super::WorldText::new("Never drawn live", (0.0, 0.0), 10.0));
    let cold = crate::engine::in_cold_frame(|| w.snapshot());
    assert!(cold.unshaped.is_empty(), "the cold paint shapes everything itself");
    w.0.with_untracked(|s| {
        assert!(s.snapshot.with(Option::is_none), "no snapshot cached");
        assert!(s.base.with(Option::is_none), "no base cached");
        assert_eq!(s.unshaped.with(Vec::len), 1, "the text still waits for the live paint");
    });
}
