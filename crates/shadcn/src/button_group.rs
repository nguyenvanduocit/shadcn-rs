//! Button group (`ui/button-group`): buttons (or an input) joined into one
//! control, with a separator and a text cell to put between them.

use gui::prelude::*;
use gui::{Color, Div, Element, IntoElement, Style, div};

use crate::separator::{Orientation, separator};
use crate::theme::theme;

/// `items` joined edge to edge: every item but the first loses its leading
/// border and corners, every item but the last its trailing corners
/// (`[&>*:not(:first-child)]:rounded-l-none border-l-0`,
/// `[&>*:not(:last-child)]:rounded-r-none`); an input among them fills the
/// free space (`[&>input]:flex-1`).
#[must_use]
pub fn button_group<E: IntoElement>(orientation: Orientation, items: impl IntoIterator<Item = E>) -> Div {
    let items: Vec<Element> = items.into_iter().map(IntoElement::into_element).collect();
    let last = items.len().saturating_sub(1);
    let joined = items
        .into_iter()
        .enumerate()
        .map(|(i, item)| join(item, orientation, i == 0, i == last));
    root(orientation).children(joined)
}

/// Groups side by side with a gap between them
/// (`has-[>[data-slot=button-group]]:gap-2`), as a toolbar nests them.
#[must_use]
pub fn button_group_of_groups(orientation: Orientation, groups: impl IntoIterator<Item = Div>) -> Div {
    root(orientation).gap(8.0).children(groups)
}

/// A rule between two items (`ButtonGroupSeparator`), vertical by default
/// as shadcn's is.
#[must_use]
pub fn button_group_separator(orientation: Orientation) -> Div {
    let t = theme();
    let d = separator(orientation)
        .named("button-group-separator")
        .background(t.input)
        .margin(0.0)
        .align_self(Align::Stretch);
    match orientation {
        Orientation::Vertical => d.height(Length::Auto),
        Orientation::Horizontal => d,
    }
}

/// A non-interactive cell in a group (a label, a unit); icons in it are
/// `size-4`, as `icon()` makes them.
#[must_use]
pub fn button_group_text() -> Div {
    let t = theme();
    div()
        .named("button-group-text")
        .background(t.muted)
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(8.0)
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .padding_x(16.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        })
}

/// `flex w-fit items-stretch`.
fn root(orientation: Orientation) -> Div {
    let d = div()
        .named("button-group")
        .display(Display::Flex)
        .width(Length::FitContent)
        .align_items(Align::Stretch);
    match orientation {
        Orientation::Horizontal => d,
        Orientation::Vertical => d.display(Display::Flex).flex_direction(FlexDirection::Column),
    }
}

fn join(item: Element, orientation: Orientation, first: bool, last: bool) -> Element {
    restyle(item, move |s| {
        let s = match orientation {
            Orientation::Horizontal if !first => {
                let mut s = s.radius_left(0.0);
                s.border_widths.left = 0.0;
                s
            }
            Orientation::Vertical if !first => {
                let mut s = s.radius_top(0.0);
                s.border_widths.top = 0.0;
                s
            }
            _ => s,
        };
        match orientation {
            _ if last => s,
            Orientation::Horizontal => s.radius_right(0.0),
            Orientation::Vertical => s.radius_bottom(0.0),
        }
    })
}

/// `item` with `f` applied to its style; an input also takes `flex-1`.
fn restyle(item: Element, f: impl Fn(Style) -> Style + 'static) -> Element {
    if matches!(item, Element::Input(_)) || item.name() == Some("input") {
        item.restyle(move |s| f(s).flex(Flex::grow(1.0)))
    } else {
        item.restyle(f)
    }
}
