//! A node's shape and decoration (M10 S3, ADR-0022): a ring at an offset,
//! an edge bar, an inset layer, a border over the children (blended with
//! them), a blurred backdrop, a superellipse's clip, hit testing by shape
//! and hit slop, the placement a layer reports; and writing any of them
//! through a binding allocates nothing.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::cast_precision_loss,
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    reason = "a test: pixels addressed by their coordinates"
)]

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gui::app::Scenario;
use gui::prelude::*;
use gui::style::Style;
use gui::{Color, Positioning, Side, SideAlign, Signal, deferred, div};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const BLUE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};

/// `view` painted on white, `w`×`h` at scale 1: a pixel's colour.
fn paint(view: gui::Div, w: u32, h: u32) -> impl Fn(u32, u32) -> [u8; 3] {
    let out = std::env::temp_dir().join(format!(
        "gui-decoration-{}-{:?}.png",
        std::process::id(),
        std::thread::current().id()
    ));
    let root = div()
        .width(w as f32)
        .height(h as f32)
        .background(Color::WHITE)
        .child(view);
    gui::snapshot(root, &out, w, h, 1.0).expect("a snapshot (CI paints on lavapipe)");
    let (width, rgba) = read_png(&out);
    std::fs::remove_file(&out).ok();
    move |x, y| {
        let i = ((y * width + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }
}

fn read_png(path: &Path) -> (u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).expect("a png")));
    let mut reader = decoder.read_info().expect("a png header");
    let mut buf = vec![0; reader.output_buffer_size().expect("a size")];
    let info = reader.next_frame(&mut buf).expect("a frame");
    assert_eq!(info.color_type, png::ColorType::Rgba);
    buf.truncate(info.buffer_size());
    (info.width, buf)
}

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

#[test]
fn a_ring_at_an_offset_leaves_the_gap_unpainted() {
    let at = paint(
        div()
            .margin(10.0)
            .size(20.0)
            .background(Color::BLACK)
            .ring(Ring::new(2.0, RED).offset(2.0)),
        40,
        40,
    );
    assert_eq!(at(15, 20), BLACK);
    // The 2 px gap, then the 2 px ring, then nothing.
    assert_eq!(at(9, 20), WHITE);
    assert_eq!(at(8, 20), WHITE);
    assert_eq!(at(7, 20), [255, 0, 0]);
    assert_eq!(at(6, 20), [255, 0, 0]);
    assert_eq!(at(5, 20), WHITE);
}

#[test]
fn a_ring_keeps_a_square_corner_square() {
    // lyra's and sera's square corners: Chrome keeps a zero radius sharp
    // for an outline at an offset and for a spread ring alike.
    for ring in [Ring::new(2.0, RED), Ring::outline(2.0, RED)] {
        let at = paint(
            div()
                .margin(10.0)
                .size(20.0)
                .background(Color::BLACK)
                .ring(ring.offset(2.0)),
            40,
            40,
        );
        // The ring's outer corner pixel and the gap's inner one.
        assert_eq!(at(6, 6), [255, 0, 0], "{ring:?}");
        assert_eq!(at(33, 33), [255, 0, 0], "{ring:?}");
        assert_eq!(at(8, 8), WHITE, "{ring:?}");
        assert_eq!(at(10, 10), BLACK, "{ring:?}");
    }
}

#[test]
fn a_ring_follows_a_round_corner_out() {
    // A 6 px corner, the ring 2 px out and 2 px wide: its outer edge is a
    // 10 px curve, so the corner's last pixel stays white and the curve's
    // middle is red.
    let at = paint(
        div()
            .margin(10.0)
            .size(20.0)
            .shape(6.0)
            .background(Color::BLACK)
            .ring(Ring::new(2.0, RED).offset(2.0)),
        40,
        40,
    );
    assert_eq!(at(6, 6), WHITE);
    // On the diagonal, 10 − 10/√2 ≈ 2.9 px in from the outer corner.
    assert_eq!(at(9, 9), [255, 0, 0]);
}

#[test]
fn an_edge_bar_covers_its_side_over_the_border() {
    let at = paint(
        div()
            .width(60.0)
            .height(30.0)
            .border(1.0)
            .border_color(BLUE)
            .edge_bar(EdgeBar {
                side: Side::Left,
                width: 2.0,
                color: RED,
            }),
        60,
        30,
    );
    assert_eq!(at(0, 15), [255, 0, 0]);
    assert_eq!(at(1, 15), [255, 0, 0]);
    assert_eq!(at(2, 15), WHITE);
    assert_eq!(at(0, 0), [255, 0, 0], "the bar runs the border box's length");
    assert_eq!(at(59, 15), [0, 0, 255]);
}

#[test]
fn an_inset_layer_paints_inside_behind_the_content() {
    let at = paint(
        div()
            .size(60.0)
            // A flex container: its child's margin stays inside it.
            .display(Display::Flex)
            .inset_layer(InsetLayer {
                inset: Edges::all(8.0),
                shape: Shape::Rounded(Corners::all(8.0)),
                background: Some(BLUE),
                border: 1.0,
                border_color: Some(RED),
                shadows: Shadows::new(),
            })
            .child(div().margin(20.0).size(20.0).background(Color::BLACK)),
        60,
        60,
    );
    assert_eq!(at(4, 30), WHITE, "outside the inset");
    assert_eq!(at(8, 30), [255, 0, 0], "its border");
    assert_eq!(at(12, 30), [0, 0, 255], "its fill");
    assert_eq!(at(30, 30), BLACK, "the content over it");
    assert_eq!(at(9, 9), WHITE, "its own corners");
    assert_eq!(at(12, 12), [0, 0, 255]);
}

#[test]
fn an_inset_layer_and_an_overlay_border_lie_inside_the_border() {
    // CSS places `before:inset-*` and `after:inset-0` against the padding
    // box: a node's own 4 px border stays outside both.
    let at = paint(
        div()
            .size(40.0)
            .border(4.0)
            .border_color(Color::BLACK)
            .inset_layer(InsetLayer {
                inset: Edges::all(4.0),
                shape: Shape::Rect,
                background: Some(BLUE),
                border: 0.0,
                border_color: None,
                shadows: Shadows::new(),
            })
            .overlay_border(OverlayBorder {
                width: 2.0,
                color: RED,
                blend: Blend::Normal,
            }),
        40,
        40,
    );
    assert_eq!(at(2, 20), BLACK, "the node's border");
    assert_eq!(at(5, 20), [255, 0, 0], "the overlay, inside it");
    assert_eq!(at(7, 20), WHITE, "the inset's gap from the padding box");
    assert_eq!(at(9, 20), [0, 0, 255], "the inset layer");
}

#[test]
fn an_overlay_border_paints_over_the_children() {
    let at = paint(
        div()
            .size(40.0)
            .shape(Shape::Stadium)
            .overlay_border(OverlayBorder {
                width: 4.0,
                color: RED,
                blend: Blend::Normal,
            })
            .child(div().size(40.0).background(Color::BLACK)),
        40,
        40,
    );
    assert_eq!(at(20, 20), BLACK);
    assert_eq!(at(1, 20), [255, 0, 0], "over the child, along the shape");
    assert_eq!(at(20, 2), [255, 0, 0]);
}

/// An avatar's `after:border after:mix-blend-darken`: the line darkens
/// what is lighter than it and leaves what is darker.
#[test]
fn a_darkening_overlay_border_keeps_the_darker_of_each_channel() {
    let gray = Color {
        r: 0.5,
        g: 0.5,
        b: 0.5,
        a: 1.0,
    };
    let at = paint(
        div()
            .size(40.0)
            .display(Display::Flex)
            .overlay_border(OverlayBorder {
                width: 4.0,
                color: gray,
                blend: Blend::Darken,
            })
            .child(div().width(20.0).height(40.0).background(Color::WHITE))
            .child(div().width(20.0).height(40.0).background(Color::BLACK)),
        40,
        40,
    );
    assert_eq!(at(1, 20), [128, 128, 128], "darker than white");
    assert_eq!(at(38, 20), BLACK, "black is darker than the line");
    assert_eq!(at(10, 20), WHITE, "inside the border");
}

/// A translucent menu's `backdrop-blur`: what lies below shows through its
/// fill blurred, inside its shape only.
#[test]
fn a_backdrop_blurs_what_lies_below_inside_the_shape() {
    let stripes = (0..20).fold(div().size(80.0).display(Display::Flex), |row, _| {
        row.child(div().width(2.0).height(80.0).background(Color::BLACK))
            .child(div().width(2.0).height(80.0))
    });
    let menu = div()
        .position(Position::Absolute)
        .left(20.0)
        .top(20.0)
        .size(40.0)
        .shape(Shape::Rounded(Corners::all(8.0)))
        .backdrop(Some(Backdrop::blur(4.0)));
    let at = paint(
        div().size(80.0).position(Position::Relative).child(stripes).child(menu),
        80,
        80,
    );
    let mid = at(40, 40)[0];
    assert!(mid.abs_diff(128) <= 8, "blurred to the stripes' mean: {mid}");
    assert_eq!(at(9, 40), BLACK, "outside, the stripes stay");
    assert_eq!(at(21, 21)[0], at(21, 10)[0], "the corner is outside the shape");
}

/// A squircle that clips clips its children to its own curve, not to a
/// circle's, as its background follows it.
#[test]
fn a_superellipse_clips_its_children_to_its_curve() {
    let clip = |shape: Shape| {
        paint(
            div()
                .size(40.0)
                .shape(shape)
                .overflow(Overflow::Hidden)
                .child(div().size(40.0).background(Color::BLACK)),
            40,
            40,
        )
    };
    // The point (4, 4): a squircle of radius 20 holds it, a circle not.
    assert_eq!(clip(Shape::squircle(20.0))(4, 4), BLACK);
    assert_eq!(clip(Shape::Rounded(Corners::all(20.0)))(4, 4), WHITE);
    assert_eq!(clip(Shape::squircle(20.0))(0, 0), WHITE);
}

/// A squircle's shadow follows its curve: the point (10, 10) lies in the
/// shadow of a squircle grown by 4 px, not in a circle's.
#[test]
fn a_superellipse_casts_a_shadow_of_its_curve() {
    let shadowed = |shape: Shape| {
        paint(
            div().margin(10.0).size(40.0).shape(shape).shadow(Shadow {
                x: 0.0,
                y: 0.0,
                blur: 0.0,
                spread: 4.0,
                color: RED,
            }),
            60,
            60,
        )
    };
    assert_eq!(shadowed(Shape::squircle(20.0))(10, 10), [255, 0, 0]);
    assert_eq!(shadowed(Shape::Rounded(Corners::all(20.0)))(10, 10), WHITE);
}

#[test]
fn hit_testing_follows_the_shape_and_its_slop() {
    let clicks = Rc::new(Cell::new(0));
    let c = Rc::clone(&clicks);
    let slop = Signal::new(0.0_f32);
    let mut s = Scenario::measuring(
        move || {
            let c = Rc::clone(&c);
            div()
                .padding(20.0)
                .child(
                    div()
                        .size(40.0)
                        .shape(Shape::Stadium)
                        .hit_slop(move || Edges::all(slop.get()))
                        .on_click(move |_| c.set(c.get() + 1)),
                )
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.frame("open").expect("a frame");
    s.click(21.0, 21.0);
    assert_eq!(clicks.get(), 0, "outside the round corner");
    s.click(40.0, 40.0);
    assert_eq!(clicks.get(), 1);
    s.click(15.0, 40.0);
    assert_eq!(clicks.get(), 1, "beside it");
    s.act("slop", || slop.set(8.0));
    s.frame("slop").expect("a frame");
    s.click(15.0, 40.0);
    assert_eq!(clicks.get(), 2, "within its slop");
}

#[test]
fn hit_testing_follows_a_rounded_clip() {
    // A child filling a rounded `overflow: hidden` parent: the corner the
    // clip cuts off paints nothing of it, and takes no click for it.
    let clicks = Rc::new(Cell::new(0));
    let c = Rc::clone(&clicks);
    let mut s = Scenario::measuring(
        move || {
            let c = Rc::clone(&c);
            div()
                .padding(20.0)
                .child(
                    div()
                        .size(40.0)
                        .shape(20.0)
                        .overflow(Overflow::Hidden)
                        .child(div().size(40.0).on_click(move |_| c.set(c.get() + 1))),
                )
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.frame("open").expect("a frame");
    s.click(22.0, 22.0);
    assert_eq!(clicks.get(), 0, "in the corner the clip cuts off");
    s.click(40.0, 40.0);
    assert_eq!(clicks.get(), 1);
    s.click(22.0, 40.0);
    assert_eq!(clicks.get(), 2, "inside the clip at its edge");
}

#[test]
fn a_layer_reports_how_it_was_placed() {
    let placed: Signal<Option<gui::Placement>> = Signal::new(None);
    let top = Signal::new(10.0_f32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .padding_top(move || top.get())
                .child(
                    div().margin_left(20.0).width(60.0).height(20.0).child(
                        deferred(
                            Positioning::Anchored {
                                side: Side::Bottom,
                                align: SideAlign::Center,
                                offset: 4.0,
                                match_width: false,
                                fit_height: false,
                            },
                            div().width(100.0).height(50.0),
                        )
                        .placement(placed),
                    ),
                )
                .into_element()
        },
        300,
        200,
        1.0,
    )
    .expect("a scenario");
    s.frame("open").expect("a frame");
    let p = placed.get_untracked().expect("placed");
    assert_eq!((p.side, p.align), (Side::Bottom, SideAlign::Center));
    assert_eq!((p.anchor.x, p.anchor.y, p.anchor.width), (20.0, 10.0, 60.0));
    assert_eq!(p.origin, (50.0, 0.0), "the middle of its top edge");
    // From the anchor's bottom and the gap to the window's edge less its margin.
    assert_eq!(p.available.1, 200.0 - 8.0 - 30.0 - 4.0);
    // No room below: it flips, and says so.
    s.act("low", || top.set(150.0));
    s.frame("low").expect("a frame");
    let p = placed.get_untracked().expect("placed");
    assert_eq!(p.side, Side::Top);
    assert_eq!(p.origin, (50.0, 50.0), "the middle of its bottom edge");
}

#[test]
fn an_overlay_coming_and_going_records_as_a_cold_frame_does() {
    let on = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            div()
                .size(60.0)
                .overlay_border(move || {
                    on.get().then_some(OverlayBorder {
                        width: 1.0,
                        color: RED,
                        blend: Blend::Normal,
                    })
                })
                .child(div().size(30.0).background(Color::BLACK))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.frame("off").expect("a frame");
    for (i, v) in [true, false, true].into_iter().enumerate() {
        s.act("toggle", || on.set(v));
        s.frame(&format!("toggle {i}")).expect("a frame");
    }
}

#[test]
fn writing_shape_and_decoration_allocates_nothing() {
    let hovered = Signal::new(false);
    let shadow = |y: f32| Shadow {
        x: 0.0,
        y,
        blur: 2.0 * y,
        spread: 0.0,
        color: Color::BLACK.opacity(0.1),
    };
    let mut node = div()
        .shape(move || {
            if hovered.get() {
                Shape::Stadium
            } else {
                Shape::squircle(12.0)
            }
        })
        .radius_left(move || if hovered.get() { 0.0 } else { 4.0 })
        .ring(move || hovered.get().then_some(Ring::new(3.0, BLUE).offset(2.0)))
        .edge_bar(move || {
            Some(EdgeBar {
                side: if hovered.get() { Side::Left } else { Side::Top },
                width: 2.0,
                color: RED,
            })
        })
        .inset_layer(move || {
            Some(InsetLayer {
                inset: Edges::all(if hovered.get() { 8.0 } else { 4.0 }),
                shape: Shape::Rounded(Corners::all(26.0)),
                background: Some(Color::WHITE),
                border: 1.0,
                border_color: Some(BLUE),
                shadows: [shadow(1.0), shadow(8.0)].into(),
            })
        })
        .overlay_border(move || {
            Some(OverlayBorder {
                width: 1.0,
                color: BLUE,
                blend: if hovered.get() { Blend::Darken } else { Blend::Lighten },
            })
        })
        .backdrop(move || Some(Backdrop::blur(if hovered.get() { 40.0 } else { 8.0 })))
        .hit_slop(move || Edges::all(if hovered.get() { 8.0 } else { 0.0 }));
    let (style, bindings) = node.style_and_bindings();
    let base: Style = style.clone();
    let bindings = bindings.expect("an element keeps its bindings");
    let mut s = base.clone();
    bindings.apply_to(&mut s);
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let ((), heap) = gui::heap::measure(|| {
        for i in 0..1_000 {
            hovered.set(i % 2 == 0);
            let mut s = base.clone();
            bindings.apply_to(&mut s);
            std::hint::black_box(&s);
        }
    });
    let made = heap.map_or(u64::MAX, |h| h.allocations);
    assert_eq!(made, 0, "{made} allocations over 1,000 replays");
}
