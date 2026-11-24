//! A leaf with children: an inline formatting context (a paragraph holding
//! elements, CSS's atomic inlines: an `inline-flex` button in a `<p>`).
//!
//! The leaf measures as any leaf does ([`super::leaf::compute`]), its
//! children taken into its lines as boxes ([`Measure::measure_inline`]).
//! Each child is sized as CSS sizes an `inline-block` (css-sizing-3 §5.2,
//! CSS 2.2 §10.3.9): its width shrinks to fit, `min(max(min-content,
//! available), max-content)` less its margins, its own `width` and min and
//! max kept; its height is what its content takes at that width. Its
//! baseline is its first baseline (an `inline-flex` box's is its first
//! line's, a scroll container's its bottom edge, Taffy's
//! `child_first_baseline`), and with none its bottom margin edge; the
//! leaf's lines align it there (`vertical-align: baseline`) and grow to
//! hold it, the paragraph's strut still taken (Parley's line box). The
//! final layout places each child where its line put it.
//!
//! A paragraph reads its children's content to size its lines, so a child
//! is never a relayout boundary here: its change lays the paragraph out
//! again. Every child's answer is a pure function of its subtree and the
//! space given, so the result is the same on any thread.

use taffy::{
    AvailableSpace, CoreStyle, Display, Layout, LayoutInput, LayoutOutput, Line, Point, Rect, RequestedAxis,
    ResolveOrZero, RunMode, Size, SizingMode,
};

use super::grid::{child_first_baseline, overflow_contribution};
use super::{
    InlineBox, Measure, Node, NodeStyle, Schedule, Stats, View, compute as compute_node, compute_missed, leaf,
};

/// Lay `view`'s node, a leaf with children, out for `input`.
#[expect(clippy::arithmetic_side_effects, reason = "f32 layout arithmetic")]
pub(super) fn compute<S: NodeStyle, L: Measure>(view: &mut View<'_, S, L>, input: LayoutInput) -> LayoutOutput {
    let View {
        node,
        cx,
        schedule,
        stats,
    } = view;
    let schedule = *schedule;
    let Node {
        style,
        leaf,
        children,
        run,
        inline,
        ..
    } = &mut **node;
    let run = *run;
    let Some(leaf) = leaf.as_ref() else {
        return LayoutOutput::HIDDEN;
    };
    let style = style.view();
    let calc = S::calc;
    let mut out = leaf::compute(input, &style, calc, leaf.replaced(), |known, available| {
        let laid = boxes(children, known, available, run, &mut **cx, schedule, stats);
        let boxes: Vec<InlineBox> = laid.iter().map(|l| l.boxed).collect();
        stats.measures = stats.measures.saturating_add(1);
        leaf.measure_inline(&mut **cx, known, available, &boxes)
    });
    if input.run_mode != RunMode::PerformLayout {
        return out;
    }
    // Placed in the content box the final layout gave it, at its width, as
    // its text is painted there.
    let padding = style.padding().resolve_or_zero(input.parent_size.width, calc);
    let border = style.border().resolve_or_zero(input.parent_size.width, calc);
    let inset = padding + border;
    let width = (out.size.width - inset.left - inset.right).max(0.0);
    let height = (out.size.height - inset.top - inset.bottom).max(0.0);
    let known = Size {
        width: Some(width),
        height: Some(height),
    };
    let space = Size {
        width: AvailableSpace::Definite(width),
        height: input.available_space.height,
    };
    let laid = boxes(children, known, space, run, &mut **cx, schedule, stats);
    let boxed: Vec<InlineBox> = laid.iter().map(|l| l.boxed).collect();
    let places = leaf.place_inline(&mut **cx, width, &boxed);
    let scroll_container = style.overflow().x.is_scroll_container() || style.overflow().y.is_scroll_container();
    let mut overflow = out.scrollable_overflow_rect;
    for (i, c) in children.iter_mut().enumerate() {
        let order = u32::try_from(i).unwrap_or(u32::MAX);
        if c.style.display() == Display::None {
            set_layout(c, Layout::with_order(order));
            set_hidden(c, false);
            let hidden = LayoutInput {
                run_mode: RunMode::PerformHiddenLayout,
                ..input
            };
            compute_node(c, hidden, None, &mut **cx, schedule, stats);
            continue;
        }
        let (at, shown) = places.get(i).copied().unwrap_or((Point::ZERO, true));
        set_hidden(c, !shown);
        let (margin, c_padding, c_border, c_overflow, c_contain, scrollbar) = {
            let cs = c.style.view();
            (
                cs.margin().resolve_or_zero(Some(width), calc),
                cs.padding().resolve_or_zero(Some(width), calc),
                cs.border().resolve_or_zero(Some(width), calc),
                cs.overflow(),
                cs.contain(),
                cs.scrollbar_width(),
            )
        };
        let (size, scrollable) = laid.get(i).map_or((Size::ZERO, Rect::ZERO), |l| (l.size, l.overflow));
        let location = Point {
            x: inset.left + at.x + margin.left,
            y: inset.top + at.y + margin.top,
        };
        set_layout(
            c,
            Layout {
                order,
                location,
                size,
                scrollable_overflow_rect: scrollable,
                scrollbar_size: Size {
                    width: if c_overflow.y == taffy::Overflow::Scroll {
                        scrollbar
                    } else {
                        0.0
                    },
                    height: if c_overflow.x == taffy::Overflow::Scroll {
                        scrollbar
                    } else {
                        0.0
                    },
                },
                border: c_border,
                padding: c_padding,
                margin,
            },
        );
        overflow = overflow.union(overflow_contribution(
            location,
            size,
            scrollable,
            c_overflow,
            c_contain,
            scroll_container,
        ));
    }
    out.scrollable_overflow_rect = overflow;
    if *inline != boxed {
        *inline = boxed;
    }
    out
}

/// Set `c`'s layout, marking it moved when it changed.
fn set_layout<S: NodeStyle, L: Measure>(c: &mut Node<S, L>, layout: Layout) {
    if c.layout != layout {
        c.layout = layout;
        c.moved = true;
    }
}

/// Set whether `c` is hidden by its paragraph's clamp or cut, marking it
/// moved when that changed (its frame node is written again).
fn set_hidden<S: NodeStyle, L: Measure>(c: &mut Node<S, L>, hidden: bool) {
    if c.inline_hidden != hidden {
        c.inline_hidden = hidden;
        c.moved = true;
    }
}

/// A child laid out at its shrink-to-fit width: its margin box in the
/// lines, and its border box's size and scrollable overflow.
struct Laid {
    boxed: InlineBox,
    size: Size<f32>,
    overflow: Rect<f32>,
}

/// Each of `children` in a content box `known` and `available` give, laid
/// out (a final layout) at its shrink-to-fit width.
fn boxes<S: NodeStyle, L: Measure>(
    children: &mut [Node<S, L>],
    known: Size<Option<f32>>,
    available: Size<AvailableSpace>,
    run: u64,
    cx: &mut L::Cx,
    schedule: Schedule,
    stats: &mut Stats,
) -> Vec<Laid> {
    let width = known.width.map_or(available.width, AvailableSpace::Definite);
    let parent_size = Size {
        width: known.width.or(match available.width {
            AvailableSpace::Definite(w) => Some(w),
            _ => None,
        }),
        height: known.height,
    };
    let calc = S::calc;
    children
        .iter_mut()
        .map(|c| {
            if c.style.display() == Display::None {
                return Laid {
                    boxed: InlineBox::default(),
                    size: Size::ZERO,
                    overflow: Rect::ZERO,
                };
            }
            let (margin, overflow_y) = {
                let cs = c.style.view();
                (cs.margin().resolve_or_zero(parent_size.width, calc), cs.overflow().y)
            };
            let (mx, my) = (margin.left + margin.right, margin.top + margin.bottom);
            let mut probe = |space: AvailableSpace| {
                lay(
                    c,
                    LayoutInput {
                        run_mode: RunMode::ComputeSize,
                        sizing_mode: SizingMode::InherentSize,
                        axis: RequestedAxis::Horizontal,
                        known_dimensions: Size::NONE,
                        known_dimensions_are_definite: Size {
                            width: true,
                            height: true,
                        },
                        parent_size,
                        available_space: Size {
                            width: space,
                            height: available.height,
                        },
                        vertical_margins_are_collapsible: Line::FALSE,
                    },
                    run,
                    cx,
                    schedule,
                    stats,
                )
                .size
                .width
            };
            let w = match width {
                AvailableSpace::Definite(room) => {
                    let min = probe(AvailableSpace::MinContent);
                    let max = probe(AvailableSpace::MaxContent);
                    max.min(min.max(room - mx))
                }
                space => probe(space),
            };
            let out = lay(
                c,
                LayoutInput {
                    run_mode: RunMode::PerformLayout,
                    sizing_mode: SizingMode::InherentSize,
                    axis: RequestedAxis::Both,
                    known_dimensions: Size {
                        width: Some(w),
                        height: None,
                    },
                    known_dimensions_are_definite: Size {
                        width: true,
                        height: true,
                    },
                    parent_size,
                    available_space: Size {
                        width: AvailableSpace::Definite(w),
                        height: available.height,
                    },
                    vertical_margins_are_collapsible: Line::FALSE,
                },
                run,
                cx,
                schedule,
                stats,
            );
            let baseline = child_first_baseline(out.baselines.first, out.size.height, overflow_y);
            Laid {
                boxed: InlineBox {
                    width: out.size.width + mx,
                    height: out.size.height + my,
                    baseline: baseline.map(|b| b + margin.top),
                },
                size: out.size,
                overflow: out.scrollable_overflow_rect,
            }
        })
        .collect()
}

/// Ask child `c` `input`, from its cache when it holds the answer: what
/// [`View::child_layout`] does for a child, the paragraph reading its
/// content (no relayout boundary). The out-of-flow boxes it passes up are
/// laid out by it: a paragraph is no containing block for them here.
fn lay<S: NodeStyle, L: Measure>(
    c: &mut Node<S, L>,
    input: LayoutInput,
    run: u64,
    cx: &mut L::Cx,
    schedule: Schedule,
    stats: &mut Stats,
) -> LayoutOutput {
    c.tight = false;
    stats.asked.count(&input);
    if input.run_mode == RunMode::PerformLayout {
        c.input = Some(input);
        c.placed_in = run;
    }
    if let Some(out) = c.cache.get(&input, c.reads()) {
        c.in_flow = false;
        stats.calls = stats.calls.saturating_add(1);
        return out;
    }
    compute_missed(c, input, cx, schedule, stats)
}
