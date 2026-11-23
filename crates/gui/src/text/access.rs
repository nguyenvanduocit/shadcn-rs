//! Laid out text as a screen reader moves through it (ADR-0021): each line
//! cut into runs of one direction, each run's characters (grapheme
//! clusters, the smallest unit a caret steps over) with their byte lengths,
//! where they sit along the line and which of them start a word. AccessKit's
//! `TextRun` nodes are built from these, as Parley's editor example builds
//! them from a `PlainEditor` (`references/parley/examples/editor/src/accessibility.rs`).

use std::ops::Range;

use crate::geometry::Bounds;
use crate::style::{Direction, ResolvedText};

use super::{Keep, Shaped, TextSystem, placed_units};

/// The most characters one run holds: AccessKit counts a run's positions
/// (its `word_starts`, a position at its end) in a `u8`, so its end, one
/// past its last character, must be one too.
const MAX_CHARACTERS: usize = 255;

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
    /// The visual line it is on, counted from the first: runs sharing one
    /// are linked on it (AccessKit's previous and next on line). What a
    /// cut hides of the cut line's hard line is on the cut line, by its
    /// ellipsis; each hard line it hides past that is a line of its own.
    pub line: usize,
    /// It shows nothing: what a cut hides, read zero wide at the ellipsis.
    pub hidden: bool,
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

/// The byte lengths a character `len` bytes long is told in: AccessKit
/// counts a character's bytes in a `u8`, so one longer (a letter under a
/// hundred marks) is told as pieces of at most 255 bytes, cut between
/// `text`'s code points (every 255 bytes, its text not known); the first
/// takes its width, the rest none, at its end, and offsets past it stay
/// right. Itself when at most 255; an empty one is one empty piece.
fn character_pieces(text: Option<&str>, len: usize) -> impl Iterator<Item = u8> {
    const MAX: usize = 255;
    let text = text.filter(|t| t.len() == len);
    let (mut at, mut done) = (0_usize, false);
    std::iter::from_fn(move || {
        if done {
            return None;
        }
        let rest = len.saturating_sub(at);
        if rest <= MAX {
            done = true;
            return Some(u8::try_from(rest).unwrap_or(u8::MAX));
        }
        let mut cut = at.saturating_add(MAX);
        while text.is_some_and(|t| !t.is_char_boundary(cut)) && cut > at {
            cut = cut.saturating_sub(1);
        }
        let piece = cut.saturating_sub(at);
        at = cut;
        Some(u8::try_from(piece).unwrap_or(u8::MAX))
    })
}

/// Where the lines of a [`TextSystem::text_runs`] come from: a shaped
/// piece (a paragraph's, or the prefix a clamp keeps, laid out alone), the
/// paragraph byte its text starts at (after its lead), its x shift in the
/// box, and its line's top and bottom there.
struct Source<'a> {
    s: &'a Shaped,
    start: usize,
    shift: f32,
    y: (f32, f32),
}

/// A unit a truncated line keeps, as drawn: its bytes, x in the box and
/// width shown, and x and advance as laid out.
struct Drawn {
    range: (usize, usize),
    x: f32,
    shown: f32,
    laid: f32,
    advance: f32,
}

/// Where what a cut hides is read: on the cut line (`y`, its top and
/// bottom), each character zero wide at `at`, where the kept text ends by
/// the ellipsis. The last such run alone has a box, over the ellipsis
/// (`x`, its left and right; in RTL from the text box's left, where the
/// ellipsis is): a screen reader's point query there hits it, past its
/// characters, the text's end, as the pointer's hit finds there
/// ([`TextSystem::hit`]). Without a box there, AccessKit reads a point
/// left of an RTL line's first run as the text's start.
#[derive(Clone, Copy)]
struct Hidden {
    x: (f32, f32),
    y: (f32, f32),
    at: f32,
}

impl Hidden {
    /// A run that shows nothing: zero wide at `at`, its characters there.
    fn place(&self, p: &mut TextRun) {
        p.hidden = true;
        p.bounds = Bounds {
            x: self.at,
            y: self.y.0,
            width: 0.0,
            height: self.y.1 - self.y.0,
        };
        p.positions.fill(0.0);
    }

    /// The one hidden run with a box: over the ellipsis, its characters at `at`.
    fn over_ellipsis(&self, p: &mut TextRun) {
        p.bounds.x = self.x.0;
        p.bounds.width = self.x.1 - self.x.0;
        let from_start = if p.rtl { self.x.1 - self.at } else { self.at - self.x.0 };
        p.positions.fill(from_start);
    }
}

/// What a clamp hides past what it breaks into lines, read whole all the
/// same, of one text: its grapheme clusters and which start a word, cut
/// by whole hard lines (a word's start known in its line, never in a
/// fresh string starting mid-word, where a clamp stopped breaking). Read
/// again at another width, or with more or fewer lines shown, nothing is
/// cut again (only hard lines before those it has). [`TextSystem`] keeps
/// one per text read, as many as its working set needs
/// ([`TextSystem::bound_hidden`]).
#[derive(Default)]
pub(super) struct HiddenLines {
    /// Each cluster's first byte in the text, from the first hard line it
    /// has on, and the text's end after the last.
    starts: Vec<usize>,
    /// Whether each starts a word (that is not whitespace).
    words: Vec<bool>,
    /// Each hard line it has: its first byte and its first cluster.
    lines: Vec<(usize, usize)>,
    /// The frame it was last read in.
    pub(super) used: u64,
}

/// What [`HiddenLines`] are kept by: a text and its direction.
pub(super) fn hidden_key(content: &str, rtl: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut h);
    rtl.hash(&mut h);
    h.finish()
}

impl HiddenLines {
    /// Have the hard lines from the one holding byte `from` on cut, the
    /// hard lines it cuts counted in `segmented`.
    fn cover(&mut self, content: &str, from: usize, segmented: &mut u64) {
        let start = content
            .get(..from)
            .and_then(|b| b.rfind('\n'))
            .map_or(0, |n| n.saturating_add(1));
        let have = self.lines.first().map(|l| l.0);
        if have.is_some_and(|h| h <= start) {
            return;
        }
        // Cut from `start` up to the lines it has (or the end), and put
        // them before those.
        let stop = have.unwrap_or(usize::MAX);
        let (mut starts, mut words, mut lines) = (Vec::new(), Vec::new(), Vec::new());
        // Its clusters and words as Parley's analysis has them (its
        // dictionary-aware word boundaries: Thai, Lao, Khmer, Burmese,
        // Chinese, Japanese), so what is hidden starts the words what is
        // shown would: the analysis shaping runs on each hard line. A hard
        // line break is a boundary of both, so the lines are analysed at once.
        let region = content.get(start..stop.min(content.len())).unwrap_or("");
        let mut analysis = parley_engine::Analysis::new();
        parley_engine::Analyzer::new().analyze(
            region,
            &parley_engine::AnalysisOptions::default(),
            &mut analysis,
            &parley_engine::AnalysisDataSources::new(None),
        );
        let mut chars = region.char_indices().zip(analysis.char_info()).peekable();
        let mut at = start;
        loop {
            let end = super::piece_end(content, at);
            lines.push((at, starts.len()));
            let rel = |b: usize| b.saturating_sub(start);
            while let Some(((i, c), info)) = chars.next_if(|((i, _), _)| *i < rel(end)) {
                if i < rel(at) {
                    continue;
                }
                if i == rel(at) || info.is_grapheme_start() {
                    starts.push(start.saturating_add(i));
                    // A word starts at a boundary, not at a space (as a
                    // shaped cluster's does, `TextSystem::text_runs`).
                    words.push(info.is_word_boundary() && !matches!(c, ' ' | '\u{a0}'));
                }
            }
            *segmented = segmented.saturating_add(1);
            if end >= content.len() {
                starts.push(content.len());
                break;
            }
            at = end.saturating_add(1);
            if at >= stop {
                break;
            }
        }
        if have.is_some() {
            let shift = starts.len();
            lines.extend(self.lines.iter().map(|&(b, c)| (b, c.saturating_add(shift))));
            starts.append(&mut self.starts);
            words.append(&mut self.words);
        }
        (self.starts, self.words, self.lines) = (starts, words, lines);
    }

    /// The runs of `content` from byte `from` on (its clusters there, each
    /// hard line's ending in its line break), hidden; each `line` its hard
    /// line counted from `from`'s.
    fn runs(&mut self, content: &str, from: usize, rtl: bool, segmented: &mut u64, out: &mut Vec<TextRun>) {
        self.cover(content, from, segmented);
        let first = self.lines.partition_point(|l| l.0 <= from).saturating_sub(1);
        let end = self.starts.len().saturating_sub(1);
        for (rel, li) in (first..self.lines.len()).enumerate() {
            let Some(&(line_start, c0)) = self.lines.get(li) else {
                break;
            };
            let c1 = self.lines.get(li.saturating_add(1)).map_or(end, |l| l.1);
            // Its clusters from `from` (in its first line) on: the one
            // holding it, from it.
            let mut c = c0;
            if line_start < from {
                let past = self.starts.get(c0..c1).map_or(0, |s| s.partition_point(|&b| b <= from));
                c = c0.saturating_add(past.saturating_sub(1));
            }
            let empty = |at: usize| TextRun {
                range: at..at,
                bounds: Bounds::default(),
                rtl,
                lengths: Vec::new(),
                positions: Vec::new(),
                widths: Vec::new(),
                word_starts: Vec::new(),
                line: rel,
                hidden: true,
            };
            let mut run = empty(line_start.max(from));
            while c < c1 {
                let (Some(&b), Some(&e)) = (self.starts.get(c), self.starts.get(c.saturating_add(1))) else {
                    break;
                };
                // A cluster's end is the next's start, or its line's break.
                let e = if c.saturating_add(1) == c1 {
                    super::piece_end(content, b)
                } else {
                    e
                };
                let (b, whole) = (b.max(from), b >= from);
                let word = whole && self.words.get(c).copied().unwrap_or(false);
                // Each entry it tells (a piece of a long grapheme too)
                // counts toward its run's limit: a full run gives way to
                // the next, even inside a grapheme.
                for (k, piece) in character_pieces(content.get(b..e), e.saturating_sub(b)).enumerate() {
                    if run.lengths.len() >= MAX_CHARACTERS {
                        let next = empty(run.range.end);
                        out.push(std::mem::replace(&mut run, next));
                    }
                    if k == 0 && word {
                        run.word_starts.push(u8::try_from(run.lengths.len()).unwrap_or(u8::MAX));
                    }
                    run.lengths.push(piece);
                    run.positions.push(0.0);
                    run.widths.push(0.0);
                    run.range.end = run.range.end.saturating_add(usize::from(piece));
                }
                c = c.saturating_add(1);
            }
            out.push(run);
            let line_end = super::piece_end(content, line_start);
            if line_end < content.len() {
                line_break(out, line_end);
            }
        }
    }
}

/// Where a character shows: its box's x and width (its padding a cut takes
/// out, out), given its bytes in its layout, x as laid out and advance;
/// none when a cut hides it.
type Place<'a> = &'a dyn Fn((usize, usize), f32, f32) -> Option<(f32, f32)>;

/// The runs of `line` of `src`, its characters before layout byte `skip`
/// left out, each placed by `place`: a run it shows none of is `hidden`.
fn line_runs(
    out: &mut Vec<TextRun>,
    src: &Source<'_>,
    line: &parley::Line<'_, u32>,
    skip: usize,
    place: Place<'_>,
    hidden: Hidden,
) {
    let lead = src.s.key.lead;
    let m = line.metrics();
    // Runs in visual order give each its offset; then in reading order.
    let mut runs = Vec::new();
    let mut x = m.offset;
    for run in line.runs() {
        let advance = run.advance();
        runs.push((run, x));
        x += advance;
    }
    runs.sort_by_key(|(r, _)| r.text_range().start);
    for (run, x) in runs {
        let rtl = run.is_rtl();
        let run_advance = run.advance();
        // A part's box: over the characters it shows, which sit side by
        // side; at the ellipsis when it shows none.
        let mut part: Option<(TextRun, Option<f32>)> = None;
        let close = |(mut p, left): (TextRun, Option<f32>)| {
            match left {
                Some(left) => {
                    p.bounds.x = left;
                    p.bounds.y = src.y.0;
                    p.bounds.height = src.y.1 - src.y.0;
                }
                None => hidden.place(&mut p),
            }
            p
        };
        let mut along = 0.0_f32;
        for cluster in run.clusters() {
            let r = cluster.text_range();
            let advance = cluster.advance();
            // Where it is laid out (from the run's start edge).
            let laid = if rtl {
                x + run_advance - along - advance
            } else {
                x + along
            };
            along += advance;
            if r.end <= lead || r.end <= skip {
                continue;
            }
            let from = src.start.saturating_add(r.start.saturating_sub(lead));
            let to = src.start.saturating_add(r.end.saturating_sub(lead));
            let shown = place((r.start, r.end), laid + src.shift, advance);
            // A part is all shown or all hidden: a run the cut ends (in
            // RTL, as in LTR) splits where what it hides starts, its hidden
            // part read at the ellipsis. Each entry it tells (a piece of a
            // long grapheme too) counts toward the run's limit: a full
            // part gives way to the next, even inside a grapheme.
            let word = cluster.is_word_boundary() && !cluster.is_space_or_nbsp();
            let text = src.s.content.get(r.start.max(lead)..r.end);
            let mut at = from;
            for (k, piece) in character_pieces(text, to.saturating_sub(from)).enumerate() {
                if part.as_ref().is_some_and(|p| {
                    p.0.lengths.len() >= MAX_CHARACTERS || (k == 0 && p.1.is_some() != shown.is_some())
                }) {
                    out.extend(part.take().map(close));
                }
                let (p, left) = part.get_or_insert_with(|| {
                    (
                        TextRun {
                            range: at..at,
                            bounds: Bounds::default(),
                            rtl,
                            lengths: Vec::new(),
                            positions: Vec::new(),
                            widths: Vec::new(),
                            word_starts: Vec::new(),
                            line: 0,
                            hidden: false,
                        },
                        None,
                    )
                });
                // The first piece is the character's box; the rest sit at
                // its logical end, zero wide: its right, or in RTL its left.
                let width = shown.map_or(0.0, |(x, w)| {
                    let (x, w) = match (k, rtl) {
                        (0, _) => (x, w),
                        (_, true) => (x, 0.0),
                        (_, false) => (x + w, 0.0),
                    };
                    *left = Some(left.map_or(x, |l: f32| l.min(x)));
                    w
                });
                if k == 0 && word {
                    p.word_starts.push(u8::try_from(p.lengths.len()).unwrap_or(u8::MAX));
                }
                p.positions.push(p.bounds.width);
                p.widths.push(width);
                p.lengths.push(piece);
                p.bounds.width += width;
                at = at.saturating_add(usize::from(piece));
                p.range.end = at;
            }
        }
        out.extend(part.map(close));
    }
}

/// The hard line break at byte `at` ends the last run of `out` (AccessKit's
/// contract: the last run of a line holds it); a run of its own after that
/// one, on its line at its end, when that one is full.
fn line_break(out: &mut Vec<TextRun>, at: usize) {
    let Some(last) = out.last_mut() else { return };
    if last.lengths.len() >= MAX_CHARACTERS {
        let end = if last.rtl {
            last.bounds.x
        } else {
            last.bounds.x + last.bounds.width
        };
        let run = TextRun {
            range: last.range.end..last.range.end,
            bounds: Bounds {
                x: end,
                width: 0.0,
                ..last.bounds
            },
            rtl: last.rtl,
            lengths: Vec::new(),
            positions: Vec::new(),
            widths: Vec::new(),
            word_starts: Vec::new(),
            line: last.line,
            hidden: last.hidden,
        };
        out.push(run);
    }
    if let Some(last) = out.last_mut() {
        last.range.end = at.saturating_add(1);
        last.positions.push(last.bounds.width);
        last.widths.push(0.0);
        last.lengths.push(1);
    }
}

impl TextSystem {
    /// `content` laid out at `width` in style `t`, as runs in reading order:
    /// what a text field tells a screen reader about its text. Empty text
    /// has one empty run, for the caret to sit in. Cut short (an ellipsis,
    /// a clamp), they follow what shows: the kept text where it is drawn,
    /// what is hidden there too (the whole value stays readable) but zero
    /// wide at the ellipsis, where the caret stands at a hidden offset.
    pub fn text_runs(&mut self, content: &str, t: &ResolvedText, width: Option<f32>) -> Vec<TextRun> {
        let p = super::Paragraph::from(content);
        let laid = self.laid_out(p, t, width);
        let cut = self.cut(p, &laid, t, width);
        // Where what a cut hides sits: zero wide at the ellipsis, on the cut line.
        let hidden = cut.and_then(|c| {
            let (piece, top) = laid.get(c.piece)?;
            let s = self.shaped.get(&piece.hash)?;
            let m = s.layout.get(c.line)?.metrics();
            let right = c.ellipsis_at + c.ellipsis;
            let (x, at) = if s.key.rtl {
                ((c.ellipsis_at.min(0.0), right), right)
            } else {
                ((c.ellipsis_at, right), c.ellipsis_at)
            };
            Some(Hidden {
                x,
                y: (top + m.block_min_coord, top + m.block_max_coord),
                at,
            })
        });
        let hidden = hidden.unwrap_or(Hidden {
            x: (0.0, 0.0),
            y: (0.0, t.line_height),
            at: 0.0,
        });
        // A truncated line's units as drawn: bytes, box x, width shown,
        // and x and advance as laid out.
        let mut drawn: Vec<Drawn> = Vec::new();
        if let Some(c) = cut.filter(|c| c.prefix.is_none())
            && let Some(s) = laid.get(c.piece).and_then(|(piece, _)| self.shaped.get(&piece.hash))
        {
            placed_units(s, c, &self.trims, |bx, shown, x, advance, range| {
                drawn.push(Drawn {
                    range,
                    x: bx,
                    shown,
                    laid: x,
                    advance,
                });
            });
        }
        // Drawn in visual order; read in reading order: the clusters of
        // the cut line come by their bytes (its runs in reading order, each
        // run's clusters logically), so one pointer over the units by
        // bytes meets each cluster's unit, never stepping back.
        drawn.sort_unstable_by_key(|u| u.range);
        let next = std::cell::Cell::new(0_usize);
        let shown: Place<'_> = &|_, x, advance| Some((x, advance));
        let none: Place<'_> = &|_, _, _| None;
        let truncated: Place<'_> = &|r: (usize, usize), x, advance| {
            let mut i = next.get();
            // A unit ending before this cluster does holds no later one.
            while drawn.get(i).is_some_and(|u| {
                #[cfg(test)]
                super::tests::UNIT_LOOKS.with(|n| n.set(n.get().saturating_add(1)));
                u.range.1 < r.1
            }) {
                i = i.saturating_add(1);
            }
            next.set(i);
            let u = drawn.get(i).filter(|u| u.range.0 <= r.0 && r.1 <= u.range.1)?;
            let trim = if r.1 == u.range.1 { u.advance - u.shown } else { 0.0 };
            Some((u.x + (x - u.laid), (advance - trim).max(0.0)))
        };
        let mut out: Vec<TextRun> = Vec::new();
        // The visual line each line is, and the cut line's (where what
        // the cut hides of its hard line is read; each hard line after it
        // is a line of its own).
        let (mut next_line, mut cut_line) = (0_usize, None);
        let line_of = |next: &mut usize, cut: Option<usize>, soft_hidden: bool| match cut {
            Some(c) if soft_hidden => c,
            _ => {
                let n = *next;
                *next = next.saturating_add(1);
                n
            }
        };
        for (pi, (piece, top)) in laid.iter().enumerate() {
            let first_of_piece = out.len();
            let Some(s) = self.shaped.get(&piece.hash) else {
                continue;
            };
            let piece_hidden = cut.is_some_and(|c| pi > c.piece);
            let layout = &s.layout;
            // A hard line the cut hides: one line, however it wraps.
            let piece_line = piece_hidden.then(|| line_of(&mut next_line, None, false));
            for (li, line) in layout.lines().enumerate() {
                let m = line.metrics();
                let y = (top + m.block_min_coord, top + m.block_max_coord);
                let line_start = out.len();
                let src = Source {
                    s,
                    start: piece.start,
                    shift: s.shift,
                    y,
                };
                match cut.filter(|c| c.piece == pi && li >= c.line) {
                    _ if piece_hidden => line_runs(&mut out, &src, &line, 0, none, hidden),
                    Some(c) if li > c.line => line_runs(&mut out, &src, &line, 0, none, hidden),
                    Some(c) => match (c.prefix, c.keep) {
                        (Some(k), Keep::Logical { end }) => {
                            // The kept prefix laid out alone, then the rest
                            // of the line, hidden.
                            if let Some(ks) = self.shaped.get(&k.hash)
                                && let Some(kl) = ks.layout.get(0)
                            {
                                let kept = Source {
                                    s: ks,
                                    start: k.start,
                                    shift: c.dx,
                                    y,
                                };
                                line_runs(&mut out, &kept, &kl, 0, shown, hidden);
                            }
                            line_runs(&mut out, &src, &line, end, none, hidden);
                        }
                        _ => {
                            let src = Source { shift: 0.0, ..src };
                            line_runs(&mut out, &src, &line, 0, truncated, hidden);
                        }
                    },
                    None => line_runs(&mut out, &src, &line, 0, shown, hidden),
                }
                let soft_hidden = cut.is_some_and(|c| c.piece == pi && li > c.line);
                let line_hidden = piece_hidden || soft_hidden;
                let number = piece_line.unwrap_or_else(|| line_of(&mut next_line, cut_line, soft_hidden));
                if cut.is_some_and(|c| c.piece == pi && li == c.line) {
                    cut_line = Some(number);
                }
                if out.len() == line_start {
                    // A line with no text (an empty piece): a run for the caret.
                    let mut run = TextRun {
                        range: piece.start..piece.start,
                        bounds: Bounds {
                            x: m.offset + s.shift,
                            y: y.0,
                            width: 0.0,
                            height: y.1 - y.0,
                        },
                        rtl: layout.is_rtl(),
                        lengths: Vec::new(),
                        positions: Vec::new(),
                        widths: Vec::new(),
                        word_starts: Vec::new(),
                        line: 0,
                        hidden: false,
                    };
                    if line_hidden {
                        hidden.place(&mut run);
                    }
                    out.push(run);
                }
                for r in out.iter_mut().skip(line_start) {
                    r.line = number;
                }
            }
            // A clamp stopped breaking its hard line short of its end
            // ([`TextSystem::lay_out_into`]): the rest, and its line
            // break, are read with the lines it hides, below.
            let partial = self.laid_end(piece) < piece.end;
            if out.len() == first_of_piece {
                let number = piece_line.unwrap_or_else(|| line_of(&mut next_line, None, false));
                let mut run = TextRun {
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
                    line: number,
                    hidden: false,
                };
                if piece_hidden {
                    hidden.place(&mut run);
                }
                out.push(run);
            }
            if piece.end < content.len() && !partial {
                line_break(&mut out, piece.end);
            }
        }
        // What a clamp hides past what it breaks into lines: the rest of
        // the hard line it stopped breaking (on the cut line), and the
        // lines after, never shaped ([`TextSystem::lay_out_into`]): read
        // whole all the same, by their grapheme clusters (cut once per
        // text, by whole hard lines), hidden, each hard line a line.
        if let Some(&(last, _)) = laid.last() {
            let laid_end = self.laid_end(&last);
            let (from, on_cut) = if laid_end < last.end {
                (laid_end, cut_line)
            } else {
                (last.end.saturating_add(1), None)
            };
            if from <= content.len() && (on_cut.is_some() || last.end < content.len()) {
                let rtl = t.direction == Direction::Rtl;
                let lines = self.hidden_lines.entry(hidden_key(content, rtl)).or_default();
                lines.used = self.frame;
                let at = out.len();
                lines.runs(content, from, rtl, &mut self.hidden_segmented, &mut out);
                for r in out.iter_mut().skip(at) {
                    // The rest of the cut line's hard line on it; each
                    // hard line after, the next line.
                    r.line = match on_cut {
                        Some(c) if r.line == 0 => c,
                        Some(_) => next_line.saturating_add(r.line).saturating_sub(1),
                        None => next_line.saturating_add(r.line),
                    };
                    hidden.place(r);
                }
            }
        }
        // Of what is hidden, the last run alone has a box: over the ellipsis.
        if let Some(r) = out.iter_mut().rev().find(|r| r.hidden) {
            hidden.over_ellipsis(r);
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
                line: 0,
                hidden: false,
            });
        }
        out
    }
}
