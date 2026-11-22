//! Data chunks on the GPU against the CPU and against what they must look
//! like (ADR-0018).

#![expect(
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    reason = "test arithmetic on small, known values"
)]

use super::kinds::LinesState;
use super::kinds::scatter::ScatterState;
use super::series::{SeriesState, m4};
use super::{DataItem, LineCurve, LineJoin, LineParams, Lines, MarkView, Scatter, ScatterParams, Source};
use crate::color::Color;
use crate::geometry::Bounds;
use crate::gpu::GpuRenderer;
use crate::image::Pixels;
use crate::scene::{Item, Renderer, Scene};
use crate::text::Glyphs;
use std::sync::Arc;

fn gpu() -> GpuRenderer {
    GpuRenderer::headless().expect("a GPU (CI paints on lavapipe)")
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds {
    Bounds { x, y, width, height }
}

fn at(p: &Pixels, x: u32, y: u32) -> [u8; 4] {
    p.pixel(x, y).unwrap_or_else(|| panic!("({x}, {y}) is outside"))
}

fn series_of(xs: &[i64], ys: &[f64]) -> SeriesState {
    let mut s = SeriesState::new(None, false);
    s.append(xs, ys);
    s
}

fn line(view: MarkView, color: Color, width: f32) -> LineParams {
    LineParams {
        view,
        color,
        width,
        area: None,
        join: LineJoin::Miter,
        miter_limit: 4.0,
        curve: LineCurve::Linear,
        floor: false,
        dash: None,
    }
}

fn line_item(key: u64, bounds: Bounds, p: &LineParams, s: &SeriesState) -> Item {
    Item::Data(Arc::new(DataItem::new::<Lines>(
        key,
        bounds,
        bounds.outset(4.0),
        &p.words(),
        vec![Source::Series(s.snapshot())],
    )))
}

fn shoot(gpu: &mut GpuRenderer, scene: &Scene) -> Pixels {
    let shot = gpu.capture(scene, None, &mut Glyphs::for_os()).expect("read back");
    gpu.end_frame();
    shot
}

#[test]
fn a_chunk_paints_in_its_place_between_ui_items() {
    // A red box, a black line across it, a blue box over half the line.
    let s = series_of(&[0, 100], &[0.5, 0.5]);
    let view = MarkView {
        x0: 0,
        x1: 100,
        y0: 0.0,
        y1: 1.0,
        base: None,
    };
    let mut scene = Scene::new((100, 100), 1.0, Some(Color::WHITE));
    scene.push(vec![
        Item::Rect {
            bounds: rect(0.0, 0.0, 100.0, 100.0),
            color: Color::hex(0x00ff_0000),
        },
        line_item(1, rect(0.0, 0.0, 100.0, 100.0), &line(view, Color::BLACK, 10.0), &s),
        Item::Rect {
            bounds: rect(50.0, 0.0, 50.0, 100.0),
            color: Color::hex(0x0000_00ff),
        },
    ]);
    let mut gpu = gpu();
    let shot = shoot(&mut gpu, &scene);
    assert_eq!(at(&shot, 20, 50), [0, 0, 0, 255], "the line over the red box");
    assert_eq!(at(&shot, 20, 20), [255, 0, 0, 255], "the red box beside the line");
    assert_eq!(at(&shot, 80, 50), [0, 0, 255, 255], "the blue box over the line");
    // Drawn again unchanged, the same pixels and nothing brought up to date.
    let again = shoot(&mut gpu, &scene);
    assert_eq!(again.data(), shot.data());
    assert_eq!(
        gpu.chunk_probe().2.updated,
        0,
        "an unchanged chunk is not brought up to date"
    );
}

/// A series crossing a page, with gaps, runs of equal values and jumps.
fn hard_series(n: usize) -> (Vec<i64>, Vec<f64>) {
    let mut rng = crate::testing::Rng::new(0x5eed);
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    let mut x = -3_000_000_000_i64;
    for i in 0..n {
        x += i64::try_from(rng.below(3)).unwrap();
        xs.push(x);
        ys.push(if i % 10_007 == 5 {
            f64::NAN
        } else if i % 5_003 < 40 {
            7.0
        } else {
            f64::from(rng.unit()) * 1e6 - 5e5
        });
    }
    (xs, ys)
}

#[test]
fn m4_on_the_gpu_equals_the_cpu_bit_for_bit() {
    let n = (1 << 22) + 300_000;
    let (xs, ys) = hard_series(n);
    let s = series_of(&xs, &ys);
    let snap = s.snapshot();
    let first = xs[0];
    let last = *xs.last().unwrap();
    let mut gpu = gpu();
    // The whole series, a zoom across the page boundary, a view past its
    // end, each at a fractional left edge.
    let mid = xs[1 << 22];
    let views = [
        (first, last, rect(3.25, 0.0, 777.5, 100.0)),
        (mid - 9_000, mid + 7_000, rect(0.6, 0.0, 1203.0, 100.0)),
        (first - 500, first + 900, rect(10.0, 0.0, 333.3, 100.0)),
        (last - 3, last + 1_000_000, rect(0.0, 0.0, 64.0, 100.0)),
    ];
    for (k, (x0, x1, b)) in views.into_iter().enumerate() {
        let view = MarkView {
            x0,
            x1,
            y0: -6e5,
            y1: 6e5,
            base: None,
        };
        let mut scene = Scene::new((1300, 100), 1.0, Some(Color::WHITE));
        scene.push(vec![line_item(7, b, &line(view, Color::BLACK, 1.0), &s)]);
        shoot(&mut gpu, &scene);
        let (device, queue, rt) = gpu.chunk_probe();
        let state = rt.state_of::<LinesState>(7).expect("the chunk's state");
        let bytes = rt.read_back(device, queue, state.m4.columns.expect("columns"));
        let words: &[u32] = bytemuck::cast_slice(&bytes);
        let frac = b.x - b.x.floor();
        let cpu = m4(&snap, &snap.columns(x0, x1, b.width, frac));
        assert_eq!(usize::try_from(state.m4.n).unwrap(), cpu.len(), "view {k}: columns");
        for (e, c) in cpu.iter().enumerate() {
            let w = &words[e * 20..e * 20 + 20];
            let idx: Vec<u32> = (0..usize::try_from(w[0]).unwrap()).map(|j| w[1 + j * 4 + 3]).collect();
            assert_eq!(
                (w[0], idx.as_slice()),
                (c.count, &c.idx[..usize::try_from(c.count).unwrap()]),
                "view {k}, extended column {e}"
            );
        }
    }
}

#[test]
fn a_gap_breaks_a_line() {
    // Points 10 px apart along y = 0.5, one not a number in the middle.
    let xs: Vec<i64> = (0..=10).collect();
    let ys: Vec<f64> = (0..=10).map(|i| if i == 5 { f64::NAN } else { 0.5 }).collect();
    let s = series_of(&xs, &ys);
    let view = MarkView {
        x0: 0,
        x1: 10,
        y0: 0.0,
        y1: 1.0,
        base: None,
    };
    let mut scene = Scene::new((100, 40), 1.0, Some(Color::WHITE));
    scene.push(vec![line_item(
        3,
        rect(0.0, 0.0, 100.0, 40.0),
        &line(view, Color::BLACK, 4.0),
        &s,
    )]);
    let shot = shoot(&mut gpu(), &scene);
    assert_eq!(at(&shot, 25, 20), [0, 0, 0, 255], "drawn before the gap");
    assert_eq!(at(&shot, 75, 20), [0, 0, 0, 255], "drawn after it");
    for x in 42..58 {
        assert_eq!(
            at(&shot, x, 20),
            [255, 255, 255, 255],
            "no line through the gap at x = {x}"
        );
    }
}

#[test]
fn dots_are_culled_and_kept_in_order() {
    let mut rng = crate::testing::Rng::new(9);
    let n = 200_000_usize;
    let xs: Vec<i64> = (0..n).map(|i| i64::try_from(i).unwrap()).collect();
    let ys: Vec<f64> = (0..n).map(|_| f64::from(rng.unit()) * 3.0 - 1.0).collect();
    let s = series_of(&xs, &ys);
    let view = MarkView {
        x0: 50_000,
        x1: 150_000,
        y0: 0.0,
        y1: 1.0,
        base: None,
    };
    let p = ScatterParams {
        view,
        color: Color::BLACK,
        radius: 1.0,
    };
    let b = rect(0.0, 0.0, 200.0, 200.0);
    let mut scene = Scene::new((200, 200), 1.0, Some(Color::WHITE));
    scene.push(vec![Item::Data(Arc::new(DataItem::new::<Scatter>(
        5,
        b,
        b,
        &p.words(),
        vec![Source::Series(s.snapshot())],
    )))]);
    let mut gpu = gpu();
    shoot(&mut gpu, &scene);
    let (device, queue, rt) = gpu.chunk_probe();
    let state = rt.state_of::<ScatterState>(5).expect("the chunk's state");
    let args = rt.read_back(device, queue, state.args.expect("args"));
    let shown_n = bytemuck::cast_slice::<u8, u32>(&args)[1];
    let shown = rt.read_back(device, queue, state.bufs[0].shown.expect("shown"));
    let shown: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&shown)[..usize::try_from(shown_n).unwrap()].to_vec();
    // What the CPU keeps: in the view, widened by a dot (radius + 1 px).
    let reach_x = 2.0 / (200.0 / 100_000.0);
    let reach_y = 2.0 / 200.0;
    let cpu: Vec<u32> = (0..u32::try_from(n).unwrap())
        .filter(|i| {
            let (x, y) = (xs[*i as usize] as f64, ys[*i as usize] as f32 as f64);
            x >= 50_000.0 - reach_x && x <= 150_000.0 + reach_x && (-reach_y..=1.0 + reach_y).contains(&y)
        })
        .collect();
    assert!(shown.windows(2).all(|w| w[0] < w[1]), "in index order");
    // The GPU's test is in f32 pixels: the set differs from the CPU's at most
    // at the very edges.
    let edge = cpu.len().abs_diff(shown.len());
    assert!(edge <= 8, "{} shown, {} on the CPU", shown.len(), cpu.len());
    let lo = s.snapshot().lower_bound(50_000 - 1000);
    assert!(shown.iter().all(|i| *i >= lo), "only the points in the view's reach");
}

#[test]
fn a_stream_uploads_only_its_new_points() {
    let mut s = SeriesState::new(Some(1_000_000), false);
    let mut x = 0_i64;
    let mut push = |s: &mut SeriesState, n: i64| {
        let xs: Vec<i64> = (x..x + n).collect();
        let ys: Vec<f64> = xs.iter().map(|v| (*v as f64 * 0.001).sin()).collect();
        s.append(&xs, &ys);
        x += n;
    };
    push(&mut s, 2_000_000);
    let mut gpu = gpu();
    let b = rect(0.0, 0.0, 400.0, 100.0);
    let frame = |s: &SeriesState| {
        let snap = s.snapshot();
        let first = snap.get(0).unwrap().0;
        let last = snap.get(snap.len() - 1).unwrap().0;
        let view = MarkView {
            x0: first,
            x1: last,
            y0: -1.0,
            y1: 1.0,
            base: None,
        };
        let mut scene = Scene::new((400, 100), 1.0, Some(Color::WHITE));
        scene.push(vec![line_item(9, b, &line(view, Color::BLACK, 1.0), s)]);
        scene
    };
    let mut text = Glyphs::for_os();
    let mut render = |gpu: &mut GpuRenderer, scene: &Scene| {
        gpu.render(scene, &crate::scene::Changed::all(), &mut text);
        let up = gpu.drawn().uploaded;
        gpu.end_frame();
        up
    };
    let first = render(&mut gpu, &frame(&s));
    let kept = u64::from(s.snapshot().len());
    assert!(
        (1_000_000..1_400_000).contains(&kept),
        "about the last million kept: {kept}"
    );
    assert!(first >= kept * 12, "the first frame uploads every point kept: {first}");
    // 120 frames of 8 333 points (a million a second at 120 Hz).
    for _ in 0..120 {
        push(&mut s, 8_333);
        let up = render(&mut gpu, &frame(&s));
        // The points (x as two words, y) and the pyramid nodes over them.
        assert!(
            up < 8_333 * 12 + 8_333 * 12 / 10 + 65_536,
            "{up} bytes for 8 333 points"
        );
    }
}

/// The golden's points (`golden/*.svg`, y up from its bottom edge).
const GOLDEN: [(i64, f64); 6] = [(8, 20.0), (40, 64.0), (56, 30.0), (88, 50.0), (120, 12.0), (152, 40.0)];

/// `items` drawn at 160×80 against Chrome's shot of `name`.svg: how far
/// apart (the mean difference of a channel, 0 to 255, and the share of
/// pixels a channel of which is more than 32 apart). Where they differ
/// most, ours is the exact area coverage and Skia's is not: an edge pixel
/// whose box a 3 px line covers three quarters of is 0.75 here and 0.5 in
/// Chrome, and a miter's tip pixel is covered 0.9 here and 0.5 there
/// (ADR-0018, as built).
fn against_chrome(name: &str, png: &[u8], items: Vec<Item>) -> (f64, f64) {
    let mut scene = Scene::new((160, 80), 1.0, Some(Color::WHITE));
    scene.push(items);
    let ours = shoot(&mut gpu(), &scene);
    let chrome = Pixels::decode_png(png).expect("the golden decodes");
    let (mut sum, mut far) = (0_u64, 0_u32);
    for y in 0..80 {
        for x in 0..160 {
            let (a, b) = (at(&ours, x, y), at(&chrome, x, y));
            let mut worst = 0;
            for c in 0..3 {
                let d = a[c].abs_diff(b[c]);
                sum += u64::from(d);
                worst = worst.max(d);
            }
            far += u32::from(worst > 32);
        }
    }
    let mean = sum as f64 / (160.0 * 80.0 * 3.0);
    if std::env::var_os("GUI_GOLDEN_OUT").is_some() {
        std::fs::write(
            std::env::temp_dir().join(format!("gui-golden-{name}.png")),
            ours.encode_png().unwrap(),
        )
        .unwrap();
    }
    (mean, f64::from(far) / (160.0 * 80.0))
}

fn golden_series() -> SeriesState {
    let xs: Vec<i64> = GOLDEN.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = GOLDEN.iter().map(|p| p.1).collect();
    series_of(&xs, &ys)
}

const GOLDEN_VIEW: MarkView = MarkView {
    x0: 0,
    x1: 160,
    y0: 0.0,
    y1: 80.0,
    base: None,
};

fn golden_bounds() -> Bounds {
    rect(0.0, 0.0, 160.0, 80.0)
}

#[test]
fn lines_and_areas_match_svg_in_chrome() {
    let s = golden_series();
    let blue = Color::hex(0x0025_63eb);
    let cases: [(&str, &[u8], LineParams); 5] = [
        ("line", include_bytes!("golden/line.png"), line(GOLDEN_VIEW, blue, 3.0)),
        (
            "line_dash",
            include_bytes!("golden/line_dash.png"),
            LineParams {
                dash: super::LineDash::new(&[10.0, 5.0], 3.0),
                ..line(GOLDEN_VIEW, blue, 3.0)
            },
        ),
        (
            "line_round",
            include_bytes!("golden/line_round.png"),
            LineParams {
                join: LineJoin::Round,
                ..line(GOLDEN_VIEW, blue, 6.0)
            },
        ),
        (
            "line_bevel",
            include_bytes!("golden/line_bevel.png"),
            LineParams {
                join: LineJoin::Bevel,
                ..line(GOLDEN_VIEW, blue, 6.0)
            },
        ),
        (
            "area",
            include_bytes!("golden/area.png"),
            LineParams {
                area: Some((blue.opacity(0.4), blue.opacity(0.4))),
                ..line(GOLDEN_VIEW, blue, 1.0)
            },
        ),
    ];
    for (name, png, p) in cases {
        let (mean, far) = against_chrome(name, png, vec![line_item(11, golden_bounds(), &p, &s)]);
        assert!(
            mean < 1.0 && far < 0.01,
            "{name}: mean {mean:.3}, {far:.4} of pixels far apart"
        );
    }
}

#[test]
fn bars_and_dots_match_svg_in_chrome() {
    let s = golden_series();
    let bars = super::BarParams {
        view: GOLDEN_VIEW,
        color: Color::hex(0x0016_a34a),
        width: 12.0,
        offset: -6.0,
        radius: 4.0,
        floor: false,
    };
    let dots = ScatterParams {
        view: GOLDEN_VIEW,
        color: Color::hex(0x00dc_2626).opacity(0.8),
        radius: 3.0,
    };
    let b = golden_bounds();
    let item = |kind: Item| vec![kind];
    let cases = [
        (
            "bars",
            &include_bytes!("golden/bars.png")[..],
            item(Item::Data(Arc::new(DataItem::new::<super::Bars>(
                12,
                b,
                b.outset(8.0),
                &bars.words(),
                vec![Source::Series(s.snapshot())],
            )))),
        ),
        (
            "scatter",
            &include_bytes!("golden/scatter.png")[..],
            item(Item::Data(Arc::new(DataItem::new::<Scatter>(
                13,
                b,
                b.outset(8.0),
                &dots.words(),
                vec![Source::Series(s.snapshot())],
            )))),
        ),
    ];
    for (name, png, items) in cases {
        let (mean, far) = against_chrome(name, png, items);
        assert!(
            mean < 1.0 && far < 0.01,
            "{name}: mean {mean:.3}, {far:.4} of pixels far apart"
        );
    }
}

#[test]
fn candles_and_heatmaps_match_svg_in_chrome() {
    // (x, open, high, low, close), as `golden/candles.svg` draws them.
    let c = [
        (20, 30.0, 50.0, 20.0, 45.0),
        (50, 45.0, 60.0, 35.0, 40.0),
        (80, 40.0, 70.0, 38.0, 66.0),
        (110, 66.0, 72.0, 30.0, 34.0),
        (140, 34.0, 44.0, 10.0, 20.0),
    ];
    let xs: Vec<i64> = c.iter().map(|v| v.0).collect();
    let part = |f: fn(&(i64, f64, f64, f64, f64)) -> f64| series_of(&xs, &c.iter().map(f).collect::<Vec<_>>());
    let ohlc = [part(|v| v.1), part(|v| v.2), part(|v| v.3), part(|v| v.4)];
    let p = super::CandleParams {
        view: GOLDEN_VIEW,
        width: 10.0,
        wick: 1.0,
        up: Color::hex(0x0016_a34a),
        down: Color::hex(0x00dc_2626),
    };
    let b = golden_bounds();
    let candles = Item::Data(Arc::new(DataItem::new::<super::Candles>(
        14,
        b,
        b,
        &p.words(),
        ohlc.iter().map(|s| Source::Series(s.snapshot())).collect(),
    )));
    let (mean, far) = against_chrome("candles", include_bytes!("golden/candles.png"), vec![candles]);
    assert!(
        mean < 1.0 && far < 0.01,
        "candles: mean {mean:.3}, {far:.4} of pixels far apart"
    );

    let values: Vec<f32> = (0..4)
        .flat_map(|j| (0..8).map(move |i| f32::from(u8::try_from((i * 3 + j * 5) % 8).unwrap()) / 7.0))
        .collect();
    let h = super::HeatmapParams {
        low: Color::hex(0x00ef_f6ff),
        high: Color::hex(0x001e_3a8a),
        min: 0.0,
        max: 1.0,
    };
    let heat = Item::Data(Arc::new(DataItem::new::<super::Heatmap>(
        15,
        b,
        b,
        &h.words(),
        vec![Source::Grid(super::GridSnapshot::new(8, 4, values))],
    )));
    let (mean, far) = against_chrome("heatmap", include_bytes!("golden/heatmap.png"), vec![heat]);
    assert!(
        mean < 1.0 && far < 0.01,
        "heatmap: mean {mean:.3}, {far:.4} of pixels far apart"
    );
}

mod plots {
    use super::super::plot::{Hover, Series, line, plot, scatter, series};
    use crate::app::Scenario;
    use crate::color::Color;
    use crate::element::{IntoElement, div, text_bound};
    use crate::prelude::*;
    use crate::reactive::Signal;

    fn ramp(s: Series, n: i64) {
        let xs: Vec<i64> = (0..n).collect();
        let ys: Vec<f64> = xs.iter().map(|x| (*x as f64 * 0.37).sin()).collect();
        s.append(&xs, &ys);
    }

    #[test]
    fn appending_records_the_plots_chunk_alone() {
        let s = series();
        ramp(s, 1000);
        let mut sc = Scenario::measuring(
            move || {
                div()
                    .size(pct(100.0))
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .child("a label beside the plot")
                    .child(
                        plot()
                            .y_auto(Default::default())
                            .mark(line(s).color(Color::BLACK))
                            .build()
                            .width(300.0)
                            .height(100.0),
                    )
                    .into_element()
            },
            400,
            200,
            1.0,
        )
        .unwrap();
        sc.settle().unwrap();
        sc.take_reports();
        s.push(1000, 3.0);
        sc.settle().unwrap();
        let r = sc.take_reports().pop().unwrap();
        assert!(r.rendered.is_empty(), "no instance renders: {:?}", r.rendered);
        assert_eq!(r.paint.recorded, 1, "only the plot's chunk records: {}", r.paint);
        assert!(r.drawn.uploaded < 4096, "one point and its nodes: {}", r.drawn.uploaded);
    }

    #[test]
    fn hovering_finds_the_point_in_the_frame_the_pointer_moved() {
        let s = series();
        ramp(s, 101);
        let dots = series();
        dots.append(&[10, 50, 90], &[0.5, -0.5, 0.0]);
        let hovered: Signal<Option<Hover>> = Signal::new(None);
        let mut sc = Scenario::measuring(
            move || {
                div()
                    .size(pct(100.0))
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .child(
                        plot()
                            .x(0, 100)
                            .y(-1.0, 1.0)
                            .mark(line(s))
                            .mark(scatter(dots).radius(4.0))
                            .on_hover(12.0, move |h| hovered.set(h))
                            .build()
                            .width(400.0)
                            .height(200.0),
                    )
                    .child(text_bound(move || match hovered.get() {
                        Some(h) => format!("mark {} point {} at {}", h.mark, h.index, h.x),
                        None => "nothing".to_owned(),
                    }))
                    .into_element()
            },
            400,
            300,
            1.0,
        )
        .unwrap();
        sc.settle().unwrap();
        let shown = |sc: &Scenario, t: &str| crate::inspect::find_text(&sc.last().nodes, t).is_some();
        assert!(shown(&sc, "nothing"));
        // Over x = 25 (100 px), away from every dot: the line's point.
        sc.hover(100.0, 100.0);
        sc.settle().unwrap();
        assert!(
            shown(&sc, "mark 0 point 25 at 25"),
            "{}",
            crate::inspect::dump(&sc.last().nodes)
        );
        // Next to the dot at (50, -0.5), y 150 px: the dot's point.
        sc.hover(203.0, 148.0);
        sc.settle().unwrap();
        assert!(
            shown(&sc, "mark 1 point 1 at 50"),
            "{}",
            crate::inspect::dump(&sc.last().nodes)
        );
        sc.leave();
        sc.settle().unwrap();
        assert!(shown(&sc, "nothing"));
    }

    #[test]
    fn rules_cross_the_plot_where_their_values_are_dashed_or_solid() {
        let s = series();
        ramp(s, 101);
        let red = Color::hex(0x00ff_0000);
        let blue = Color::hex(0x0000_00ff);
        let mut sc = Scenario::measuring(
            move || {
                div()
                    .size(pct(100.0))
                    .child(
                        plot()
                            .x(0, 100)
                            .y(0.0, 1.0)
                            .mark(super::super::plot::rule_y(&[0.5]).color(red).width(2.0))
                            .mark(
                                super::super::plot::rule_x(&[25, 500])
                                    .color(blue)
                                    .width(2.0)
                                    .dash(&[4.0, 4.0], 0.0),
                            )
                            .build()
                            .width(200.0)
                            .height(100.0),
                    )
                    .into_element()
            },
            200,
            100,
            1.0,
        )
        .unwrap();
        sc.settle().unwrap();
        let shot = sc.shot(false).unwrap();
        let px = |x: u32, y: u32| shot.pixel(x, y).unwrap();
        // y = 0.5 is the middle row: 2 px about y = 50, solid all across.
        for x in [10, 100, 190] {
            assert_eq!(px(x, 49)[..3], [255, 0, 0], "row 49 at {x}");
            assert_eq!(px(x, 50)[..3], [255, 0, 0], "row 50 at {x}");
            assert_eq!(px(x, 46)[..3], [255, 255, 255], "row 46 at {x}");
        }
        // x = 25 is 50 px in: 4 px dashes and gaps down from the top, the
        // red rule over it.
        for y in 0..40 {
            let on = (y / 4) % 2 == 0;
            let want = if on { [0, 0, 255] } else { [255, 255, 255] };
            assert_eq!(px(49, y)[..3], want, "column 49 at {y}");
            assert_eq!(px(50, y)[..3], want, "column 50 at {y}");
        }
        // x = 500 is outside the view: nothing past the first rule.
        for x in 60..200 {
            assert_eq!(px(x, 20)[..3], [255, 255, 255], "column {x}");
        }
    }

    #[test]
    fn chunks_drawn_after_changes_equal_ones_built_cold() {
        let s = series();
        ramp(s, 2000);
        let below = series();
        below.append(&(0..2000).collect::<Vec<i64>>(), &vec![-1.5; 2000]);
        let view = Signal::new((0_i64, 2000_i64));
        let mut sc = Scenario::measuring(
            move || {
                div()
                    .size(pct(100.0))
                    .child(
                        plot()
                            .x_with(move || view.get())
                            .y_auto(Default::default())
                            .mark(line(below).color(Color::BLACK))
                            .mark(
                                line(s)
                                    .color(Color::hex(0x0025_63eb))
                                    .width(3.0)
                                    .dash(&[6.0, 3.0], 1.0)
                                    .fill(Color::hex(0x0025_63eb).opacity(0.3), Color::TRANSPARENT)
                                    .on(below),
                            )
                            .mark(super::super::plot::rule_y(&[0.0]).dash(&[2.0, 2.0], 0.0))
                            .build()
                            .width(300.0)
                            .height(120.0),
                    )
                    .into_element()
            },
            320,
            140,
            1.5,
        )
        .unwrap();
        sc.check_cold(true);
        sc.settle().unwrap();
        // Points appended, then a pan, a zoom in and one out past the data.
        for step in 0..4 {
            match step {
                0 => {
                    let xs: Vec<i64> = (2000..2600).collect();
                    let ys: Vec<f64> = xs.iter().map(|x| (*x as f64 * 0.37).sin()).collect();
                    s.append(&xs, &ys);
                    below.append(&xs, &vec![-1.5; xs.len()]);
                }
                1 => view.set((300, 2300)),
                2 => view.set((1000, 1100)),
                _ => view.set((-500, 4000)),
            }
            sc.settle().unwrap();
        }
        let (_, headless) = sc.parts();
        assert!(
            headless.gpu.chunks_checked_cold() >= 2 * 5,
            "both lines checked at every step: {}",
            headless.gpu.chunks_checked_cold()
        );
    }
}

/// A user kind for the readback belt: its compute writes `2i + 7` into 64
/// words and reads them back, tagged with its first param word.
struct Doubler {
    fill: super::ComputeId,
}

#[derive(Default)]
struct DoublerState {
    out: Option<super::BufferId>,
}

impl super::ChunkKind for Doubler {
    type Params = super::Words;
    type State = DoublerState;

    fn pipelines(ctx: &mut super::PipelineBuilder<'_>) -> Self {
        Self {
            fill: ctx.compute(
                "test doubler",
                "@group(0) @binding(0) var<storage, read_write> out: array<u32>;
                 @compute @workgroup_size(64)
                 fn main(@builtin(global_invocation_id) id: vec3<u32>) { out[id.x] = id.x * 2u + 7u; }",
                "main",
                &[super::BindKind::StorageWrite],
            ),
        }
    }

    fn update(
        &self,
        ctx: &mut super::ChunkUpload<'_>,
        state: &mut DoublerState,
        _: &super::Words,
        _: &[super::DataOp],
    ) {
        ctx.buffer(&mut state.out, 256);
    }

    fn compute(
        &self,
        pass: &mut super::ChunkCompute<'_>,
        state: &DoublerState,
        params: &super::Words,
        _: &super::FrameInfo,
    ) {
        if let Some(b) = state.out {
            pass.dispatch(self.fill, &[super::Bind::Buffer(b)], [1, 1, 1]);
            pass.read_back(b, 0, 256, params[0]);
        }
    }

    fn draw(&self, _: &mut super::ChunkDraw<'_>, _: &DoublerState, _: &super::Words, _: &super::FrameInfo) {}
}

#[test]
fn a_user_chunk_reads_back_what_its_compute_wrote() {
    super::register::<Doubler>();
    let mut gpu = gpu();
    let rb = super::readback();
    let item = |tag: u32, reading: bool| {
        let mut words = [0_u32; 64];
        words[0] = tag;
        let item = DataItem::new::<Doubler>(
            31,
            rect(0.0, 0.0, 16.0, 16.0),
            rect(0.0, 0.0, 16.0, 16.0),
            &words,
            vec![],
        );
        let item = if reading { item.reading_back(rb.to()) } else { item };
        let mut scene = Scene::new((16, 16), 1.0, Some(Color::WHITE));
        scene.push(vec![Item::Data(Arc::new(item))]);
        scene
    };
    // Not asked to: nothing is read.
    shoot(&mut gpu, &item(5, false));
    assert!(!gpu.raster_pending(), "no read is out");
    // Asked to: what the compute wrote arrives through the mailbox, and
    // sets the status signal, without any frame waiting for it.
    shoot(&mut gpu, &item(9, true));
    // The frame did not wait: what came back (if the GPU was that quick)
    // sits in the mailbox until the UI thread takes it.
    assert_eq!(rb.latest().get(), None, "nothing waited for the GPU");
    for _ in 0..100 {
        if !gpu.wait_reads() {
            break;
        }
    }
    assert!(crate::mailbox::deliver());
    let got = rb.latest().get().expect("a readout");
    assert_eq!(got.tag, 9);
    let words: Vec<u32> = got
        .bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|w| u32::from_le_bytes(*w))
        .collect();
    assert_eq!(words, (0..64).map(|i| i * 2 + 7).collect::<Vec<u32>>());
    // Unchanged, the chunk computes nothing and reads nothing again.
    shoot(&mut gpu, &item(9, true));
    assert!(!gpu.raster_pending(), "no new read");
}
