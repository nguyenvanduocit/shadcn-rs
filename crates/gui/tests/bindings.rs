//! Setters take a value or a closure (ADR-0013, ADR-0022): a closure is
//! bound to its node and runs where the node resolves; a property has one
//! owner; and replaying a binding writes its property without allocating.

use gui::app::Scenario;
use gui::prelude::*;
use gui::style::Style;
use gui::{Color, Signal, div, text};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

fn shadow(y: f32) -> Shadow {
    Shadow {
        x: 0.0,
        y,
        blur: 2.0 * y,
        spread: 0.0,
        color: Color::BLACK.opacity(0.1),
    }
}

/// The node's fixed style with its bindings replayed over it.
fn resolved(mut node: gui::Div) -> Style {
    let (style, bindings) = node.style_and_bindings();
    let mut s = style.clone();
    if let Some(b) = bindings {
        b.apply_to(&mut s);
    }
    s
}

#[test]
fn a_setter_takes_a_value_a_conversion_or_a_closure() {
    let wide = Signal::new(true);
    let s = resolved(
        div()
            .width(200.0)
            .height(Length::Auto)
            .max_width(pct(50.0))
            .padding(16.0)
            .padding_x(move || if wide.get() { 10.0 } else { 2.0 })
            .radius(8.0)
            .line_height(20.0)
            .font_weight(500)
            .gap(move || 6.0)
            .align_items(Align::Center)
            .background(Color::WHITE)
            .shadow(shadow(1.0))
            .grid_columns([Track::Fr(1.0), Track::Auto]),
    );
    assert_eq!(s.width, Length::Px(200.0));
    assert_eq!(s.max_width, Length::Fraction(0.5));
    assert_eq!(
        s.padding,
        Edges {
            top: 16.0,
            right: 10.0,
            bottom: 16.0,
            left: 10.0
        }
    );
    assert_eq!(s.shape, gui::Shape::Rounded(Corners::all(8.0)));
    assert_eq!(s.content.text.line_height, Some(LineHeight::Px(20.0)));
    assert_eq!(s.content.text.weight, Some(500));
    assert_eq!((s.row_gap, s.column_gap), (6.0, 6.0));
    assert_eq!(s.align_items, Some(Align::Center));
    assert_eq!(s.background, Some(Color::WHITE));
    assert_eq!(&*s.shadows, &[shadow(1.0)]);
    assert_eq!(&*s.grid_columns, &[Track::Fr(1.0), Track::Auto]);
}

#[test]
fn the_last_write_of_a_property_owns_it() {
    let p = Signal::new(8.0_f32);
    // A fixed write after a binding of the same property drops the binding.
    let mut node = div().background(move || Color::BLACK).background(Color::WHITE);
    assert!(node.style_and_bindings().1.is_some_and(|b| b.is_empty()));
    assert_eq!(resolved(node).background, Some(Color::WHITE));
    // A binding after a fixed write owns the property.
    assert_eq!(
        resolved(div().width(10.0).width(move || p.get())).width,
        Length::Px(8.0)
    );
    // Overlapping properties apply in the order they were set.
    let s = resolved(div().padding(move || p.get()).padding_x(4.0));
    assert_eq!((s.padding.top, s.padding.left, s.padding.right), (8.0, 4.0, 4.0));
    let s = resolved(div().padding_x(4.0).padding(move || p.get()));
    assert_eq!((s.padding.top, s.padding.left), (8.0, 8.0));
    // A write is known by the fields it sets, not by its setter: a later
    // write of every field a binding wrote drops it, whatever its setter.
    let mut node = div().padding_x(move || p.get()).padding(8.0);
    assert!(node.style_and_bindings().1.is_some_and(|b| b.is_empty()));
    let mut node = div().width(move || p.get() * 2.0).size(40.0);
    assert!(node.style_and_bindings().1.is_some_and(|b| b.is_empty()));
    assert_eq!(
        resolved(div().width(move || p.get() * 2.0).size(40.0)).width,
        Length::Px(40.0)
    );
    let s = resolved(div().padding_x(move || p.get()).padding(move || 2.0));
    assert_eq!((s.padding.top, s.padding.left), (2.0, 2.0));
    // A write of only some of them keeps the binding for the rest.
    let mut node = div().padding(move || p.get()).padding_x(4.0);
    assert!(node.style_and_bindings().1.is_some_and(|b| !b.is_empty()));
    let mut node = div().size(move || p.get()).width(40.0);
    assert!(node.style_and_bindings().1.is_some_and(|b| !b.is_empty()));
    let s = resolved(div().size(move || p.get()).width(40.0));
    assert_eq!((s.width, s.height), (Length::Px(40.0), Length::Px(8.0)));
    // A bare style runs a closure as it is given.
    assert_eq!(Style::default().width(move || p.get()).width, Length::Px(8.0));
}

#[test]
fn replaying_a_binding_allocates_nothing() {
    let hovered = Signal::new(false);
    let mut node = div()
        .background(move || Some(if hovered.get() { Color::BLACK } else { Color::WHITE }))
        .shadow(move || {
            if hovered.get() {
                Shadows::from_iter([shadow(1.0), shadow(4.0)])
            } else {
                Shadows::from_iter([shadow(1.0)])
            }
        })
        .grid_columns(move || Tracks::repeat(if hovered.get() { 4 } else { 3 }, Track::Fr(1.0)))
        .padding(move || if hovered.get() { 12.0 } else { 8.0 })
        .text_size(14.0);
    let (style, bindings) = node.style_and_bindings();
    let base = style.clone();
    let bindings = bindings.expect("an element keeps its bindings");
    let mut s = base.clone();
    bindings.apply_to(&mut s);
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let ((), heap) = gui::heap::measure(|| {
        for i in 0..1_000 {
            hovered.set(i % 2 == 0);
            let mut s = base.clone();
            bindings.apply_to(&mut s);
            assert!(!s.shadows.spilled() && !s.grid_columns.spilled());
            std::hint::black_box(&s);
        }
    });
    let made = heap.map_or(u64::MAX, |h| h.allocations);
    assert_eq!(made, 0, "{made} allocations over 1,000 replays");
}

#[test]
fn a_bound_padding_lays_out_without_rendering() {
    let pad = Signal::new(4.0_f32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .children((0..20).map(move |i| {
                    let row = div().height(10.0).child(text(format!("row {i}")));
                    if i == 7 { row.padding(move || pad.get()) } else { row }
                }))
                .into_element()
        },
        300,
        400,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.take_reports();
    s.act("pad", || pad.set(12.0));
    let r = s.frame("padded").unwrap();
    assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
    assert_eq!(r.visits.resolved, 1, "the bound node alone resolves: {}", r.visits);
    assert!(r.layout.misses > 0, "a padding lays out");
}

#[test]
fn a_fixed_write_after_a_binding_leaves_nothing_tracked() {
    let colour = Signal::new(Color::BLACK);
    let mut s = Scenario::measuring(
        move || {
            div()
                .size(100.0)
                .background(move || colour.get())
                .background(Color::WHITE)
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.settle().unwrap();
    s.take_reports();
    s.act("colour", || colour.set(Color::TRANSPARENT));
    let r = s.frame("coloured").unwrap();
    assert_eq!(r.visits.resolved, 0, "the dropped binding is not tracked: {}", r.visits);
}
