//! Skeleton (`ui/skeleton`): a pulsing placeholder box
//! (`animate-pulse rounded-md bg-primary/10`).

use gui::prelude::*;
use gui::{Animation, Div, div};

use crate::theme::theme;

/// A placeholder; give it the size of what it stands for, `skeleton().height(16.0).width(250.0)`.
#[must_use]
pub fn skeleton() -> Div {
    let t = theme();
    div()
        .named("skeleton")
        .radius(8.0)
        .background(t.primary.opacity(0.1))
        .animation(Animation::pulse())
}
