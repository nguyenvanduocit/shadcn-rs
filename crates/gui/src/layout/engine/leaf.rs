//! A leaf's layout: Taffy 0.15.1's `compute_leaf_layout`, ported step for
//! step, with two differences.
//!
//! - **The measure gives baselines** ([`Measured`]): text has a first and a
//!   last baseline, which baseline alignment in flexbox, grid and block
//!   reads from the leaf's final layout. Taffy's leaf has none, so a
//!   paragraph aligned by its baseline was aligned by its bottom edge.
//! - **`aspect-ratio` as css-sizing-4 §5 has it.** Taffy ends with
//!   `height = max(height, width / ratio)` whatever the height was, so a
//!   leaf whose width and height are both definite (`width: 120px; height:
//!   40px; aspect-ratio: 1`, or one a flex or grid container sized both
//!   ways) still took its height from the ratio in its final layout, though
//!   its size probe, answered early, did not. Here a ratio sizes an axis
//!   only when that axis is not definite and the other is (or neither is:
//!   the height then follows the width the content gave), in the box
//!   `box-sizing` names; the result is the larger of that and the
//!   content's size in the axis (its automatic minimum, which a scroll
//!   container and a replaced element do not have), clamped by the axis's
//!   min and max.

use taffy::{
    AvailableSpace, Baselines, BoxSizing, CollapsibleMarginSet, CoreStyle, LayoutInput, LayoutOutput, MaybeMath,
    MaybeResolve, Overflow, Rect, ResolveOrZero, RunMode, Size, SizingMode,
};

use super::sizing::{ratio_of, transfer};

/// What a leaf's measure found: its content size, and its baselines from
/// the top of its content box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measured {
    pub size: Size<f32>,
    pub baselines: Baselines,
}

impl Measured {
    /// A content size with no baselines (a box aligned by its baseline then
    /// uses its edge).
    #[must_use]
    pub const fn size(size: Size<f32>) -> Self {
        Self {
            size,
            baselines: Baselines::NONE,
        }
    }
}

impl From<Size<f32>> for Measured {
    fn from(size: Size<f32>) -> Self {
        Self::size(size)
    }
}

/// Lay a leaf out for `inputs`: `measure` gets the known content size and
/// the space available to the content box, as Taffy's measure function
/// does; `replaced` says it is a replaced element (an image), whose size in
/// an axis its aspect ratio decides has no content-based minimum.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "f32 layout arithmetic, as in Taffy's own leaf layout"
)]
pub fn compute(
    inputs: LayoutInput,
    style: &impl CoreStyle,
    calc: impl Fn(*const (), f32) -> f32,
    replaced: bool,
    mut measure: impl FnMut(Size<Option<f32>>, Size<AvailableSpace>) -> Measured,
) -> LayoutOutput {
    let LayoutInput {
        known_dimensions,
        parent_size,
        available_space,
        sizing_mode,
        run_mode,
        ..
    } = inputs;

    // Percentage padding and border resolve against the inline size on both axes.
    let padding = style.padding().resolve_or_zero(parent_size.width, &calc);
    let border = style.border().resolve_or_zero(parent_size.width, &calc);
    let padding_border = padding + border;
    let pb_sum = padding_border.sum_axes();
    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
        pb_sum
    } else {
        Size::ZERO
    };

    // The preferred, min and max sizes; a content-size probe ignores them.
    // `set` is what is definite before the ratio: known, or the style's.
    let (set, node_min_size, node_max_size, aspect_ratio) = match sizing_mode {
        // A content-size probe with one axis known still sees the ratio
        // carry it to the other: a box's min-content size in an axis its
        // ratio sizes is the ratio's (css-sizing-4 §5.1).
        SizingMode::ContentSize => (
            known_dimensions,
            Size::NONE,
            Size::NONE,
            style
                .aspect_ratio()
                .filter(|_| known_dimensions.width.is_some() != known_dimensions.height.is_some()),
        ),
        SizingMode::InherentSize => {
            let aspect_ratio = style.aspect_ratio();
            let style_size = style
                .size()
                .maybe_resolve(parent_size, &calc)
                .maybe_add(box_sizing_adjustment);
            // A min carries through the ratio only into an `auto` axis.
            let style_min_size = transfer(
                style.min_size().maybe_resolve(parent_size, &calc),
                ratio_of(style),
                style.size(),
            )
            .maybe_add(box_sizing_adjustment);
            let style_max_size = style
                .max_size()
                .maybe_resolve(parent_size, &calc)
                .maybe_add(box_sizing_adjustment);
            (
                known_dimensions.or(style_size),
                style_min_size,
                style_max_size,
                aspect_ratio,
            )
        }
    };
    let ratio = aspect_ratio.filter(|r| r.is_finite() && *r > 0.0);
    // The ratio in the box `box-sizing` names: one axis's border-box size
    // to the other's.
    let height_of = |r: f32, width: f32| (width - box_sizing_adjustment.width) / r + box_sizing_adjustment.height;
    let width_of = |r: f32, height: f32| (height - box_sizing_adjustment.height) * r + box_sizing_adjustment.width;
    // Each axis, definite or through the ratio from the other's size
    // within its min and max.
    let node_size = match (ratio, set.width, set.height) {
        (Some(r), Some(width), None) => Size {
            width: Some(width),
            height: Some(height_of(
                r,
                width.maybe_clamp(node_min_size.width, node_max_size.width),
            )),
        },
        (Some(r), None, Some(height)) => Size {
            width: Some(width_of(
                r,
                height.maybe_clamp(node_min_size.height, node_max_size.height),
            )),
            height: Some(height),
        },
        _ => set,
    };
    // A size the ratio decides is at least the content's in that axis
    // (its automatic minimum), unless it is a scroll container or replaced.
    let is_scroll_container = style.overflow().x.is_scroll_container() || style.overflow().y.is_scroll_container();
    let has_content_minimum = !replaced && !is_scroll_container;
    let from_ratio = Size {
        width: ratio.is_some() && set.width.is_none() && set.height.is_some() && has_content_minimum,
        height: ratio.is_some() && set.height.is_none(),
    };

    // A box that scrolls on one axis reserves its scrollbar on the other.
    let scrollbar_gutter = style.overflow().transpose().map(|overflow| match overflow {
        Overflow::Scroll => style.scrollbar_width(),
        _ => 0.0,
    });
    let mut content_box_inset = padding_border;
    content_box_inset.right += scrollbar_gutter.x;
    content_box_inset.bottom += scrollbar_gutter.y;

    let has_styles_preventing_being_collapsed_through = !style.is_block()
        || is_scroll_container
        || style.position().is_out_of_flow()
        || style.contain().establishes_independent_formatting_context()
        || padding.top > 0.0
        || padding.bottom > 0.0
        || border.top > 0.0
        || border.bottom > 0.0
        || matches!(node_size.height, Some(h) if h > 0.0)
        || matches!(node_min_size.height, Some(h) if h > 0.0);

    // Both sizes known, and the content cannot make either larger: a size
    // probe needs no measure.
    if run_mode == RunMode::ComputeSize
        && has_styles_preventing_being_collapsed_through
        && !(from_ratio.width || (from_ratio.height && has_content_minimum))
        && let Size {
            width: Some(width),
            height: Some(height),
        } = node_size
    {
        let size = Size { width, height }
            .maybe_clamp(node_min_size, node_max_size)
            .maybe_max(padding_border.sum_axes().map(Some));
        return LayoutOutput {
            size,
            scrollable_overflow_rect: Rect::ZERO,
            baselines: Baselines::NONE,
            top_margin: CollapsibleMarginSet::ZERO,
            bottom_margin: CollapsibleMarginSet::ZERO,
            margins_can_collapse_through: false,
            oof_candidates: taffy::OofCandidates::NONE,
            oof_positioning_area: None,
        };
    }

    // A width the ratio decides is at least the content's min-content width.
    let node_size = match node_size {
        Size {
            width: Some(width),
            height: Some(height),
        } if from_ratio.width => {
            let height = height.maybe_clamp(node_min_size.height, node_max_size.height);
            let content_height = height - content_box_inset.vertical_axis_sum();
            let min_content = measure(
                Size {
                    width: None,
                    height: Some(content_height),
                },
                Size {
                    width: AvailableSpace::MinContent,
                    height: AvailableSpace::Definite(content_height),
                },
            );
            Size {
                width: Some(width.max(min_content.size.width + content_box_inset.horizontal_axis_sum())),
                height: Some(height),
            }
        }
        other => other,
    };

    // The space the content box has.
    let available_space = Size {
        width: known_dimensions
            .width
            .map(AvailableSpace::from)
            .unwrap_or(available_space.width)
            .maybe_set(known_dimensions.width)
            .maybe_set(node_size.width)
            .map_definite_value(|size| {
                size.maybe_clamp(node_min_size.width, node_max_size.width) - content_box_inset.horizontal_axis_sum()
            }),
        height: known_dimensions
            .height
            .map(AvailableSpace::from)
            .unwrap_or(available_space.height)
            .maybe_set(known_dimensions.height)
            .maybe_set(node_size.height)
            .map_definite_value(|size| {
                size.maybe_clamp(node_min_size.height, node_max_size.height) - content_box_inset.vertical_axis_sum()
            }),
    };

    let measured = measure(
        match run_mode {
            RunMode::ComputeSize => known_dimensions,
            RunMode::PerformLayout | RunMode::PerformHiddenLayout => Size::NONE,
        },
        available_space,
    );
    let measured_size = measured.size;
    let content_size = measured_size + content_box_inset.sum_axes();
    let clamped_size = node_size
        .unwrap_or(content_size)
        .maybe_clamp(node_min_size, node_max_size);
    // A height the ratio decides follows the width the box took; the
    // content may make it taller.
    let height = match ratio {
        Some(r) if from_ratio.height => {
            let minimum = if has_content_minimum { content_size.height } else { 0.0 };
            height_of(r, clamped_size.width)
                .max(minimum)
                .maybe_clamp(node_min_size.height, node_max_size.height)
        }
        _ => clamped_size.height,
    };
    let size = Size {
        width: clamped_size.width,
        height,
    }
    .maybe_max(padding_border.sum_axes().map(Some));

    // A scroll container's own end padding is part of its scrollable
    // overflow; another box's is not.
    let scrollable_overflow_rect = {
        let is_rtl = style.direction().is_rtl();
        let start_padding = if is_rtl { padding.right } else { padding.left };
        let end_padding = if is_rtl { padding.left } else { padding.right };
        Rect {
            left: 0.0,
            right: start_padding + measured_size.width + if is_scroll_container { end_padding } else { 0.0 },
            top: 0.0,
            bottom: padding.top + measured_size.height + if is_scroll_container { padding.bottom } else { 0.0 },
        }
    };

    // Baselines from the top of the border box.
    let top = padding_border.top;
    let baselines = Baselines {
        first: measured.baselines.first.map(|b| b + top),
        last: measured.baselines.last.map(|b| b + top),
    };

    LayoutOutput {
        size,
        scrollable_overflow_rect,
        baselines,
        top_margin: CollapsibleMarginSet::ZERO,
        bottom_margin: CollapsibleMarginSet::ZERO,
        margins_can_collapse_through: !has_styles_preventing_being_collapsed_through
            && size.height == 0.0
            && measured_size.height == 0.0,
        oof_candidates: taffy::OofCandidates::NONE,
        oof_positioning_area: None,
    }
}

#[cfg(test)]
mod tests {
    use taffy::prelude::{length, percent};
    use taffy::{Line, RequestedAxis};

    use super::*;

    type Style = taffy::Style<String>;

    fn input(run_mode: RunMode, known: Size<Option<f32>>) -> LayoutInput {
        LayoutInput {
            run_mode,
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            known_dimensions: known,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: Size {
                width: Some(400.0),
                height: Some(600.0),
            },
            available_space: Size {
                width: AvailableSpace::Definite(400.0),
                height: AvailableSpace::Definite(600.0),
            },
            vertical_margins_are_collapsible: Line::FALSE,
        }
    }

    /// Text-like content: `height` tall at any width, baseline 18 px down.
    fn content(height: f32) -> impl FnMut(Size<Option<f32>>, Size<AvailableSpace>) -> Measured {
        move |_, _| Measured {
            size: Size { width: 30.0, height },
            baselines: Baselines {
                first: Some(18.0),
                last: Some(height - 6.0),
            },
        }
    }

    fn lay_out(style: &Style, run_mode: RunMode, known: Size<Option<f32>>, height: f32) -> LayoutOutput {
        compute(input(run_mode, known), style, |_, _| 0.0, false, content(height))
    }

    #[test]
    fn a_box_sized_both_ways_ignores_its_aspect_ratio() {
        // `width: 120px; height: 40px; aspect-ratio: 1`: 120x40 in Chrome.
        let style = Style {
            size: Size {
                width: length(120.0),
                height: length(40.0),
            },
            aspect_ratio: Some(1.0),
            ..Style::default()
        };
        for mode in [RunMode::ComputeSize, RunMode::PerformLayout] {
            let out = lay_out(&style, mode, Size::NONE, 24.0);
            assert_eq!(
                out.size,
                Size {
                    width: 120.0,
                    height: 40.0
                },
                "{mode:?}"
            );
        }
        // Sized both ways by its container (a flex item, a stretched grid cell).
        let out = lay_out(
            &Style {
                aspect_ratio: Some(1.0),
                ..Style::default()
            },
            RunMode::PerformLayout,
            Size {
                width: Some(120.0),
                height: Some(40.0),
            },
            24.0,
        );
        assert_eq!(
            out.size,
            Size {
                width: 120.0,
                height: 40.0
            }
        );
    }

    #[test]
    fn a_ratio_height_is_at_least_the_content_unless_it_scrolls() {
        let mut style = Style {
            aspect_ratio: Some(4.0),
            ..Style::default()
        };
        let known = Size {
            width: Some(100.0),
            height: None,
        };
        // The content is shorter: the ratio's 25 px.
        assert_eq!(lay_out(&style, RunMode::PerformLayout, known, 20.0).size.height, 25.0);
        // Taller: the content's, in a probe and a final layout alike.
        assert_eq!(lay_out(&style, RunMode::ComputeSize, known, 72.0).size.height, 72.0);
        assert_eq!(lay_out(&style, RunMode::PerformLayout, known, 72.0).size.height, 72.0);
        // A scroll container has no content-based minimum.
        style.overflow = taffy::Point {
            x: Overflow::Hidden,
            y: Overflow::Hidden,
        };
        assert_eq!(lay_out(&style, RunMode::PerformLayout, known, 72.0).size.height, 25.0);
        // Nor a replaced element.
        let plain = Style {
            aspect_ratio: Some(4.0),
            ..Style::default()
        };
        let image = compute(
            input(RunMode::PerformLayout, known),
            &plain,
            |_, _| 0.0,
            true,
            content(72.0),
        );
        assert_eq!(image.size.height, 25.0);
    }

    #[test]
    fn the_ratio_starts_from_the_clamped_size_and_clamps_its_result() {
        // `width: 200px; max-width: 100px; aspect-ratio: 1`: 100x100 in Chrome.
        let style = Style {
            size: Size {
                width: length(200.0),
                height: taffy::Dimension::auto(),
            },
            max_size: Size {
                width: length(100.0),
                height: taffy::LengthPercentageAuto::auto(),
            },
            aspect_ratio: Some(1.0),
            ..Style::default()
        };
        let out = lay_out(&style, RunMode::PerformLayout, Size::NONE, 0.0);
        assert_eq!(
            out.size,
            Size {
                width: 100.0,
                height: 100.0
            }
        );
        // `width: 50%; aspect-ratio: 2; max-height: 30px`: the height is clamped.
        let style = Style {
            size: Size {
                width: percent(0.5),
                height: taffy::Dimension::auto(),
            },
            max_size: Size {
                width: taffy::LengthPercentageAuto::auto(),
                height: length(30.0),
            },
            aspect_ratio: Some(2.0),
            ..Style::default()
        };
        let out = lay_out(&style, RunMode::PerformLayout, Size::NONE, 0.0);
        assert_eq!(
            out.size,
            Size {
                width: 200.0,
                height: 30.0
            }
        );
    }

    #[test]
    fn a_ratio_width_is_at_least_the_min_content_width() {
        // `height: 24px; aspect-ratio: 1` around a 30 px word: 30 wide.
        let style = Style {
            size: Size {
                width: taffy::Dimension::auto(),
                height: length(24.0),
            },
            aspect_ratio: Some(1.0),
            ..Style::default()
        };
        let out = lay_out(&style, RunMode::PerformLayout, Size::NONE, 24.0);
        assert_eq!(
            out.size,
            Size {
                width: 30.0,
                height: 24.0
            }
        );
        let wide = Style {
            aspect_ratio: Some(2.0),
            ..style
        };
        assert_eq!(
            lay_out(&wide, RunMode::PerformLayout, Size::NONE, 24.0).size.width,
            48.0
        );
    }

    #[test]
    fn baselines_are_from_the_top_of_the_border_box() {
        let style = Style {
            padding: taffy::Rect {
                top: length(10.0),
                ..taffy::Rect::zero()
            },
            border: taffy::Rect {
                top: length(2.0),
                ..taffy::Rect::zero()
            },
            ..Style::default()
        };
        let out = lay_out(&style, RunMode::PerformLayout, Size::NONE, 48.0);
        assert_eq!(out.baselines.first, Some(30.0));
        assert_eq!(out.baselines.last, Some(54.0));
    }
}
