//! Corners checked against Chrome (M10 S3): every box of
//! `chrome_corners/samples.txt` (radii that overlap and are scaled down by
//! CSS Backgrounds 3 §5.5, `calc(infinity * 1px)` stadiums, radii past
//! half a side, `corner-shape: superellipse(k)`) is laid out and painted
//! through `gui` (a [`Shape`] set on a node, the GPU renderer) and compared
//! pixel by pixel with Chrome's drawing of the same CSS, which
//! `crates/shadcn/verdict/corners.mjs` wrote to `chrome_corners/sheet.png`.
//! The expectations are committed, so the test needs no browser.
//!
//! A box passes when no pixel differs by more than [`MAX_ROUND`] for round
//! corners (an edge's anti-aliasing, not a corner of another size: a
//! radius off by one pixel moves its arc by most of a pixel and fails it)
//! or [`MAX_CURVE`] for a superellipse (Chrome approximates the curve,
//! which the draft leaves to the implementation; gui computes it), and the
//! mean difference stays under [`MEAN`].
//!
//! Regenerate the expectations after changing the samples:
//! `node crates/shadcn/verdict/corners.mjs`.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    reason = "a test: a malformed fixture fails it, and pixels are addressed by their coordinates"
)]

use std::path::{Path, PathBuf};

use gui::prelude::*;
use gui::{Color, Shape, div};

/// The largest difference of one channel at one pixel, of 255, for round corners.
const MAX_ROUND: u8 = 32;
/// The same for a superellipse: about a third of a pixel's coverage.
const MAX_CURVE: u8 = 80;
/// The largest mean difference over a box and its 2 px surround, of 255.
const MEAN: f32 = 2.0;

struct Sample {
    line: String,
    w: f32,
    h: f32,
    shape: Shape,
    x: u32,
    y: u32,
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/chrome_corners")
}

fn samples() -> (u32, u32, Vec<Sample>) {
    let text = std::fs::read_to_string(dir().join("samples.chrome")).expect("samples.chrome");
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    let size: Vec<u32> = lines
        .next()
        .expect("the sheet's size")
        .split(' ')
        .map(|v| v.parse().unwrap())
        .collect();
    let samples = lines
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            let words: Vec<&str> = cols[0].split_whitespace().collect();
            let radius = |s: &str| if s == "full" { Corners::FULL } else { s.parse().unwrap() };
            let radii = Corners {
                top_left: radius(words[2]),
                top_right: radius(words[3]),
                bottom_right: radius(words[4]),
                bottom_left: radius(words[5]),
            };
            let shape = match words.get(6) {
                None => Shape::Rounded(radii),
                Some(k) => Shape::Superellipse {
                    radii,
                    k: k.parse().unwrap(),
                },
            };
            Sample {
                line: cols[0].to_owned(),
                w: words[0].parse().unwrap(),
                h: words[1].parse().unwrap(),
                shape,
                x: cols[1].parse().unwrap(),
                y: cols[2].parse().unwrap(),
            }
        })
        .collect();
    (size[0], size[1], samples)
}

/// An RGBA image's width, height and bytes.
fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).expect("a png")));
    let mut reader = decoder.read_info().expect("a png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("a size")];
    let info = reader.next_frame(&mut buf).expect("a frame");
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => panic!("{other:?}"),
    };
    let rgba = buf[..info.buffer_size()]
        .chunks(channels)
        .flat_map(|p| [p[0], p[1], p[2], if channels == 4 { p[3] } else { 255 }])
        .collect();
    (info.width, info.height, rgba)
}

#[test]
fn corners_match_chrome() {
    let (width, height, samples) = samples();
    let sheet = samples.iter().fold(
        div()
            .position(Position::Relative)
            .width(width as f32)
            .height(height as f32)
            .background(Color::WHITE),
        |sheet, s| {
            sheet.child(
                div()
                    .position(Position::Absolute)
                    .left(s.x as f32)
                    .top(s.y as f32)
                    .width(s.w)
                    .height(s.h)
                    .background(Color::BLACK)
                    .shape(s.shape),
            )
        },
    );
    let out = std::env::temp_dir().join(format!("gui-chrome-corners-{}.png", std::process::id()));
    gui::snapshot(sheet, &out, width, height, 1.0).expect("a snapshot (CI paints on lavapipe)");
    let ours = read_png(&out);
    std::fs::remove_file(&out).ok();
    let chrome = read_png(&dir().join("sheet.png"));
    assert_eq!((ours.0, ours.1), (chrome.0, chrome.1), "sheet size");
    let at = |img: &(u32, u32, Vec<u8>), x: u32, y: u32| {
        let i = ((y * img.0 + x) * 4) as usize;
        [img.2[i], img.2[i + 1], img.2[i + 2]]
    };
    let mut failures = Vec::new();
    for s in &samples {
        let (mut worst, mut sum, mut n) = (0_u8, 0_u64, 0_u64);
        let (x0, y0) = (s.x.saturating_sub(2), s.y.saturating_sub(2));
        let (x1, y1) = (
            (s.x + s.w.ceil() as u32 + 2).min(width),
            (s.y + s.h.ceil() as u32 + 2).min(height),
        );
        let mut at_worst = (0, 0);
        for y in y0..y1 {
            for x in x0..x1 {
                let (a, b) = (at(&ours, x, y), at(&chrome, x, y));
                for c in 0..3 {
                    let d = a[c].abs_diff(b[c]);
                    if d > worst {
                        worst = d;
                        at_worst = (x - s.x, y - s.y);
                    }
                    sum += u64::from(d);
                    n += 1;
                }
            }
        }
        let mean = sum as f32 / n.max(1) as f32;
        let max = if s.shape.curve() == 1.0 { MAX_ROUND } else { MAX_CURVE };
        if worst > max || mean > MEAN {
            failures.push(format!(
                "{}: worst {worst} at {at_worst:?} of the box, mean {mean:.2}",
                s.line
            ));
        }
    }
    assert!(failures.is_empty(), "differs from Chrome:\n{}", failures.join("\n"));
}
