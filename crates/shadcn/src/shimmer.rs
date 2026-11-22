//! Shimmer (shadcn's `shimmer` utility, `tailwind.css`): text drawn through a
//! background gradient whose highlight sweeps across it, for "working on it"
//! labels. `text("Generating response…").shimmer(Shimmer::default())`.

use gui::motion::{Keyframe, Pose};
use gui::prelude::*;
use gui::style::{BackgroundClip, BackgroundImage, Calc, LinearGradient, Stop, StopColor};
use gui::{Animation, Color, Easing};

use crate::theme::theme;

/// The utility and its modifiers (`shimmer-angle-*`, `shimmer-color-*`,
/// `shimmer-spread-*`, `shimmer-duration-*`, `shimmer-once`, `shimmer-reverse`).
/// `shimmer-none` is not applying it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shimmer {
    /// Tilt of the highlight from vertical, in degrees (`--shimmer-angle`).
    angle: f32,
    /// The highlight; `None` derives it from the text colour.
    color: Option<Color>,
    /// Half the highlight's width, in pixels; `None` is `3ch + 40px`.
    spread: Option<f32>,
    /// One sweep, in seconds.
    duration: f32,
    /// Sweep once instead of forever.
    once: bool,
    reverse: bool,
}

impl Default for Shimmer {
    fn default() -> Self {
        Self {
            angle: 20.0,
            color: None,
            spread: None,
            duration: 2.0,
            once: false,
            reverse: false,
        }
    }
}

impl Shimmer {
    /// `shimmer-angle-n`.
    #[must_use]
    pub fn angle(self, degrees: f32) -> Self {
        Self { angle: degrees, ..self }
    }

    /// `shimmer-color-*`: the highlight's colour (`/60` is `.opacity(0.6)`).
    #[must_use]
    pub fn color(self, color: Color) -> Self {
        Self {
            color: Some(color),
            ..self
        }
    }

    /// `shimmer-spread-*`: the highlight's width, in pixels.
    #[must_use]
    pub fn spread(self, px: f32) -> Self {
        Self {
            spread: Some(px),
            ..self
        }
    }

    /// `shimmer-duration-n`, `n` milliseconds.
    #[must_use]
    pub fn duration_ms(self, ms: f32) -> Self {
        Self {
            duration: ms / 1000.0,
            ..self
        }
    }

    /// `shimmer-once`: one sweep from when the node appears; replay it by
    /// giving the node a new key, as Vue's demo bumps its `:key`.
    #[must_use]
    pub fn once(self) -> Self {
        Self { once: true, ..self }
    }

    /// `shimmer-reverse`, and what `dir="rtl"` does: sweep right to left.
    #[must_use]
    pub fn reverse(self) -> Self {
        Self { reverse: true, ..self }
    }

    /// `tw-shimmer`: the background's x position from 100% to 0% of the
    /// room it moves in, linear, forever unless `once`, backwards when `reverse`.
    fn sweep(&self) -> Animation {
        let at = |at, x| Keyframe {
            at,
            pose: Pose {
                background_x: Some(x),
                ..Pose::default()
            },
        };
        let sweep = Animation::new(vec![at(0.0, 1.0), at(1.0, 0.0)], self.duration, Easing::Linear);
        let sweep = if self.once { sweep } else { sweep.infinite() };
        if self.reverse { sweep.reverse() } else { sweep }
    }

    /// The background image this shimmer paints with, at rest (`background-position: 0 0`).
    fn image(&self) -> BackgroundImage {
        let spread = self.spread.map_or(
            Calc {
                fraction: 0.0,
                px: 40.0,
                ch: 3.0,
            },
            Calc::px,
        );
        let from_middle = |sign: f32| Calc {
            fraction: 0.5,
            px: sign * spread.px,
            ch: sign * spread.ch,
        };
        let highlight = self.color.map_or_else(
            || {
                if theme().dark {
                    // `oklch(from currentColor max(0.8, l + 0.4) c h / alpha + 0.4)`
                    StopColor::Current {
                        lightness_floor: 0.8,
                        lightness_lift: 0.4,
                        alpha_scale: 1.0,
                        alpha_add: 0.4,
                    }
                } else {
                    // `oklch(from currentColor l c h / alpha * 0.2)`
                    StopColor::Current {
                        lightness_floor: 0.0,
                        lightness_lift: 0.0,
                        alpha_scale: 0.2,
                        alpha_add: 0.0,
                    }
                }
            },
            StopColor::Color,
        );
        let stop = |ink, at| Stop { color: ink, at };
        BackgroundImage {
            // The utility's two half-way stops are the OKLCH mix of their
            // neighbours, which the OKLab interpolation between those yields.
            gradient: LinearGradient {
                angle: 90.0 + self.angle,
                stops: [
                    stop(StopColor::CURRENT, from_middle(-1.0)),
                    stop(highlight, Calc::fraction(0.5)),
                    stop(StopColor::CURRENT, from_middle(1.0)),
                ]
                .into(),
            },
            size: [
                Calc {
                    fraction: 2.0,
                    px: 2.0 * spread.px,
                    ch: 2.0 * spread.ch,
                },
                Calc::fraction(1.0),
            ],
            position: [Calc::default(), Calc::default()],
            clip: BackgroundClip::Text,
        }
    }
}

/// `.shimmer(…)` on any element: put it on the text node it sweeps.
pub trait ShimmerExt: Styled {
    #[must_use]
    fn shimmer(self, shimmer: Shimmer) -> Self {
        self.background_image(shimmer.image()).animation(shimmer.sweep())
    }
}

impl<T: Styled> ShimmerExt for T {}
