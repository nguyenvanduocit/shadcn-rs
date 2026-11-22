//! Decorations checked against Chrome (M10 S3): the avatar's
//! `after:border after:border-border after:mix-blend-darken
//! dark:after:mix-blend-lighten` over a photo, and the translucent menu's
//! `bg-popover/70` over `backdrop-blur-2xl backdrop-saturate-150`, light
//! and dark, over a busy page, as `gui` paints them (an
//! [`OverlayBorder`] with a [`Blend`], a [`Backdrop`]) and as Chrome does
//! (`crates/shadcn/verdict/decoration.mjs` wrote `chrome_decoration/`).
//! The expectations are committed, so the test needs no browser.
//!
//! Each region of the sheet is compared pixel by pixel. The avatars and the
//! page pass within a few levels (measured: 2 at most, Chrome 145). The
//! menus' blur is a 40 px Gaussian that Skia and `gui` both run scaled
//! down, each its own way (measured: 8 at most, a mean of 2.6), so a menu
//! passes on a looser bound; a border not blended (a normal one), a blur
//! that read past its box, or none, fails them by far more.
//!
//! Regenerate the expectations after changing the sheet (here and in the
//! script, in step): `node crates/shadcn/verdict/decoration.mjs`.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    reason = "a test: a malformed fixture fails it, and pixels are addressed by their coordinates"
)]

use std::path::{Path, PathBuf};

use gui::image::ImageSource;
use gui::prelude::*;
use gui::{Color, Shape, div, image};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/chrome_decoration")
}

/// `decoration.mjs`'s `SHEET`, in step with it.
const SIZE: (u32, u32) = (320, 200);
const BARS: [u32; 6] = [0xef4444, 0x22c55e, 0x3b82f6, 0xeab308, 0xa855f7, 0x111827];
const BAR: f32 = 40.0;
const LINE: (f32, f32) = (12.0, 2.0);

struct Menu {
    at: (f32, f32),
    size: (f32, f32),
    background: Color,
}

struct Avatar {
    at: (f32, f32),
    panel: Color,
    border: Color,
    blend: Blend,
}

fn menus() -> [Menu; 2] {
    [
        Menu {
            at: (20.0, 20.0),
            size: (150.0, 90.0),
            background: Color::oklch(1.0, 0.0, 0.0).opacity(0.7),
        },
        Menu {
            at: (90.0, 110.0),
            size: (140.0, 70.0),
            background: Color::oklch(0.205, 0.0, 0.0).opacity(0.7),
        },
    ]
}

fn avatars() -> [Avatar; 2] {
    [
        Avatar {
            at: (256.0, 24.0),
            panel: Color::oklch(1.0, 0.0, 0.0),
            border: Color::oklch(0.922, 0.0, 0.0),
            blend: Blend::Darken,
        },
        Avatar {
            at: (256.0, 128.0),
            panel: Color::oklch(0.145, 0.0, 0.0),
            border: Color::oklch(1.0, 0.0, 0.0).opacity(0.1),
            blend: Blend::Lighten,
        },
    ]
}

fn abs(x: f32, y: f32, w: f32, h: f32) -> gui::Div {
    div().position(Position::Absolute).left(x).top(y).width(w).height(h)
}

/// The sheet as `gui` draws it.
fn sheet() -> gui::Div {
    let (w, h) = (SIZE.0 as f32, SIZE.1 as f32);
    let photo =
        ImageSource::from_png(&std::fs::read(dir().join("photo.png")).expect("photo.png")).expect("a png photo");
    let mut root = div()
        .position(Position::Relative)
        .width(w)
        .height(h)
        .background(Color::WHITE);
    for (i, c) in BARS.iter().enumerate() {
        root = root.child(abs(i as f32 * BAR, 0.0, BAR, h).background(Color::hex(*c)));
    }
    let page = BARS.len() as f32 * BAR;
    let mut y = 0.0;
    while y < h {
        root = root.child(abs(0.0, y, page, LINE.1).background(Color::BLACK));
        y += LINE.0;
    }
    for m in menus() {
        root = root.child(
            abs(m.at.0, m.at.1, m.size.0, m.size.1)
                .shape(Shape::Rounded(Corners::all(8.0)))
                .background(m.background)
                .backdrop(Some(Backdrop {
                    blur: 40.0,
                    saturate: 1.5,
                })),
        );
    }
    for (i, a) in avatars().into_iter().enumerate() {
        root = root
            .child(abs(page, i as f32 * h / 2.0, w - page, h / 2.0).background(a.panel))
            .child(
                abs(a.at.0, a.at.1, 48.0, 48.0)
                    .shape(Shape::Stadium)
                    .overlay_border(OverlayBorder {
                        width: 1.0,
                        color: a.border,
                        blend: a.blend,
                    })
                    .child(image(photo.clone()).width(48.0).height(48.0).shape(Shape::Stadium)),
            );
    }
    root
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

/// A region of the sheet and what it may differ by: the largest
/// difference of one channel at one pixel and the mean, of 255.
struct Region {
    name: &'static str,
    rect: (u32, u32, u32, u32),
    worst: u8,
    mean: f32,
}

#[test]
fn decorations_match_chrome() {
    let text = std::fs::read_to_string(dir().join("sheet.chrome")).expect("sheet.chrome");
    let size: Vec<u32> = text
        .lines()
        .find(|l| !l.starts_with('#'))
        .expect("the sheet's size")
        .split(' ')
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!((size[0], size[1]), SIZE, "the sheet the script drew is the test's");
    let out = std::env::temp_dir().join(format!("gui-chrome-decoration-{}.png", std::process::id()));
    gui::snapshot(sheet(), &out, SIZE.0, SIZE.1, 1.0).expect("a snapshot (CI paints on lavapipe)");
    let ours = read_png(&out);
    // `GUI_DECORATION_KEEP=1` keeps ours beside Chrome's, to look at.
    if std::env::var_os("GUI_DECORATION_KEEP").is_some() {
        println!("ours: {}", out.display());
    } else {
        std::fs::remove_file(&out).ok();
    }
    let chrome = read_png(&dir().join("sheet.png"));
    assert_eq!((ours.0, ours.1), (chrome.0, chrome.1), "sheet size");
    let at = |img: &(u32, u32, Vec<u8>), x: u32, y: u32| {
        let i = ((y * img.0 + x) * 4) as usize;
        [img.2[i], img.2[i + 1], img.2[i + 2]]
    };
    let [light, dark] = menus();
    let menu = |m: &Menu| {
        (
            m.at.0 as u32,
            m.at.1 as u32,
            (m.at.0 + m.size.0) as u32,
            (m.at.1 + m.size.1) as u32,
        )
    };
    let avatar = |a: &Avatar| {
        let (x, y) = (a.at.0 as u32, a.at.1 as u32);
        (x - 2, y - 2, x + 50, y + 50)
    };
    let [a_light, a_dark] = avatars();
    let regions = [
        Region {
            name: "the page",
            rect: (0, 0, 20, 200),
            worst: 2,
            mean: 0.5,
        },
        Region {
            name: "the light translucent menu",
            rect: menu(&light),
            worst: 16,
            mean: 4.0,
        },
        Region {
            name: "the dark translucent menu",
            rect: menu(&dark),
            worst: 16,
            mean: 4.0,
        },
        Region {
            name: "the light avatar (darken)",
            rect: avatar(&a_light),
            worst: 6,
            mean: 0.5,
        },
        Region {
            name: "the dark avatar (lighten)",
            rect: avatar(&a_dark),
            worst: 6,
            mean: 0.5,
        },
    ];
    let mut report = Vec::new();
    let mut failed = false;
    for r in &regions {
        let (x0, y0, x1, y1) = r.rect;
        let (mut worst, mut sum, mut n, mut at_worst) = (0_u8, 0_u64, 0_u64, (0, 0));
        for y in y0..y1 {
            for x in x0..x1 {
                let (a, b) = (at(&ours, x, y), at(&chrome, x, y));
                for c in 0..3 {
                    let d = a[c].abs_diff(b[c]);
                    if d > worst {
                        worst = d;
                        at_worst = (x, y);
                    }
                    sum += u64::from(d);
                    n += 1;
                }
            }
        }
        let mean = sum as f32 / n.max(1) as f32;
        let ok = worst <= r.worst && mean <= r.mean;
        failed |= !ok;
        report.push(format!(
            "{}{}: worst {worst} at {at_worst:?} (ours {:?}, Chrome {:?}), mean {mean:.2}",
            if ok { "" } else { "FAIL " },
            r.name,
            at(&ours, at_worst.0, at_worst.1),
            at(&chrome, at_worst.0, at_worst.1),
        ));
    }
    println!("{}", report.join("\n"));
    assert!(!failed, "differs from Chrome:\n{}", report.join("\n"));
}
