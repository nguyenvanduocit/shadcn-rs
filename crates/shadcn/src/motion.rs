//! The animations shadcn-vue's classes give its parts, written once where
//! the classes repeat (tw-animate-css under reka's `data-state` and `data-side`).

use gui::style::Origin;
use gui::{Animation, Div, Styled};

/// A popper's content opening: `data-[state=open]:animate-in fade-in-0
/// zoom-in-95` and `data-[side=bottom]:slide-in-from-top-2` with its three
/// siblings, about `origin` (`origin-(--reka-*-content-transform-origin)` is
/// [`Origin::Anchor`]; content without it scales about its centre).
pub(crate) fn popper_in(content: Div, origin: Origin) -> Div {
    content
        .origin(origin)
        .animate(Animation::enter().fade(0.0).zoom(0.95).slide_from_anchor(2.0))
}

/// [`popper_in`], and closing: `data-[state=closed]:animate-out fade-out-0 zoom-out-95`.
pub(crate) fn popper(content: Div, origin: Origin) -> Div {
    popper_in(content, origin).animate_out(Animation::exit().fade(0.0).zoom(0.95))
}

/// A modal's scrim: `data-[state=open]:animate-in fade-in-0
/// data-[state=closed]:animate-out fade-out-0`.
pub(crate) fn fade(d: Div) -> Div {
    d.animate(Animation::enter().fade(0.0))
        .animate_out(Animation::exit().fade(0.0))
}
