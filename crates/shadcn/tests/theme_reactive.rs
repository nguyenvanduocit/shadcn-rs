//! ADR-0022's reactivity: the theme and the style in scope are one group per field with equality cutoff, so a
//! switch reruns only the bindings that read a group it changed. Switching the theme or the scheme renders no
//! instance and lays out no node; a subtree's override follows an app-wide switch.

use std::cell::Cell;
use std::rc::Rc;

use gui::prelude::*;
use gui::{ComponentElement, Effect, Scenario, Signal, component, div, text};
use shadcn::design::{AccordionFrame, Corner, use_design, with_design};
use shadcn::design::{DesignHandle, TextRole, VEGA};
use shadcn::theme::{
    Accent, BaseColor, ColorRole, Menu, MenuAccent, RadiusStep, Scheme, ShadowStep, TextStep, Theme, ThemeHandle, Tone,
    install, use_theme, with_theme,
};

/// How many times `f` has run, as an effect.
fn runs(f: impl Fn() + 'static) -> (Effect, Rc<Cell<u32>>) {
    let n = Rc::new(Cell::new(0_u32));
    let count = Rc::clone(&n);
    let e = Effect::new(move || {
        f();
        count.set(count.get().saturating_add(1));
    });
    (e, n)
}

#[test]
fn each_group_reruns_only_what_read_it() {
    // Outside any instance, `install` provides for the whole thread.
    let (t, d) = install(Theme::default(), VEGA);
    let (_ink, ink) = runs(move || {
        std::hint::black_box(t.ink(Tone::role(ColorRole::Accent)));
    });
    let (_corner, corner) = runs(move || {
        std::hint::black_box(t.corner(Corner::Step(RadiusStep::Md)));
    });
    let (_text, text_runs) = runs(move || {
        std::hint::black_box(t.text(TextRole::step(TextStep::Sm, 500)));
    });
    let (_shadow, shadow) = runs(move || {
        std::hint::black_box(t.shadow(ShadowStep::Md));
    });
    let (_size, size) = runs(move || {
        std::hint::black_box(d.size());
    });
    let (_accordion, accordion) = runs(move || {
        std::hint::black_box(d.accordion());
    });
    let all = |want: [u32; 6]| {
        assert_eq!(
            [
                ink.get(),
                corner.get(),
                text_runs.get(),
                shadow.get(),
                size.get(),
                accordion.get()
            ],
            want,
            "ink, corner, text, shadow, size, accordion"
        );
    };
    all([1, 1, 1, 1, 1, 1]);

    // Light to dark: only what resolves a tone.
    t.update(|t| t.scheme = Scheme::Dark);
    all([2, 1, 1, 1, 1, 1]);
    assert_eq!(t.palette(), Theme::default().colors.dark);

    // The same theme again: nothing.
    t.set(t.get());
    all([2, 1, 1, 1, 1, 1]);

    // A radius edit: only shapes.
    t.update(|t| t.radius = 14.0);
    all([2, 2, 1, 1, 1, 1]);
    assert!((t.corner(Corner::Step(RadiusStep::Md)) - 11.2).abs() < 1e-5);
    assert_eq!(t.corner(Corner::Capped(RadiusStep::Md, 8.0)), 8.0);

    // Translucent menus change no colour token: the inks stop at the cutoff.
    t.update(|t| t.menu.translucent = true);
    all([2, 2, 1, 1, 1, 1]);
    // A bold menu accent does.
    t.update(|t| t.menu.accent = MenuAccent::Bold);
    all([3, 2, 1, 1, 1, 1]);
    assert_eq!(t.palette().accent, t.palette().primary);

    // A whole new theme with the same radius, scale and shadows: inks only.
    t.set(
        Theme::new(BaseColor::Stone)
            .accent(Accent::Blue)
            .scheme(Scheme::Dark)
            .menu(Menu {
                accent: MenuAccent::Bold,
                inverted: false,
                translucent: true,
            }),
    );
    all([4, 3, 1, 1, 1, 1]);

    // A style edit of one part reruns that part's readers only.
    d.update(|d| {
        d.parts.accordion.frame = AccordionFrame::Boxed {
            corner: Corner::Step(RadiusStep::Xl2),
            padding: 16.0,
            open: Tone::of(shadcn::theme::Ink::Alpha(ColorRole::Muted, 0.5)),
        }
    });
    all([4, 3, 1, 1, 1, 2]);
    d.set(VEGA);
    all([4, 3, 1, 1, 1, 3]);
}

/// A swatch reading the theme and style through bindings only, as S8's components do.
fn swatch(i: usize) -> ComponentElement {
    component(i, |_| {
        let (t, d) = (use_theme(), use_design());
        move || {
            div()
                .named("swatch")
                .width(move || d.size().default)
                .height(24.0)
                .border(1.0)
                .background(move || t.ink(Tone::role(ColorRole::Card)))
                .border_color(move || t.ink(Tone::role(ColorRole::Border)))
                .radius(move || t.corner(d.corners().popup))
                .shadow(move || t.shadow(d.elevation().popup.shadow))
                .child(text("swatch").text_color(move || t.ink(Tone::role(ColorRole::CardForeground))))
                .into_element()
        }
    })
}

type Installed = Rc<Cell<Option<(ThemeHandle, DesignHandle)>>>;

/// An app installing `Theme::default()` and vega in its root's setup, around what `body` builds.
fn app<B: IntoElement>(body: impl Fn() -> B + Clone + 'static) -> (impl Fn() -> gui::Element, Installed) {
    let installed: Installed = Rc::new(Cell::new(None));
    let keep = Rc::clone(&installed);
    let render = move || {
        let (keep, body) = (Rc::clone(&keep), body.clone());
        div()
            .child(component((), move |_| {
                keep.set(Some(install(Theme::default(), VEGA)));
                move || body().into_element()
            }))
            .into_element()
    };
    (render, installed)
}

fn background(s: &Scenario, name: &str) -> Option<gui::Color> {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some(name))
        .and_then(|n| n.look.background)
}

/// One switch: it renders no instance, lays out no node and paints the swatches again.
fn switch(s: &mut Scenario, cause: &str, f: &dyn Fn()) {
    s.act(cause, f);
    let r = s.frame(cause);
    assert!(r.is_ok(), "{cause}: a frame");
    let Ok(r) = r else { return };
    assert!(r.rendered.is_empty(), "{cause}: no instance renders: {:?}", r.rendered);
    assert_eq!(r.layout.misses, 0, "{cause}: nothing lays out: {:?}", r.layout);
    assert_eq!(
        r.layout.placements, 0,
        "{cause}: nothing is placed again: {:?}",
        r.layout
    );
    assert!(r.paint.recorded > 0, "{cause}: the swatches paint again: {}", r.paint);
}

#[test]
fn switching_theme_or_scheme_renders_and_lays_out_nothing() {
    let (render, installed) = app(|| {
        div()
            .display(Display::Flex)
            .flex_wrap(true)
            .gap(4.0)
            .children((0..40).map(swatch))
    });
    let mut s = Scenario::measuring(render, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let (t, d) = installed.get().expect("the app installed its theme");
    s.take_reports();
    let painted = |theme: &Theme| theme.palette().painted().card;
    assert_eq!(background(&s, "swatch"), Some(painted(&Theme::default())));

    switch(&mut s, "dark", &|| t.update(|t| t.scheme = Scheme::Dark));
    assert_eq!(
        background(&s, "swatch"),
        Some(painted(&Theme::default().scheme(Scheme::Dark)))
    );

    let blue = Theme::new(BaseColor::Stone).accent(Accent::Blue).scheme(Scheme::Dark);
    let next = blue.clone();
    switch(&mut s, "stone blue", &move || t.set(next.clone()));
    assert_eq!(background(&s, "swatch"), Some(painted(&blue)));

    switch(&mut s, "light", &|| t.update(|t| t.scheme = Scheme::Light));
    switch(&mut s, "radius", &|| t.update(|t| t.radius = 14.0));
    switch(&mut s, "popup corner", &|| {
        d.update(|d| d.corners.popup = Corner::Step(RadiusStep::Xl))
    });

    // A height edit lays out, and still renders nothing.
    s.act("height", || d.update(|d| d.size.default = 40.0));
    let r = s.frame("height");
    assert!(
        r.as_ref().is_ok_and(|r| r.rendered.is_empty()),
        "a style switch renders nothing"
    );
    assert!(r.is_ok_and(|r| r.layout.misses > 0), "a height lays out");
}

#[test]
fn a_subtree_override_follows_an_app_wide_switch() {
    let inner: Installed = Rc::new(Cell::new(None));
    let keep = Rc::clone(&inner);
    let probe = move || {
        let keep = Rc::clone(&keep);
        component((), move |_| {
            let (t, d) = (use_theme(), use_design());
            keep.set(Some((t, d)));
            move || {
                div()
                    .named("inner")
                    .size(10.0)
                    .background(move || t.ink(Tone::role(ColorRole::Background)))
                    .into_element()
            }
        })
    };
    let (render, installed) = app(move || with_theme(|t| t.radius = 0.0, with_design(|d| d.focus.ring = 1.0, probe())));
    let mut s = Scenario::measuring(render, 200, 200, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let (t, d) = installed.get().expect("the app installed its theme");
    let (it, id) = inner.get().expect("the subtree read its theme");
    assert_eq!((it.radius(), t.radius()), (0.0, 10.0));
    assert_eq!((id.focus().ring, d.focus().ring), (1.0, 3.0));

    s.act("dark", || t.update(|t| t.scheme = Scheme::Dark));
    let r = s.frame("dark");
    assert!(r.is_ok_and(|r| r.rendered.is_empty()), "the override renders nothing");
    assert_eq!(it.scheme(), Scheme::Dark, "the subtree follows the app's scheme");
    assert_eq!(it.radius(), 0.0, "and keeps its own radius");
    assert_eq!(
        background(&s, "inner"),
        Some(Theme::default().colors.dark.painted().background)
    );

    d.update(|d| d.size.default = 44.0);
    assert_eq!(id.size().default, 44.0, "the subtree follows a style edit");
    assert_eq!(id.focus().ring, 1.0, "and keeps its own ring");

    // Writing through the subtree's handle writes the app's theme; the override applies again on top.
    it.update(|t| t.radius = 7.0);
    assert_eq!((t.radius(), it.radius()), (7.0, 0.0));
}

#[test]
fn an_inverted_menu_stays_dark_in_both_schemes() {
    let inner: Rc<Cell<Option<ThemeHandle>>> = Rc::new(Cell::new(None));
    let keep = Rc::clone(&inner);
    let (render, installed) = app(move || {
        let keep = Rc::clone(&keep);
        with_theme(
            |t| t.scheme = Scheme::Dark,
            component((), move |_| {
                keep.set(Some(use_theme()));
                || div().size(4.0).into_element()
            }),
        )
    });
    let mut s = Scenario::measuring(render, 100, 100, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let (t, _) = installed.get().expect("the app installed its theme");
    let menu = inner.get().expect("the menu read its theme");
    assert_eq!(menu.scheme(), Scheme::Dark);
    assert!(menu.colors().dark);
    t.update(|t| t.scheme = Scheme::Dark);
    assert_eq!(menu.scheme(), Scheme::Dark);
    t.update(|t| t.scheme = Scheme::Light);
    assert_eq!((t.scheme(), menu.scheme()), (Scheme::Light, Scheme::Dark));
}

#[test]
fn a_subtree_override_follows_a_new_edit_from_its_parent() {
    let inner: Installed = Rc::new(Cell::new(None));
    let flag: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
    let (keep, keep_flag) = (Rc::clone(&inner), Rc::clone(&flag));
    let (render, _installed) = app(move || {
        let (keep, keep_flag) = (Rc::clone(&keep), Rc::clone(&keep_flag));
        // The parent reads its own state in render and passes new edits each time.
        component((), move |_| {
            let dark = Signal::new(false);
            keep_flag.set(Some(dark));
            let keep = Rc::clone(&keep);
            move || {
                let on = dark.get();
                let keep = Rc::clone(&keep);
                with_theme(
                    move |t| t.scheme = if on { Scheme::Dark } else { Scheme::Light },
                    with_design(
                        move |d| d.focus.ring = if on { 2.0 } else { 1.0 },
                        component((), move |_| {
                            keep.set(Some((use_theme(), use_design())));
                            || div().size(4.0).into_element()
                        }),
                    ),
                )
                .into_element()
            }
        })
    });
    let mut s = Scenario::measuring(render, 100, 100, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let ((t, d), dark) = (
        inner.get().expect("the subtree read its theme"),
        flag.get().expect("the parent ran"),
    );
    assert_eq!((t.scheme(), d.focus().ring), (Scheme::Light, 1.0));

    s.act("dark", || dark.set(true));
    s.frame("dark").expect("a frame");
    assert_eq!(
        (t.scheme(), d.focus().ring),
        (Scheme::Dark, 2.0),
        "the subtree takes the parent's new edits"
    );

    s.act("light", || dark.set(false));
    s.frame("light").expect("a frame");
    assert_eq!((t.scheme(), d.focus().ring), (Scheme::Light, 1.0));
}
