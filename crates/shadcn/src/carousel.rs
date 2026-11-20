//! Carousel (`ui/carousel`): slides in a row (or column) shown a view at a
//! time, moved with the previous and next buttons, the arrow keys or autoplay.
//!
//! Embla moves the track with a transform each animation frame; here the
//! track is translated by a fraction of its own size per snap, the move eased
//! over [`SLIDE_SECONDS`] as a transform transition. Embla measures each slide; `gui`
//! hands a render no sizes, so a slide's step is its declared basis
//! ([`CarouselState::basis`]), which holds while no slide outgrows it.

use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::prelude::*;
use gui::style::Calc;
use gui::{Div, Easing, Icon, NamedKey, Signal, div, icon, relative};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::separator::Orientation;

/// How long a move to the next slide takes.
pub const SLIDE_SECONDS: f32 = 0.4;

/// Which snap is shown, and the move towards it.
#[derive(Clone)]
pub struct CarouselState {
    pub orientation: Orientation,
    pub count: usize,
    /// The share of the viewport one slide takes (`basis-1/3` → 1/3).
    pub basis: f32,
    /// The snap shown, from 0 to [`Self::snaps`] - 1.
    pub index: Signal<usize>,
    /// The autoplay's next advance, while it plays.
    autoplay: Signal<Option<TimerId>>,
}

fn float(n: usize) -> f32 {
    u16::try_from(n).map_or(f32::from(u16::MAX), f32::from)
}

impl CarouselState {
    /// `count` slides along `orientation`, each a full viewport, the first shown.
    #[must_use]
    pub fn new(count: usize, orientation: Orientation) -> Self {
        Self {
            orientation,
            count,
            basis: 1.0,
            index: Signal::labeled("carousel index", 0),
            autoplay: Signal::labeled("carousel autoplay", None),
        }
    }

    /// Each slide takes `fraction` of the viewport, snapping to the start
    /// (`align: 'start'`), the last snap trimmed so no space trails the last slide.
    #[must_use]
    pub fn basis(mut self, fraction: f32) -> Self {
        self.basis = fraction;
        self
    }

    /// The furthest the track goes, in viewports.
    fn end(&self) -> f32 {
        (float(self.count) * self.basis - 1.0).max(0.0)
    }

    /// How many places the track stops at (Embla's `scrollSnapList().length`).
    #[must_use]
    pub fn snaps(&self) -> usize {
        // Every slide start short of the end, then the end itself; the
        // epsilon keeps a start that lands on the end from counting twice.
        let short = (self.end() / self.basis - 1e-3).ceil().max(0.0);
        (0..self.count)
            .take_while(|&i| float(i) < short)
            .count()
            .saturating_add(1)
            .min(self.count.max(1))
    }

    #[must_use]
    pub fn can_scroll_prev(&self) -> bool {
        self.index.get() > 0
    }

    #[must_use]
    pub fn can_scroll_next(&self) -> bool {
        self.index.get().saturating_add(1) < self.snaps()
    }

    pub fn scroll_prev(&self) {
        self.scroll_to(self.index.get().saturating_sub(1));
    }

    pub fn scroll_next(&self) {
        self.scroll_to(self.index.get().saturating_add(1));
    }

    /// Move to snap `index`, from wherever the track is now.
    pub fn scroll_to(&self, index: usize) {
        let to = index.min(self.snaps().saturating_sub(1));
        if to == self.index.get() {
            return;
        }
        self.index.set(to);
    }

    /// Advance a snap every `delay`, back to the first after the last
    /// (Embla's Autoplay plugin), until [`Self::stop`].
    pub fn play(&self, delay: Duration) {
        self.stop();
        let me = self.clone();
        let next = clock::after(delay, move || {
            if me.can_scroll_next() {
                me.scroll_next();
            } else {
                me.scroll_to(0);
            }
            me.play(delay);
        });
        self.autoplay.set(Some(next));
    }

    /// Stop the autoplay; the slide shown stays.
    pub fn stop(&self) {
        if let Some(next) = self.autoplay.get() {
            clock::cancel(next);
            self.autoplay.set(None);
        }
    }

    /// Where the track stops for the snap shown, in viewports from the start.
    fn position(&self) -> f32 {
        (float(self.index.get()) * self.basis).min(self.end())
    }
}

/// The carousel's region: focusable, arrow keys move it.
#[must_use]
pub fn carousel(state: &CarouselState) -> Div {
    let keys = state.clone();
    let (prev, next) = match state.orientation {
        Orientation::Horizontal => (NamedKey::ArrowLeft, NamedKey::ArrowRight),
        Orientation::Vertical => (NamedKey::ArrowUp, NamedKey::ArrowDown),
    };
    div().named("carousel").relative().focusable().on_key_down(move |e| {
        if e.is(prev) {
            keys.scroll_prev();
        } else if e.is(next) {
            keys.scroll_next();
        } else {
            return false;
        }
        true
    })
}

/// The viewport and its track of `items` ([`carousel_item`]s); `build`
/// styles the track, where shadcn's `CarouselContent` puts its class.
#[must_use]
pub fn carousel_content(
    state: &CarouselState,
    build: impl FnOnce(Div) -> Div,
    items: impl IntoIterator<Item = Div>,
) -> Div {
    let shift = Calc::fraction(-state.position());
    let track = div().named("carousel-track").flex();
    // The track moves by its own size per viewport: a box filling it,
    // translated by that fraction, as Embla's transform does, easing out
    // (`easeOutCubic`) from wherever it is.
    let slider = div()
        .named("carousel-slider")
        .flex()
        .relative()
        .shrink_0()
        .transition_transform()
        .duration_ms(SLIDE_SECONDS * 1000.0)
        .ease(Easing::CubicBezier(0.33, 1.0, 0.68, 1.0))
        .children(items);
    let (track, slider) = match state.orientation {
        Orientation::Horizontal => (track.ml(-4.0), slider.w_full().translate_x(shift)),
        Orientation::Vertical => (
            track.mt(-4.0).flex_col().h_full(),
            slider.flex_col().h_full().translate_y(shift),
        ),
    };
    div()
        .named("carousel-content")
        .overflow_hidden()
        .child(build(track).child(slider))
}

/// One slide, [`CarouselState::basis`] of the track.
#[must_use]
pub fn carousel_item(state: &CarouselState) -> Div {
    let d = div()
        .named("carousel-item")
        .min_w(0.0)
        .shrink_0()
        .basis_len(relative(state.basis));
    match state.orientation {
        Orientation::Horizontal => d.pl(4.0),
        Orientation::Vertical => d.pt(4.0),
    }
}

fn arrow(state: &CarouselState, enabled: bool, glyph: Icon) -> Div {
    let d = button(ButtonVariant::Outline, ButtonSize::Icon)
        .absolute()
        .size(8.0)
        .rounded_full()
        .disabled(!enabled);
    // `-translate-1/2` of its own 32px. Vertical turns the button a quarter
    // (`rotate-90`); it is a circle, so only its arrow shows the turn.
    match state.orientation {
        Orientation::Horizontal => d.top_len(relative(0.5)).mt(-4.0).child(icon(glyph)),
        Orientation::Vertical => d.left_len(relative(0.5)).ml(-4.0).child(icon(glyph).rotate(90.0)),
    }
}

/// The button before the slides; disabled on the first snap.
#[must_use]
pub fn carousel_previous(state: &CarouselState) -> Div {
    let go = state.clone();
    let d = arrow(state, state.can_scroll_prev(), Icon::ArrowLeft)
        .named("carousel-previous")
        .on_click(move |_| go.scroll_prev());
    match state.orientation {
        Orientation::Horizontal => d.left(-12.0),
        Orientation::Vertical => d.top(-12.0),
    }
}

/// The button after the slides; disabled on the last snap.
#[must_use]
pub fn carousel_next(state: &CarouselState) -> Div {
    let go = state.clone();
    let d = arrow(state, state.can_scroll_next(), Icon::ArrowRight)
        .named("carousel-next")
        .on_click(move |_| go.scroll_next());
    match state.orientation {
        Orientation::Horizontal => d.right(-12.0),
        Orientation::Vertical => d.bottom(-12.0),
    }
}
