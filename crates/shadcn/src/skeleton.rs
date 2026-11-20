//! Skeleton (`ui/skeleton`): a pulsing placeholder box
//! (`animate-pulse rounded-md bg-primary/10`).

use gui::{Animation, Div, Styled, div};

use crate::theme::theme;

/// A placeholder; give it the size of what it stands for, `skeleton().h(4.0).w_px(250.0)`.
#[must_use]
pub fn skeleton() -> Div {
    let t = theme();
    div()
        .named("skeleton")
        .rounded_md()
        .bg(t.primary.opacity(0.1))
        .animate(Animation::pulse())
}
