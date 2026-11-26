//! The readback belt for user chunks (ADR-0018): bytes a kind's compute
//! wrote, copied after its pass into a buffer the CPU can map, mapped
//! once the GPU is done with them, and sent to the UI thread through a
//! [`crate::mailbox`], where they set a status signal. Nothing waits: a
//! readout arrives a frame or two after the compute that wrote it, as
//! Chromium's and wgpu's async readbacks do; a chart's hover never needs
//! it (the CPU keeps its data), a user's GPU-side picking or histogram does.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::reactive::Signal;

/// What a chunk read back: the tag its kind gave the read, and the bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readout {
    pub tag: u32,
    pub bytes: Vec<u8>,
}

/// Where a chunk's readouts go ([`super::DataItem::reading_back`]): the
/// sending half of a [`Readback`], compared by who it is.
#[derive(Clone)]
pub struct ReadbackTo {
    id: u64,
    tx: crate::mailbox::Sender<Readout>,
}

impl PartialEq for ReadbackTo {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl std::fmt::Debug for ReadbackTo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReadbackTo({})", self.id)
    }
}

impl ReadbackTo {
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    fn send(&self, r: Readout) {
        // A closed mailbox (its scope ended) drops what arrives.
        self.tx.send(r);
    }
}

/// The UI thread's end of a chunk's readbacks: the latest readout as a
/// status signal (`None` until the first arrives).
#[derive(Clone)]
pub struct Readback {
    latest: Signal<Option<Arc<Readout>>>,
    to: ReadbackTo,
}

impl Readback {
    /// The latest readout, tracked.
    #[must_use]
    pub fn latest(&self) -> Signal<Option<Arc<Readout>>> {
        self.latest
    }

    /// Where a chunk sends its readouts: give it to [`super::DataItem::reading_back`].
    #[must_use]
    pub fn to(&self) -> ReadbackTo {
        self.to.clone()
    }
}

/// A readback on this (the UI) thread, owned by the scope it is made in:
/// what the GPU reads back for it sets [`Readback::latest`], before the
/// next frame.
#[must_use]
pub fn readback() -> Readback {
    /// The readbacks made, numbered.
    static NEXT: crate::engine::IdKind = crate::engine::IdKind::new("readback");
    let latest = Signal::new(None);
    let tx = crate::mailbox::mailbox(move |r: Readout| latest.set(Some(Arc::new(r))));
    Readback {
        latest,
        to: ReadbackTo {
            id: crate::engine::unique_id(&NEXT),
            tx,
        },
    }
}

/// A read in flight.
struct Flight {
    buffer: wgpu::Buffer,
    size: u64,
    tag: u32,
    to: ReadbackTo,
    /// Copied, mapping, mapped, failed.
    state: Arc<AtomicU8>,
}

const COPIED: u8 = 0;
const MAPPING: u8 = 1;
const MAPPED: u8 = 2;
const FAILED: u8 = 3;

/// `size` rounded up to whole words, as copies and maps move them.
fn padded(size: u64) -> u64 {
    size.div_ceil(wgpu::COPY_BUFFER_ALIGNMENT)
        .saturating_mul(wgpu::COPY_BUFFER_ALIGNMENT)
}

/// Mappable buffers kept for reuse.
const KEPT: usize = 8;

/// The belt: buffers being read, and free ones.
#[derive(Default)]
pub(crate) struct Belt {
    flights: Vec<Flight>,
    free: Vec<wgpu::Buffer>,
}

impl Belt {
    /// Copy `size` bytes of `src` from `offset` for `to`, tagged `tag`,
    /// once the frame's commands before it in `encoder` ran.
    #[expect(clippy::too_many_arguments, reason = "a copy and where it goes")]
    pub(crate) fn copy(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::Buffer,
        offset: u64,
        size: u64,
        tag: u32,
        to: &ReadbackTo,
    ) {
        // Copies move whole words.
        let size = size.min(src.size().saturating_sub(offset));
        if size == 0 || !offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT) {
            return;
        }
        let padded = padded(size);
        if offset.saturating_add(padded) > src.size() {
            return;
        }
        let at = self.free.iter().position(|b| b.size() >= padded);
        let buffer = match at {
            Some(i) => self.free.swap_remove(i),
            None => device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gui chunk readback"),
                size: padded.next_power_of_two().max(256),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        };
        encoder.copy_buffer_to_buffer(src, offset, &buffer, 0, padded);
        self.flights.push(Flight {
            buffer,
            size,
            tag,
            to: to.clone(),
            state: Arc::new(AtomicU8::new(COPIED)),
        });
    }

    /// After the frame's submit: map what it copied.
    pub(crate) fn after_submit(&mut self) {
        for f in &mut self.flights {
            if f.state.load(Ordering::Acquire) == COPIED {
                f.state.store(MAPPING, Ordering::Release);
                let state = Arc::clone(&f.state);
                f.buffer
                    .slice(0..padded(f.size))
                    .map_async(wgpu::MapMode::Read, move |r| {
                        state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
                    });
            }
        }
    }

    /// Whether reads are in flight.
    pub(crate) fn busy(&self) -> bool {
        !self.flights.is_empty()
    }

    /// Send what came back (`wait`: wait for the GPU first, as a test does).
    pub(crate) fn deliver(&mut self, device: &wgpu::Device, wait: bool) {
        if self.flights.iter().all(|f| f.state.load(Ordering::Acquire) == COPIED) {
            return;
        }
        let poll = if wait {
            wgpu::PollType::wait_indefinitely()
        } else {
            wgpu::PollType::Poll
        };
        device.poll(poll).ok();
        let mut i = 0;
        while let Some(f) = self.flights.get(i) {
            match f.state.load(Ordering::Acquire) {
                MAPPED => {
                    let f = self.flights.remove(i);
                    let mut bytes = f
                        .buffer
                        .slice(0..padded(f.size))
                        .get_mapped_range()
                        .map(|v| v.to_vec())
                        .unwrap_or_default();
                    bytes.truncate(usize::try_from(f.size).unwrap_or(usize::MAX));
                    f.buffer.unmap();
                    f.to.send(Readout { tag: f.tag, bytes });
                    if self.free.len() < KEPT {
                        self.free.push(f.buffer);
                    }
                }
                FAILED => {
                    self.flights.remove(i);
                }
                _ => i = i.saturating_add(1),
            }
        }
    }
}
