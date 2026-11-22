//! The GPU renderer's tests: what it draws against exact references, and
//! that a steady frame makes nothing anew.

use super::path::{TileRect, reference};
use super::*;
use crate::convert::i32_to_f32;
use crate::scene::{Clip, EffectNode, Gradient, GroupTransform, Item, PixelRect, State};
use crate::shape::{Cap, FillRule, Join, PathBuilder};

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
    assert_eq!(at(&shot, 20, 20), [128, 128, 128, 255], "half black over white");
    let mut scene = chunks((128, 64), &[(1, State::default(), &a), (3, State::default(), &over)]);
    fade(&mut scene, 0.5, 0.0);
    gpu.render(&scene, &Changed::all(), &mut text);
    let d = gpu.drawn();
    gpu.end_frame();
    assert_eq!((d.folded, d.layers), (0, 1), "overlapping: a layer, {d}");
    canvas_is_whole(&mut gpu, &scene, &mut text);
    let shot = gpu.capture(&scene, None, &mut text).expect("read back");
    assert_eq!(at(&shot, 25, 25), [128, 128, 128, 255], "the overlap is faded once");
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
