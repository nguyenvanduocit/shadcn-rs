//! The GPU renderer's tests: what it draws against exact references, and
//! that a steady frame makes nothing anew.

use super::path::{TileRect, reference};
use super::*;
use crate::convert::i32_to_f32;
use crate::figure::{Cap, FillRule, Join, PathBuilder};
use crate::scene::{Clip, EffectNode, Gradient, GroupTransform, Item, PixelRect, State};

/// A device on the first adapter, when the machine has one.
pub(super) fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = pollster::block_on(instance().request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gui test"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()
}

fn gpu() -> GpuRenderer {
    GpuRenderer::headless().expect("a GPU (CI paints on lavapipe)")
}

/// Half black over white, 127.5, as a GPU rounds it (127 or 128), opaque.
fn half_grey(p: [u8; 4]) -> bool {
    p[3] == 255 && p[..3].iter().all(|&c| c == 127 || c == 128) && p[0] == p[1] && p[1] == p[2]
}

fn at(p: &Pixels, x: u32, y: u32) -> [u8; 4] {
    p.pixel(x, y).unwrap_or_else(|| panic!("({x}, {y}) is outside"))
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds {
    Bounds {
        x,
        y,
        width: w,
        height: h,
    }
}

/// `items` as a chunk under a scale and turn of its own (an effect node
/// with a transform), its subtree painting `region`.
fn group(scene: &mut Scene, items: Vec<Item>, region: PixelRect, transform: GroupTransform) {
    let t = scene.add_transform((0.0, 0.0), Some(transform));
    let effect = scene.add_effect(EffectNode {
        transform: t,
        transformed: true,
        region: Some(region),
        ..EffectNode::default()
    });
    scene.push_under(
        State {
            transform: t,
            clip: 0,
            effect,
        },
        items,
    );
}

#[test]
fn a_turned_group_turns_about_its_origin() {
    let mut scene = Scene::new((100, 100), 1.0, Some(Color::WHITE));
    group(
        &mut scene,
        vec![Item::Rect {
            bounds: rect(40.0, 40.0, 20.0, 20.0),
            color: Color::BLACK,
        }],
        PixelRect {
            x: 40,
            y: 40,
            width: 20,
            height: 20,
        },
        GroupTransform {
            origin: (50.0, 50.0),
            scale: 1.0,
            rotate: 45.0,
        },
    );
    let mut gpu = gpu();
    let shot = gpu.capture(&scene, None, &mut Glyphs::for_os()).expect("read back");
    // A diamond reaching 14.1 px from the centre along the axes: its tips
    // pass the square's sides, its corners are cut off.
    for (x, y) in [(50, 50), (50, 38), (38, 50), (50, 62), (62, 50)] {
        assert_eq!(at(&shot, x, y), [0, 0, 0, 255], "({x}, {y}) is inside the diamond");
    }
    for (x, y) in [(41, 41), (59, 59), (41, 59), (59, 41)] {
        assert_eq!(at(&shot, x, y), [255, 255, 255, 255], "({x}, {y}) was turned away");
    }
}

/// The paths the coverage tests draw: what is hard to cover exactly.
fn hard_paths() -> Vec<(&'static str, Path, PathStyle)> {
    let fill = |p: PathBuilder| p.build();
    let white = Gradient::Solid(Color::WHITE);
    let star = |rule: FillRule| {
        let mut b = Path::builder().fill_rule(rule);
        for (i, k) in [0_u8, 2, 4, 1, 3].into_iter().enumerate() {
            let a = (f32::from(k) * 0.4 - 0.5) * std::f32::consts::PI;
            let (x, y) = (48.0 + 40.0 * a.cos(), 48.0 + 40.0 * a.sin());
            b = if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) };
        }
        b.close().build()
    };
    vec![
        (
            "a star, non-zero",
            star(FillRule::NonZero),
            PathStyle::fill(white.clone()),
        ),
        (
            "a star, even-odd",
            star(FillRule::EvenOdd),
            PathStyle::fill(white.clone()),
        ),
        (
            "a sliver",
            fill(
                Path::builder()
                    .move_to(3.3, 5.1)
                    .line_to(90.7, 9.4)
                    .line_to(3.4, 5.6)
                    .close(),
            ),
            PathStyle::fill(white.clone()),
        ),
        (
            "a cubic with a cusp",
            fill(
                Path::builder()
                    .move_to(10.0, 80.0)
                    .cubic_to(90.0, 10.0, 10.0, 10.0, 85.0, 85.0)
                    .close(),
            ),
            PathStyle::fill(white.clone()),
        ),
        (
            "a ring, even-odd",
            fill(Path::band(
                rect(8.0, 8.0, 80.0, 80.0),
                Corners {
                    top_left: 40.0,
                    top_right: 40.0,
                    bottom_right: 40.0,
                    bottom_left: 40.0,
                },
                rect(30.0, 28.0, 40.0, 40.0),
                Corners {
                    top_left: 20.0,
                    top_right: 20.0,
                    bottom_right: 20.0,
                    bottom_left: 20.0,
                },
            )),
            PathStyle::fill(white.clone()),
        ),
        (
            "a zigzag with sharp miters",
            fill(
                Path::builder()
                    .move_to(8.0, 70.0)
                    .line_to(30.0, 20.0)
                    .line_to(40.0, 75.0)
                    .line_to(60.0, 25.0)
                    .line_to(88.0, 60.0),
            ),
            PathStyle::stroke(white.clone(), Stroke::new(6.0).join(Join::Miter).miter_limit(10.0)),
        ),
        (
            "a curve with round caps",
            fill(Path::builder().move_to(10.0, 50.0).quad_to(48.0, -20.0, 86.0, 50.0)),
            PathStyle::stroke(white.clone(), Stroke::round(9.0)),
        ),
        (
            "square caps and bevels",
            fill(
                Path::builder()
                    .move_to(15.0, 15.0)
                    .line_to(80.0, 30.0)
                    .line_to(20.0, 80.0),
            ),
            PathStyle::stroke(white.clone(), Stroke::new(7.0).cap(Cap::Square).join(Join::Bevel)),
        ),
        (
            "a hairline",
            fill(Path::builder().move_to(5.0, 90.0).line_to(91.0, 7.0)),
            PathStyle::stroke(white.clone(), Stroke::new(0.4)),
        ),
        (
            "a circle far larger than the window",
            fill(Path::circle(48.0, 3000.0, 2960.0)),
            PathStyle::fill(white),
        ),
    ]
}

fn shot_of(gpu: &mut GpuRenderer, path: &Path, style: &PathStyle, origin: (f32, f32)) -> Pixels {
    let mut scene = Scene::new((96, 96), 1.0, Some(Color::BLACK));
    scene.push(vec![Item::Path {
        origin,
        path: path.clone(),
        style: style.clone(),
    }]);
    gpu.capture(&scene, None, &mut Glyphs::for_os()).expect("read back")
}

/// Each pass's CPU reference against the oracle, and the GPU against both:
/// within 1/255 a pixel (ADR-0017).
#[test]
fn paths_are_covered_within_one_level_of_the_exact_area() {
    let mut gpu = gpu();
    let rect = TileRect {
        x0: -1,
        y0: -1,
        w: 8,
        h: 8,
    };
    for (name, path, style) in hard_paths() {
        let origin = (0.3, 0.6);
        let m = [1.0, 0.0, 0.0, 1.0, origin.0, origin.1];
        let lines = reference::lines(&path, &style, m, rect);
        let rule = match style.draw {
            PathDraw::Fill => path.fill_rule(),
            PathDraw::Stroke(_) => FillRule::NonZero,
        };
        let even_odd = rule == FillRule::EvenOdd;
        let binned = reference::bin(&lines, rect, rule);
        let hairline = match &style.draw {
            PathDraw::Stroke(s) => path::StrokeParams::new(s, 1.0).alpha,
            PathDraw::Fill => 1.0,
        };
        let shot = shot_of(&mut gpu, &path, &style, origin);
        let mut worst: (f64, u32, u32) = (0.0, 0, 0);
        for y in 0..96_u32 {
            for x in 0..96_u32 {
                let (xi, yi) = (i32::try_from(x).unwrap_or(0), i32::try_from(y).unwrap_or(0));
                let exact = reference::exact(&lines, xi, yi, even_odd) * f64::from(hairline);
                let cpu = f64::from(reference::coverage(&binned, xi, yi)) * f64::from(hairline);
                assert!(
                    (cpu - exact).abs() < 1e-3,
                    "{name}: the CPU passes give {cpu} at ({x}, {y}), the area is {exact}"
                );
                let got = f64::from(at(&shot, x, y)[0]) / 255.0;
                let d = (got - exact).abs();
                if d > worst.0 {
                    worst = (d, x, y);
                }
            }
        }
        assert!(
            worst.0 <= 1.0 / 255.0 + 1e-9,
            "{name}: {:.2}/255 off the exact area at ({}, {})",
            worst.0 * 255.0,
            worst.1,
            worst.2
        );
    }
}

/// A path moved by whole pixels is not binned again: its tiles move.
#[test]
fn a_whole_pixel_pan_reuses_the_tiles() {
    let mut gpu = gpu();
    let (_, path, style) = hard_paths().swap_remove(3);
    let first = shot_of(&mut gpu, &path, &style, (0.25, 0.5));
    gpu.end_frame();
    let binned = gpu.paths.binned;
    let moved = shot_of(&mut gpu, &path, &style, (3.25, -2.5));
    gpu.end_frame();
    assert_eq!(gpu.paths.binned, binned, "nothing binned again");
    for y in 3..90 {
        for x in 0..90 {
            assert_eq!(at(&first, x, y), at(&moved, x + 3, y - 3), "({x}, {y}) moved");
        }
    }
    shot_of(&mut gpu, &path, &style, (3.5, -2.5));
    assert_eq!(gpu.paths.binned, binned + 1, "a fraction of a pixel bins anew");
}

fn busy_scene(size: u32) -> Scene {
    let mut scene = Scene::new((size, size), 1.0, Some(Color::WHITE));
    scene.push(vec![Item::RoundedRect {
        bounds: rect(4.0, 4.0, 40.0, 20.0),
        radius: Corners {
            top_left: 6.0,
            top_right: 6.0,
            bottom_right: 6.0,
            bottom_left: 6.0,
        },
        color: Color::BLACK,
    }]);
    let (_, path, style) = hard_paths().swap_remove(0);
    let clip = scene.add_clip(
        Clip {
            bounds: rect(0.0, 0.0, 60.0, 60.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    scene.push_under(
        State {
            clip,
            ..State::default()
        },
        vec![Item::Path {
            origin: (0.0, 0.0),
            path,
            style,
        }],
    );
    scene.push(icons(16));
    scene
}

/// Three icons `size` pixels square.
fn icons(size: u32) -> Vec<Item> {
    [Icon::Check, Icon::X, Icon::Star]
        .into_iter()
        .enumerate()
        .map(|(i, icon)| Item::Icon {
            icon,
            size,
            color: Color::BLACK,
            stroke_width: 2.0,
            x: 60_i32.saturating_add(i32::try_from(i).unwrap_or(0).saturating_mul(18)),
            y: 4,
        })
        .collect()
}

#[test]
fn a_steady_frame_makes_no_bind_group() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let scene = busy_scene(128);
    for _ in 0..3 {
        gpu.draw_and_wait(&scene, &Changed::all(), &mut text);
        gpu.end_frame();
    }
    let made = gpu.bind_groups_made();
    for _ in 0..5 {
        gpu.draw_and_wait(&scene, &Changed::all(), &mut text);
        gpu.end_frame();
    }
    assert_eq!(gpu.bind_groups_made(), made, "steady frames reuse every bind group");
}

/// Icons of many sizes, more than one small page holds.
fn many_icons() -> Scene {
    let mut scene = Scene::new((256, 256), 1.0, Some(Color::WHITE));
    for (i, size) in (10_u32..40).enumerate() {
        let i = i32::try_from(i).unwrap_or(0);
        scene.push(vec![Item::Icon {
            icon: Icon::ALL
                .get(
                    usize::try_from(i)
                        .unwrap_or(0)
                        .checked_rem(Icon::ALL.len())
                        .unwrap_or(0),
                )
                .copied()
                .unwrap_or(Icon::X),
            size,
            color: Color::BLACK,
            stroke_width: 2.0,
            x: (i % 6).saturating_mul(42),
            y: (i / 6).saturating_mul(42),
        }]);
    }
    scene
}

#[test]
fn a_full_atlas_adds_a_page_and_rasterizes_nothing_again() {
    let mut gpu = GpuRenderer::headless_with(GpuOptions {
        atlas_side: 128,
        max_pages: 16,
        ..GpuOptions::default()
    })
    .expect("a GPU");
    let mut text = Glyphs::for_os();
    let scene = many_icons();
    let first = gpu.capture(&scene, None, &mut text).expect("read back");
    gpu.end_frame();
    assert!(gpu.mono.page_count() > 1, "the icons fill more than a page");
    let rasters = gpu.rasters();
    assert_eq!(rasters, 30, "each icon once");
    for _ in 0..3 {
        let again = gpu.capture(&scene, None, &mut text).expect("read back");
        gpu.end_frame();
        assert_eq!(again.data(), first.data(), "the same pixels from the grown atlas");
    }
    assert_eq!(gpu.rasters(), rasters, "a full atlas does not rasterize again");
}

#[test]
fn an_atlas_at_its_most_pages_evicts_what_is_not_drawn() {
    let mut gpu = GpuRenderer::headless_with(GpuOptions {
        atlas_side: 128,
        max_pages: 3,
        ..GpuOptions::default()
    })
    .expect("a GPU");
    let mut text = Glyphs::for_os();
    let scene = many_icons();
    gpu.capture(&scene, None, &mut text);
    gpu.end_frame();
    assert_eq!(gpu.mono.page_count(), 3);
    // Another set, while the first is not drawn: pages are emptied for it.
    let mut other = Scene::new((128, 128), 1.0, Some(Color::WHITE));
    other.push(icons(100));
    let shot = gpu.capture(&other, None, &mut text).expect("read back");
    assert!(
        at(&shot, 60 + 50, 4 + 50)[0] < 255 || at(&shot, 60 + 8, 4 + 8)[0] < 255,
        "drawn"
    );
}

#[test]
fn an_icon_is_drawn_from_the_atlas_and_as_paths_when_turned() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let icon = Item::Icon {
        icon: Icon::Plus,
        size: 24,
        color: Color::BLACK,
        stroke_width: 2.0,
        x: 20,
        y: 20,
    };
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    scene.push(vec![icon.clone()]);
    let flat = gpu.capture(&scene, None, &mut text).expect("read back");
    assert_eq!(at(&flat, 32, 32), [0, 0, 0, 255], "the plus's centre");
    assert_eq!(at(&flat, 22, 22), [255; 4], "its corner is empty");
    // The same icon in a group a hair off scale 1: drawn as paths, the same.
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    group(
        &mut scene,
        vec![icon],
        PixelRect {
            x: 20,
            y: 20,
            width: 24,
            height: 24,
        },
        GroupTransform {
            origin: (32.0, 32.0),
            scale: 1.000_01,
            rotate: 0.0,
        },
    );
    let paths = gpu.capture(&scene, None, &mut text).expect("read back");
    for y in 0..64 {
        for x in 0..64 {
            let (a, b) = (at(&flat, x, y)[0], at(&paths, x, y)[0]);
            assert!(a.abs_diff(b) <= 2, "({x}, {y}): {a} from the atlas, {b} as paths");
        }
    }
}

/// A dash pattern leaves gaps where it says.
#[test]
fn a_dashed_line_has_gaps() {
    let mut gpu = gpu();
    let path = Path::builder().move_to(8.0, 48.0).line_to(88.0, 48.0).build();
    let style = PathStyle::stroke(
        Gradient::Solid(Color::WHITE),
        Stroke::new(4.0).dash(vec![10.0, 10.0], 0.0),
    );
    let shot = shot_of(&mut gpu, &path, &style, (0.0, 0.0));
    assert_eq!(at(&shot, 13, 48)[0], 255, "in the first dash");
    assert_eq!(at(&shot, 23, 48)[0], 0, "in the first gap");
    assert_eq!(at(&shot, 33, 48)[0], 255, "in the second dash");
}

/// Edges under a scale move with it and never stick to pixels (ADR-0020).
#[test]
fn a_continuous_zoom_does_not_jitter() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut last = 0.0;
    for step in 0..40_u8 {
        let k = 1.0 + f32::from(step) * 0.005;
        let mut scene = Scene::new((128, 16), 1.0, Some(Color::BLACK));
        group(
            &mut scene,
            vec![Item::Rect {
                bounds: rect(10.0, 0.0, 50.3, 16.0),
                color: Color::WHITE,
            }],
            PixelRect {
                x: 10,
                y: 0,
                width: 51,
                height: 16,
            },
            GroupTransform {
                origin: (0.0, 8.0),
                scale: k,
                rotate: 0.0,
            },
        );
        let shot = gpu.capture(&scene, None, &mut text).expect("read back");
        gpu.end_frame();
        let width: f32 = (0..128).map(|x| f32::from(at(&shot, x, 8)[0]) / 255.0).sum();
        let want = 50.3 * k;
        assert!(
            (width - want).abs() < 0.02,
            "at scale {k} the rect is {width} wide, not {want}"
        );
        assert!(width > last, "it grows with every step");
        last = width;
    }
}

/// Dual-source blending off (an adapter without it): everything blends by
/// its alpha and is drawn as with it; subpixel text falls back to grey.
#[test]
fn every_primitive_draws_the_same_without_dual_source_blending() {
    let mut with = gpu();
    let mut without = GpuRenderer::headless_with(GpuOptions {
        dual_source: false,
        ..GpuOptions::default()
    })
    .expect("a GPU");
    assert!(!without.dual_source());
    let scene = busy_scene(128);
    let a = with.capture(&scene, None, &mut Glyphs::for_os()).expect("read back");
    let b = without.capture(&scene, None, &mut Glyphs::for_os()).expect("read back");
    for (p, q) in a.data().iter().zip(b.data()) {
        assert!(p.abs_diff(*q) <= 1, "the same pixels either way");
    }
}

fn glyph_scene(text: &mut Glyphs, ppem: f32, transform: Option<GroupTransform>) -> (Scene, GlyphKey) {
    let key = text.test_glyph(ppem).expect("a glyph");
    let size = (crate::convert::f32_to_u32(ppem * 1.4)).max(32);
    let mut scene = Scene::new((size, size), 1.0, Some(Color::WHITE));
    let glyph = Item::Glyph {
        key,
        x: crate::convert::f32_to_i32(ppem * 0.2),
        y: crate::convert::f32_to_i32(ppem * 0.9),
        color: Color::BLACK,
    };
    match transform {
        None => scene.push(vec![glyph]),
        Some(t) => group(
            &mut scene,
            vec![glyph],
            PixelRect {
                x: 0,
                y: 0,
                width: size,
                height: size,
            },
            t,
        ),
    }
    (scene, key)
}

/// Curve text against the exact winding of its curves: solid inside, empty
/// outside, only edges in between.
fn check_curves(scene: &Scene, key: &GlyphKey, text: &mut Glyphs, m: [f32; 6]) {
    let mut gpu = gpu();
    let shot = gpu.capture(scene, None, text).expect("read back");
    if let Ok(dir) = std::env::var("GUI_TEST_DUMP") {
        drop(shot.save_png(format!("{dir}/slug-{}.png", scene.size.0)));
    }
    let curves = text.curves(key).expect("curves");
    let inv = path::invert(m).expect("invertible");
    let (w, h) = scene.size;
    let mut edges: u32 = 0;
    for y in 0..h {
        for x in 0..w {
            let v = at(&shot, x, y)[0];
            // The winding at the centre and a pixel's reach around it.
            let inside = |dx: f32, dy: f32| {
                let p = path::apply(inv, [u32_to_f32(x) + 0.5 + dx, u32_to_f32(y) + 0.5 + dy]);
                slug::winding(&curves, [f64::from(p[0]), f64::from(p[1])]) != 0
            };
            let c = inside(0.0, 0.0);
            // Within a pixel and a half of an edge, coverage is partial.
            let near = (0..32_u8).any(|i| {
                let a = f32::from(i % 16) * std::f32::consts::PI / 8.0;
                let r = if i < 16 { 1.5 } else { 0.75 };
                inside(a.cos() * r, a.sin() * r) != c
            });
            if near {
                edges = edges.saturating_add(1);
                continue;
            }
            if c {
                assert!(v <= 2, "({x}, {y}) is inside the glyph, but {v}");
            } else {
                assert!(v >= 253, "({x}, {y}) is outside the glyph, but {v}");
            }
        }
    }
    assert!(edges > 0, "the glyph was found");
}

#[test]
fn large_text_is_drawn_from_its_curves() {
    let mut text = Glyphs::for_os();
    let (scene, key) = glyph_scene(&mut text, 200.0, None);
    let Some(Item::Glyph { x, y, .. }) = scene.items().next().cloned() else {
        panic!()
    };
    check_curves(
        &scene,
        &key,
        &mut text,
        glyph_matrix(&key, i32_to_f32(x), i32_to_f32(y), None),
    );
}

#[test]
fn turned_text_is_drawn_from_its_curves() {
    let mut text = Glyphs::for_os();
    let t = GroupTransform {
        origin: (40.0, 40.0),
        scale: 1.3,
        rotate: 30.0,
    };
    let (scene, key) = glyph_scene(&mut text, 56.0, Some(t));
    let Some(Item::Glyph { x, y, .. }) = scene.items().next().cloned() else {
        panic!()
    };
    check_curves(
        &scene,
        &key,
        &mut text,
        glyph_matrix(&key, i32_to_f32(x), i32_to_f32(y), Some(t.matrix())),
    );
}

#[test]
fn banded_curve_text_matches_the_full_scan_here() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    assert!(
        gpu.slug_self_test(&mut text),
        "this adapter draws banded curve text as the full scan"
    );
}

/// A scene of chunks with stable ids: `(id, state, display)`.
fn chunks(size: (u32, u32), list: &[(u64, State, &std::sync::Arc<crate::scene::Display>)]) -> Scene {
    let mut scene = Scene::new(size, 1.0, Some(Color::WHITE));
    for (id, state, display) in list {
        scene.chunks.push(crate::scene::Chunk {
            id: crate::scene::ChunkId(*id),
            state: *state,
            display: std::sync::Arc::clone(display),
        });
    }
    scene
}

fn solid(b: Bounds, color: Color) -> std::sync::Arc<crate::scene::Display> {
    std::sync::Arc::new(crate::scene::Display::new(vec![Item::Rect { bounds: b, color }]))
}

/// What damage left in the canvas is what drawing the frame whole gives.
fn canvas_is_whole(gpu: &mut GpuRenderer, scene: &Scene, text: &mut Glyphs) {
    let canvas = gpu.read_canvas().expect("a canvas");
    let whole = gpu.capture(scene, None, text).expect("read back");
    assert_eq!(
        canvas.data(),
        whole.data(),
        "the canvas after damage equals a whole redraw"
    );
}

#[test]
fn an_unchanged_scene_submits_nothing() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let a = solid(rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
    let scene = chunks((64, 64), &[(1, State::default(), &a)]);
    gpu.render(&scene, &Changed::all(), &mut text);
    assert!(gpu.drawn().submitted, "the first frame is drawn");
    gpu.end_frame();
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    assert!(!d.submitted, "nothing changed: {d}");
    assert_eq!((d.uploaded, d.damage, d.encoded), (0, 0, 0));
    gpu.end_frame();
}

/// The render thread takes a window texture at each frame's start to wait
/// for the vblank: a frame that changes nothing still presents nothing,
/// and keeps the texture for the next that does. A display that was
/// exposed is shown the canvas again, changed or not.
#[test]
fn a_frame_that_changes_nothing_presents_nothing_though_it_holds_a_texture() {
    let mut gpu = GpuRenderer::with_virtual_window((64, 64)).expect("a GPU (CI paints on lavapipe)");
    let mut text = Glyphs::for_os();
    let a = solid(rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
    let scene = chunks((64, 64), &[(1, State::default(), &a)]);
    let frame = |gpu: &mut GpuRenderer, scene: &Scene, text: &mut Glyphs| {
        assert!(gpu.acquire().is_some(), "a texture, as at a frame's start");
        gpu.render(scene, &Changed::all(), text);
        gpu.end_frame();
        gpu.presents()
    };
    assert_eq!(frame(&mut gpu, &scene, &mut text), 1, "the first frame is presented");
    for _ in 0..3 {
        assert_eq!(
            frame(&mut gpu, &scene, &mut text),
            1,
            "nothing changed: nothing presented"
        );
        assert!(
            gpu.holds_frame(),
            "the texture is kept for the next frame that presents"
        );
    }
    gpu.expose();
    assert_eq!(frame(&mut gpu, &scene, &mut text), 2, "exposed: shown the canvas again");
    assert_eq!(frame(&mut gpu, &scene, &mut text), 2);
    let b = solid(rect(10.0, 10.0, 20.0, 20.0), Color::hex(0x2563eb));
    let changed = chunks((64, 64), &[(1, State::default(), &b)]);
    assert_eq!(frame(&mut gpu, &changed, &mut text), 3, "damage is presented");
    assert!(!gpu.holds_frame());
}

/// A window dragged larger is configured to its new size at once, as
/// Vulkan on X11 and Win32 wants (a swapchain not the window's size is out
/// of date there), and the last frame keeps being presented, at its own
/// size in the corner, until the UI commits one laid out at the new size.
#[test]
fn a_window_dragged_larger_is_configured_once_and_presents_before_the_ui_commits() {
    let mut gpu = GpuRenderer::with_virtual_window((64, 64)).expect("a GPU (CI paints on lavapipe)");
    gpu.virtual_display().strict = true;
    let mut text = Glyphs::for_os();
    let a = solid(rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
    let old = chunks((64, 64), &[(1, State::default(), &a)]);
    let frame = |gpu: &mut GpuRenderer, scene: &Scene, text: &mut Glyphs| {
        gpu.acquire();
        gpu.render(scene, &Changed::all(), text);
        gpu.end_frame();
    };
    frame(&mut gpu, &old, &mut text);
    let configured = gpu.virtual_display().configures;
    for size in [(80, 72), (96, 80), (128, 96)] {
        gpu.virtual_display().window = size;
        gpu.resize(size);
        // No commit at the new size yet: the old scene again.
        frame(&mut gpu, &old, &mut text);
        assert_eq!(gpu.virtual_display().presented.last(), Some(&size), "presented at once");
    }
    let shown = gpu.virtual_display().presented.len();
    frame(&mut gpu, &old, &mut text);
    assert_eq!(gpu.virtual_display().presented.len(), shown, "and only once");
    assert_eq!(
        gpu.virtual_display().configures - configured,
        3,
        "configured once a size"
    );
    // The UI's frame at the new size presents with no configure.
    frame(&mut gpu, &chunks((128, 96), &[(1, State::default(), &a)]), &mut text);
    assert_eq!(gpu.virtual_display().presented.len(), shown + 1);
    assert_eq!(gpu.virtual_display().configures - configured, 3);
}

/// The last frame, at its own size in the corner of a window resized: red
/// where it drew, the window's background (the root's: dark) where a
/// larger window shows nothing of it, cut where a smaller one ends.
#[test]
fn a_resized_window_shows_the_last_frame_in_its_corner_over_the_roots_background() {
    let mut gpu = GpuRenderer::with_virtual_window((64, 48)).expect("a GPU (CI paints on lavapipe)");
    let mut text = Glyphs::for_os();
    let dark = Color::hex(0x09090b);
    let red = Color::hex(0xff0000);
    let a = solid(rect(0.0, 0.0, 64.0, 48.0), dark);
    let b = solid(rect(40.0, 30.0, 24.0, 18.0), red);
    let mut scene = chunks((64, 48), &[(1, State::default(), &a), (2, State::default(), &b)]);
    scene.background = Some(dark);
    let frame = |gpu: &mut GpuRenderer, text: &mut Glyphs| {
        gpu.acquire();
        gpu.render(&scene, &Changed::all(), text);
        gpu.end_frame();
        gpu.read_shown().expect("a frame shown")
    };
    frame(&mut gpu, &mut text);
    let dark_px = [9, 9, 11, 255];
    gpu.virtual_display().window = (100, 70);
    gpu.resize((100, 70));
    let grown = frame(&mut gpu, &mut text);
    assert_eq!((grown.width(), grown.height()), (100, 70));
    assert_eq!(at(&grown, 50, 40), [255, 0, 0, 255], "the last frame, in the corner");
    assert_eq!(
        at(&grown, 90, 60),
        dark_px,
        "beyond it, the root's background, not white"
    );
    assert_eq!(at(&grown, 10, 60), dark_px);
    gpu.virtual_display().window = (48, 36);
    gpu.resize((48, 36));
    let shrunk = frame(&mut gpu, &mut text);
    assert_eq!((shrunk.width(), shrunk.height()), (48, 36));
    assert_eq!(
        at(&shrunk, 45, 33),
        [255, 0, 0, 255],
        "the corner of the last frame, cut"
    );
    assert_eq!(at(&shrunk, 10, 10), dark_px);
}

/// An overlay over a window resized before the UI laid out at its size is
/// drawn at the frame's size, in the corner with it: not stretched to the
/// window's.
#[test]
fn an_overlay_over_a_resized_window_stays_with_the_frame_in_its_corner() {
    let mut gpu = GpuRenderer::with_virtual_window((64, 64)).expect("a GPU (CI paints on lavapipe)");
    let mut text = Glyphs::for_os();
    let a = solid(rect(0.0, 0.0, 64.0, 64.0), Color::WHITE);
    let scene = chunks((64, 64), &[(1, State::default(), &a)]);
    let o = solid(rect(20.0, 20.0, 10.0, 10.0), Color::hex(0x0000ff));
    let overlay = chunks((64, 64), &[(9, State::default(), &o)]);
    gpu.virtual_display().window = (128, 128);
    gpu.resize((128, 128));
    gpu.acquire();
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.overlay_as_of(&overlay, 1, &mut text);
    gpu.end_frame();
    let shown = gpu.read_shown().expect("a frame shown");
    assert_eq!(
        at(&shown, 25, 25),
        [0, 0, 255, 255],
        "where the overlay is over the frame"
    );
    assert_eq!(
        at(&shown, 50, 50),
        [255, 255, 255, 255],
        "not stretched over the window"
    );
}

/// A frame meant to present whose texture could not be taken (the acquire
/// timed out, at the frame's start and again when drawing) leaves the
/// presentation owed: the next frame presents, though nothing changed.
#[test]
fn a_present_whose_texture_timed_out_is_owed_to_the_next_frame() {
    let mut gpu = GpuRenderer::with_virtual_window((64, 64)).expect("a GPU (CI paints on lavapipe)");
    let mut text = Glyphs::for_os();
    let a = solid(rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
    let b = solid(rect(10.0, 10.0, 20.0, 20.0), Color::hex(0x2563eb));
    let (first, changed) = (
        chunks((64, 64), &[(1, State::default(), &a)]),
        chunks((64, 64), &[(1, State::default(), &b)]),
    );
    assert!(gpu.acquire().is_some());
    gpu.render(&first, &Changed::all(), &mut text);
    gpu.end_frame();
    assert_eq!(gpu.presents(), 1);
    assert!(!gpu.owes_present(), "the window shows the canvas");
    gpu.virtual_display().faults.extend([Fault::Timeout, Fault::Timeout]);
    assert_eq!(gpu.acquire(), None, "timed out at the frame's start");
    gpu.render(&changed, &Changed::all(), &mut text);
    gpu.end_frame();
    assert_eq!(gpu.presents(), 1, "no texture: nothing presented");
    assert!(gpu.owes_present(), "the change is owed to the window");
    assert!(gpu.acquire().is_some());
    gpu.render(&changed, &Changed::all(), &mut text);
    gpu.end_frame();
    assert_eq!(
        gpu.presents(),
        2,
        "the next frame presents it, though nothing changed since"
    );
    assert!(!gpu.owes_present());
}

/// An out-of-date surface is configured again; a lost one is made again
/// (from its window) and configured: either way the frame gets its texture.
#[test]
fn an_outdated_surface_is_configured_again_and_a_lost_one_made_again() {
    let mut gpu = GpuRenderer::with_virtual_window((64, 64)).expect("a GPU (CI paints on lavapipe)");
    let configured = gpu.virtual_display().configures;
    gpu.virtual_display().faults.push_back(Fault::Outdated);
    assert!(gpu.acquire().is_some(), "outdated: configured and taken again");
    assert_eq!(gpu.virtual_display().configures - configured, 1);
    assert_eq!(gpu.virtual_display().remade, 0, "an outdated surface is not made again");
    let mut text = Glyphs::for_os();
    let a = solid(rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
    gpu.render(
        &chunks((64, 64), &[(1, State::default(), &a)]),
        &Changed::all(),
        &mut text,
    );
    gpu.end_frame();
    gpu.virtual_display().faults.push_back(Fault::Lost);
    assert!(gpu.acquire().is_some(), "lost: made again and taken");
    assert_eq!(gpu.virtual_display().remade, 1);
    assert_eq!(gpu.virtual_display().configures - configured, 2);
}

/// The vsync is timed by the acquire that gave the texture only: an
/// out-of-date surface configured slowly before it is no vblank waited for.
#[test]
fn the_wait_measured_is_the_acquire_that_gave_the_texture_not_the_configure_before_it() {
    let mut gpu = GpuRenderer::with_virtual_window((64, 64)).expect("a GPU (CI paints on lavapipe)");
    let v = gpu.virtual_display();
    v.configure_takes = std::time::Duration::from_millis(20);
    v.faults.push_back(Fault::Outdated);
    let waited = gpu.acquire().expect("a texture after the configure");
    assert!(
        waited < std::time::Duration::from_millis(5),
        "the configure is not a wait for the display: {waited:?}"
    );
}

/// An overlay unchanged over frames that change nothing (an animation
/// holding still) presents nothing; one that changed presents, and so does
/// one hidden, once.
#[test]
fn an_unchanged_overlay_over_an_unchanged_frame_presents_nothing() {
    let mut gpu = GpuRenderer::with_virtual_window((64, 64)).expect("a GPU (CI paints on lavapipe)");
    let mut text = Glyphs::for_os();
    let a = solid(rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
    let scene = chunks((64, 64), &[(1, State::default(), &a)]);
    let o = solid(rect(30.0, 30.0, 10.0, 10.0), Color::hex(0x2563eb));
    let overlay = chunks((64, 64), &[(9, State::default(), &o)]);
    let frame = |gpu: &mut GpuRenderer, generation: Option<u64>, text: &mut Glyphs| {
        gpu.acquire();
        gpu.render(&scene, &Changed::all(), text);
        if let Some(g) = generation {
            gpu.overlay_as_of(&overlay, g, text);
        }
        gpu.end_frame();
        gpu.presents()
    };
    assert_eq!(frame(&mut gpu, Some(1), &mut text), 1);
    for _ in 0..3 {
        assert_eq!(
            frame(&mut gpu, Some(1), &mut text),
            1,
            "the same overlay: nothing presented"
        );
    }
    assert_eq!(frame(&mut gpu, Some(2), &mut text), 2, "another overlay is presented");
    assert_eq!(frame(&mut gpu, None, &mut text), 3, "hidden: presented once");
    assert_eq!(frame(&mut gpu, None, &mut text), 3);
}

/// A machine with two GPUs draws the UI on the one that uses least power,
/// unless `WGPU_POWER_PREF` says otherwise.
#[test]
fn the_adapter_asked_for_uses_least_power() {
    if std::env::var_os("WGPU_POWER_PREF").is_some() {
        eprintln!("WGPU_POWER_PREF is set: skipped");
        return;
    }
    assert_eq!(adapter_options(None).power_preference, wgpu::PowerPreference::LowPower);
}

/// Golden damage: a chunk recorded again redraws its rectangle, its
/// anti-aliased edge included, and nothing else; a moved one its old and
/// new places; the canvas then is what a whole redraw gives.
#[test]
fn damage_is_what_changed_and_the_canvas_stays_whole() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let a = solid(rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
    let b = solid(rect(200.0, 150.0, 30.0, 10.0), Color::hex(0x2563eb));
    let scene = chunks((256, 192), &[(1, State::default(), &a), (2, State::default(), &b)]);
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    // `b` recolored: 30x10 and a pixel around.
    let b2 = solid(rect(200.0, 150.0, 30.0, 10.0), Color::hex(0xdc2626));
    let scene = chunks((256, 192), &[(1, State::default(), &a), (2, State::default(), &b2)]);
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    assert_eq!((d.damage, d.damage_rects, d.encoded), (32 * 12, 1, 1), "{d}");
    gpu.end_frame();
    canvas_is_whole(&mut gpu, &scene, &mut text);
    // `a` moved by a transform: its old and new places, recorded as it was;
    // the two are close, so one rectangle over both is cheaper than two.
    let mut scene = chunks((256, 192), &[(1, State::default(), &a), (2, State::default(), &b2)]);
    let t = scene.add_transform((100.0, 0.0), None);
    if let Some(c) = scene.chunks.first_mut() {
        c.state.transform = t;
    }
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    assert_eq!((d.damage, d.damage_rects, d.encoded), (122 * 22, 1, 0), "{d}");
    gpu.end_frame();
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// Recording one chunk again uploads its records alone; a property
/// change uploads a table entry.
#[test]
fn uploads_are_what_changed() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let many: Vec<std::sync::Arc<crate::scene::Display>> = (0..200_u16)
        .map(|i| {
            let (x, y) = (f32::from(i % 20) * 12.0, f32::from(i / 20) * 12.0);
            solid(rect(x, y, 10.0, 10.0), Color::BLACK)
        })
        .collect();
    let list: Vec<(u64, State, &std::sync::Arc<crate::scene::Display>)> =
        (1_u64..).zip(&many).map(|(id, d)| (id, State::default(), d)).collect();
    let scene = chunks((256, 128), &list);
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let first = gpu.drawn().uploaded;
    let other = solid(rect(0.0, 0.0, 10.0, 10.0), Color::WHITE);
    let mut list2 = list.clone();
    if let Some(l) = list2.first_mut() {
        l.2 = &other;
    }
    let scene = chunks((256, 128), &list2);
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    // A rectangle is 8 words; its colour is two of them.
    assert!(
        d.uploaded <= 64,
        "{} bytes uploaded for one chunk (the first frame: {first})",
        d.uploaded
    );
    let mut scene = chunks((256, 128), &list2);
    let t = scene.add_transform((1.0, 0.0), None);
    if let Some(c) = scene.chunks.get_mut(5) {
        c.state.transform = t;
    }
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    assert_eq!(d.encoded, 0, "a move encodes nothing");
    assert!(d.uploaded <= 64, "{} bytes uploaded for a move", d.uploaded);
}

/// An effect over primitives that do not overlap is folded into them; over
/// ones that do, it gets a layer, kept while what it holds is the same.
#[test]
fn a_fade_is_folded_unless_what_it_holds_overlaps() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let fade = |scene: &mut Scene, opacity: f32, offset: f32| {
        let t = scene.add_transform((offset, 0.0), None);
        let e = scene.add_effect(EffectNode {
            opacity,
            transform: t,
            region: Some(PixelRect {
                x: 0,
                y: 0,
                width: 128,
                height: 64,
            }),
            ..EffectNode::default()
        });
        for c in &mut scene.chunks {
            c.state = State {
                transform: t,
                clip: 0,
                effect: e,
            };
        }
    };
    let a = solid(rect(10.0, 10.0, 20.0, 20.0), Color::BLACK);
    let apart = solid(rect(40.0, 10.0, 20.0, 20.0), Color::BLACK);
    let over = solid(rect(20.0, 20.0, 20.0, 20.0), Color::BLACK);
    let mut scene = chunks((128, 64), &[(1, State::default(), &a), (2, State::default(), &apart)]);
    fade(&mut scene, 0.5, 0.0);
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    assert_eq!((d.folded, d.layers), (1, 0), "apart: folded, {d}");
    let shot = gpu.capture(&scene, None, &mut text).expect("read back");
    // 127.5 either way: Metal rounds it up, lavapipe down.
    assert!(
        half_grey(at(&shot, 20, 20)),
        "half black over white: {:?}",
        at(&shot, 20, 20)
    );
    let mut scene = chunks((128, 64), &[(1, State::default(), &a), (3, State::default(), &over)]);
    fade(&mut scene, 0.5, 0.0);
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    assert_eq!((d.folded, d.layers), (0, 1), "overlapping: a layer, {d}");
    canvas_is_whole(&mut gpu, &scene, &mut text);
    let shot = gpu.capture(&scene, None, &mut text).expect("read back");
    assert!(
        half_grey(at(&shot, 25, 25)),
        "the overlap is faded once: {:?}",
        at(&shot, 25, 25)
    );
    // Moved and faded further: the layer is composited as kept.
    let mut scene = chunks((128, 64), &[(1, State::default(), &a), (3, State::default(), &over)]);
    fade(&mut scene, 0.25, 2.0);
    if let Some(e) = scene.properties.effects.get_mut(1)
        && let Some(r) = e.region.as_mut()
    {
        r.x = 2;
    }
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    assert_eq!((d.layers, d.layers_kept, d.encoded), (0, 1, 0), "{d}");
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// Black and white stripes `w` px wide over the whole of a `size` square.
fn stripes(size: f32, w: f32) -> std::sync::Arc<crate::scene::Display> {
    let mut items = Vec::new();
    let mut x = 0.0;
    while x < size {
        items.push(Item::Rect {
            bounds: rect(x, 0.0, w, size),
            color: Color::BLACK,
        });
        x += 2.0 * w;
    }
    std::sync::Arc::new(crate::scene::Display::new(items))
}

fn read(items: Vec<Item>) -> std::sync::Arc<crate::scene::Display> {
    std::sync::Arc::new(crate::scene::Display::new(items))
}

fn gray(v: f32, a: f32) -> Color {
    Color { r: v, g: v, b: v, a }
}

/// `mix-blend-mode: darken` takes the darker of each channel, faded by
/// the colour's alpha (Compositing 1): over white a half-alpha mid gray
/// gives 0.75, over black it leaves black, where a normal blend would
/// lighten black to a quarter.
#[test]
fn a_darkened_border_takes_the_darker_of_each_channel() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let black = solid(rect(32.0, 0.0, 32.0, 32.0), Color::BLACK);
    let blended = |blend, a| {
        read(vec![Item::Blended {
            outer: rect(0.0, 0.0, 64.0, 32.0),
            outer_radius: Corners::default(),
            inner: Bounds::default(),
            inner_radius: Corners::default(),
            curve: 1.0,
            color: gray(0.5, a),
            blend,
        }])
    };
    let cases = [
        (crate::shape::Blend::Darken, 1.0, [128, 0]),
        (crate::shape::Blend::Darken, 0.5, [191, 0]),
        (crate::shape::Blend::Lighten, 1.0, [255, 128]),
        (crate::shape::Blend::Lighten, 0.5, [255, 64]),
    ];
    for (blend, a, want) in cases {
        let top = blended(blend, a);
        let scene = chunks((64, 32), &[(1, State::default(), &black), (2, State::default(), &top)]);
        let shot = gpu.capture(&scene, None, &mut text).expect("read back");
        let got = [at(&shot, 8, 16)[0], at(&shot, 48, 16)[0]];
        for (g, w) in got.iter().zip(want) {
            assert!(g.abs_diff(w) <= 1, "{blend:?} at alpha {a}: {got:?}, not {want:?}");
        }
    }
}

/// A backdrop shows what lies below its box blurred (stripes become their
/// mean), at a small sigma straight and at a large one through halvings;
/// outside its box nothing changes.
#[test]
fn a_backdrop_blurs_what_lies_below_inside_its_box() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let below = stripes(96.0, 1.0);
    for blur in [3.0, 24.0] {
        let top = read(vec![Item::Backdrop {
            bounds: rect(16.0, 16.0, 64.0, 64.0),
            radius: Corners::default(),
            curve: 1.0,
            backdrop: crate::shape::Backdrop::blur(blur),
        }]);
        let scene = chunks((96, 96), &[(1, State::default(), &below), (2, State::default(), &top)]);
        let shot = gpu.capture(&scene, None, &mut text).expect("read back");
        for (x, y) in [(48, 48), (20, 40), (75, 75)] {
            let v = at(&shot, x, y)[0];
            assert!(v.abs_diff(128) <= 8, "blur {blur} at ({x}, {y}): {v}");
        }
        assert_eq!(at(&shot, 4, 4)[0], 0, "outside the box");
        assert_eq!(at(&shot, 5, 4)[0], 255, "outside the box");
    }
}

/// `saturate(2)` of Filter Effects 1, on what lies below.
#[test]
fn a_backdrop_saturates() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let c = Color::hex(0xc86432);
    let below = solid(rect(0.0, 0.0, 32.0, 32.0), c);
    let top = read(vec![Item::Backdrop {
        bounds: rect(0.0, 0.0, 32.0, 32.0),
        radius: Corners::default(),
        curve: 1.0,
        backdrop: crate::shape::Backdrop {
            blur: 0.0,
            saturate: 2.0,
        },
    }]);
    let scene = chunks((32, 32), &[(1, State::default(), &below), (2, State::default(), &top)]);
    let shot = gpu.capture(&scene, None, &mut text).expect("read back");
    let k = 2.0_f32;
    let rgb = [c.r, c.g, c.b];
    let rows = [
        [0.213 + 0.787 * k, 0.715 - 0.715 * k, 0.072 - 0.072 * k],
        [0.213 - 0.213 * k, 0.715 + 0.285 * k, 0.072 - 0.072 * k],
        [0.213 - 0.213 * k, 0.715 - 0.715 * k, 0.072 + 0.928 * k],
    ];
    let got = at(&shot, 16, 16);
    for (i, row) in rows.iter().enumerate() {
        let v = row.iter().zip(rgb).map(|(m, c)| m * c).sum::<f32>().clamp(0.0, 1.0) * 255.0;
        let g = f32::from(got[i]);
        assert!((g - v).abs() <= 2.0, "channel {i}: {g}, not {v}");
    }
}

/// Damage that meets a blurred backdrop redraws its whole box (the blur
/// reads all of it), so the canvas stays what a whole redraw gives.
#[test]
fn damage_under_a_blurred_backdrop_redraws_its_box() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let below = stripes(128.0, 2.0);
    let top = read(vec![Item::Backdrop {
        bounds: rect(32.0, 32.0, 64.0, 48.0),
        radius: Corners::all(12.0),
        curve: 2.0,
        backdrop: crate::shape::Backdrop::blur(6.0),
    }]);
    let red = solid(rect(36.0, 36.0, 4.0, 4.0), Color::hex(0xdc2626));
    let blue = solid(rect(36.0, 36.0, 4.0, 4.0), Color::hex(0x2563eb));
    let scene = |spot| {
        chunks(
            (128, 128),
            &[
                (1, State::default(), &below),
                (2, State::default(), spot),
                (3, State::default(), &top),
            ],
        )
    };
    gpu.render(&scene(&red), &Changed::all(), &mut text);
    gpu.end_frame();
    let next = scene(&blue);
    gpu.render(&next, &Changed::all(), &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    assert!(d.damage >= 64 * 48, "the box is redrawn whole: {d}");
    canvas_is_whole(&mut gpu, &next, &mut text);
}

/// A clip within another ([`crate::scene::ClipNode::parent`]): the shaders
/// walk each clip's chain, so when the outer one alone changes (the
/// compositor moving it), its entry alone is written, and what the inner
/// one clips is drawn again, cut anew.
#[test]
fn a_clip_within_another_cuts_anew_when_the_outer_one_alone_changes() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let clip = |h: f32| Clip {
        bounds: rect(0.0, 0.0, 64.0, h),
        radius: Corners::default(),
        curve: crate::shape::ROUND,
    };
    let outer = scene.add_clip(clip(40.0), 0);
    let inner = scene.add_clip(clip(64.0), 0);
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(inner).unwrap_or(0)) {
        n.parent = outer;
    }
    scene.push_under(
        State {
            clip: inner,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let ink = |p: &Pixels, y: u32| p.pixel(10, y).is_some_and(|c| c[0] < 128);
    let first = gpu.read_canvas().expect("a canvas");
    assert!(ink(&first, 30) && !ink(&first, 50), "cut at the outer clip's 40");
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(outer).unwrap_or(0)) {
        n.clip.bounds = rect(0.0, 0.0, 64.0, 20.0);
    }
    let changed = Changed {
        clips: vec![outer],
        ..Changed::default()
    };
    gpu.render(&scene, &changed, &mut text);
    gpu.end_frame();
    assert_eq!(gpu.drawn().clips_written, 1, "the outer clip's entry alone");
    let next = gpu.read_canvas().expect("a canvas");
    assert!(ink(&next, 10) && !ink(&next, 30), "cut at the outer clip's 20 now");
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A scene of 64 by 64 on white: a clip `outer`, and `inner` within it,
/// a black rect over all of it under `inner`, and a white star path
/// under it too, drawn by the path pipeline.
fn clipped_in_clip(outer: Clip, inner: Clip) -> Scene {
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let outer = scene.add_clip(outer, 0);
    let inner = scene.add_clip(inner, 0);
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(inner).unwrap_or(0)) {
        n.parent = outer;
    }
    let under = State {
        clip: inner,
        ..State::default()
    };
    scene.push_under(
        under,
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    // A path over the bottom left corner (the path pipeline's clips).
    let path = Path::builder()
        .move_to(0.0, 40.0)
        .line_to(24.0, 40.0)
        .line_to(24.0, 64.0)
        .line_to(0.0, 64.0)
        .close()
        .build();
    scene.push_under(
        under,
        vec![Item::Path {
            origin: (0.0, 0.0),
            path,
            style: PathStyle::fill(Gradient::Solid(Color::hex(0xdc2626))),
        }],
    );
    scene
}

/// A square clip (a text field's) within a rounded one (an overflow
/// container's): the container's corners stay as the page was, under
/// the quads and the paths alike; inside, all is drawn.
#[test]
fn a_square_clip_in_a_rounded_one_leaves_the_rounded_ones_corners_untouched() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let scene = clipped_in_clip(
        Clip {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            radius: Corners::all(16.0),
            curve: crate::shape::ROUND,
        },
        Clip {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    for (x, y) in [(1, 1), (62, 1), (62, 62), (1, 62)] {
        assert_eq!(
            at(&p, x, y),
            [255, 255, 255, 255],
            "the container's corner at ({x}, {y})"
        );
    }
    assert_eq!(at(&p, 32, 32), [0, 0, 0, 255], "inside: drawn");
    assert_eq!(at(&p, 12, 52), [0xdc, 0x26, 0x26, 255], "the path, inside");
}

/// A rounded clip (a text field's) its container cuts square across: its
/// own corners mask it, and the cut is a straight edge, no corner on it.
#[test]
fn a_rounded_clip_cut_by_a_square_one_keeps_its_corners_on_its_own_corners() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let scene = clipped_in_clip(
        Clip {
            bounds: rect(0.0, 0.0, 64.0, 32.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        Clip {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            radius: Corners::all(12.0),
            curve: crate::shape::ROUND,
        },
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 1, 1), [255, 255, 255, 255], "its own top left corner");
    assert_eq!(at(&p, 62, 1), [255, 255, 255, 255], "its own top right corner");
    for x in [0, 1, 62, 63] {
        assert_eq!(at(&p, x, 31), [0, 0, 0, 255], "on the cut at ({x}, 31): no corner");
    }
    assert_eq!(at(&p, 32, 40), [255, 255, 255, 255], "past the cut: none");
}

/// Fifty fields, each its own clip within a container's that moves each
/// frame: each frame writes the container's entry alone and signs no
/// chunk again, however many fields lie in it; its damage, what the
/// container showed and shows, draws its corners where they now are.
#[test]
fn moving_a_clip_over_many_fields_writes_one_clip_entry_a_frame() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((128, 128), 1.0, Some(Color::WHITE));
    let outer = scene.add_clip(
        Clip {
            bounds: rect(0.0, 0.0, 128.0, 64.0),
            radius: Corners::all(8.0),
            curve: crate::shape::ROUND,
        },
        0,
    );
    for k in 0..50_u16 {
        let y = f32::from(k) * 2.0;
        let field = scene.add_clip(
            Clip {
                bounds: rect(0.0, y, 128.0, 2.0),
                radius: Corners::default(),
                curve: crate::shape::ROUND,
            },
            0,
        );
        if let Some(n) = scene.properties.clips.get_mut(usize::try_from(field).unwrap_or(0)) {
            n.parent = outer;
        }
        scene.push_under(
            State {
                clip: field,
                ..State::default()
            },
            vec![Item::Rect {
                bounds: rect(0.0, y, 128.0, 2.0),
                color: Color::BLACK,
            }],
        );
    }
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    for frame in 1..=3_u8 {
        let h = 64.0 + f32::from(frame) * 8.0;
        if let Some(n) = scene.properties.clips.get_mut(usize::try_from(outer).unwrap_or(0)) {
            n.clip.bounds = rect(0.0, 0.0, 128.0, h);
        }
        let changed = Changed {
            clips: vec![outer],
            ..Changed::default()
        };
        gpu.render(&scene, &changed, &mut text);
        let d = gpu.drawn();
        assert_eq!(d.clips_written, 1, "frame {frame}: one entry");
        assert!(!d.planned_whole, "frame {frame}: planned from what changed");
        assert_eq!(d.signed, 0, "frame {frame}: no chunk signed again");
        gpu.end_frame();
        // The corners went down with it: where the last frame's bottom
        // corners were is drawn now, and where they are now is not.
        let p = gpu.read_canvas().expect("a canvas");
        let bottom = crate::convert::f32_to_u32(h) - 1;
        if frame > 1 {
            assert_eq!(
                at(&p, 1, bottom - 8),
                [0, 0, 0, 255],
                "frame {frame}: the old corner, drawn"
            );
        }
        assert_eq!(at(&p, 1, bottom), [255, 255, 255, 255], "frame {frame}: the new corner");
        canvas_is_whole(&mut gpu, &scene, &mut text);
    }
}

/// A chain of `n` clips and a black rect over all under the innermost
/// (`clips[n - 1]`): the first square over the window, the second square
/// from 1 to 40 across, and every other rounded with its corners off the
/// window (so in the chain the shaders walk, the second `n - 2` clips up
/// it). The scene, and the clips' ids, outermost first.
fn nested_clips(n: u16) -> (Scene, Vec<u32>) {
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let mut ids = Vec::new();
    let mut up = 0;
    for k in 0..n {
        let (bounds, radius) = match k {
            0 => (rect(0.0, 0.0, 64.0, 64.0), Corners::default()),
            1 => (rect(1.0, 1.0, 39.0, 63.0), Corners::default()),
            _ => (rect(-8.0, -8.0, 80.0, 80.0), Corners::all(2.0)),
        };
        let id = scene.add_clip(
            Clip {
                bounds,
                radius,
                curve: crate::shape::ROUND,
            },
            0,
        );
        if let Some(c) = scene.properties.clips.get_mut(usize::try_from(id).unwrap_or(0)) {
            c.parent = up;
        }
        ids.push(id);
        up = id;
    }
    scene.push_under(
        State {
            clip: up,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    (scene, ids)
}

/// Sixty-six nested clips: the shaders walk each chain to its end (no
/// depth they stop at), so the second, 64 clips up the innermost's chain,
/// still cuts at 1 and 40; the outermost, left out of the chains it
/// contains, moving to cut alone cuts what the innermost holds (its
/// children decided again, and theirs as they changed).
#[test]
fn sixty_six_nested_clips_intersect_and_the_outermost_moving_moves_the_leaf() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let (mut scene, ids) = nested_clips(66);
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    assert_eq!(gpu.clip_tree.walk_len(ids.last().copied().unwrap_or(0)), 65);
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 0, 50), [255, 255, 255, 255], "before the second: not drawn");
    assert_eq!(at(&p, 2, 50), [0, 0, 0, 255], "inside all: drawn");
    assert_eq!(at(&p, 38, 50), [0, 0, 0, 255], "inside all: drawn");
    assert_eq!(at(&p, 40, 50), [255, 255, 255, 255], "past the second: not drawn");
    let outer = ids.first().copied().unwrap_or(0);
    if let Some(c) = scene.properties.clips.get_mut(usize::try_from(outer).unwrap_or(0)) {
        c.clip.bounds = rect(0.0, 0.0, 36.0, 64.0);
    }
    let changed = Changed {
        clips: vec![outer],
        ..Changed::default()
    };
    gpu.render(&scene, &changed, &mut text);
    assert!(!gpu.drawn().planned_whole);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 35, 50), [0, 0, 0, 255], "inside all: drawn");
    assert_eq!(
        at(&p, 37, 50),
        [255, 255, 255, 255],
        "past the outermost, moved: not drawn"
    );
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A rounded clip seventeen deep, every clip of its chain walked, cut
/// square across by an ancestor: its own corners stay on its own
/// corners, never on the cut.
#[test]
fn a_rounded_clip_seventeen_deep_cut_by_a_square_ancestor_keeps_its_own_corners() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let mut up = 0;
    for k in 0..17_u16 {
        let (bounds, radius) = match k {
            // The cut, square, the seventeenth clip of the chain.
            0 => (rect(0.0, 0.0, 64.0, 32.0), Corners::default()),
            16 => (rect(0.0, 0.0, 64.0, 64.0), Corners::all(12.0)),
            // Rounded, so each is in the chain the shaders walk; their
            // corners off the window.
            _ => (rect(-8.0, -8.0, 80.0, 80.0), Corners::all(2.0)),
        };
        let id = scene.add_clip(
            Clip {
                bounds,
                radius,
                curve: crate::shape::ROUND,
            },
            0,
        );
        if let Some(c) = scene.properties.clips.get_mut(usize::try_from(id).unwrap_or(0)) {
            c.parent = up;
        }
        up = id;
    }
    scene.push_under(
        State {
            clip: up,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 1, 1), [255, 255, 255, 255], "its own top left corner");
    for x in [0, 1, 62, 63] {
        assert_eq!(at(&p, x, 31), [0, 0, 0, 255], "on the cut at ({x}, 31): no corner");
    }
    assert_eq!(at(&p, 32, 40), [255, 255, 255, 255], "past the cut: none");
}

/// A typical deep tree (nested containers, each square and inside the
/// last, a rounded card among them, a scroll viewport cutting): the
/// effective chain the shaders walk stays short however deep it is.
#[test]
fn the_effective_clip_chain_of_a_deep_tree_stays_short() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((256, 256), 1.0, Some(Color::WHITE));
    let mut up = 0;
    for k in 0..40_u16 {
        let d = f32::from(k);
        let (bounds, radius) = match k {
            // A card.
            10 => (rect(d, d, 200.0, 200.0), Corners::all(12.0)),
            // A viewport cutting what is in it.
            20 => (rect(d, d, 200.0, 60.0), Corners::default()),
            _ => (rect(d, d, 220.0 - d, 220.0 - d), Corners::default()),
        };
        let id = scene.add_clip(
            Clip {
                bounds,
                radius,
                curve: crate::shape::ROUND,
            },
            0,
        );
        if let Some(c) = scene.properties.clips.get_mut(usize::try_from(id).unwrap_or(0)) {
            c.parent = up;
        }
        up = id;
    }
    scene.push_under(
        State {
            clip: up,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 256.0, 256.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    // The leaf, the viewport that cuts it and the card's corners.
    assert!(d.clip_chain <= 3, "forty deep, walked: {}", d.clip_chain);
    assert_eq!(gpu.clip_tree.walk_len(up), 3);
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A clip's corners changing inside a layer that a filter keeps: the
/// layer's texture is drawn again (its content holds the clip chain's
/// revision), under a whole plan as under one from what changed.
#[test]
fn a_clip_radius_change_inside_a_cached_filter_layer_shows_the_new_corners() {
    for whole in [true, false] {
        let mut gpu = gpu();
        let mut text = Glyphs::for_os();
        let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
        let clip = scene.add_clip(
            Clip {
                bounds: rect(0.0, 0.0, 64.0, 64.0),
                radius: Corners::default(),
                curve: crate::shape::ROUND,
            },
            0,
        );
        let effect = scene.add_effect(EffectNode {
            filters: crate::style::Filters::repeat(1, crate::style::Filter::Brightness(1.0)),
            region: Some(PixelRect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            }),
            ..EffectNode::default()
        });
        scene.push_under(
            State {
                clip,
                effect,
                ..State::default()
            },
            vec![Item::Rect {
                bounds: rect(0.0, 0.0, 64.0, 64.0),
                color: Color::BLACK,
            }],
        );
        gpu.render(&scene, &Changed::all(), &mut text);
        gpu.end_frame();
        assert_eq!(at(&gpu.read_canvas().expect("a canvas"), 1, 1), [0, 0, 0, 255]);
        if let Some(c) = scene.properties.clips.get_mut(usize::try_from(clip).unwrap_or(0)) {
            c.clip.radius = Corners::all(16.0);
        }
        let changed = if whole {
            Changed::all()
        } else {
            Changed {
                clips: vec![clip],
                ..Changed::default()
            }
        };
        gpu.render(&scene, &changed, &mut text);
        gpu.end_frame();
        let p = gpu.read_canvas().expect("a canvas");
        assert_eq!(at(&p, 1, 1), [255, 255, 255, 255], "the new corner (whole: {whole})");
        assert_eq!(at(&p, 32, 32), [0, 0, 0, 255], "inside (whole: {whole})");
        canvas_is_whole(&mut gpu, &scene, &mut text);
    }
}

/// A filtered layer turned half round under a clip of the canvas that
/// stays, its region and origin the same: the clip keeps other texels of
/// the layer now, so its texture is drawn again (its content's key holds
/// the clip as the layer's texels see it).
#[test]
fn a_cached_filtered_layer_turned_under_a_clip_that_stays_is_drawn_again() {
    for whole in [true, false] {
        let mut gpu = gpu();
        let mut text = Glyphs::for_os();
        let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
        let turn = |rotate: f32| GroupTransform {
            origin: (32.0, 32.0),
            scale: 1.0,
            rotate,
        };
        let t = scene.add_transform((0.0, 0.0), Some(turn(0.0)));
        // The canvas's left half.
        let clip = scene.add_clip(
            Clip {
                bounds: rect(0.0, 0.0, 32.0, 64.0),
                radius: Corners::default(),
                curve: crate::shape::ROUND,
            },
            0,
        );
        let effect = scene.add_effect(EffectNode {
            filters: crate::style::Filters::repeat(1, crate::style::Filter::Brightness(1.0)),
            transform: t,
            transformed: true,
            region: Some(PixelRect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            }),
            ..EffectNode::default()
        });
        // The layer's top half.
        scene.push_under(
            State {
                transform: t,
                clip,
                effect,
            },
            vec![Item::Rect {
                bounds: rect(0.0, 0.0, 64.0, 32.0),
                color: Color::BLACK,
            }],
        );
        gpu.render(&scene, &Changed::all(), &mut text);
        gpu.end_frame();
        let p = gpu.read_canvas().expect("a canvas");
        assert_eq!(at(&p, 16, 16), [0, 0, 0, 255], "top left");
        assert_eq!(at(&p, 48, 16), [255, 255, 255, 255], "top right, clipped");
        if let Some(n) = scene.properties.transforms.get_mut(usize::try_from(t).unwrap_or(0)) {
            n.matrix = Some(turn(180.0));
        }
        let changed = if whole {
            Changed::all()
        } else {
            Changed {
                transforms: vec![t],
                effects: vec![effect],
                ..Changed::default()
            }
        };
        gpu.render(&scene, &changed, &mut text);
        let d = gpu.drawn();
        gpu.end_frame();
        assert_eq!(d.layers, 1, "drawn again (whole: {whole}): {d}");
        let p = gpu.read_canvas().expect("a canvas");
        // The top half turned to the bottom, the left half kept.
        assert_eq!(at(&p, 16, 48), [0, 0, 0, 255], "bottom left (whole: {whole})");
        assert_eq!(at(&p, 48, 48), [255, 255, 255, 255], "bottom right (whole: {whole})");
        assert_eq!(at(&p, 16, 16), [255, 255, 255, 255], "top left (whole: {whole})");
        canvas_is_whole(&mut gpu, &scene, &mut text);
    }
}

/// A filtered card scrolled with its own clip: the clip keeps the same
/// texels of its layer, so the texture is kept and only composited where
/// it lies now.
#[test]
fn a_filtered_card_scrolled_with_its_clip_keeps_its_layer() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((128, 128), 1.0, Some(Color::WHITE));
    let t = scene.add_transform((0.0, 0.0), None);
    let clip = scene.add_clip(
        Clip {
            bounds: rect(20.0, 40.0, 60.0, 60.0),
            radius: Corners::all(12.0),
            curve: crate::shape::ROUND,
        },
        t,
    );
    let effect = scene.add_effect(EffectNode {
        filters: crate::style::Filters::repeat(1, crate::style::Filter::Brightness(1.0)),
        transform: t,
        region: Some(PixelRect {
            x: 20,
            y: 40,
            width: 60,
            height: 60,
        }),
        ..EffectNode::default()
    });
    scene.push_under(
        State {
            transform: t,
            clip,
            effect,
        },
        vec![Item::Rect {
            bounds: rect(20.0, 40.0, 60.0, 60.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    // Scrolled up 24: the transform, the clip and the region together.
    if let Some(n) = scene.properties.transforms.get_mut(usize::try_from(t).unwrap_or(0)) {
        n.offset = (0.0, -24.0);
    }
    if let Some(c) = scene.properties.clips.get_mut(usize::try_from(clip).unwrap_or(0)) {
        c.clip.bounds = rect(20.0, 16.0, 60.0, 60.0);
    }
    if let Some(r) = scene
        .properties
        .effects
        .get_mut(usize::try_from(effect).unwrap_or(0))
        .and_then(|e| e.region.as_mut())
    {
        r.y = 16;
    }
    gpu.render(
        &scene,
        &Changed {
            transforms: vec![t],
            clips: vec![clip],
            effects: vec![effect],
            ..Changed::default()
        },
        &mut text,
    );
    let d = gpu.drawn();
    gpu.end_frame();
    assert_eq!((d.layers, d.layers_kept), (0, 1), "the texture kept: {d}");
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 50, 30), [0, 0, 0, 255], "the card where it lies now");
    assert_eq!(at(&p, 21, 17), [255, 255, 255, 255], "its corner, cut");
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A root chunk's clip changing its corners alone, on an opaque canvas,
/// under a whole plan: its key holds the chain's revision, so it is
/// damage and drawn.
#[test]
fn a_clip_radius_change_alone_repaints_under_a_whole_plan() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let clip = scene.add_clip(
        Clip {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    scene.push_under(
        State {
            clip,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    if let Some(c) = scene.properties.clips.get_mut(usize::try_from(clip).unwrap_or(0)) {
        c.clip.radius = Corners::all(16.0);
    }
    gpu.render(&scene, &Changed::all(), &mut text);
    assert!(gpu.drawn().damage > 0, "the corners are damage");
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 1, 1), [255, 255, 255, 255], "the new corner");
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A clip moved from a parent on the left to one on the right: the damage
/// is what its old chain kept and what its new one keeps, each frame's
/// from its own ancestry, so nothing is left where it was.
#[test]
fn reparenting_a_clip_from_left_to_right_leaves_nothing_behind() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let square = |x: f32, w: f32| Clip {
        bounds: rect(x, 0.0, w, 64.0),
        radius: Corners::default(),
        curve: crate::shape::ROUND,
    };
    let left = scene.add_clip(square(0.0, 24.0), 0);
    let right = scene.add_clip(square(40.0, 24.0), 0);
    let leaf = scene.add_clip(square(0.0, 64.0), 0);
    if let Some(c) = scene.properties.clips.get_mut(usize::try_from(leaf).unwrap_or(0)) {
        c.parent = left;
    }
    scene.push_under(
        State {
            clip: leaf,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    assert_eq!(at(&gpu.read_canvas().expect("a canvas"), 10, 10), [0, 0, 0, 255]);
    if let Some(c) = scene.properties.clips.get_mut(usize::try_from(leaf).unwrap_or(0)) {
        c.parent = right;
    }
    let changed = Changed {
        clips: vec![leaf],
        ..Changed::default()
    };
    gpu.render(&scene, &changed, &mut text);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 10, 10), [255, 255, 255, 255], "the left: nothing left behind");
    assert_eq!(at(&p, 50, 10), [0, 0, 0, 255], "the right: drawn");
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A chunk as large as the window under a small clip, recorded again: its
/// damage is what its chain keeps, not all it draws.
#[test]
fn a_viewport_sized_chunk_under_a_small_clip_damages_no_more_than_the_clip() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let key = text.test_glyph(24.0).expect("a glyph");
    let build = |color: Color| {
        let mut scene = Scene::new((256, 256), 1.0, Some(Color::WHITE));
        let clip = scene.add_clip(
            Clip {
                bounds: rect(100.0, 100.0, 16.0, 16.0),
                radius: Corners::default(),
                curve: crate::shape::ROUND,
            },
            0,
        );
        // Text over all of the window.
        let mut items = Vec::new();
        for row in 0..10_i32 {
            for col in 0..12_i32 {
                items.push(Item::Glyph {
                    key,
                    x: col * 20,
                    y: 20 + row * 24,
                    color,
                });
            }
        }
        scene.chunks.push(crate::scene::Chunk {
            id: crate::scene::ChunkId(7),
            state: State {
                clip,
                ..State::default()
            },
            display: std::sync::Arc::new(crate::scene::Display::new(items)),
        });
        scene
    };
    gpu.render(&build(Color::BLACK), &Changed::all(), &mut text);
    gpu.end_frame();
    let next = build(Color::hex(0x2563eb));
    let changed = Changed {
        chunks: vec![0],
        ..Changed::default()
    };
    gpu.render(&next, &changed, &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    assert!(!d.planned_whole);
    // The clip and a pixel of anti-aliasing around it.
    assert!(d.damage > 0 && d.damage <= 18 * 18, "{d}");
    canvas_is_whole(&mut gpu, &next, &mut text);
}

/// Curve text (`slug.wgsl`) under a square clip within a rounded one: the
/// rounded one's corner cuts its ink, as it cuts quads and paths.
#[test]
fn curve_text_in_a_square_clip_within_a_rounded_one_is_cut_by_its_corner() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let (plain, key) = glyph_scene(&mut text, 200.0, None);
    let whole = gpu.capture(&plain, None, &mut text).expect("read back");
    let (w, h) = (whole.width(), whole.height());
    let ink: Vec<(u32, u32)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| at(&whole, x, y)[0] < 32)
        .collect();
    // The ink's top left: the rounded clip's corner goes just above it.
    let &(px, py) = ink.iter().min_by_key(|(x, y)| x + y).expect("ink");
    let &(qx, qy) = ink
        .iter()
        .find(|&&(x, y)| x >= px + 40 && y >= py + 40)
        .expect("ink well inside");
    let (cx, cy) = (i32_to_f32(px.cast_signed()) - 5.0, i32_to_f32(py.cast_signed()) - 5.0);
    let size = (i32_to_f32(w.cast_signed()), i32_to_f32(h.cast_signed()));
    let Some(Item::Glyph { x, y, .. }) = plain.items().next().cloned() else {
        panic!()
    };
    let mut scene = Scene::new((w, h), 1.0, Some(Color::WHITE));
    let bounds = rect(cx, cy, size.0 - cx, size.1 - cy);
    let outer = scene.add_clip(
        Clip {
            bounds,
            radius: Corners::all(30.0),
            curve: crate::shape::ROUND,
        },
        0,
    );
    let inner = scene.add_clip(
        Clip {
            bounds,
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(inner).unwrap_or(0)) {
        n.parent = outer;
    }
    scene.push_under(
        State {
            clip: inner,
            ..State::default()
        },
        vec![Item::Glyph {
            key,
            x,
            y,
            color: Color::BLACK,
        }],
    );
    let cut = gpu.capture(&scene, None, &mut text).expect("read back");
    assert_eq!(
        at(&cut, px, py),
        [255, 255, 255, 255],
        "outside the rounded corner: no ink"
    );
    assert_eq!(at(&cut, qx, qy), at(&whole, qx, qy), "well inside: as drawn unclipped");
}

/// Two turned groups, each inside a clip of its own (the outer group in
/// one, the inner in one inside the outer group): what the inner holds,
/// drawn through both turns as paths and curve glyphs, is cut by both
/// clips, each tested in its own space (one chain across the turns).
#[test]
fn nested_turned_groups_cut_paths_and_curve_glyphs_by_both_clips() {
    use crate::app::Scenario;
    use crate::element::{IntoElement, div, text};
    use crate::style::Overflow;
    use crate::styled::Styled;
    // The outer clip keeps 20..160 across; the outer group, wider (160),
    // turns half about (100, 100); the clip inside it keeps 20..100 across
    // there, so 100..180 on screen; the inner group, turned half about its
    // own centre, spans -20..180 on screen. The clips cross: 24..96 only
    // the inner one cuts, 164..176 only the outer one, 104..156 neither.
    let shot = |content: fn() -> crate::element::Element| {
        let mut s = Scenario::measuring(
            move || {
                div()
                    .background(Color::WHITE)
                    .width(200.0)
                    .height(200.0)
                    .padding_top(20.0)
                    .padding_left(20.0)
                    .child(
                        div().width(140.0).height(160.0).overflow(Overflow::Hidden).child(
                            div().width(160.0).height(160.0).flex_shrink(0.0).rotate(180.0).child(
                                div().width(80.0).height(160.0).overflow(Overflow::Hidden).child(
                                    div()
                                        .width(200.0)
                                        .height(160.0)
                                        .flex_shrink(0.0)
                                        .rotate(180.0)
                                        .child(content()),
                                ),
                            ),
                        ),
                    )
                    .into_element()
            },
            200,
            200,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        s.shot(false).expect("a shot")
    };
    let dark = |p: &Pixels, x0: u32, x1: u32| {
        (24..176)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| at(p, x, y)[0] < 128)
            .count()
    };
    // A box: a path through the turns.
    let boxed = shot(|| div().width(200.0).height(160.0).background(Color::BLACK).into_element());
    assert_eq!(dark(&boxed, 24, 96), 0, "the inner clip cuts the path");
    assert_eq!(dark(&boxed, 104, 156), 52 * 152, "inside both: the path");
    assert_eq!(dark(&boxed, 164, 176), 0, "the outer clip cuts the path");
    assert_eq!(
        at(&boxed, 60, 100),
        [255, 255, 255, 255],
        "only the inner clip cuts here"
    );
    assert_eq!(at(&boxed, 130, 100), [0, 0, 0, 255], "neither cuts here");
    assert_eq!(
        at(&boxed, 170, 100),
        [255, 255, 255, 255],
        "only the outer clip cuts here"
    );
    // Text: curve glyphs through the turns.
    let glyphs = shot(|| {
        div()
            .width(200.0)
            .height(160.0)
            .text_size(40.0)
            .text_color(Some(Color::BLACK))
            .child(text(
                "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM",
            ))
            .into_element()
    });
    assert_eq!(dark(&glyphs, 24, 96), 0, "the inner clip cuts the glyphs");
    assert!(dark(&glyphs, 104, 156) > 300, "inside both: the glyphs");
    assert_eq!(dark(&glyphs, 164, 176), 0, "the outer clip cuts the glyphs");
}

/// Pure green: a pixel [`super::path::ClipSkip::Mark`] marked.
fn marked(p: [u8; 4]) -> bool {
    p[0] < 32 && p[1] > 224 && p[2] < 32
}

/// Paths and curve glyphs whose quad lies wholly inside their clip chain
/// test no clip per pixel (`clip_needed` in `path_render.wgsl` and
/// `slug.wgsl`), and draw what testing it would: marked where skipped,
/// not where a quad crosses a clip's edge, and the same pixels skipped or
/// not.
#[test]
fn paths_and_curve_glyphs_inside_their_clips_skip_the_clip_test_and_draw_the_same() {
    use super::path::ClipSkip;
    let mut text = Glyphs::for_os();
    // Paths: one wholly inside a square clip, one across its edge.
    let mut paths = Scene::new((128, 64), 1.0, Some(Color::WHITE));
    let clip = paths.add_clip(
        Clip {
            bounds: rect(8.0, 8.0, 112.0, 48.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    let square = |x: f32, y: f32, w: f32| {
        Path::builder()
            .move_to(x, y)
            .line_to(x + w, y)
            .line_to(x + w, y + w)
            .line_to(x, y + w)
            .close()
            .build()
    };
    for (x, y, w) in [(40.0, 24.0, 16.0), (0.0, 0.0, 14.0)] {
        paths.push_under(
            State {
                clip,
                ..State::default()
            },
            vec![Item::Path {
                origin: (0.0, 0.0),
                path: square(x, y, w),
                style: PathStyle::fill(Gradient::Solid(Color::BLACK)),
            }],
        );
    }
    // A curve glyph in a clip containing it, and in one cutting it.
    let (plain, key) = glyph_scene(&mut text, 200.0, None);
    let Some(Item::Glyph { x, y, .. }) = plain.items().next().cloned() else {
        panic!()
    };
    let glyph = |cut: bool| {
        let mut scene = Scene::new(plain.size, 1.0, Some(Color::WHITE));
        let (w, h) = (
            crate::convert::u32_to_f32(plain.size.0),
            crate::convert::u32_to_f32(plain.size.1),
        );
        let clip = scene.add_clip(
            Clip {
                bounds: if cut {
                    rect(0.0, 0.0, w / 2.0, h)
                } else {
                    rect(0.0, 0.0, w, h)
                },
                radius: Corners::default(),
                curve: crate::shape::ROUND,
            },
            0,
        );
        scene.push_under(
            State {
                clip,
                ..State::default()
            },
            vec![Item::Glyph {
                key,
                x,
                y,
                color: Color::BLACK,
            }],
        );
        scene
    };
    let shots = |mode: ClipSkip, text: &mut Glyphs| {
        let mut gpu = gpu();
        gpu.paths.clip_skip = mode;
        [&paths, &glyph(false), &glyph(true)].map(|s| gpu.capture(s, None, text).expect("read back"))
    };
    let skip = shots(ClipSkip::Skip, &mut text);
    let never = shots(ClipSkip::Never, &mut text);
    let mark = shots(ClipSkip::Mark, &mut text);
    for (a, b) in skip.iter().zip(&never) {
        assert_eq!(a.data(), b.data(), "skipped or not, the same pixels");
    }
    let [paths_marked, inside, cut] = &mark;
    assert!(marked(at(paths_marked, 48, 32)), "the path inside: skipped");
    assert_eq!(
        at(paths_marked, 10, 10),
        [0, 0, 0, 255],
        "the path across the edge: tested"
    );
    assert_eq!(at(paths_marked, 4, 4), [255, 255, 255, 255], "and cut");
    let count = |p: &Pixels| {
        (0..p.height())
            .flat_map(|y| (0..p.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| marked(at(p, x, y)))
            .count()
    };
    assert!(count(inside) > 500, "the glyph inside its clip: skipped");
    assert_eq!(count(cut), 0, "the glyph its clip cuts: tested");
    assert!(
        never.iter().all(|p| count(p) == 0) && skip.iter().all(|p| count(p) == 0),
        "marked only when asked"
    );
}

/// V3: the renderer's own code makes every pipeline and bind group layout,
/// on the device it requests for each variant (its features from the
/// variant, as `with_adapter` asks for them), and a real draw or dispatch
/// goes through each with the real bind groups and dynamic offsets: quads,
/// atlas icons, paths (the compute passes and the render pass), curve text
/// and a backdrop read. A validation error anywhere (a layout that is not
/// the shader's, a `dynamic` that is not the draw's, a feature the shader
/// needs and the device lacks) fails.
#[test]
fn every_pipeline_is_made_and_draws_in_both_variants_without_a_validation_error() {
    for dual in [true, false] {
        let mut gpu = GpuRenderer::headless_with(GpuOptions {
            dual_source: dual,
            ..GpuOptions::default()
        })
        .expect("a GPU");
        assert_eq!(gpu.dual_source(), dual, "the variant the device was asked for");
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut text = Glyphs::for_os();
        gpu.capture(&busy_scene(128), None, &mut text).expect("read back");
        let (curves, _) = glyph_scene(&mut text, 200.0, None);
        gpu.capture(&curves, None, &mut text).expect("read back");
        let below = stripes(96.0, 1.0);
        let top = read(vec![Item::Backdrop {
            bounds: rect(16.0, 16.0, 64.0, 64.0),
            radius: Corners::default(),
            curve: 1.0,
            backdrop: crate::shape::Backdrop::blur(8.0),
        }]);
        let blurred = chunks((96, 96), &[(1, State::default(), &below), (2, State::default(), &top)]);
        gpu.capture(&blurred, None, &mut text).expect("read back");
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "dual-source {dual}: {error:?}");
    }
}

/// V3: a job's flags word reaches the compute and render shaders as Rust
/// wrote it: the same pentagram filled non-zero has its centre, filled
/// even-odd has none, stroked has none but its outline.
#[test]
fn a_fill_an_even_odd_fill_and_a_stroke_are_told_apart_on_the_gpu() {
    let mut gpu = gpu();
    let star = |rule: FillRule| {
        let mut b = Path::builder().fill_rule(rule);
        for (i, k) in [0_u8, 2, 4, 1, 3].into_iter().enumerate() {
            let a = (f32::from(k) * 0.4 - 0.5) * std::f32::consts::PI;
            let (x, y) = (48.0 + 40.0 * a.cos(), 48.0 + 40.0 * a.sin());
            b = if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) };
        }
        b.close().build()
    };
    let white = Gradient::Solid(Color::WHITE);
    let nonzero = shot_of(
        &mut gpu,
        &star(FillRule::NonZero),
        &PathStyle::fill(white.clone()),
        (0.0, 0.0),
    );
    let even_odd = shot_of(
        &mut gpu,
        &star(FillRule::EvenOdd),
        &PathStyle::fill(white.clone()),
        (0.0, 0.0),
    );
    let stroked = shot_of(
        &mut gpu,
        &star(FillRule::NonZero),
        &PathStyle::stroke(white, Stroke::round(4.0)),
        (0.0, 0.0),
    );
    assert_eq!(at(&nonzero, 48, 48), [255; 4], "non-zero fills the centre");
    assert_eq!(at(&even_odd, 48, 48), [0, 0, 0, 255], "even-odd leaves the centre");
    assert_eq!(at(&even_odd, 48, 20), [255; 4], "and fills a point");
    assert_eq!(at(&stroked, 48, 48), [0, 0, 0, 255], "a stroke leaves the centre");
    assert_eq!(at(&stroked, 48, 9)[0], 255, "and covers its outline at the point");
    assert_eq!(at(&stroked, 48, 20), [0, 0, 0, 255], "and not the inside");
}

/// Curve text (`slug.wgsl`) in a layer a filter keeps, under a turned
/// rounded clip of the canvas: drawn into the layer, it is cut by the clip
/// tested in the clip's own space, as it is with no layer.
#[test]
fn curve_text_in_a_filter_layer_under_a_turned_clip_is_cut_as_without_the_layer() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let (plain, key) = glyph_scene(&mut text, 200.0, None);
    let Some(Item::Glyph { x, y, .. }) = plain.items().next().cloned() else {
        panic!()
    };
    let size = plain.size;
    let (w, h) = (crate::convert::u32_to_f32(size.0), crate::convert::u32_to_f32(size.1));
    let scene_of = |layered: bool| {
        let mut scene = Scene::new(size, 1.0, Some(Color::WHITE));
        let t = scene.add_transform(
            (0.0, 0.0),
            Some(GroupTransform {
                origin: (w / 2.0, h / 2.0),
                scale: 1.0,
                rotate: 30.0,
            }),
        );
        let clip = scene.add_clip(
            Clip {
                bounds: rect(w * 0.25, h * 0.25, w * 0.5, h * 0.5),
                radius: Corners::all(12.0),
                curve: crate::shape::ROUND,
            },
            t,
        );
        let effect = if layered {
            scene.add_effect(EffectNode {
                filters: crate::style::Filters::repeat(1, crate::style::Filter::Brightness(1.0)),
                region: Some(PixelRect {
                    x: 0,
                    y: 0,
                    width: size.0,
                    height: size.1,
                }),
                ..EffectNode::default()
            })
        } else {
            0
        };
        scene.push_under(
            State {
                clip,
                effect,
                ..State::default()
            },
            vec![Item::Glyph {
                key,
                x,
                y,
                color: Color::BLACK,
            }],
        );
        scene
    };
    let whole = gpu.capture(&plain, None, &mut text).expect("read back");
    let bare = gpu.capture(&scene_of(false), None, &mut text).expect("read back");
    let layered = gpu.capture(&scene_of(true), None, &mut text).expect("read back");
    let ink = |p: &Pixels| {
        p.data()
            .chunks(4)
            .filter(|c| c.first().is_some_and(|r| *r < 128))
            .count()
    };
    assert!(ink(&bare) > 0 && ink(&bare) < ink(&whole), "the clip cuts the ink");
    let most = bare
        .data()
        .iter()
        .zip(layered.data())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(most <= 2, "in the layer as without it: {most} levels apart");
}

/// A clip inside a filter layer moved from one rounded parent of the
/// canvas to another, nothing else changed: the frame from what changed
/// draws what a whole redraw does, and the old parent, changing after, no
/// longer reaches it.
#[test]
fn a_clip_moved_to_another_parent_across_a_layer_draws_as_a_whole_redraw() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let rounded = |b: Bounds| Clip {
        bounds: b,
        radius: Corners::all(8.0),
        curve: crate::shape::ROUND,
    };
    let left = scene.add_clip(rounded(rect(0.0, 0.0, 32.0, 64.0)), 0);
    let top = scene.add_clip(rounded(rect(0.0, 0.0, 64.0, 32.0)), 0);
    let inner = scene.add_clip(
        Clip {
            bounds: rect(4.0, 4.0, 56.0, 56.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    let parent = |scene: &mut Scene, p: u32| {
        if let Some(n) = scene.properties.clips.get_mut(usize::try_from(inner).unwrap_or(0)) {
            n.parent = p;
        }
    };
    parent(&mut scene, left);
    let effect = scene.add_effect(EffectNode {
        filters: crate::style::Filters::repeat(1, crate::style::Filter::Brightness(1.0)),
        region: Some(PixelRect {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        }),
        ..EffectNode::default()
    });
    scene.push_under(
        State {
            clip: inner,
            effect,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    assert_eq!(at(&gpu.read_canvas().expect("a canvas"), 16, 48), [0, 0, 0, 255]);
    parent(&mut scene, top);
    gpu.render(
        &scene,
        &Changed {
            clips: vec![inner],
            ..Changed::default()
        },
        &mut text,
    );
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 48, 16), [0, 0, 0, 255], "in the new parent");
    assert_eq!(at(&p, 16, 48), [255; 4], "out of it, in the old");
    canvas_is_whole(&mut gpu, &scene, &mut text);
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(left).unwrap_or(0)) {
        n.clip.bounds = rect(0.0, 0.0, 16.0, 64.0);
    }
    gpu.render(
        &scene,
        &Changed {
            clips: vec![left],
            ..Changed::default()
        },
        &mut text,
    );
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 48, 16), [0, 0, 0, 255], "the old parent no longer cuts it");
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A filtered layer turned under the clip its group lies in (its output
/// clip, the canvas's left half), holding a black square: `turn` its
/// turn, `clipped` whether the effect names the clip it lies in.
fn turned_layer_under_its_output_clip(turn: f32, clipped: bool) -> (Scene, u32, u32) {
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let t = scene.add_transform(
        (0.0, 0.0),
        Some(GroupTransform {
            origin: (32.0, 32.0),
            scale: 1.0,
            rotate: turn,
        }),
    );
    let clip = scene.add_clip(
        Clip {
            bounds: rect(0.0, 0.0, 32.0, 64.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    let effect = scene.add_effect(EffectNode {
        filters: crate::style::Filters::repeat(1, crate::style::Filter::Brightness(1.0)),
        transform: t,
        transformed: true,
        region: Some(PixelRect {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        }),
        clip: if clipped { clip } else { 0 },
        ..EffectNode::default()
    });
    scene.push_under(
        State {
            transform: t,
            clip,
            effect,
        },
        vec![Item::Rect {
            bounds: rect(8.0, 8.0, 48.0, 48.0),
            color: Color::BLACK,
        }],
    );
    (scene, t, effect)
}

/// A layer turned a twelfth round under its output clip, a square edge at
/// x = 32 of the canvas: the clip cuts the composite in the canvas, after
/// the layer is resampled, so the edge is the canvas's pixel edge, whole
/// on one side and empty on the other; cut in the layer's texels first,
/// resampling would smear it.
#[test]
fn a_turned_layers_output_clip_cuts_its_composite_on_the_canvas_pixel_edge() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let (scene, ..) = turned_layer_under_its_output_clip(30.0, true);
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    for y in [24, 32, 40] {
        assert_eq!(at(&p, 31, y), [0, 0, 0, 255], "left of the edge at y {y}");
        assert_eq!(at(&p, 32, y), [255; 4], "right of it at y {y}");
    }
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A filtered layer turned under its output clip, which stays: its texels
/// hold nothing the clip decides (the clip cuts its composite), so its
/// texture is kept and only composited again, and the canvas is what a
/// whole redraw draws.
#[test]
fn a_layer_turned_under_its_output_clip_keeps_its_texture() {
    for whole in [true, false] {
        let mut gpu = gpu();
        let mut text = Glyphs::for_os();
        let (mut scene, t, effect) = turned_layer_under_its_output_clip(0.0, true);
        gpu.render(&scene, &Changed::all(), &mut text);
        gpu.end_frame();
        if let Some(n) = scene.properties.transforms.get_mut(usize::try_from(t).unwrap_or(0)) {
            n.matrix = Some(GroupTransform {
                origin: (32.0, 32.0),
                scale: 1.0,
                rotate: 90.0,
            });
        }
        let changed = if whole {
            Changed::all()
        } else {
            Changed {
                transforms: vec![t],
                effects: vec![effect],
                ..Changed::default()
            }
        };
        gpu.render(&scene, &changed, &mut text);
        let d = gpu.drawn();
        gpu.end_frame();
        assert_eq!((d.layers, d.layers_kept), (0, 1), "kept (whole: {whole}): {d}");
        let p = gpu.read_canvas().expect("a canvas");
        assert_eq!(at(&p, 16, 32), [0, 0, 0, 255], "left: kept (whole: {whole})");
        assert_eq!(at(&p, 48, 32), [255; 4], "right: clipped (whole: {whole})");
        canvas_is_whole(&mut gpu, &scene, &mut text);
    }
}

fn filtered(effect: EffectNode) -> EffectNode {
    EffectNode {
        filters: crate::style::Filters::repeat(1, crate::style::Filter::Brightness(1.0)),
        ..effect
    }
}

fn whole_region(side: u32) -> Option<PixelRect> {
    Some(PixelRect {
        x: 0,
        y: 0,
        width: side,
        height: side,
    })
}

/// A layer whose output clip (the canvas's left half) is not one its
/// chunk's clip (the top half, another branch) lies in: its walk would end
/// at once, losing the top half, and the bottom left quarter would show.
/// The scene breaks the output clip's contract, and planning says so.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "does not lie in the layer's output clip")]
fn an_output_clip_its_chunks_clips_do_not_lie_in_stops_a_debug_build() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let square = |b: Bounds| Clip {
        bounds: b,
        radius: Corners::default(),
        curve: crate::shape::ROUND,
    };
    let left = scene.add_clip(square(rect(0.0, 0.0, 32.0, 64.0)), 0);
    let top = scene.add_clip(square(rect(0.0, 0.0, 64.0, 32.0)), 0);
    let effect = scene.add_effect(filtered(EffectNode {
        region: whole_region(64),
        clip: left,
        ..EffectNode::default()
    }));
    scene.push_under(
        State {
            clip: top,
            effect,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
}

/// A layer whose output clip alone changes, from a rounded clip to the
/// square one it lies in: the rounded one now lies inside the layer, its
/// corners in the texels, so the texture is drawn again (a texture kept
/// would show them square, the square parent alone cutting the composite).
#[test]
fn a_layers_output_clip_moved_out_to_its_parent_draws_its_texture_again() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let outer = scene.add_clip(
        Clip {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    let rounded = scene.add_clip(
        Clip {
            bounds: rect(8.0, 8.0, 48.0, 48.0),
            radius: Corners::all(16.0),
            curve: crate::shape::ROUND,
        },
        0,
    );
    let inner = scene.add_clip(
        Clip {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    for (c, p) in [(rounded, outer), (inner, rounded)] {
        if let Some(n) = scene.properties.clips.get_mut(usize::try_from(c).unwrap_or(0)) {
            n.parent = p;
        }
    }
    let effect = scene.add_effect(filtered(EffectNode {
        region: whole_region(64),
        clip: rounded,
        ..EffectNode::default()
    }));
    scene.push_under(
        State {
            clip: inner,
            effect,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 9, 9), [255; 4], "the rounded corner cuts the composite");
    assert_eq!(at(&p, 32, 32), [0, 0, 0, 255]);
    if let Some(n) = scene.properties.effects.get_mut(usize::try_from(effect).unwrap_or(0)) {
        n.clip = outer;
    }
    gpu.render(
        &scene,
        &Changed {
            effects: vec![effect],
            ..Changed::default()
        },
        &mut text,
    );
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    assert_eq!(at(&p, 9, 9), [255; 4], "the rounded corner cuts the texels now");
    assert_eq!(at(&p, 32, 32), [0, 0, 0, 255]);
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// A black square cut by a rounded clip in a filtered layer scaled twice,
/// drawn as a quad and as a path: each is cut in the layer's texels, by
/// the same coverage one texel wide, so every pixel matches (the quad was
/// cut in the canvas's pixels, half a texel wide, the path in texels).
#[test]
fn a_quad_and_a_path_cut_by_one_rounded_clip_in_a_layer_scaled_twice_cover_alike() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let scene_of = |as_path: bool| {
        let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
        let t = scene.add_transform(
            (0.0, 0.0),
            Some(GroupTransform {
                origin: (0.0, 0.0),
                scale: 2.0,
                rotate: 0.0,
            }),
        );
        let clip = scene.add_clip(
            Clip {
                bounds: rect(4.0, 4.0, 24.0, 24.0),
                radius: Corners::all(7.0),
                curve: crate::shape::ROUND,
            },
            t,
        );
        let effect = scene.add_effect(filtered(EffectNode {
            transform: t,
            transformed: true,
            region: whole_region(32),
            ..EffectNode::default()
        }));
        let item = if as_path {
            Item::Path {
                origin: (0.0, 0.0),
                path: Path::builder()
                    .move_to(0.0, 0.0)
                    .line_to(32.0, 0.0)
                    .line_to(32.0, 32.0)
                    .line_to(0.0, 32.0)
                    .close()
                    .build(),
                style: PathStyle::fill(Gradient::Solid(Color::BLACK)),
            }
        } else {
            Item::Rect {
                bounds: rect(0.0, 0.0, 32.0, 32.0),
                color: Color::BLACK,
            }
        };
        scene.push_under(
            State {
                transform: t,
                clip,
                effect,
            },
            vec![item],
        );
        scene
    };
    let quad = gpu.capture(&scene_of(false), None, &mut text).expect("read back");
    let path = gpu.capture(&scene_of(true), None, &mut text).expect("read back");
    let partly = quad
        .data()
        .chunks(4)
        .filter(|c| c.first().is_some_and(|r| (16..240).contains(r)))
        .count();
    assert!(partly > 0, "the corners are cut part way");
    let most = quad
        .data()
        .iter()
        .zip(path.data())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(most <= 2, "a quad and a path cut alike: {most} levels apart");
}

fn square_clip(b: Bounds) -> Clip {
    Clip {
        bounds: b,
        radius: Corners::default(),
        curve: crate::shape::ROUND,
    }
}

/// A filtered layer of 64 by 64 canvas pixels scaled twice about the
/// canvas's corner (its texel 10 spans canvas pixels 20 and 21), a black
/// square in it: its region and transform.
fn scaled_twice(scene: &mut Scene) -> (u32, u32) {
    let t = scene.add_transform(
        (0.0, 0.0),
        Some(GroupTransform {
            origin: (0.0, 0.0),
            scale: 2.0,
            rotate: 0.0,
        }),
    );
    let effect = scene.add_effect(filtered(EffectNode {
        transform: t,
        transformed: true,
        region: whole_region(32),
        ..EffectNode::default()
    }));
    (t, effect)
}

/// The black square of a layer scaled twice ([`scaled_twice`]), under `clip`.
fn square_in(scene: &mut Scene, (t, effect): (u32, u32), clip: u32) {
    scene.push_under(
        State {
            transform: t,
            clip,
            effect,
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 32.0, 32.0),
            color: Color::BLACK,
        }],
    );
}

/// Codex's fixture: clips of the canvas, an inner (x 21 to 30) in an
/// outer (x 21 to 64), cutting the square of a layer scaled twice: the
/// outer and the inner clip.
fn coincident_edges_scaled_twice() -> (Scene, (u32, u32)) {
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let layer = scaled_twice(&mut scene);
    let outer = scene.add_clip(square_clip(rect(21.0, 0.0, 43.0, 64.0)), 0);
    let inner = scene.add_clip(square_clip(rect(21.0, 8.0, 30.0, 48.0)), 0);
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(inner).unwrap_or(0)) {
        n.parent = outer;
    }
    square_in(&mut scene, layer, inner);
    (scene, (outer, inner))
}

/// Codex's fixture after a frame, `change` made: drawn from what changed,
/// the canvas is what a whole redraw draws.
fn redrawn_as_whole(mut scene: Scene, change: impl FnOnce(&mut Scene) -> Changed) {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let changed = change(&mut scene);
    gpu.render(&scene, &changed, &mut text);
    gpu.end_frame();
    canvas_is_whole(&mut gpu, &scene, &mut text);
}

/// The outer clip of Codex's fixture moved out a pixel (left edge 21 to
/// 20): the canvas leaves it out of the inner's walk before and after (it
/// keeps each canvas pixel the inner touches whole), so the inner's
/// revision holds; but the layer walks it, and its texel 10 goes from a
/// quarter to half. The layer read the outer clip: it is drawn again.
#[test]
fn an_outer_clip_the_canvas_leaves_out_moved_draws_a_scaled_layer_again() {
    let (scene, (outer, _)) = coincident_edges_scaled_twice();
    redrawn_as_whole(scene, |scene| {
        if let Some(n) = scene.properties.clips.get_mut(usize::try_from(outer).unwrap_or(0)) {
            n.clip.bounds = rect(20.0, 0.0, 44.0, 64.0);
        }
        Changed {
            clips: vec![outer],
            ..Changed::default()
        }
    });
}

/// The outer clip of Codex's fixture under a transform of its own, which
/// scales it about x = 41 so its left edge moves from 21 to 20: it is
/// moved, not reshaped, still left out of the inner's walk on the canvas,
/// and nothing else changes; the layer that read it is drawn again.
#[test]
fn an_outer_clip_the_canvas_leaves_out_moved_by_its_transform_draws_a_scaled_layer_again() {
    let about = |scale: f32| GroupTransform {
        origin: (41.0, 0.0),
        scale,
        rotate: 0.0,
    };
    let (mut scene, (outer, _)) = coincident_edges_scaled_twice();
    let space = scene.add_transform((0.0, 0.0), Some(about(1.0)));
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(outer).unwrap_or(0)) {
        n.transform = space;
    }
    redrawn_as_whole(scene, |scene| {
        if let Some(n) = scene.properties.transforms.get_mut(usize::try_from(space).unwrap_or(0)) {
            n.matrix = Some(about(1.05));
        }
        Changed {
            transforms: vec![space],
            ..Changed::default()
        }
    });
}

/// A clip lying under a transform within a layer scaled twice, that
/// transform moved out to the root: the clip now lies in the canvas, so
/// the texels it keeps move. The layer's keys composed through that
/// transform: they go, and the layer is drawn again (the transform no
/// longer lies under the layer, so asking where it lies now misses it).
#[test]
fn a_clips_transform_moved_from_under_a_scaled_layer_to_the_root_draws_it_again() {
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let layer = scaled_twice(&mut scene);
    let u = scene.add_transform(
        (0.0, 0.0),
        Some(GroupTransform {
            origin: (0.0, 0.0),
            scale: 1.0,
            rotate: 0.0,
        }),
    );
    if let Some(n) = scene.properties.transforms.get_mut(usize::try_from(u).unwrap_or(0)) {
        n.parent = layer.0;
    }
    let c = scene.add_clip(square_clip(rect(4.0, 4.0, 8.0, 8.0)), u);
    square_in(&mut scene, layer, c);
    redrawn_as_whole(scene, |scene| {
        if let Some(n) = scene.properties.transforms.get_mut(usize::try_from(u).unwrap_or(0)) {
            n.parent = 0;
        }
        Changed {
            transforms: vec![u],
            ..Changed::default()
        }
    });
}

/// A layer scaled twice under a clip of the canvas (no output clip): its
/// key went through the canvas, so it read the layer's own map there. The
/// layer scaled to 1.5, its region as it was, the clip cuts other texels:
/// it is drawn again.
#[test]
fn a_layer_scaled_under_a_clip_of_the_canvas_draws_its_texels_again() {
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let layer = scaled_twice(&mut scene);
    let c = scene.add_clip(square_clip(rect(20.0, 20.0, 4.0, 4.0)), 0);
    // Drawn in the canvas's space, so the layer's motion signs no chunk
    // again: what draws the layer again is what its key read.
    square_in(&mut scene, (0, layer.1), c);
    redrawn_as_whole(scene, |scene| {
        if let Some(n) = scene
            .properties
            .transforms
            .get_mut(usize::try_from(layer.0).unwrap_or(0))
        {
            n.matrix = Some(GroupTransform {
                origin: (0.0, 0.0),
                scale: 1.5,
                rotate: 0.0,
            });
        }
        Changed {
            transforms: vec![layer.0],
            effects: vec![layer.1],
            ..Changed::default()
        }
    });
}

/// A layer under its output clip (the left half), its chunk's clip in it;
/// that clip moved to a sibling branch (the top half) by its parent alone:
/// the contract breaks, and the frame drawn from what changed says so.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "does not lie in the layer's output clip")]
fn a_clip_moved_to_a_sibling_branch_of_its_layers_output_clip_stops_a_debug_build() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let left = scene.add_clip(square_clip(rect(0.0, 0.0, 32.0, 64.0)), 0);
    let top = scene.add_clip(square_clip(rect(0.0, 0.0, 64.0, 32.0)), 0);
    let c = scene.add_clip(square_clip(rect(4.0, 4.0, 20.0, 20.0)), 0);
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(c).unwrap_or(0)) {
        n.parent = left;
    }
    let effect = scene.add_effect(filtered(EffectNode {
        region: whole_region(64),
        clip: left,
        ..EffectNode::default()
    }));
    scene.push_under(
        State {
            clip: c,
            effect,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 64.0, 64.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(c).unwrap_or(0)) {
        n.parent = top;
    }
    gpu.render(
        &scene,
        &Changed {
            clips: vec![c],
            ..Changed::default()
        },
        &mut text,
    );
}

/// Two clips of the canvas, one in the other, their left edges both at
/// x = 21, cutting a filtered layer scaled twice: the outer keeps each
/// canvas pixel the inner touches whole, so the canvas's chain leaves it
/// out; but the layer's texel 10 spans canvas pixels 20 and 21, and each
/// clip keeps half of it, a quarter in all. Walking parents in the
/// layer's pixels, the texel holds a quarter (the effective chain's half
/// would show past the edge as darker than half).
#[test]
fn coincident_edges_in_a_layer_scaled_twice_each_cut_its_texels() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let (scene, _) = coincident_edges_scaled_twice();
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    // Texels 9, 10 and 11 hold 0, a quarter and all: pixel 21 samples
    // texel 10 with some of 11, under half either way.
    let [r, ..] = at(&p, 21, 32);
    assert!(r > 135, "pixel 21 keeps {} of black", 255 - u16::from(r));
    assert_eq!(at(&p, 30, 32), [0, 0, 0, 255]);
}

/// A clip of no width, under a transform scaling two and a half times
/// about the canvas's corner, at x = 9 (22.5 on the canvas: pixel 22's
/// centre), cutting a black square: it keeps nothing, not even half of the
/// pixel whose centre lies on the line it collapsed to.
#[test]
fn a_collapsed_clip_in_a_scaled_space_paints_nothing() {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
    let t = scene.add_transform(
        (0.0, 0.0),
        Some(GroupTransform {
            origin: (0.0, 0.0),
            scale: 2.5,
            rotate: 0.0,
        }),
    );
    let c = scene.add_clip(square_clip(rect(9.0, 0.0, 0.0, 20.0)), t);
    scene.push_under(
        State {
            transform: t,
            clip: c,
            ..State::default()
        },
        vec![Item::Rect {
            bounds: rect(0.0, 0.0, 20.0, 20.0),
            color: Color::BLACK,
        }],
    );
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    let p = gpu.read_canvas().expect("a canvas");
    for x in 20..26 {
        assert_eq!(at(&p, x, 20), [255; 4], "pixel {x}");
    }
}

/// A filtered layer turned or scaled under its output clip, holding a
/// rounded clip in its own space and a clip in that: the clips move with
/// it, so their keys in its texels (through its space, not the canvas)
/// hold, none is keyed again, and its texture is kept, composited again.
#[test]
fn a_layer_moved_with_the_clips_inside_it_keeps_its_texture() {
    for (from, to) in [((30.0, 1.0), (75.0, 1.0)), ((0.0, 0.9), (0.0, 1.3))] {
        let mut gpu = gpu();
        let mut text = Glyphs::for_os();
        let mut scene = Scene::new((64, 64), 1.0, Some(Color::WHITE));
        let g = |(rotate, scale): (f32, f32)| GroupTransform {
            origin: (32.0, 32.0),
            scale,
            rotate,
        };
        let t = scene.add_transform((0.0, 0.0), Some(g(from)));
        let out = scene.add_clip(
            Clip {
                bounds: rect(0.0, 0.0, 64.0, 64.0),
                radius: Corners::default(),
                curve: crate::shape::ROUND,
            },
            0,
        );
        let card = scene.add_clip(
            Clip {
                bounds: rect(12.0, 12.0, 40.0, 40.0),
                radius: Corners::all(9.0),
                curve: crate::shape::ROUND,
            },
            t,
        );
        let row = scene.add_clip(
            Clip {
                bounds: rect(8.0, 20.0, 48.0, 10.0),
                radius: Corners::default(),
                curve: crate::shape::ROUND,
            },
            t,
        );
        for (c, p) in [(card, out), (row, card)] {
            if let Some(n) = scene.properties.clips.get_mut(usize::try_from(c).unwrap_or(0)) {
                n.parent = p;
            }
        }
        let effect = scene.add_effect(filtered(EffectNode {
            transform: t,
            transformed: true,
            region: whole_region(64),
            clip: out,
            ..EffectNode::default()
        }));
        scene.push_under(
            State {
                transform: t,
                clip: row,
                effect,
            },
            vec![Item::Rect {
                bounds: rect(0.0, 0.0, 64.0, 64.0),
                color: Color::BLACK,
            }],
        );
        gpu.render(&scene, &Changed::all(), &mut text);
        gpu.end_frame();
        gpu.clip_keys_walked();
        if let Some(n) = scene.properties.transforms.get_mut(usize::try_from(t).unwrap_or(0)) {
            n.matrix = Some(g(to));
        }
        gpu.render(
            &scene,
            &Changed {
                transforms: vec![t],
                effects: vec![effect],
                ..Changed::default()
            },
            &mut text,
        );
        let d = gpu.drawn();
        gpu.end_frame();
        assert_eq!((d.layers, d.layers_kept), (0, 1), "kept ({from:?} to {to:?}): {d}");
        assert_eq!(gpu.clip_keys_walked(), 0, "keyed again ({from:?} to {to:?})");
        canvas_is_whole(&mut gpu, &scene, &mut text);
    }
}

/// A dialog's layer fading and growing (its opacity and its scale moving
/// each frame) over `rows` rows, each clipped in a rounded panel inside
/// it: the dialog's scene, and its transform, effect and panel's clip.
fn dialog(rows: u16) -> (Scene, [u32; 3]) {
    let mut scene = Scene::new((400, 400), 1.0, Some(Color::WHITE));
    let t = scene.add_transform(
        (0.0, 0.0),
        Some(GroupTransform {
            origin: (200.0, 200.0),
            scale: 0.9,
            rotate: 0.0,
        }),
    );
    let out = scene.add_clip(
        Clip {
            bounds: rect(0.0, 0.0, 400.0, 400.0),
            radius: Corners::default(),
            curve: crate::shape::ROUND,
        },
        0,
    );
    let panel = scene.add_clip(
        Clip {
            bounds: rect(50.0, 50.0, 300.0, 300.0),
            radius: Corners::all(12.0),
            curve: crate::shape::ROUND,
        },
        t,
    );
    if let Some(n) = scene.properties.clips.get_mut(usize::try_from(panel).unwrap_or(0)) {
        n.parent = out;
    }
    let effect = scene.add_effect(filtered(EffectNode {
        opacity: 0.5,
        transform: t,
        transformed: true,
        region: whole_region(400),
        clip: out,
        ..EffectNode::default()
    }));
    for k in 0..rows {
        let y = 50.0 + f32::from(k % 300);
        let row = scene.add_clip(
            Clip {
                bounds: rect(50.0, y, 300.0, 1.0),
                radius: Corners::default(),
                curve: crate::shape::ROUND,
            },
            t,
        );
        if let Some(n) = scene.properties.clips.get_mut(usize::try_from(row).unwrap_or(0)) {
            n.parent = panel;
        }
        scene.push_under(
            State {
                transform: t,
                clip: row,
                effect,
            },
            vec![Item::Rect {
                bounds: rect(40.0, y, 320.0, 1.0),
                color: Color::BLACK,
            }],
        );
    }
    (scene, [t, effect, panel])
}

/// The dialog fading and growing: frame `f`'s change.
fn grow(scene: &mut Scene, [t, effect, _]: [u32; 3], f: u16) -> Changed {
    let k = f32::from(f);
    if let Some(n) = scene.properties.transforms.get_mut(usize::try_from(t).unwrap_or(0)) {
        n.matrix = Some(GroupTransform {
            origin: (200.0, 200.0),
            scale: 0.9 + 0.015 * k,
            rotate: 0.0,
        });
    }
    if let Some(n) = scene.properties.effects.get_mut(usize::try_from(effect).unwrap_or(0)) {
        n.opacity = 0.5 + 0.08 * k;
    }
    Changed {
        transforms: vec![t],
        effects: vec![effect],
        ..Changed::default()
    }
}

/// The dialog's frames after the first, `rows` rows in it, each made by
/// `change`: the clips keyed and the allocations made in each, from the
/// frame's start through its submission.
fn dialog_frames(rows: u16, change: impl Fn(&mut Scene, [u32; 3], u16) -> Changed) -> Vec<(usize, Option<u64>)> {
    let mut gpu = gpu();
    let mut text = Glyphs::for_os();
    let (mut scene, ids) = dialog(rows);
    gpu.render(&scene, &Changed::all(), &mut text);
    gpu.end_frame();
    gpu.clip_keys_walked();
    let mut out = Vec::new();
    for f in 1..=5_u16 {
        let changed = change(&mut scene, ids, f);
        // Each frame begins with the GPU's work done, as the last left it.
        drop(gpu.device.poll(wgpu::PollType::wait_indefinitely()));
        let ((), spent) = crate::heap::allocations(|| {
            gpu.render(&scene, &changed, &mut text);
            gpu.end_frame();
        });
        // Under `GUI_COLD_CHECK` a frame also plans whole to check itself:
        // its allocations are the check's, not the frame's.
        out.push((gpu.clip_keys_walked(), (!gpu.cold_plan).then_some(spent)));
    }
    out
}

/// A dialog fading and growing over thousands of clipped rows: its own
/// motion moves no clip within it, so after the first frame no clip is
/// keyed again, and each frame after the second makes a few allocations,
/// whatever the rows (each row's walk was keyed anew a frame, each into a
/// vector of its own, the keys into a map made anew).
#[test]
fn a_dialog_growing_over_thousands_of_clipped_rows_keys_no_clip_and_allocates_alike() {
    for rows in [1000, 4000] {
        let frames = dialog_frames(rows, grow);
        for (f, (keyed, _)) in frames.iter().enumerate() {
            assert_eq!(*keyed, 0, "frame {f} over {rows} rows: clips keyed again");
        }
        allocates_little(&frames, rows);
    }
}

/// The panel of the dialog rounding (its radius animated) over thousands
/// of rows: every row's walk reads the panel, in the layer's texels, so
/// each is keyed again a frame and the layer drawn again, and still a
/// frame from its start through its submission allocates a few times,
/// whatever the rows.
#[test]
fn a_radius_animated_in_a_layer_over_thousands_of_rows_allocates_alike() {
    let round = |scene: &mut Scene, [_, _, panel]: [u32; 3], f: u16| {
        if let Some(n) = scene.properties.clips.get_mut(usize::try_from(panel).unwrap_or(0)) {
            n.clip.radius = Corners::all(12.0 + f32::from(f));
        }
        Changed {
            clips: vec![panel],
            ..Changed::default()
        }
    };
    for rows in [1000, 4000] {
        let frames = dialog_frames(rows, round);
        for (f, (keyed, _)) in frames.iter().enumerate() {
            assert!(*keyed > usize::from(rows), "frame {f} over {rows} rows: {keyed} keyed");
        }
        allocates_little(&frames, rows);
    }
}

/// Every frame after the first of `frames` (over `rows` rows) made no more
/// than [`FRAME_ALLOCATIONS`]: the first sizes what later ones reuse, and
/// none is made per row (a frame under `GUI_COLD_CHECK` is not counted).
fn allocates_little(frames: &[(usize, Option<u64>)], rows: u16) {
    for (f, frame) in frames.iter().enumerate().skip(1) {
        if let Some(spent) = frame.1 {
            assert!(
                spent <= FRAME_ALLOCATIONS,
                "frame {f} over {rows} rows: {spent} allocations"
            );
        }
    }
}

/// The most allocations a frame that moves, fades or reshapes what is
/// drawn, and adds nothing, may make from its start through its
/// submission: wgpu's own per-submission bookkeeping (some 130 to 160
/// here, a thousand rows or four), with room for a frame that meets
/// the device's housekeeping.
const FRAME_ALLOCATIONS: u64 = 192;
