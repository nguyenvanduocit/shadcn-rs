//! GPU time by kind of pass, from timestamp queries (ADR-0018, devtools).
//!
//! Every pass of a frame brackets itself with a pair of queries tagged by
//! what it is ([`Stamped`]). Two query sets take turns: a set is resolved
//! only once the GPU has finished the frame that wrote it (Metal samples a
//! render pass's end when its fragments finish, which a resolve recorded in
//! the same command buffer may overtake), then read back asynchronously.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::scene::GpuPasses;

/// Timestamp queries a frame may write (two a pass).
const QUERIES: u32 = 256;

/// What a pair of timestamps brackets: a kind of pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stamped {
    /// The path rasterizer's compute passes.
    Paths,
    /// A data chunk's compute pass.
    Compute,
    /// A data chunk's coverage pass.
    Coverage,
    /// A pass of the frame: the canvas, a layer, a tile, a mask, the blit.
    Main,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Free to be written by the frame being built.
    Free,
    /// Written by a frame not yet submitted.
    Written,
    /// Submitted; `done` says when the GPU finished it.
    Waiting,
    /// Resolved into its read buffer in the frame being built.
    Resolving,
    /// Being mapped to be read.
    Mapping,
}

struct Set {
    query: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    state: State,
    queries: u32,
    tags: Vec<Stamped>,
    done: Arc<AtomicBool>,
    mapped: Arc<AtomicBool>,
}

impl Set {
    fn new(device: &wgpu::Device) -> Self {
        let bytes = u64::from(QUERIES).saturating_mul(8);
        Self {
            query: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gui timing"),
                ty: wgpu::QueryType::Timestamp,
                count: QUERIES,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gui timing"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gui timing read"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            state: State::Free,
            queries: 0,
            tags: Vec::new(),
            done: Arc::new(AtomicBool::new(false)),
            mapped: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// The frame's timestamp queries and the last times read back.
pub(crate) struct Timing {
    sets: [Set; 2],
    /// The set the frame being built writes.
    cur: usize,
    period: f32,
    /// The GPU time of the last frame read back: its first stamp to its
    /// last, and each kind of pass's share.
    pub(crate) frame: Option<Duration>,
    pub(crate) passes: Option<GpuPasses>,
}

impl Timing {
    /// Timing, when the adapter has timestamp queries.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| Self {
                sets: [Set::new(device), Set::new(device)],
                cur: 0,
                period: queue.get_timestamp_period(),
                frame: None,
                passes: None,
            })
    }

    /// A pair of queries for a pass of kind `tag` to write its start and end
    /// into ([`Self::query_set`]), when the set is free and has room.
    pub(crate) fn stamp(&mut self, tag: Stamped) -> Option<u32> {
        let set = self.sets.get_mut(self.cur)?;
        if set.state != State::Free || set.queries.saturating_add(2) > QUERIES {
            return None;
        }
        let at = set.queries;
        set.queries = at.saturating_add(2);
        set.tags.push(tag);
        Some(at)
    }

    /// The set [`Self::stamp`]'s queries are in.
    pub(crate) fn query_set(&self) -> Option<&wgpu::QuerySet> {
        self.sets.get(self.cur).map(|s| &s.query)
    }

    /// At the end of the frame's commands: resolve a set whose frame the GPU
    /// finished, and close the set this frame wrote.
    pub(crate) fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        for set in &mut self.sets {
            if set.state == State::Waiting && set.done.load(Ordering::Acquire) {
                let n = set.queries;
                encoder.resolve_query_set(&set.query, 0..n, &set.resolve, 0);
                encoder.copy_buffer_to_buffer(&set.resolve, 0, &set.read, 0, u64::from(n).saturating_mul(8));
                set.state = State::Resolving;
            }
        }
        let cur = self.cur;
        if let Some(set) = self.sets.get_mut(cur)
            && set.state == State::Free
            && set.queries > 0
        {
            set.state = State::Written;
            self.cur = 1_usize.saturating_sub(cur);
        }
    }

    /// After the frame's submit: learn when its work is done, and map what
    /// was resolved.
    pub(crate) fn after_submit(&mut self, queue: &wgpu::Queue) {
        for set in &mut self.sets {
            match set.state {
                State::Written => {
                    set.done.store(false, Ordering::Release);
                    let done = Arc::clone(&set.done);
                    queue.on_submitted_work_done(move || done.store(true, Ordering::Release));
                    set.state = State::Waiting;
                }
                State::Resolving => {
                    let mapped = Arc::clone(&set.mapped);
                    set.read.slice(0..u64::from(set.queries).saturating_mul(8)).map_async(
                        wgpu::MapMode::Read,
                        move |r| {
                            if r.is_ok() {
                                mapped.store(true, Ordering::Release);
                            }
                        },
                    );
                    set.state = State::Mapping;
                }
                State::Free | State::Waiting | State::Mapping => {}
            }
        }
    }

    /// Before a frame: read the times that came back.
    pub(crate) fn read(&mut self, device: &wgpu::Device) {
        if self
            .sets
            .iter()
            .all(|s| matches!(s.state, State::Free | State::Written))
        {
            return;
        }
        device.poll(wgpu::PollType::Poll).ok();
        let period = f64::from(self.period);
        let time = |ticks: u64| {
            let ns = crate::convert::u64_to_f64(ticks) * period;
            Duration::from_nanos(crate::convert::f64_to_u64(ns))
        };
        for set in &mut self.sets {
            if set.state != State::Mapping || !set.mapped.load(Ordering::Acquire) {
                continue;
            }
            let bytes = u64::from(set.queries).saturating_mul(8);
            let measured = set
                .read
                .slice(0..bytes)
                .get_mapped_range()
                .ok()
                .map(|view| sum(bytemuck::cast_slice(&view), &set.tags));
            set.read.unmap();
            set.mapped.store(false, Ordering::Release);
            set.state = State::Free;
            set.queries = 0;
            set.tags.clear();
            if let Some((span, by)) = measured {
                self.frame = Some(time(span));
                self.passes = Some(GpuPasses {
                    paths: time(by[0]),
                    compute: time(by[1]),
                    coverage: time(by[2]),
                    main: time(by[3]),
                });
            }
        }
    }
}

/// A frame's span (its first stamp to its last) and each kind's sum, from
/// `stamps` in pairs tagged by `tags`; a pair a pass did not write is left out.
fn sum(stamps: &[u64], tags: &[Stamped]) -> (u64, [u64; 4]) {
    let mut by = [0_u64; 4];
    let (mut first, mut last) = (u64::MAX, 0_u64);
    for (p, tag) in stamps.as_chunks::<2>().0.iter().zip(tags) {
        let [a, b] = *p;
        if a == 0 || b < a {
            continue;
        }
        first = first.min(a);
        last = last.max(b);
        let i = match tag {
            Stamped::Paths => 0,
            Stamped::Compute => 1,
            Stamped::Coverage => 2,
            Stamped::Main => 3,
        };
        if let Some(v) = by.get_mut(i) {
            *v = v.saturating_add(b.saturating_sub(a));
        }
    }
    (last.saturating_sub(first.min(last)), by)
}

#[cfg(test)]
mod tests {
    use super::{Stamped, sum};

    #[test]
    fn a_pass_that_did_not_write_its_stamps_is_left_out() {
        let tags = [Stamped::Compute, Stamped::Coverage, Stamped::Main, Stamped::Main];
        // The last pass's end was not written (older than its start).
        let stamps = [100, 150, 150, 170, 175, 300, 310, 90];
        let (span, by) = sum(&stamps, &tags);
        assert_eq!(span, 200);
        assert_eq!(by, [0, 50, 20, 125]);
    }
}
