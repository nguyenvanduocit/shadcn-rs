//! Laid out text as a screen reader moves through it (ADR-0021): each line
//! cut into runs of one direction, each run's characters (grapheme
//! clusters, the smallest unit a caret steps over) with their byte lengths,
//! where they sit along the line and which of them start a word. AccessKit's
//! `TextRun` nodes are built from these, as Parley's editor example builds
//! them from a `PlainEditor` (`references/parley/examples/editor/src/accessibility.rs`).

use std::ops::Range;

use crate::geometry::Bounds;
use crate::style::ResolvedText;

use super::TextSystem;

/// The most characters one run holds: AccessKit's `word_starts` are `u8`.
const MAX_CHARACTERS: usize = 256;

/// A run of laid out text: a line, or part of one, in one direction.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// The bytes of the content it covers, the hard line break that ends
    /// its line included (AccessKit's contract: the last run of a line holds it).
    pub range: Range<usize>,
    /// Its box, relative to the text box.
    pub bounds: Bounds,
    pub rtl: bool,
    /// Each character's length in bytes.
    pub lengths: Vec<u8>,
    /// Each character's offset from the run's start edge (its right edge
    /// when right to left), and its advance.
    pub positions: Vec<f32>,
    pub widths: Vec<f32>,
    /// The characters that start a word.
    pub word_starts: Vec<u8>,
}

impl TextRun {
    /// The character holding byte `at` (or the run's end, after its last).
    #[must_use]
    pub fn character_at(&self, at: usize) -> usize {
        let mut start = self.range.start;
        for (i, len) in self.lengths.iter().enumerate() {
            let end = start.saturating_add(usize::from(*len));
            if at < end {
                return i;
            }
            start = end;
        }
        self.lengths.len()
    }

    /// The byte where character `index` starts (the run's end past its last).
    #[must_use]
    pub fn byte_of(&self, index: usize) -> usize {
        self.lengths
            .iter()
            .take(index)
            .fold(self.range.start, |at, len| at.saturating_add(usize::from(*len)))
    }
}

impl TextSystem {
    /// `content` laid out at `width` in style `t`, as runs in reading order:
    /// what a text field tells a screen reader about its text. Empty text
    /// has one empty run, for the caret to sit in.
    pub fn text_runs(&mut self, content: &str, t: &ResolvedText, width: Option<f32>) -> Vec<TextRun> {
        let laid = self.laid_out(content.into(), t, width);
        let mut out: Vec<TextRun> = Vec::new();
        for (piece, top) in &laid {
            let first_of_piece = out.len();
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
            let lead = s.key.lead;
            let layout = &s.layout;
            for line in layout.lines() {
                let m = line.metrics();
                let (y0, y1) = (top + m.block_min_coord, top + m.block_max_coord);
                // Runs in visual order give each its offset; then in reading order.
                let mut runs = Vec::new();
                let mut x = m.offset;
                for run in line.runs() {
                    let advance = run.advance();
                    runs.push((run, x));
                    x += advance;
                }
                runs.sort_by_key(|(r, _)| r.text_range().start);
                let line_start = out.len();
                for (run, x) in runs {
                    let rtl = run.is_rtl();
                    let run_advance = run.advance();
                    // A part's box: from its first character's offset along
                    // the run, from the run's right edge when right to left.
                    let place = |mut p: TextRun| {
                        if rtl {
                            p.bounds.x = x + run_advance - (p.bounds.x + p.bounds.width) + s.shift;
                        } else {
                            p.bounds.x += x + s.shift;
                        }
                        p
                    };
                    let mut part: Option<TextRun> = None;
                    let mut along = 0.0_f32;
                    for cluster in run.clusters() {
                        let r = cluster.text_range();
                        if r.end <= lead {
                            continue;
                        }
                        let from = piece.start.saturating_add(r.start.saturating_sub(lead));
                        let to = piece.start.saturating_add(r.end.saturating_sub(lead));
                        if part.as_ref().is_some_and(|p| p.lengths.len() >= MAX_CHARACTERS) {
                            out.extend(part.take().map(place));
                        }
                        let p = part.get_or_insert_with(|| TextRun {
                            range: from..from,
                            bounds: Bounds {
                                x: along,
                                y: y0,
                                width: 0.0,
                                height: y1 - y0,
                            },
                            rtl,
                            lengths: Vec::new(),
                            positions: Vec::new(),
                            widths: Vec::new(),
                            word_starts: Vec::new(),
                        });
                        let advance = cluster.advance();
                        if cluster.is_word_boundary() && !cluster.is_space_or_nbsp() {
                            p.word_starts.push(u8::try_from(p.lengths.len()).unwrap_or(u8::MAX));
                        }
                        p.positions.push(p.bounds.width);
                        p.widths.push(advance);
                        p.lengths.push(u8::try_from(to.saturating_sub(from)).unwrap_or(u8::MAX));
                        p.bounds.width += advance;
                        p.range.end = to;
                        along += advance;
                    }
                    out.extend(part.map(place));
                }
                if out.len() == line_start {
                    // A line with no text (an empty piece): a run for the caret.
                    out.push(TextRun {
                        range: piece.start..piece.start,
                        bounds: Bounds {
                            x: m.offset + s.shift,
                            y: y0,
                            width: 0.0,
                            height: y1 - y0,
                        },
                        rtl: layout.is_rtl(),
                        lengths: Vec::new(),
                        positions: Vec::new(),
                        widths: Vec::new(),
                        word_starts: Vec::new(),
                    });
                }
            }
            if out.len() == first_of_piece {
                out.push(TextRun {
                    range: piece.start..piece.start,
                    bounds: Bounds {
                        x: s.shift,
                        y: *top,
                        width: 0.0,
                        height: t.line_height,
                    },
                    rtl: layout.is_rtl(),
                    lengths: Vec::new(),
                    positions: Vec::new(),
                    widths: Vec::new(),
                    word_starts: Vec::new(),
                });
            }
            // The hard line break after the piece ends its last run.
            if piece.end < content.len()
                && let Some(last) = out.last_mut()
            {
                last.range.end = piece.end.saturating_add(1);
                last.positions.push(last.bounds.width);
                last.widths.push(0.0);
                last.lengths.push(1);
            }
        }
        if out.is_empty() {
            out.push(TextRun {
                range: 0..0,
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: t.line_height,
                },
                rtl: false,
                lengths: Vec::new(),
                positions: Vec::new(),
                widths: Vec::new(),
                word_starts: Vec::new(),
            });
        }
        out
    }
}
