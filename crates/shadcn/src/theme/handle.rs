//! The theme in scope, as reactive groups (ADR-0022, research §7.12): one node per field of [`Theme`], each a
//! computed of the whole value with equality cutoff, so writing a whole theme wakes only the bindings that read a
//! group it changed (Flutter's `InheritedModel` aspects). A subtree's override is a computed of the enclosing theme,
//! so an app-wide change still reaches it.

use std::cell::OnceCell;
use std::ops::Deref;
use std::rc::Rc;

use gui::color::Oklch;
use gui::style::{FontFamily, Shadows};
use gui::{
    Color, ComponentElement, Computed, Element, IntoElement, Signal, Slot, component, detached, provide, use_context,
};

use super::{
    ColorRole, Fonts, Menu, MenuAccent, Palette, Palettes, Scheme, ShadowScale, ShadowStep, Theme, Tone, TypeScale,
};
use crate::design::{Case, Corner, Design, DesignHandle, FontRole, Leading, TextRole, TextSize, Tracking};
use crate::icons::IconSet;

/// The radius `rounded-full` draws: CSS's `calc(infinity * 1px)`, which `gui` shrinks to a stadium.
const FULL: f32 = 9999.0;

/// A whole value in scope: the app's own, written by hand, or a subtree's, computed from the one around it.
pub(crate) enum Whole<T: 'static> {
    Root(Signal<T>),
    Derived(Computed<T>),
}

impl<T: 'static> Clone for Whole<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Whole<T> {}

impl<T: 'static> Whole<T> {
    /// Borrow the value, subscribing the running reader.
    pub(crate) fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        match self {
            Self::Root(s) => s.with(f),
            Self::Derived(c) => c.with(f),
        }
    }
}

/// A subtree's edit of the value around it. It holds a closure, which cannot be compared, so it compares equal; the
/// props it sits in never do (their child is a slot), so every parent render stores the new edit. The computed it
/// feeds reads the props tracked, so it reruns with the new edit on every parent render, and when a signal the edit
/// read changes; the groups' equality cutoff keeps that from reaching any binding the result did not change.
pub(crate) struct Edit<T: 'static>(pub(crate) Rc<dyn Fn(&mut T)>);

impl<T: 'static> Clone for Edit<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: 'static> PartialEq for Edit<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// What [`with_theme`] and [`crate::design::with_design`] are given. Its child is the parent's slot, so it never
/// compares equal.
#[derive(Clone, PartialEq)]
pub(crate) struct Override<T: 'static> {
    pub(crate) edit: Edit<T>,
    pub(crate) child: Slot<Element>,
}

/// The groups of a theme in scope.
#[derive(Clone, Copy)]
struct Groups {
    root: Signal<Theme>,
    whole: Whole<Theme>,
    scheme: Computed<Scheme>,
    palettes: Computed<Palettes>,
    /// `palettes` with the menu accent applied: what inks resolve against.
    read: Computed<Palettes>,
    radius: Computed<f32>,
    fonts: Computed<Fonts>,
    type_scale: Computed<TypeScale>,
    shadows: Computed<ShadowScale>,
    menu: Computed<Menu>,
    icons: Computed<IconSet>,
}

impl Groups {
    fn new(root: Signal<Theme>, whole: Whole<Theme>) -> Self {
        let palettes = Computed::new(move || whole.with(|t| t.colors));
        let menu = Computed::new(move || whole.with(|t| t.menu));
        let accent = Computed::new(move || menu.with(|m| m.accent));
        Self {
            root,
            whole,
            scheme: Computed::new(move || whole.with(|t| t.scheme)),
            palettes,
            read: Computed::new(move || match accent.get() {
                MenuAccent::Subtle => palettes.get(),
                MenuAccent::Bold => palettes.with(|p| p.bold()),
            }),
            radius: Computed::new(move || whole.with(|t| t.radius)),
            fonts: Computed::new(move || whole.with(|t| t.fonts)),
            type_scale: Computed::new(move || whole.with(|t| t.type_scale)),
            shadows: Computed::new(move || whole.with(|t| t.shadows.clone())),
            menu,
            icons: Computed::new(move || whole.with(|t| t.icons)),
        }
    }
}

/// The theme in scope: a `Copy` handle a binding captures. Each read subscribes the running reader to the one group
/// it needs: [`Self::ink`] to the scheme and colours, [`Self::corner`] to the radius, [`Self::text`] to the type
/// scale and fonts, [`Self::shadow`] to the shadows.
#[derive(Clone, Copy)]
pub struct ThemeHandle(Signal<Groups>);

impl std::fmt::Debug for ThemeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThemeHandle")
    }
}

impl ThemeHandle {
    fn root(theme: Theme) -> Self {
        let root = Signal::labeled("theme", theme);
        Self(Signal::new(Groups::new(root, Whole::Root(root))))
    }

    fn derived(root: Signal<Theme>, whole: Computed<Theme>) -> Self {
        Self(Signal::new(Groups::new(root, Whole::Derived(whole))))
    }

    fn groups(self) -> Groups {
        self.0.with_untracked(|g| *g)
    }

    /// The whole theme in scope (subscribing to all of it).
    #[must_use]
    pub fn get(self) -> Theme {
        self.groups().whole.with(Theme::clone)
    }

    /// Write the app's theme. Each group stops at equality cutoff, so only what changed reruns; a subtree's override
    /// applies again on top.
    pub fn set(self, theme: Theme) {
        self.groups().root.set(theme);
    }

    /// Change the app's theme in place: `theme.update(|t| t.scheme = t.scheme.toggled())`.
    pub fn update(self, f: impl FnOnce(&mut Theme)) {
        let root = self.groups().root;
        let mut theme = root.get_untracked();
        f(&mut theme);
        root.set(theme);
    }

    #[must_use]
    pub fn scheme(self) -> Scheme {
        self.groups().scheme.get()
    }

    /// The palettes as the theme writes them.
    #[must_use]
    pub fn palettes(self) -> Palettes {
        self.groups().palettes.get()
    }

    /// The palette of the scheme in scope, as read (the menu accent applied).
    #[must_use]
    pub fn palette(self) -> Palette {
        let g = self.groups();
        let scheme = g.scheme.get();
        g.read.with(|p| *p.get(scheme))
    }

    /// `--radius`, in logical pixels.
    #[must_use]
    pub fn radius(self) -> f32 {
        self.groups().radius.get()
    }

    #[must_use]
    pub fn fonts(self) -> Fonts {
        self.groups().fonts.get()
    }

    #[must_use]
    pub fn type_scale(self) -> TypeScale {
        self.groups().type_scale.get()
    }

    #[must_use]
    pub fn shadows(self) -> ShadowScale {
        self.groups().shadows.get()
    }

    #[must_use]
    pub fn menu(self) -> Menu {
        self.groups().menu.get()
    }

    #[must_use]
    pub fn icons(self) -> IconSet {
        self.groups().icons.get()
    }

    /// `tone` in the scheme in scope, kept in OKLCH; [`super::Ink::Content`] is `content`.
    #[must_use]
    pub fn oklch_on(self, tone: Tone, content: Oklch) -> Oklch {
        let g = self.groups();
        let scheme = g.scheme.get();
        g.read.with(|p| tone.get(scheme).resolve(p.get(scheme), content))
    }

    /// `tone` in the scheme in scope, kept in OKLCH, with the palette's `foreground` as the content colour.
    #[must_use]
    pub fn oklch(self, tone: Tone) -> Oklch {
        let g = self.groups();
        let scheme = g.scheme.get();
        g.read.with(|p| {
            let palette = p.get(scheme);
            tone.get(scheme).resolve(palette, palette.foreground)
        })
    }

    /// `tone` painted, in the scheme in scope; [`super::Ink::Content`] is the palette's `foreground`.
    #[must_use]
    pub fn ink(self, tone: Tone) -> Color {
        self.oklch(tone).into()
    }

    /// `tone` painted, with `content` as the enclosing content colour.
    #[must_use]
    pub fn ink_on(self, tone: Tone, content: Oklch) -> Color {
        self.oklch_on(tone, content).into()
    }

    /// The token `role`, painted, in the scheme in scope.
    #[must_use]
    pub fn color(self, role: ColorRole) -> Color {
        self.ink(Tone::role(role))
    }

    /// The radius `corner` draws, in logical pixels. Only a corner that follows the theme's radius reads it.
    #[must_use]
    pub fn corner(self, corner: Corner) -> f32 {
        match corner {
            Corner::None => 0.0,
            Corner::Px(px) => px,
            Corner::Full => FULL,
            Corner::Step(s) => s.of(self.radius()),
            Corner::Plus(s, px) => (s.of(self.radius()) + px).max(0.0),
            Corner::Capped(s, px) => s.of(self.radius()).min(px),
        }
    }

    /// The family of `role`.
    #[must_use]
    pub fn font(self, role: FontRole) -> FontFamily {
        let fonts = self.fonts();
        match role {
            FontRole::Sans => fonts.sans,
            FontRole::Heading => fonts.heading(),
            FontRole::Mono => fonts.mono,
        }
    }

    /// `role`'s text, resolved against the type scale and fonts.
    #[must_use]
    pub fn text(self, role: TextRole) -> TextStyle {
        let scale = self.type_scale();
        let (size, step_line) = match role.size {
            TextSize::Step(s) => {
                let step = scale.get(s);
                (step.size, step.line_height)
            }
            TextSize::Px(px) => (px, px * scale.leading.normal),
        };
        let line_height = match role.leading {
            Leading::Step => step_line,
            Leading::Relaxed => size * scale.leading.relaxed,
            Leading::None => size,
        };
        let t = scale.tracking;
        TextStyle {
            size,
            line_height,
            weight: role.weight,
            tracking: match role.tracking {
                Tracking::Tighter => t.tighter,
                Tracking::Tight => t.tight,
                Tracking::Normal => t.normal,
                Tracking::Wide => t.wide,
                Tracking::Wider => t.wider,
                Tracking::Widest => t.widest,
            },
            family: self.font(role.font),
            uppercase: role.case == Case::Upper,
        }
    }

    /// The layers of the shadow step `step`; none for [`ShadowStep::None`], without reading the scale.
    #[must_use]
    pub fn shadow(self, step: ShadowStep) -> Shadows {
        if step == ShadowStep::None {
            return Shadows::new();
        }
        self.groups().shadows.with(|s| s.get(step))
    }

    /// The painted palette of the scheme in scope and whether it is dark: what render reads today ([`super::theme`]).
    #[must_use]
    pub fn colors(self) -> Colors {
        let scheme = self.scheme();
        Colors {
            dark: scheme.is_dark(),
            palette: self.palette().painted(),
        }
    }
}

/// Text of a role, resolved: what a text node's setters take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub line_height: f32,
    pub weight: u16,
    /// Letter spacing, in em.
    pub tracking: f32,
    pub family: FontFamily,
    pub uppercase: bool,
}

/// The painted palette of a scheme and whether it is dark, as a render reads it ([`super::theme`]): token by name
/// (`t.border`, through `Deref` to its [`Palette`]) and `t.dark`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub dark: bool,
    pub palette: Palette<Color>,
}

impl Deref for Colors {
    type Target = Palette<Color>;
    fn deref(&self) -> &Palette<Color> {
        &self.palette
    }
}

thread_local! {
    /// The theme read where none is installed: neutral, light. Made once per thread, owned by nothing.
    static DEFAULT: OnceCell<ThemeHandle> = const { OnceCell::new() };
}

/// The theme in scope: the nearest [`with_theme`] above, else the one [`install`]ed, else neutral and light.
#[must_use]
pub fn use_theme() -> ThemeHandle {
    use_context::<ThemeHandle>()
        .unwrap_or_else(|| DEFAULT.with(|d| *d.get_or_init(|| detached(|| ThemeHandle::root(Theme::default())))))
}

/// Make `theme` and `design` the app's (or, in an instance, its subtree's): each a writable value split into
/// groups. The handles it returns write them: `theme.update(|t| t.scheme = Scheme::Dark)`.
pub fn install(theme: Theme, design: Design) -> (ThemeHandle, DesignHandle) {
    let t = ThemeHandle::root(theme);
    let d = DesignHandle::root(design);
    provide(t);
    provide(d);
    (t, d)
}

/// `child` under the theme around it edited by `edit` (`with_theme(|t| t.scheme = Scheme::Dark, menu)`): a
/// computed of the enclosing theme, so a change to the app's theme reaches the subtree with the edit applied again.
/// `edit` may read signals; the subtree's theme follows them.
#[must_use]
pub fn with_theme(edit: impl Fn(&mut Theme) + 'static, child: impl IntoElement) -> ComponentElement {
    component(
        Override {
            edit: Edit(Rc::new(edit)),
            child: Slot(child.into_element()),
        },
        |cx| {
            let props = cx.props();
            let outer = use_theme();
            let whole = Computed::new(move || {
                let mut t = outer.get();
                let edit = props.with(|p| p.edit.clone());
                (edit.0)(&mut t);
                t
            });
            provide(ThemeHandle::derived(outer.groups().root, whole));
            move || props.with(|p| p.child.0.clone())
        },
    )
}
