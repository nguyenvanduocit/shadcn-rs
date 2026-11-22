//! What a leaf of the layout tree measures: a paragraph, a text input, an
//! image (ADR-0006's measuring rules), with the [`TextSystem`] of the thread
//! that lays it out.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use taffy::{AvailableSpace, Baselines, Size};

use super::engine::{Measure, MeasureReads, Measured, Worker};
use crate::style::{ResolvedText, TextAlign};
use crate::text::{Paragraph, Run, TextMetrics, TextSystem};

/// What a leaf is measured from; equal leaves measure alike, so a leaf that
/// compares equal to last frame's keeps its cached layout. Made through
/// [`Leaf::measuring`], it holds nothing that only paints (a colour, an
/// underline, a run's background): a hover that recolours text lays out
/// nothing again.
#[derive(Clone, Debug, PartialEq)]
pub enum Leaf {
    Text {
        content: String,
        text: ResolvedText,
        runs: Vec<Run>,
    },
    Image(ImageMeasure),
    Input {
        content: String,
        text: ResolvedText,
        multiline: bool,
    },
}

impl Leaf {
    /// This leaf with what only paints taken out.
    #[must_use]
    pub fn measuring(self) -> Self {
        fn shape(t: ResolvedText) -> ResolvedText {
            ResolvedText {
                color: crate::color::Color::TRANSPARENT,
                underline: false,
                underline_offset: None,
                line_through: false,
                ..t
            }
        }
        match self {
            Self::Text { content, text, runs } => Self::Text {
                content,
                text: shape(text),
                runs: runs
                    .into_iter()
                    .map(|r| Run {
                        text: shape(r.text),
                        background: None,
                        radius: 0.0,
                        ..r
                    })
                    .collect(),
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

    fn measure(&self, text: &mut TextSystem, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Measured {
        match self {
            Self::Text { content, text: t, runs } => {
                let p = Paragraph::new(content, runs);
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
                    let m = text.metrics(content.as_str(), t, width_constraint(known.width, available.width));
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

/// A pool thread measured text this pass.
static USED: AtomicBool = AtomicBool::new(false);

/// End a layout pass: the pool threads forget what they shaped (the
/// layout tree keeps the answers it gave), so it is not held twice, nor
/// past the pass while the pool idles.
pub(crate) fn end_pass() {
    if USED.swap(false, Ordering::Relaxed) {
        super::engine::pool().broadcast(|_| {
            IDLE.with(|idle| {
                for text in idle.borrow_mut().iter_mut() {
                    text.forget_shaped();
                }
            });
        });
    }
}

thread_local! {
    /// This pool thread's text system. A job that waits on nested
    /// parallel work may run another job on the same thread meanwhile,
    /// which takes a second one for as long as it runs.
    static IDLE: RefCell<Vec<TextSystem>> = const { RefCell::new(Vec::new()) };
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
        USED.store(true, Ordering::Relaxed);
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
