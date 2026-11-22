//! A box's own sizes as the ported containers read them: sizing keywords
//! (Taffy's `resolve_sizing_keyword`), and `aspect-ratio` as css-sizing-4
//! §5 and Chrome have it.
//!
//! Taffy resolves a ratio on a box's size, min and max wherever it reads
//! them (`maybe_apply_aspect_ratio`), on the box's own entry and in its
//! parent's algorithm, and takes the result as definite. Two things follow
//! that Chrome does not do: a size the ratio decides cannot grow with the
//! content (the automatic minimum, css-sizing-4 §5.2: a `<p>` 100 px wide
//! with `aspect-ratio: 4` and three lines of text is 25 px tall, Chrome
//! 96), and a min or max of an axis the style sizes is carried into it from
//! the other (`width: 100px; aspect-ratio: 2; max-height: 30px` is 60 wide,
//! Chrome 100). Here:
//!
//! - A ratio sizes an axis from the other only when the axis has no size of
//!   its own (known, or its style's) and the other has one. The box does it
//!   itself ([`Ratio`], [`with_ratio`], and `leaf.rs` for a leaf), so the
//!   size it gives is at least what the content needs in that axis unless
//!   the box is a scroll container or replaced, then clamped by the axis's
//!   own min and max.
//! - A parent reads a child's min and max through the ratio only into an
//!   axis whose preferred size is `auto` ([`transfer`]): in block flow an
//!   `auto` width with `max-height: 30px` and `aspect-ratio: 2` is at most
//!   60 wide (Chrome), a `100px` one stays 100.
//! - A parent no longer gives a child a size its ratio derives as known:
//!   the child derives it, content included.
//!
//! A flex container takes the ratio's size as definite and lays its items
//! out in it; what the content then needs is what its lines (or a column's
//! items) take, a stretched item counting as its line, as Chrome does (a
//! row of stretched text keeps the ratio's height, one aligned to the start
//! grows the box).

use taffy::{
    AbsoluteAxis, AvailableSpace, CompactLength, CoreStyle, Dimension, LayoutInput, LayoutOutput, MaybeMath,
    RequestedAxis, RunMode, Size,
};

/// What a sizing keyword resolves to (Taffy's `SizingKeywordResolution`).
pub(super) enum Keyword {
    /// Measure the box under this constraint.
    Measure(AvailableSpace),
    /// This size exactly.
    Exact(f32),
}

/// Taffy's `resolve_sizing_keyword`.
pub(super) fn keyword(
    style: Dimension,
    stretch_size: Option<f32>,
    percent_basis: Option<f32>,
    calc: impl Fn(*const (), f32) -> f32,
) -> Option<Keyword> {
    match style.tag() {
        CompactLength::MIN_CONTENT_TAG => Some(Keyword::Measure(AvailableSpace::MinContent)),
        CompactLength::MAX_CONTENT_TAG => Some(Keyword::Measure(AvailableSpace::MaxContent)),
        CompactLength::FIT_CONTENT_PX_TAG => Some(Keyword::Measure(AvailableSpace::Definite(style.value()))),
        CompactLength::FIT_CONTENT_PERCENT_TAG => {
            percent_basis.map(|basis| Keyword::Measure(AvailableSpace::Definite(basis * style.value())))
        }
        CompactLength::FIT_CONTENT_KEYWORD_TAG => {
            stretch_size.map(|size| Keyword::Measure(AvailableSpace::Definite(size)))
        }
        _ if style.is_fit_content_calc() => {
            percent_basis.map(|basis| Keyword::Measure(AvailableSpace::Definite(calc(style.calc_value(), basis))))
        }
        CompactLength::STRETCH_TAG => stretch_size.map(Keyword::Exact),
        _ => None,
    }
}

/// A usable ratio (finite, positive).
pub(super) fn ratio_of(style: &impl CoreStyle) -> Option<f32> {
    style.aspect_ratio().filter(|r| r.is_finite() && *r > 0.0)
}

/// `v` (a min or a max) with each `None` axis whose preferred size is
/// `auto` filled from the other through `ratio`: Taffy's
/// `maybe_apply_aspect_ratio`, kept from an axis the style sizes.
pub(super) fn transfer(v: Size<Option<f32>>, ratio: Option<f32>, size: Size<Dimension>) -> Size<Option<f32>> {
    let Some(r) = ratio else { return v };
    match (v.width, v.height) {
        (Some(width), None) if size.height.is_auto() => Size {
            width: Some(width),
            height: Some(width / r),
        },
        (None, Some(height)) if size.width.is_auto() => Size {
            width: Some(height * r),
            height: Some(height),
        },
        _ => v,
    }
}

/// The axis a container's ratio sizes, and the size it gives there.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ratio {
    pub axis: AbsoluteAxis,
    pub size: f32,
    /// Its content can make it larger (not a scroll container).
    pub content_minimum: bool,
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// Its padding and border in that axis (the least it can be).
    pub floor: f32,
}

impl Ratio {
    /// The ratio's size in `known`, where the box's own style left the
    /// axis to it: `known` its sizes before the ratio, `definite` which of
    /// them a ratio may start from, `adjustment` its `box-sizing` one (the
    /// ratio is of the box `box-sizing` names), `min` and `max` its own.
    pub(super) fn find(
        ratio: Option<f32>,
        known: Size<Option<f32>>,
        adjustment: Size<f32>,
        min: Size<Option<f32>>,
        max: Size<Option<f32>>,
        floor: Size<f32>,
        scroll_container: bool,
    ) -> Option<Self> {
        let r = ratio?;
        let (axis, size) = match (known.width, known.height) {
            (Some(width), None) => (
                AbsoluteAxis::Vertical,
                (width - adjustment.width) / r + adjustment.height,
            ),
            (None, Some(height)) => (
                AbsoluteAxis::Horizontal,
                (height - adjustment.height) * r + adjustment.width,
            ),
            _ => return None,
        };
        let (min, max, floor) = match axis {
            AbsoluteAxis::Horizontal => (min.width, max.width, floor.width),
            AbsoluteAxis::Vertical => (min.height, max.height, floor.height),
        };
        Some(Self {
            axis,
            size: size.maybe_clamp(min, max).max(floor),
            content_minimum: !scroll_container,
            min,
            max,
            floor,
        })
    }

    /// `known` with the ratio's axis set to `size`.
    pub(super) fn set(&self, known: Size<Option<f32>>, size: f32) -> Size<Option<f32>> {
        match self.axis {
            AbsoluteAxis::Horizontal => Size {
                width: Some(size),
                height: known.height,
            },
            AbsoluteAxis::Vertical => Size {
                width: known.width,
                height: Some(size),
            },
        }
    }

    /// The size in its axis once the content needs `content` there.
    pub(super) fn settle(&self, content: f32) -> f32 {
        self.size.max(content).maybe_clamp(self.min, self.max).max(self.floor)
    }
}

/// One run of a container's algorithm with its own size already resolved:
/// its output, and what its content took in the ratio's axis when the run
/// can tell (else [`with_ratio`] asks a probe).
pub(super) struct Laid {
    pub output: LayoutOutput,
    pub content: Option<f32>,
}

/// What a container's run tells [`with_ratio`] of its content.
#[derive(Clone, Copy)]
pub(super) enum Reports {
    /// What its content took in height ([`Laid::content`]); `rerun_last`:
    /// the last run differs from the others (it joins the parent's block
    /// formatting context), so a run whose size stands is run again as it.
    Content { rerun_last: bool },
    /// Nothing: a probe without the size finds it.
    Nothing,
}

/// Lay a container out whose ratio sizes an axis (`ratio`), at least as
/// large there as its content: `inputs` carries its sizes before the ratio
/// (`known_dimensions`), and `run` its algorithm for given sizes, told
/// whether the run is the last (the one whose layout stands).
///
/// - The height, when the run reports its content: run at the ratio's
///   height, taken as definite (a percentage inside resolves against it,
///   as Chrome has it: an image `height: 100%` fills the box, not grows
///   it); if the content took more, run again at that.
/// - The height otherwise: a probe without it gives the content's height;
///   then run at the larger.
/// - The width: the min-content width (a probe without the width) is the
///   least it takes; then run at the larger.
pub(super) fn with_ratio<V>(
    view: &mut V,
    inputs: LayoutInput,
    ratio: Ratio,
    reports: Reports,
    mut run: impl FnMut(&mut V, LayoutInput, bool) -> Laid,
) -> LayoutOutput {
    let known = inputs.known_dimensions;
    let at = |size: f32| LayoutInput {
        known_dimensions: ratio.set(known, size),
        ..inputs
    };
    if !ratio.content_minimum {
        return run(view, at(ratio.size), true).output;
    }
    let content = match ratio.axis {
        AbsoluteAxis::Vertical if let Reports::Content { rerun_last } = reports => {
            let laid = run(view, at(ratio.size), false);
            let size = ratio.settle(laid.content.unwrap_or(0.0));
            if size.to_bits() == ratio.size.to_bits() && !rerun_last {
                return laid.output;
            }
            return run(view, at(size), true).output;
        }
        AbsoluteAxis::Vertical => {
            run(
                view,
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    axis: RequestedAxis::Vertical,
                    ..inputs
                },
                false,
            )
            .output
            .size
            .height
        }
        AbsoluteAxis::Horizontal => {
            run(
                view,
                LayoutInput {
                    run_mode: RunMode::ComputeSize,
                    axis: RequestedAxis::Horizontal,
                    available_space: Size {
                        width: AvailableSpace::MinContent,
                        height: inputs.available_space.height,
                    },
                    ..inputs
                },
                false,
            )
            .output
            .size
            .width
        }
    };
    run(view, at(ratio.settle(content)), true).output
}
