//! The theme as groups a setter's mount registers with (review §12.5 level 2, `ThemeHandle::bind_ink`): a node
//! bound so holds no tracker, a theme change runs the mounts of the group it changed and no other, and a closure
//! reading the theme still follows it.

use std::cell::Cell;
use std::rc::Rc;

use gui::prelude::*;
use gui::{Scenario, component, div};
use shadcn::design::{Corner, VEGA};
use shadcn::theme::{ColorRole, RadiusStep, Scheme, Theme, ThemeHandle, Tone, install};

const N: usize = 12;

type Installed = Rc<Cell<Option<ThemeHandle>>>;

/// An app installing the default theme, around `N` swatches `swatch` makes.
fn app(swatch: fn(ThemeHandle) -> gui::Div) -> (impl Fn() -> gui::Element, Installed) {
    let installed: Installed = Rc::new(Cell::new(None));
    let keep = Rc::clone(&installed);
    let render = move || {
        let keep = Rc::clone(&keep);
        div()
            .child(component((), move |_| {
                let (t, _) = install(Theme::default(), VEGA);
                keep.set(Some(t));
                move || {
                    div()
                        .display(Display::Flex)
                        .flex_wrap(true)
                        .children((0..N).map(|_| swatch(t).named("swatch").size(20.0)))
                        .into_element()
                }
            }))
            .into_element()
    };
    (render, installed)
}

fn sourced(t: ThemeHandle) -> gui::Div {
    div()
        .background(t.bind_ink(Tone::role(ColorRole::Card)))
        .radius(t.bind_corner(Corner::Step(RadiusStep::Md)))
}

fn closures(t: ThemeHandle) -> gui::Div {
    div()
        .background(move || t.ink(Tone::role(ColorRole::Card)))
        .radius(move || t.corner(Corner::Step(RadiusStep::Md)))
}

fn background(s: &Scenario) -> Option<gui::Color> {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some("swatch"))
        .and_then(|n| n.look.background)
}

/// Trackers alive once the app settled, and the scenario.
#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
fn settled(swatch: fn(ThemeHandle) -> gui::Div) -> (usize, Scenario, ThemeHandle) {
    let (render, installed) = app(swatch);
    let mut s = Scenario::measuring(render, 400, 300, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    s.take_reports();
    let t = installed.get().expect("the app installed its theme");
    (gui::reactive::graph().renders, s, t)
}

#[test]
fn theme_sources_hold_no_tracker_and_wake_by_group() {
    // What the engine makes once per thread (a debug build's check of a map) made before counting.
    drop(settled(sourced));
    let (with_closures, closure_app, _) = settled(closures);
    drop(closure_app);
    let (with_sources, mut s, t) = settled(sourced);
    assert!(
        with_closures >= with_sources + 2 * N,
        "two bound setters per swatch: a closure holds a tracker each, a source none ({with_closures} against \
         {with_sources})"
    );

    let n = u64::try_from(N).unwrap_or(u64::MAX);
    s.act("dark", || t.update(|t| t.scheme = Scheme::Dark));
    let r = s.frame("dark").expect("a frame");
    assert!(r.rendered.is_empty(), "no instance renders");
    assert_eq!(r.bindings, n, "the scheme runs the colour mounts alone");
    let painted = Theme::default().scheme(Scheme::Dark).palette().painted().card;
    assert_eq!(background(&s), Some(painted));

    s.act("radius", || t.update(|t| t.radius = 14.0));
    let r = s.frame("radius").expect("a frame");
    assert_eq!(r.bindings, n, "the radius runs the corner mounts alone");

    s.act("same", || t.set(t.get()));
    let r = s.frame("same").expect("a frame");
    assert_eq!(r.bindings, 0, "the same theme runs nothing");
}

fn constant(t: ThemeHandle) -> gui::Div {
    div()
        .radius(t.bind_corner(Corner::Full))
        .shadow(t.bind_shadow(shadcn::theme::ShadowStep::None))
}

/// A corner that does not follow the radius, and no shadow, are plain values: a theme change runs nothing for
/// them.
#[test]
fn constant_theme_values_register_nowhere() {
    let (_, mut s, t) = settled(constant);
    s.act("radius", || t.update(|t| t.radius = 14.0));
    let r = s.frame("radius").expect("a frame");
    assert_eq!(r.bindings, 0, "a full corner follows no radius");
    s.act("shadows", || t.update(|t| t.shadows.md = t.shadows.xl.clone()));
    let r = s.frame("shadows").expect("a frame");
    assert_eq!(r.bindings, 0, "no shadow follows no scale");
}

#[test]
fn a_closure_reading_the_theme_still_follows_it() {
    let (_, mut s, t) = settled(closures);
    s.act("dark", || t.update(|t| t.scheme = Scheme::Dark));
    let r = s.frame("dark").expect("a frame");
    assert_eq!(
        r.bindings,
        u64::try_from(N).unwrap_or(u64::MAX),
        "each colour closure runs"
    );
    let painted = Theme::default().scheme(Scheme::Dark).palette().painted().card;
    assert_eq!(background(&s), Some(painted));
}
