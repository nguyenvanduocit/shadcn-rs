//! ADR-0022's theme and style as values: every shadcn theme ported (7 base colours, 17 accents, radius, fonts,
//! menu), resolved as upstream composes them, and `Design`'s roles resolved against a theme.

use gui::Color;
use gui::color::Oklch;
use shadcn::design::{Case, Corner, FontRole, Leading, Size, TextRole, TextSize, Tracking, VEGA};
use shadcn::theme::{
    Accent, BaseColor, ColorRole, Font, Heading, Ink, Menu, MenuAccent, Palette, Radius, RadiusStep, Scheme,
    ShadowStep, StateTones, TextStep, Theme, Tone, TypeScale,
};

fn ok(l: f32, c: f32, h: f32) -> Oklch {
    Oklch::new(l, c, h)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn neutral_is_upstreams_and_paints_as_before() {
    let t = Theme::default();
    assert_eq!(t.scheme, Scheme::Light);
    let (light, dark) = (t.colors.light, t.colors.dark);
    assert_eq!(light.background, ok(1.0, 0.0, 0.0));
    assert_eq!(light.foreground, ok(0.145, 0.0, 0.0));
    assert_eq!(light.destructive, ok(0.577, 0.245, 27.325));
    assert_eq!(dark.card, ok(0.205, 0.0, 0.0));
    assert_eq!(dark.border, ok(1.0, 0.0, 0.0).with_alpha(0.1));
    assert_eq!(dark.input, ok(1.0, 0.0, 0.0).with_alpha(0.15));
    assert_eq!(dark.sidebar_primary, ok(0.488, 0.243, 264.376));
    assert_eq!(light.chart, [0.87, 0.556, 0.439, 0.371, 0.269].map(|l| ok(l, 0.0, 0.0)));
    // The site's code tokens: `--surface` and its foreground.
    assert_eq!(light.code, ok(0.98, 0.0, 0.0));
    assert_eq!(light.code_foreground, light.foreground);
    assert_eq!(dark.code_foreground, ok(0.708, 0.0, 0.0));
    // Painted, they are the colours the components drew before the port.
    let painted = dark.painted();
    assert_eq!(painted.border, Color::oklch(1.0, 0.0, 0.0).opacity(0.1));
    assert_eq!(painted.muted_foreground, Color::oklch(0.708, 0.0, 0.0));
}

#[test]
fn every_base_colour_and_accent_is_ported_and_distinct() {
    let bases: Vec<_> = BaseColor::ALL.iter().map(|b| b.palettes()).collect();
    assert_eq!(bases.len(), 7);
    for (i, a) in bases.iter().enumerate() {
        for b in bases.iter().skip(i.saturating_add(1)) {
            assert_ne!(a, b, "two base colours are the same");
        }
        // Every base shares upstream's destructive and dark border.
        assert_eq!(a.light.destructive, ok(0.577, 0.245, 27.325));
        assert_eq!(a.dark.border, ok(1.0, 0.0, 0.0).with_alpha(0.1));
    }
    let stone = Theme::new(BaseColor::Stone);
    let mut primaries = Vec::new();
    for accent in Accent::ALL {
        let t = stone.clone().accent(accent);
        // An accent writes primary, secondary, charts and sidebar-primary; the rest is the base's.
        assert_eq!(t.colors.light.muted, stone.colors.light.muted, "{}", accent.name());
        assert_eq!(t.colors.dark.border, stone.colors.dark.border, "{}", accent.name());
        // Every accent's secondary is zinc's, whatever the base, as upstream has it.
        assert_eq!(t.colors.light.secondary, ok(0.967, 0.001, 286.375), "{}", accent.name());
        assert_eq!(
            t.colors.dark.secondary_foreground,
            ok(0.985, 0.0, 0.0),
            "{}",
            accent.name()
        );
        // Charts are the same in light and dark.
        assert_eq!(t.colors.light.chart, t.colors.dark.chart, "{}", accent.name());
        assert_ne!(t.colors.light.primary, stone.colors.light.primary, "{}", accent.name());
        primaries.push(t.colors.light.primary);
    }
    assert_eq!(primaries.len(), 17);
    for (i, a) in primaries.iter().enumerate() {
        assert!(
            !primaries.iter().skip(i.saturating_add(1)).any(|b| b == a),
            "two accents share a primary"
        );
    }
}

#[test]
fn accents_keep_upstreams_values_and_oddities() {
    let blue = Theme::new(BaseColor::Neutral).accent(Accent::Blue);
    assert_eq!(blue.colors.light.primary, ok(0.488, 0.243, 264.376));
    assert_eq!(blue.colors.dark.primary, ok(0.424, 0.199, 265.638));
    assert_eq!(blue.colors.light.sidebar_primary, ok(0.546, 0.245, 262.881));
    // Lime's light primary is not one of its chart steps.
    let lime = Theme::new(BaseColor::Neutral).accent(Accent::Lime);
    assert_eq!(lime.colors.light.primary, ok(0.841, 0.238, 128.85));
    assert!(!lime.colors.light.chart.contains(&lime.colors.light.primary));
    // Rose's dark scheme also writes `sidebar`, with zinc's (themes.ts:946); no other accent does.
    let mauve = Theme::new(BaseColor::Mauve);
    let rose = mauve.clone().accent(Accent::Rose);
    assert_eq!(rose.colors.dark.sidebar, ok(0.21, 0.006, 285.885));
    assert_eq!(rose.colors.light.sidebar, mauve.colors.light.sidebar);
    let teal = mauve.clone().accent(Accent::Teal);
    assert_eq!(teal.colors.dark.sidebar, mauve.colors.dark.sidebar);
}

#[test]
fn bold_menu_accent_takes_primary_as_shadcn_vue_does() {
    let t = Theme::new(BaseColor::Zinc).accent(Accent::Violet).menu(Menu {
        accent: MenuAccent::Bold,
        inverted: false,
        translucent: false,
    });
    for scheme in [Scheme::Light, Scheme::Dark] {
        let written = *t.colors.get(scheme);
        let read = t.clone().scheme(scheme).palette();
        assert_eq!(read.accent, written.primary);
        assert_eq!(read.accent_foreground, written.primary_foreground);
        // shadcn-vue sets the sidebar accent too (shadcn-ui does not); the verdict compares against vue.
        assert_eq!(read.sidebar_accent, written.primary);
        assert_eq!(read.sidebar_accent_foreground, written.primary_foreground);
        assert_eq!(read.muted, written.muted);
    }
    // Subtle reads the palette as written.
    let subtle = Theme::new(BaseColor::Zinc).accent(Accent::Violet);
    assert_eq!(subtle.palette(), subtle.colors.light);
}

#[test]
fn radius_options_and_steps() {
    assert_eq!(Theme::default().radius, 10.0);
    assert_eq!(
        [
            Radius::Default,
            Radius::None,
            Radius::Small,
            Radius::Medium,
            Radius::Large
        ]
        .map(Radius::px),
        [10.0, 0.0, 7.2, 10.0, 14.0]
    );
    let steps = [
        RadiusStep::Sm,
        RadiusStep::Md,
        RadiusStep::Lg,
        RadiusStep::Xl,
        RadiusStep::Xl2,
        RadiusStep::Xl3,
        RadiusStep::Xl4,
    ];
    let px: Vec<f32> = steps.iter().map(|s| s.of(10.0)).collect();
    for (got, want) in px.iter().zip([6.0, 8.0, 10.0, 14.0, 18.0, 22.0, 26.0]) {
        assert!(close(*got, want), "{got} != {want}");
    }
}

#[test]
fn fonts_follow_upstreams_options() {
    assert_eq!(Font::ALL.len(), 24);
    let t = Theme::default();
    assert_eq!(t.fonts.sans, gui::style::FontFamily::INTER);
    assert_eq!(t.fonts.mono, gui::style::FontFamily::GEIST_MONO);
    assert_eq!(t.fonts.heading(), t.fonts.sans);
    let sera = Theme::new(BaseColor::Taupe)
        .font(Font::NotoSans)
        .heading(Font::PlayfairDisplay);
    assert_eq!(sera.fonts.sans.names, &["Noto Sans"]);
    assert_eq!(sera.fonts.heading(), Font::PlayfairDisplay.family());
    // A heading equal to the body font is `inherit`.
    let same = Theme::default().font(Font::Figtree).heading(Font::Figtree);
    assert_eq!(same.fonts.heading, Heading::AsSans);
}

#[test]
fn type_scale_and_shadows_are_tailwinds() {
    let s = TypeScale::TAILWIND;
    assert_eq!((s.sm.size, s.sm.line_height), (14.0, 20.0));
    assert_eq!((s.xs.size, s.xs.line_height), (12.0, 16.0));
    assert_eq!((s.xl9.size, s.xl9.line_height), (128.0, 128.0));
    let t = Theme::default();
    assert_eq!(t.shadows.get(ShadowStep::None).len(), 0);
    assert_eq!(t.shadows.get(ShadowStep::Md).len(), 2);
    let xs = t.shadows.get(ShadowStep::Xs);
    let layer = xs.first().copied();
    assert_eq!(layer.map(|l| (l.y, l.blur, l.color.a)), Some((1.0, 2.0, 0.05)));
}

#[test]
fn inks_resolve_against_the_palette_in_oklch() {
    let p: Palette = Theme::default().colors.dark;
    let content = ok(0.5, 0.1, 30.0);
    assert_eq!(Ink::Role(ColorRole::Primary).resolve(&p, content), p.primary);
    // `border/50` on a token that is already translucent multiplies its alpha.
    let half = Ink::Alpha(ColorRole::Border, 0.5).resolve(&p, content);
    assert!(close(half.alpha(), 0.05));
    let mixed = Ink::Mix(ColorRole::Secondary, ColorRole::Foreground, 0.05).resolve(&p, content);
    assert_eq!(mixed, p.secondary.mix(p.foreground, 0.05));
    assert_eq!(Ink::Content.resolve(&p, content), content);
    assert_eq!(Ink::Clear.resolve(&p, content).alpha(), 0.0);
    let tone = Tone {
        light: Ink::Clear,
        dark: Ink::Alpha(ColorRole::Input, 0.3),
    };
    assert_eq!(tone.get(Scheme::Light), Ink::Clear);
    assert_eq!(tone.get(Scheme::Dark), Ink::Alpha(ColorRole::Input, 0.3));
    let states = StateTones::hover(Tone::role(ColorRole::Secondary), Tone::role(ColorRole::Muted));
    assert_eq!(states.get(false, true), Tone::role(ColorRole::Secondary));
    assert_eq!(states.get(true, true), Tone::role(ColorRole::Muted));
}

#[test]
fn vega_is_a_complete_style() {
    assert_eq!(VEGA.name, "vega");
    assert_eq!(VEGA.size.get(Size::Default), 36.0);
    assert_eq!(VEGA.spacing.control_x.get(Size::Xs), 8.0);
    assert_eq!(VEGA.corners.control.get(Size::Xs), Corner::Capped(RadiusStep::Md, 8.0));
    assert_eq!(VEGA.focus.ring, 3.0);
    // An app's style starts from vega and states what it changes.
    let brand = shadcn::Design {
        name: "brand",
        corners: shadcn::design::Corners {
            control: shadcn::design::BySize::all(Corner::Full),
            ..VEGA.corners
        },
        ..VEGA
    };
    assert_ne!(brand, VEGA);
    assert_eq!(brand.field, VEGA.field);
    let role = TextRole {
        size: TextSize::Px(10.0),
        leading: Leading::Relaxed,
        weight: 600,
        case: Case::Upper,
        tracking: Tracking::Widest,
        font: FontRole::Mono,
    };
    assert_ne!(role, TextRole::step(TextStep::Xs, 600));
}
