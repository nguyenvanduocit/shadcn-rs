//! What a leaf of the layout tree measures: a paragraph, a text input, an
//! image (ADR-0006's measuring rules), with the [`TextSystem`] of the thread
//! that lays it out.

use std::cell::RefCell;

use taffy::{AvailableSpace, Baselines, Size};

use super::engine::{InlineBox, Measure, MeasureReads, Measured, Worker};
use crate::style::{ResolvedText, TextAlign};
use crate::text::{InlineSlot, Paragraph, Run, TextMetrics, TextSystem};

/// What a leaf is measured from; equal leaves measure alike, so a leaf that
/// compares equal to last frame's keeps its cached layout. Made through
/// [`Leaf::measuring`], it holds nothing that only paints (a colour, an
/// underline, a run's background): a hover that recolours text lays out
/// nothing again.
#[derive(Clone, Debug, PartialEq)]
pub enum Leaf {
    Text {
        /// Shared with the frame node that shows it.
        content: std::sync::Arc<str>,
        text: ResolvedText,
        /// Paint taken out ([`Leaf::measuring_run`]); shared with the
        /// last leaf while they compare equal.
        runs: std::sync::Arc<[Run]>,
        /// The [`crate::text::OBJECT`]s its atomic inlines stand on, in
        /// order: one to each of the layout node's children, and one to
        /// each inline that took no space, which stays empty.
        objects: Vec<Object>,
    },
    Image(ImageMeasure),
    Input {
        /// What it shows, shared with its frame node
        /// ([`crate::edit::Shown`]): it measures again only when that
        /// changes, not as its caret moves.
        content: std::sync::Arc<str>,
        text: ResolvedText,
        multiline: bool,
    },
}

impl Leaf {
    /// This leaf with what only paints taken out.
    #[must_use]
    pub fn measuring(self) -> Self {
        let shape = ResolvedText::shape;
        match self {
            Self::Text {
                content,
                text,
                runs,
                objects,
            } => Self::Text {
                content,
                objects,
                text: shape(text),
                // Most come with paint already out, and stay shared.
                runs: if runs.iter().all(|r| *r == Self::measuring_run(r)) {
                    runs
                } else {
                    runs.iter().map(Self::measuring_run).collect()
                },
            },
            Self::Input {
                content,
                text,
                multiline,
            } => Self::Input {
                content,
                text: shape(text),
                multiline,
            },
            image @ Self::Image(_) => image,
        }
    }
}

impl Leaf {
    /// `a` and `b` measure alike but for their inline objects, which
    /// `lay_objects` sets once the paragraph's children are patched.
    #[must_use]
    pub fn same_but_objects(a: Option<&Self>, b: Option<&Self>) -> bool {
        match (a, b) {
            (
                Some(Self::Text {
                    content: c0,
                    text: t0,
                    runs: r0,
                    ..
                }),
                Some(Self::Text {
                    content: c1,
                    text: t1,
                    runs: r1,
                    ..
                }),
            ) => c0 == c1 && t0 == t1 && r0 == r1,
            (
                Some(Self::Input {
                    content: c0,
                    text: t0,
                    multiline: m0,
                }),
                Some(Self::Input {
                    content: c1,
                    text: t1,
                    multiline: m1,
                }),
            ) => crate::edit::same_text(c0, c1) && t0 == t1 && m0 == m1,
            (a, b) => a == b,
        }
    }

    /// `run` with what only paints taken out: its top and bottom padding
    /// too, which grow its background and not its line (CSS pads a
    /// non-replaced inline box so, CSS 2.1 §10.8.1).
    #[must_use]
    pub fn measuring_run(run: &Run) -> Run {
        Run {
            text: ResolvedText::shape(run.text.clone()),
            background: None,
            radius: 0.0,
            padding: crate::style::Edges {
                top: 0.0,
                bottom: 0.0,
                ..run.padding
            },
            range: run.range.clone(),
        }
    }
}

/// The place of an atomic inline in a paragraph's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Object {
    /// The byte offset of its [`crate::text::OBJECT`].
    pub at: usize,
    /// It took space: it is the next of the layout node's children.
    pub laid: bool,
}

/// Names a child's box in a paragraph's lines: the child's index; an
/// inline that took no space has this bit set over its place among them.
const EMPTY_OBJECT: u64 = 1 << 62;

/// The boxes of `objects` in their paragraph, `boxes` (one to a child) the
/// children's, each named by its child's index.
pub fn slots<'a>(objects: &'a [Object], boxes: &'a [InlineBox]) -> impl Iterator<Item = InlineSlot> + 'a {
    let mut child = 0_usize;
    objects.iter().enumerate().map(move |(j, o)| {
        if o.laid {
            let b = boxes.get(child).copied().unwrap_or_default();
            let id = u64::try_from(child).unwrap_or(0);
            child = child.saturating_add(1);
            InlineSlot {
                at: o.at,
                width: b.width,
                height: b.height,
                baseline: b.baseline,
                id,
            }
        } else {
            InlineSlot {
                at: o.at,
                width: 0.0,
                height: 0.0,
                baseline: None,
                id: EMPTY_OBJECT | u64::try_from(j).unwrap_or(0),
            }
        }
    })
}

/// What an image's box is sized from: its natural size, and which of its
/// sides its style sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageMeasure {
    pub natural: (f32, f32),
    /// The style sets its width, height: a definite space on that axis is
    /// that side's size.
    pub sized: (bool, bool),
    /// Its width or max-width is a percentage: it shrinks to any width, so
    /// adds nothing to its parent's min-content width (css-sizing-3 §5.2.2).
    pub compressible: bool,
}

impl ImageMeasure {
    /// The content size: a side its style or the layout sets, the other kept
    /// to the natural ratio, as CSS sizes a replaced element.
    fn size(&self, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> (f32, f32) {
        let set = |known: Option<f32>, sized: bool, space: AvailableSpace| {
            known.or(match space {
                AvailableSpace::Definite(v) if sized => Some(v),
                _ => None,
            })
        };
        let (nw, nh) = (self.natural.0.max(1.0), self.natural.1.max(1.0));
        match (
            set(known.width, self.sized.0, available.width),
            set(known.height, self.sized.1, available.height),
        ) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * nh / nw),
            (None, Some(h)) => (h * nw / nh, h),
            (None, None) if self.compressible && available.width == AvailableSpace::MinContent => (0.0, 0.0),
            (None, None) => self.natural,
        }
    }
}

/// Width of an `<input>` that sets none, in ems of its font size.
pub(crate) const INPUT_WIDTH_EMS: f32 = 166.0 / 14.0;

/// The width a text input's content is measured in: a textarea wraps to
/// nothing at its min-content width.
fn width_constraint(known: Option<f32>, available: AvailableSpace) -> Option<f32> {
    known.or(match available {
        AvailableSpace::Definite(w) => Some(w),
        AvailableSpace::MinContent => Some(0.0),
        AvailableSpace::MaxContent => None,
    })
}

/// The width text is laid out in, given its box's content width: none for
/// one unbroken left-aligned line, else the box (to wrap, cut or align in).
pub(crate) fn wrap_width(text: &ResolvedText, content_width: f32) -> Option<f32> {
    if text.nowrap && !text.truncate && text.physical_align() == TextAlign::Left {
        None
    } else {
        Some(content_width)
    }
}

impl Measure for Leaf {
    type Cx = TextSystem;

    fn reads(&self) -> MeasureReads {
        match self {
            // Lines break in the width given; the height is what they take.
            // A known width and a definite available one are the same
            // width to it, and it reads no height (`measure` below).
            Self::Text { .. } | Self::Input { .. } => MeasureReads::TEXT,
            // A definite height sizes an image whose style sets its height,
            // and its width follows from its height by its ratio.
            Self::Image(image) => MeasureReads {
                available_height: image.sized.1,
                width_reads_height: true,
                width_alone: false,
            },
        }
    }

    fn replaced(&self) -> bool {
        matches!(self, Self::Image(_))
    }

    fn place_inline(&self, text: &mut TextSystem, width: f32, boxes: &[InlineBox]) -> Vec<(taffy::Point<f32>, bool)> {
        self.placed(text, width, boxes)
    }

    fn measure(&self, text: &mut TextSystem, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Measured {
        self.measure_inline(text, known, available, &[])
    }

    /// A text leaf's max-content and min-content answers from one
    /// paragraph: the wrapped min-content answer needs the unwrapped
    /// width, which is the max-content answer's own.
    fn measure_pair(
        &self,
        text: &mut TextSystem,
        known: Size<Option<f32>>,
        max: Size<AvailableSpace>,
        min: Size<AvailableSpace>,
    ) -> ([Measured; 2], u64) {
        let twice = |text: &mut TextSystem| ([self.measure(text, known, max), self.measure(text, known, min)], 2);
        let Self::Text {
            content,
            text: t,
            runs,
            objects,
        } = self
        else {
            return twice(text);
        };
        if known.width.is_some() || max.width != AvailableSpace::MaxContent || min.width != AvailableSpace::MinContent {
            return twice(text);
        }
        let boxes: Vec<InlineSlot> = slots(objects, &[]).collect();
        let p = Paragraph {
            content,
            runs,
            boxes: &boxes,
        };
        // As `measure_inline` with no width: one line, as wide as it is.
        let free = text.metrics(p, t, None);
        let at_max = measured(free.width, free.height, free);
        // As `measure_inline` at the min-content width.
        let given = text.min_content(p, t);
        let m = text.metrics(p, t, wrap_width(t, given));
        let w = if m.height > t.line_height && !t.nowrap && free.width > given {
            given
        } else {
            m.width
        };
        ([at_max, measured(w, m.height, m)], 1)
    }

    fn measure_inline(
        &self,
        text: &mut TextSystem,
        known: Size<Option<f32>>,
        available: Size<AvailableSpace>,
        boxes: &[InlineBox],
    ) -> Measured {
        match self {
            Self::Text {
                content,
                text: t,
                runs,
                objects,
            } => {
                let boxes: Vec<InlineSlot> = slots(objects, boxes).collect();
                let p = Paragraph {
                    content,
                    runs,
                    boxes: &boxes,
                };
                // The min-content width is the widest piece no line breaks.
                let width = match (known.width, available.width) {
                    (Some(w), _) | (None, AvailableSpace::Definite(w)) => Some(w),
                    (None, AvailableSpace::MinContent) => Some(text.min_content(p, t)),
                    (None, AvailableSpace::MaxContent) => None,
                };
                let m = text.metrics(p, t, width.and_then(|w| wrap_width(t, w)));
                // Text wrapped because it did not fit takes all the width it
                // was given, as a CSS block does, not just its longest line.
                let w = match width {
                    Some(given) if m.height > t.line_height && !t.nowrap => {
                        let (free, _) = text.measure(p, t, None);
                        if free > given { given } else { m.width }
                    }
                    _ => m.width,
                };
                measured(w, m.height, m)
            }
            Self::Image(image) => {
                let (width, height) = image.size(known, available);
                Measured::size(Size { width, height })
            }
            Self::Input {
                content,
                text: t,
                multiline,
            } => {
                if *multiline {
                    let content: &str = content;
                    let m = text.metrics(content, t, width_constraint(known.width, available.width));
                    // An empty field still has a line, and its baseline.
                    let lines = if content.is_empty() {
                        text.metrics(" ", t, None)
                    } else {
                        m
                    };
                    measured(m.width, m.height.max(t.line_height), lines)
                } else {
                    // A browser `<input>` is `size=20` wide: 171px of content
                    // for Inter at 14px, measured in Chrome (NumberFieldDemo: 168 = 166 + border).
                    // Its one line, `line_height` tall, has its primary
                    // font's baseline whatever it holds.
                    let m = text.metrics(" ", t, None);
                    let one = TextMetrics {
                        last_baseline: m.first_baseline,
                        ..m
                    };
                    measured(t.size * INPUT_WIDTH_EMS, t.line_height, one)
                }
            }
        }
    }
}

impl Leaf {
    /// Where a paragraph's children sit in a content box `width` wide, and
    /// whether each shows (a clamp or a cut hides it).
    fn placed(&self, text: &mut TextSystem, width: f32, boxes: &[InlineBox]) -> Vec<(taffy::Point<f32>, bool)> {
        let mut at = vec![(taffy::Point::ZERO, true); boxes.len()];
        let Self::Text {
            content,
            text: t,
            runs,
            objects,
        } = self
        else {
            return at;
        };
        let slots: Vec<InlineSlot> = slots(objects, boxes).collect();
        let p = Paragraph {
            content,
            runs,
            boxes: &slots,
        };
        for b in text.placed_inline_boxes(p, t, wrap_width(t, width)) {
            if let Some(place) = usize::try_from(b.id).ok().and_then(|i| at.get_mut(i)) {
                *place = (
                    taffy::Point {
                        x: b.bounds.x,
                        y: b.bounds.y,
                    },
                    b.shown,
                );
            }
        }
        at
    }
}

/// A text leaf's answer: its size, and the baselines of its lines.
fn measured(width: f32, height: f32, lines: TextMetrics) -> Measured {
    Measured {
        size: Size { width, height },
        baselines: Baselines {
            first: lines.first_baseline,
            last: lines.last_baseline,
        },
    }
}

/// End a layout pass: the pool threads forget what they shaped (the
/// layout tree keeps the answers it gave), so it is not held twice, nor
/// past the pass while the pool idles. Each pass that handed the pool jobs
/// clears after them itself, whatever another thread's pass (the pool is
/// shared) did meanwhile: no flag passes share decides it. A pass that
/// handed it none (one that changed only what paints) touches nothing.
pub(crate) fn end_pass() {
    if super::engine::take_dispatched() {
        super::engine::pool().broadcast(|_| {
            IDLE.with(|idle| {
                for text in idle.borrow_mut().iter_mut() {
                    text.forget_shaped();
                }
            });
        });
    }
}

/// Pieces of text the pool threads' idle text systems hold shaped.
#[cfg(test)]
pub(crate) fn idle_footprint() -> usize {
    super::engine::pool()
        .broadcast(|_| IDLE.with(|idle| idle.borrow().iter().map(TextSystem::footprint).sum::<usize>()))
        .into_iter()
        .sum()
}

crate::engine::engine_local! {
    /// This pool thread's text system. A job that waits on nested
    /// parallel work may run another job on the same thread meanwhile,
    /// which takes a second one for as long as it runs.
    [Fresh] static IDLE: RefCell<Vec<TextSystem>> = const { RefCell::new(Vec::new()) };
}

/// Give each pool thread its text system up front, so the pool's memory is
/// what it is from the first parallel pass, not grown as threads join in.
pub(crate) fn warm_pool() {
    static WARM: std::sync::Once = std::sync::Once::new();
    WARM.call_once(|| {
        super::engine::pool().broadcast(|_| TextSystem::with_worker(|_| ()));
    });
}

impl Worker for TextSystem {
    fn take_work(&mut self) -> u64 {
        self.take_shaped()
    }

    fn with_worker<R>(f: impl FnOnce(&mut Self) -> R) -> R {
        let mut text = IDLE.with(|idle| idle.borrow_mut().pop()).unwrap_or_default();
        let out = f(&mut text);
        IDLE.with(|idle| {
            // One is kept a thread: a second, taken by a job run while
            // another waited, is dropped, so what the pool holds does not
            // depend on how its work happened to be stolen.
            let mut idle = idle.borrow_mut();
            if idle.is_empty() {
                idle.push(text);
            }
        });
        out
    }
}
