//! A GPU buffer of `u32` words kept across frames with a copy on the CPU
//! (ADR-0016): handed out in ranges from a free list that merges
//! neighbours, written where something changed, and uploaded as the dirty
//! ranges merged (`Queue::write_buffer` each); when more than about 15% of
//! what is in use changed, or the buffer grew, it is written whole, as
//! Bevy's sparse buffers do (`sparse_buffer_vec.rs`).

/// A range of words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: u32,
    pub len: u32,
}

impl Span {
    pub(crate) fn end(&self) -> u32 {
        self.start.saturating_add(self.len)
    }
}

/// Dirty ranges closer than this many words are written as one.
const JOIN: u32 = 64;

pub(crate) struct Words {
    label: &'static str,
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) generation: u64,
    data: Vec<u32>,
    /// Free spans, sorted, never touching.
    free: Vec<Span>,
    /// Spans written since the last upload.
    dirty: Vec<Span>,
    /// The buffer must be made again, larger, and written whole.
    grown: bool,
    /// Words handed out.
    used: u64,
}

impl Words {
    pub(crate) fn new(device: &wgpu::Device, label: &'static str, capacity: u32) -> Self {
        let capacity = capacity.max(64);
        Self {
            label,
            buffer: make(device, label, capacity),
            generation: 0,
            data: vec![0; usize::try_from(capacity).unwrap_or(64)],
            free: vec![Span {
                start: 0,
                len: capacity,
            }],
            dirty: Vec::new(),
            grown: false,
            used: 0,
        }
    }

    fn capacity(&self) -> u32 {
        u32::try_from(self.data.len()).unwrap_or(u32::MAX)
    }

    /// Bytes the buffer holds.
    pub(crate) fn bytes(&self) -> u64 {
        u64::from(self.capacity()).saturating_mul(4)
    }

    /// Bytes handed out.
    pub(crate) fn used_bytes(&self) -> u64 {
        self.used.saturating_mul(4)
    }

    /// `len` words, the buffer grown (on the next upload) when it has no room.
    pub(crate) fn alloc(&mut self, len: u32) -> Span {
        let len = len.max(1);
        self.used = self.used.saturating_add(u64::from(len));
        if let Some(s) = self.take(len) {
            return s;
        }
        let old = self.capacity();
        let capacity = old
            .saturating_add(len)
            .max(old.saturating_mul(2))
            .checked_next_power_of_two()
            .unwrap_or(u32::MAX);
        self.data.resize(usize::try_from(capacity).unwrap_or(0), 0);
        self.grown = true;
        self.release_span(Span {
            start: old,
            len: capacity.saturating_sub(old),
        });
        self.take(len).unwrap_or(Span { start: 0, len })
    }

    fn take(&mut self, len: u32) -> Option<Span> {
        let i = self.free.iter().position(|r| r.len >= len)?;
        let r = self.free.get_mut(i)?;
        let out = Span { start: r.start, len };
        r.start = r.start.saturating_add(len);
        r.len = r.len.saturating_sub(len);
        if r.len == 0 {
            self.free.remove(i);
        }
        Some(out)
    }

    /// Give `s` back.
    pub(crate) fn release(&mut self, s: Span) {
        self.used = self.used.saturating_sub(u64::from(s.len));
        self.release_span(s);
    }

    fn release_span(&mut self, r: Span) {
        if r.len == 0 {
            return;
        }
        let i = self.free.partition_point(|f| f.start < r.start);
        self.free.insert(i, r);
        if let (Some(a), Some(b)) = (self.free.get(i).copied(), self.free.get(i.saturating_add(1)).copied())
            && a.end() == b.start
        {
            if let Some(m) = self.free.get_mut(i) {
                m.len = m.len.saturating_add(b.len);
            }
            self.free.remove(i.saturating_add(1));
        }
        if let Some(before) = i.checked_sub(1)
            && let (Some(a), Some(b)) = (self.free.get(before).copied(), self.free.get(i).copied())
            && a.end() == b.start
        {
            if let Some(m) = self.free.get_mut(before) {
                m.len = m.len.saturating_add(b.len);
            }
            self.free.remove(i);
        }
    }

    /// Write `words` at word `at`: only the words that differ from what
    /// is there are uploaded.
    pub(crate) fn write(&mut self, at: u32, words: &[u32]) {
        let Ok(start) = usize::try_from(at) else { return };
        let Some(dst) = self.data.get_mut(start..start.saturating_add(words.len())) else {
            return;
        };
        let Some(first) = dst.iter().zip(words).position(|(a, b)| a != b) else {
            return;
        };
        let last = dst.iter().zip(words).rposition(|(a, b)| a != b).unwrap_or(first);
        dst.copy_from_slice(words);
        self.dirty.push(Span {
            start: at.saturating_add(u32::try_from(first).unwrap_or(0)),
            len: u32::try_from(last.saturating_sub(first).saturating_add(1)).unwrap_or(0),
        });
    }

    /// Upload what was written since the last upload; the bytes written.
    pub(crate) fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> u64 {
        if self.grown {
            self.grown = false;
            self.buffer = make(device, self.label, self.capacity());
            self.generation = self.generation.saturating_add(1);
            self.dirty.clear();
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.data));
            return self.bytes();
        }
        if self.dirty.is_empty() {
            return 0;
        }
        let mut spans = std::mem::take(&mut self.dirty);
        spans.sort_by_key(|s| s.start);
        let mut merged: Vec<Span> = Vec::new();
        for s in spans.into_iter().filter(|s| s.len > 0) {
            match merged.last_mut() {
                Some(last) if s.start <= last.end().saturating_add(JOIN) => {
                    let end = last.end().max(s.end());
                    last.len = end.saturating_sub(last.start);
                }
                _ => merged.push(s),
            }
        }
        let dirty: u64 = merged.iter().map(|s| u64::from(s.len)).sum();
        // Past about 15% of what is in use, one write of the whole.
        if dirty.saturating_mul(100) > self.used.max(1).saturating_mul(15) && merged.len() > 1 {
            let end = merged.iter().map(Span::end).max().unwrap_or(0);
            let start = merged.iter().map(|s| s.start).min().unwrap_or(0);
            merged = vec![Span {
                start,
                len: end.saturating_sub(start),
            }];
        }
        let mut bytes = 0_u64;
        for s in merged {
            let (Ok(a), Ok(b)) = (usize::try_from(s.start), usize::try_from(s.end())) else {
                continue;
            };
            let Some(words) = self.data.get(a..b) else { continue };
            queue.write_buffer(
                &self.buffer,
                u64::from(s.start).saturating_mul(4),
                bytemuck::cast_slice(words),
            );
            bytes = bytes.saturating_add(u64::from(s.len).saturating_mul(4));
        }
        bytes
    }
}

fn make(device: &wgpu::Device, label: &str, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: u64::from(capacity).saturating_mul(4).max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// A table the CPU writes whole when its shape changes, uploaded where it
/// differs from what was uploaded last, and otherwise patched entry by
/// entry, only what was patched uploaded (the property tables).
pub(crate) struct Table {
    label: &'static str,
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) generation: u64,
    shown: Vec<u32>,
    /// Spans patched since the last upload.
    dirty: Vec<Span>,
}

impl Table {
    pub(crate) fn new(device: &wgpu::Device, label: &'static str) -> Self {
        Self {
            label,
            buffer: make(device, label, 256),
            generation: 0,
            shown: Vec::new(),
            dirty: Vec::new(),
        }
    }

    /// Write `words` at word `at` of what was last uploaded whole, to go up
    /// with the next [`Self::flush`]; whether it lies inside it.
    pub(crate) fn patch(&mut self, at: usize, words: &[u32]) -> bool {
        let Some(dst) = self.shown.get_mut(at..at.saturating_add(words.len())) else {
            return false;
        };
        if dst != words {
            dst.copy_from_slice(words);
            self.dirty.push(Span {
                start: u32::try_from(at).unwrap_or(u32::MAX),
                len: u32::try_from(words.len()).unwrap_or(0),
            });
        }
        true
    }

    /// Upload what was patched since the last upload; the bytes written.
    pub(crate) fn flush(&mut self, queue: &wgpu::Queue) -> u64 {
        if self.dirty.is_empty() {
            return 0;
        }
        let mut spans = std::mem::take(&mut self.dirty);
        spans.sort_by_key(|s| s.start);
        let mut bytes = 0_u64;
        let mut merged: Option<Span> = None;
        let mut write = |s: Span, shown: &[u32]| {
            let (Ok(a), Ok(b)) = (usize::try_from(s.start), usize::try_from(s.end())) else {
                return;
            };
            if let Some(words) = shown.get(a..b) {
                queue.write_buffer(
                    &self.buffer,
                    u64::from(s.start).saturating_mul(4),
                    bytemuck::cast_slice(words),
                );
                bytes = bytes.saturating_add(u64::from(s.len).saturating_mul(4));
            }
        };
        for s in spans.drain(..) {
            merged = match merged {
                Some(m) if s.start <= m.end().saturating_add(16) => {
                    let end = m.end().max(s.end());
                    Some(Span {
                        start: m.start,
                        len: end.saturating_sub(m.start),
                    })
                }
                Some(m) => {
                    write(m, &self.shown);
                    Some(s)
                }
                None => Some(s),
            };
        }
        if let Some(m) = merged {
            write(m, &self.shown);
        }
        self.dirty = spans;
        bytes
    }

    /// Make the GPU's copy `words`; the bytes written.
    pub(crate) fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, words: &[u32]) -> u64 {
        self.dirty.clear();
        let need = u64::try_from(words.len()).unwrap_or(0).saturating_mul(4);
        if self.buffer.size() < need || self.shown.len() != words.len() {
            if self.buffer.size() < need {
                let words_cap = u32::try_from(words.len())
                    .unwrap_or(0)
                    .checked_next_power_of_two()
                    .unwrap_or(u32::MAX);
                self.buffer = make(device, self.label, words_cap);
                self.generation = self.generation.saturating_add(1);
            }
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(words));
            self.shown = words.to_vec();
            return need;
        }
        let mut bytes = 0_u64;
        let mut i = 0_usize;
        let n = words.len();
        while i < n {
            if words.get(i) == self.shown.get(i) {
                i = i.saturating_add(1);
                continue;
            }
            // A run that differs, joined across short equal gaps.
            let start = i;
            let mut end = i;
            let mut gap = 0_usize;
            while i < n && gap < 16 {
                if words.get(i) == self.shown.get(i) {
                    gap = gap.saturating_add(1);
                } else {
                    gap = 0;
                    end = i.saturating_add(1);
                }
                i = i.saturating_add(1);
            }
            if let (Some(src), Some(dst)) = (words.get(start..end), self.shown.get_mut(start..end)) {
                queue.write_buffer(
                    &self.buffer,
                    u64::try_from(start).unwrap_or(0).saturating_mul(4),
                    bytemuck::cast_slice(src),
                );
                dst.copy_from_slice(src);
                bytes = bytes.saturating_add(u64::try_from(src.len()).unwrap_or(0).saturating_mul(4));
            }
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freed_spans_merge_and_writes_join() {
        let Some((device, queue)) = super::super::tests::device() else {
            return;
        };
        let mut w = Words::new(&device, "test", 64);
        let a = w.alloc(16);
        let b = w.alloc(16);
        w.release(a);
        w.release(b);
        let c = w.alloc(64);
        assert_eq!(c.start, 0, "the whole buffer, unmoved");
        assert_eq!(w.upload(&device, &queue), 0, "nothing written");
        w.write(0, &[1; 4]);
        w.write(8, &[2; 4]);
        assert_eq!(w.upload(&device, &queue), 48, "two writes joined across a short gap");
        w.write(0, &[1, 1, 3, 1]);
        assert_eq!(w.upload(&device, &queue), 4, "only the word that changed");
        w.write(0, &[1, 1, 3, 1]);
        assert_eq!(w.upload(&device, &queue), 0, "the same words: nothing");
        let _grown = w.alloc(8);
        assert_eq!(w.upload(&device, &queue), w.bytes(), "grown: written whole");
    }
}
