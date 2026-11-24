use std::cell::Cell;

use crate::geometry::Bounds;

/// One frame node a pass wrote in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Changed {
    pub(crate) index: u32,
    /// Its box before.
    pub(crate) before: Bounds,
    /// Its look or content changed.
    pub(crate) repainted: bool,
}

/// What the passes changed in a [`Frame`](super::Frame), for its readers (paint,
/// accessibility, the frame report's diff), each of which keeps where it read up to
/// ([`ReadAt`]): the nodes written in place, in order, since the frame was
/// last rebuilt (nodes came, went or moved, and every index may be another).
#[derive(Clone, Debug, Default)]
pub(crate) struct Changes {
    /// Which rebuild of which frame it describes, unique in the thread: a
    /// new one at each rebuild. 0 for a frame never built.
    pub(super) epoch: u64,
    pub(crate) log: Vec<Changed>,
    /// How many entries were logged before `log`'s first since the rebuild,
    /// and dropped: a reader that had not read them reads everything.
    pub(super) dropped: usize,
    /// Where the reader that must see every change (the frame report's
    /// diff, read every frame) read up
    /// to, since the rebuild: `None` until it first reads.
    unread_from: Option<usize>,
    /// The entries from `unread_from` on that a trim or a rebuild dropped
    /// before that reader read them, one per node (its box before the
    /// first, whether any repainted it), so it never falls behind. Each
    /// with its place when last sorted, which keeps a node's first first.
    unread: Vec<(usize, Changed)>,
}

/// Where a reader of a frame's [`Changes`] read up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReadAt {
    epoch: u64,
    /// How many entries since the rebuild it read.
    pub(super) at: usize,
    /// It has read a frame.
    read: bool,
}

/// What a reader of a frame's [`Changes`] reads since it last read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Since<'a> {
    /// The nodes written in place since, in order.
    Logged(&'a [Changed]),
    /// The frame was rebuilt since (or never read): what the reader held of
    /// it by index means nothing, and it reads the whole frame.
    Rebuilt,
    /// The same frame, but a trim dropped entries the reader had not read:
    /// it reads the whole frame.
    FellBehind,
}

impl Changes {
    /// The frame was rebuilt from node `start` on: what readers held of it
    /// by index means nothing. The nodes before `start` are as they were, so
    /// what was written into them is kept for the reader that must see
    /// every change; what was written into the others is in the nodes kept
    /// as they were ([`Frame::before`](super::Frame::before), undone there by the rebuild).
    pub(crate) fn rebuilt(&mut self, start: usize) {
        crate::engine::engine_local! {
            [Fork] static EPOCHS: Cell<u64> = const { Cell::new(0) };
        }
        self.epoch = EPOCHS.with(|e| {
            let next = e.get().wrapping_add(1).max(1);
            e.set(next);
            next
        });
        let end = self.end();
        self.keep_unread(end, |c| usize::try_from(c.index).is_ok_and(|i| i < start));
        self.log.clear();
        self.dropped = 0;
        if self.unread_from.is_some() {
            self.unread_from = Some(0);
        }
    }

    /// How many entries were logged since the rebuild, dropped or not: a
    /// count that only grows within an epoch, so `(epoch, end)` names one
    /// state of the frame (the log's length alone repeats after a trim).
    pub(crate) fn end(&self) -> usize {
        self.dropped.saturating_add(self.log.len())
    }

    /// Keep the log no longer than the frame it describes, twice over: past
    /// that its older half is dropped. A reader that had not read what was
    /// dropped reads the whole frame ([`Since::FellBehind`]): one that reads
    /// each frame does only if one frame's passes logged that much (paint
    /// then records the frame anew). It holds room for that many at once,
    /// so logging a change (a hover's repaint) allocates nothing while the
    /// frame keeps its size.
    pub(crate) fn trim(&mut self, nodes: usize) {
        let most = nodes.saturating_mul(2).saturating_add(64);
        if self.log.len() > most {
            let cut = self.log.len().saturating_sub(most / 2);
            let dropped = self.dropped.saturating_add(cut);
            self.keep_unread(dropped, |_| true);
            self.log.drain(..cut);
            self.dropped = dropped;
        }
        if self.log.capacity() < most {
            self.log
                .reserve_exact(most.saturating_add(1).saturating_sub(self.log.len()));
        }
    }

    /// Fold into `unread` the entries before `upto` (counted since the
    /// rebuild) that the reader of every change had not read and `keep`
    /// holds, one per node.
    fn keep_unread(&mut self, upto: usize, keep: impl Fn(&Changed) -> bool) {
        let Some(from) = self.unread_from else {
            return;
        };
        let lo = from.saturating_sub(self.dropped).min(self.log.len());
        let hi = upto.saturating_sub(self.dropped).min(self.log.len());
        let Some(fresh) = self.log.get(lo..hi).filter(|f| !f.is_empty()) else {
            self.unread.retain(|(_, c)| keep(c));
            return;
        };
        self.unread.extend(fresh.iter().map(|&c| (0, c)));
        self.unread.retain(|(_, c)| keep(c));
        // By node, each node's in the order written (its place breaks the
        // tie, so a sort that allocates nothing keeps that order): its first
        // holds its box before.
        for (k, (at, _)) in self.unread.iter_mut().enumerate() {
            *at = k;
        }
        self.unread.sort_unstable_by_key(|&(at, c)| (c.index, at));
        self.unread.dedup_by(|(_, later), (_, first)| {
            let same = later.index == first.index;
            if same {
                first.repainted |= later.repainted;
            }
            same
        });
    }

    /// The writes into nodes in `range` the reader of every change has not
    /// read, latest first (a node's first write comes last), each with its
    /// node's offset from the range's start. Nothing before it first read.
    pub(super) fn unread_back(&self, range: std::ops::Range<usize>, mut f: impl FnMut(usize, &Changed)) {
        let Some(from) = self.unread_from else {
            return;
        };
        let from = from.saturating_sub(self.dropped).min(self.log.len());
        let log = self.log.get(from..).unwrap_or_default();
        for c in log.iter().rev().chain(self.unread.iter().rev().map(|(_, c)| c)) {
            if let Some(i) = usize::try_from(c.index).ok().filter(|i| range.contains(i)) {
                f(i.saturating_sub(range.start), c);
            }
        }
    }

    /// What changed since `cursor`, moving it to now.
    pub(crate) fn since(&self, cursor: &mut ReadAt) -> Since<'_> {
        let end = self.end();
        let same = cursor.read && cursor.epoch == self.epoch && cursor.at <= end;
        let from = cursor.at.checked_sub(self.dropped);
        *cursor = ReadAt {
            epoch: self.epoch,
            at: end,
            read: true,
        };
        if !same {
            return Since::Rebuilt;
        }
        match from.and_then(|from| self.log.get(from..)) {
            Some(log) => Since::Logged(log),
            None => Since::FellBehind,
        }
    }

    /// For the reader that must see every change (the frame report's diff):
    /// what was
    /// written in place since it last read, into `into`, in order (what a
    /// trim or a rebuild dropped first, folded by node). False the first
    /// time it reads this frame, which has no earlier state to differ
    /// from. A rebuild since keeps only what was written into the nodes
    /// before those it rebuilt (the rebuilt ones it compares as they were).
    pub(crate) fn unread(&mut self, into: &mut Vec<Changed>) -> bool {
        into.clear();
        let end = self.end();
        let Some(from) = self.unread_from.replace(end) else {
            self.unread.clear();
            return false;
        };
        into.extend(self.unread.drain(..).map(|(_, c)| c));
        let from = from.saturating_sub(self.dropped).min(self.log.len());
        into.extend(self.log.get(from..).unwrap_or_default().iter().copied());
        true
    }
}
