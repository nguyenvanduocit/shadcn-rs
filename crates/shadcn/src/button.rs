//! Button (`ui/button`): a pressable control in six variants and eight sizes.

use gui::prelude::*;
use gui::{Color, Div, Element, div};

use crate::focus::{StateLook, destructive_ring, focus_ring};
use crate::theme::{Colors, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    #[default]
    Default,
    Destructive,
    Outline,
    Secondary,
    Ghost,
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ButtonSize {
    #[default]
    Default,
    Xs,
    Sm,
    Lg,
    Icon,
    IconXs,
    IconSm,
    IconLg,
}

/// A button box; add its label and icons with `.child`, its action with `.on_click`.
pub fn button(variant: ButtonVariant, size: ButtonSize) -> Button {
    let t = theme();
    let i = Interaction::new();
    let base = div()
        .named("button")
        .interaction(i)
        .transition(Properties::ALL)
        .display(Display::Flex)
        .inline(true)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .gap(8.0)
        .nowrap(true)
        .radius(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .flex_shrink(0.0)
        .border_color(t.border)
        .focusable()
        .dim_disabled(i, 0.5);
    Button {
        el: sized(styled(base, i, variant, &t), size),
        interaction: i,
        icon_padding_x: icon_padding_x(size),
        has_icon: false,
    }
}

/// A [`button`] being built: a box that knows whether an icon is among its
/// children, for the sizes whose padding then narrows (shadcn's
/// `has-[>svg]:px-*`). The icon is a direct child given as a
/// [`gui::icon`] element, as `:has(> svg)` sees it.
#[must_use]
pub struct Button {
    el: Div,
    /// What the pointer and keyboard do to it, which its bindings read.
    interaction: Interaction,
    /// The horizontal padding of the size when an icon is a child.
    icon_padding_x: Option<f32>,
    has_icon: bool,
}

impl Button {
    /// What the pointer and the keyboard do to the button: a caller's
    /// bindings read it (`.text_color(move || …)` over its hover).
    #[must_use]
    pub fn interaction(&self) -> Interaction {
        self.interaction
    }

    /// Add a child; an icon child narrows the padding of the text sizes.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        let child = child.into_element();
        self.has_icon |= matches!(child, Element::Icon(_));
        self.el = self.el.child(child);
        self
    }

    /// Add children, as [`Button::child`] each.
    pub fn children<E: IntoElement>(self, children: impl IntoIterator<Item = E>) -> Self {
        children.into_iter().fold(self, Self::child)
    }

    /// The button's box, its padding decided by its children.
    #[must_use]
    pub fn into_div(self) -> Div {
        match self.icon_padding_x {
            Some(px) if self.has_icon => self.el.padding_x(px),
            _ => self.el,
        }
    }
}

macro_rules! forward {
    ($($(#[$m:meta])* fn $name:ident$(<$($g:tt),*>)?($($arg:ident: $ty:ty),*);)*) => {
        impl Button {
            $(
                $(#[$m])*
                pub fn $name$(<$($g),*>)?(mut self, $($arg: $ty),*) -> Self {
                    self.el = self.el.$name($($arg),*);
                    self
                }
            )*
        }
    };
}

forward! {
    /// As on [`Div`].
    fn key(key: gui::ElementKey);
    /// As on [`Div`].
    fn named(name: &'static str);
    /// As on [`Div`].
    fn focusable();
    /// As on [`Div`].
    fn track_focus(handle: gui::FocusHandle);
    /// As on [`Div`].
    fn track_scroll(handle: &gui::scroll::ScrollHandle);
    /// As on [`Div`].
    fn disabled(disabled: bool);
    /// As on [`Div`].
    fn role(role: gui::Role);
    /// As on [`Div`].
    fn aria_label(label: impl Into<String>);
    /// As on [`Div`].
    fn aria_checked(checked: impl Into<gui::Toggled>);
    /// As on [`Div`].
    fn aria_selected(selected: bool);
    /// As on [`Div`].
    fn aria_expanded(expanded: bool);
    /// As on [`Div`].
    fn aria_pos_in_set(position: usize, size: usize);
    /// As on [`Div`].
    fn aria_hidden();
    /// As on [`Div`].
    fn pass_through();
    /// As on [`Div`].
    fn on_click(f: impl Fn(&gui::ClickEvent) + 'static);
    /// As on [`Div`].
    fn on_mouse_down(f: impl Fn(&gui::ClickEvent) + 'static);
    /// As on [`Div`].
    fn on_measured(f: impl Fn(&gui::Bounds) + 'static);
    /// As on [`Div`].
    fn on_drag(f: impl Fn(&gui::DragEvent) + 'static);
    /// As on [`Div`].
    fn on_key_down(f: impl Fn(&gui::KeyEvent) -> bool + 'static);
    /// As on [`Div`].
    fn on_secondary_click(f: impl Fn(&gui::ClickEvent) + 'static);
    /// As on [`Div`].
    fn on_hover(f: impl Fn(bool) + 'static);
    /// As on [`Div`].
    fn on_pointer_move(f: impl Fn(&gui::PointerEvent) + 'static);
    /// As on [`Div`].
    fn on_wheel(f: impl Fn(&gui::WheelEvent) + 'static);
    /// As on [`Div`].
    fn on_click_outside(f: impl Fn() + 'static);
}

impl Button {
    /// As on [`Div`].
    pub fn member<T: Clone + 'static>(mut self, list: &gui::Members<T>, value: T) -> Self {
        self.el = self.el.member(list, value);
        self
    }
}

impl Fluent for Button {}

impl Styled for Button {
    fn style_and_bindings(&mut self) -> (&mut gui::Style, Option<&mut gui::value::Bindings>) {
        self.el.style_and_bindings()
    }

    fn node_interaction(&self) -> Option<gui::Interaction> {
        Some(self.interaction)
    }
}

impl IntoElement for Button {
    fn into_element(self) -> Element {
        self.into_div().into_element()
    }
}

impl From<Button> for Div {
    fn from(b: Button) -> Self {
        b.into_div()
    }
}

/// The horizontal padding a size takes when an icon is a child.
fn icon_padding_x(size: ButtonSize) -> Option<f32> {
    match size {
        ButtonSize::Default => Some(12.0),
        ButtonSize::Xs => Some(6.0),
        ButtonSize::Sm => Some(10.0),
        ButtonSize::Lg => Some(16.0),
        ButtonSize::Icon | ButtonSize::IconXs | ButtonSize::IconSm | ButtonSize::IconLg => None,
    }
}

fn styled(d: Div, i: Interaction, variant: ButtonVariant, t: &Colors) -> Div {
    let t = *t;
    let on = move || i.hovered();
    let d = match variant {
        ButtonVariant::Default => d
            .background(move || if on() { t.primary.opacity(0.9) } else { t.primary })
            .text_color(t.primary_foreground),
        ButtonVariant::Destructive => d
            .background(move || match (on(), t.dark) {
                (true, _) => t.destructive.opacity(0.9),
                (false, true) => t.destructive.opacity(0.6),
                (false, false) => t.destructive,
            })
            .text_color(gui::Color::WHITE),
        ButtonVariant::Outline => d
            .border(1.0)
            .when(t.dark, |d| d.border_color(t.input))
            .background(move || match (on(), t.dark) {
                (true, true) => t.input.opacity(0.5),
                (true, false) => t.accent,
                (false, true) => t.input.opacity(0.3),
                (false, false) => t.background,
            })
            .shadow(Shadow {
                x: 0.0,
                y: 1.0,
                blur: 2.0,
                spread: 0.0,
                color: Color::BLACK.opacity(0.05),
            })
            .text_color(move || on().then_some(t.accent_foreground)),
        ButtonVariant::Secondary => d
            .background(move || if on() { t.secondary.opacity(0.8) } else { t.secondary })
            .text_color(t.secondary_foreground),
        // No base colour: a ghost button takes its parent's, as shadcn's does.
        ButtonVariant::Ghost => d
            .background(move || on().then(|| if t.dark { t.accent.opacity(0.5) } else { t.accent }))
            .text_color(move || on().then_some(t.accent_foreground)),
        ButtonVariant::Link => d.text_color(t.primary).underline(on),
    };
    let ring = if variant == ButtonVariant::Destructive {
        destructive_ring(gui::Style::default(), &t)
    } else {
        focus_ring(gui::Style::default(), &t)
    };
    d.ring_while(move || i.focus_visible(), &ring)
}

fn sized(d: Div, size: ButtonSize) -> Div {
    match size {
        ButtonSize::Default => d.height(36.0).padding_x(16.0).padding_y(8.0),
        ButtonSize::Xs => d
            .height(24.0)
            .gap(4.0)
            .padding_x(8.0)
            .text_size(12.0)
            .line_height(16.0)
            .icon_size(12.0),
        ButtonSize::Sm => d.height(32.0).gap(6.0).padding_x(12.0),
        ButtonSize::Lg => d.height(40.0).padding_x(24.0),
        ButtonSize::Icon => d.size(36.0),
        ButtonSize::IconXs => d.size(24.0).icon_size(12.0),
        ButtonSize::IconSm => d.size(32.0),
        ButtonSize::IconLg => d.size(40.0),
    }
}

#[cfg(test)]
mod tests {
    use gui::{Icon, icon};

    use super::*;

    fn padding_x(b: Button) -> (f32, f32) {
        let mut d = b.into_div();
        let p = d.style_mut().padding;
        (p.left, p.right)
    }

    #[test]
    fn an_icon_child_narrows_the_padding_of_a_text_size() {
        let label = |size| button(ButtonVariant::Default, size).child("Save");
        let with_icon = |size| label(size).child(icon(Icon::Check));
        for (size, base, narrow) in [
            (ButtonSize::Default, 16.0, 12.0),
            (ButtonSize::Xs, 8.0, 6.0),
            (ButtonSize::Sm, 12.0, 10.0),
            (ButtonSize::Lg, 24.0, 16.0),
        ] {
            assert_eq!(padding_x(label(size)), (base, base), "{size:?}");
            assert_eq!(padding_x(with_icon(size)), (narrow, narrow), "{size:?}");
        }
        // Over a padding written after the icon, as `:has(> svg)` was.
        let late = with_icon(ButtonSize::Default).padding_x(0.0);
        assert_eq!(padding_x(late), (12.0, 12.0));
        // An icon size keeps its square.
        assert_eq!(
            padding_x(button(ButtonVariant::Ghost, ButtonSize::Icon).child(icon(Icon::Check))),
            (0.0, 0.0)
        );
    }
}
