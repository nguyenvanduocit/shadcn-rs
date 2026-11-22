//! A GPU buffer handed out in ranges of fixed-size elements: first fit from
//! a free list that merges neighbours. A buffer too small is replaced by
//! one twice as large and the old contents copied on the GPU (in a submit
//! of its own, before the frame's writes), so what was written stays valid
//! and nothing is redone; the generation tells bind groups to be made anew.

/// A range of elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Range {
    pub start: u32,
    pub len: u32,
}

pub(crate) struct Arena {
    label: &'static str,
    usage: wgpu::BufferUsages,
    /// Bytes per element.
    unit: u64,
    pub(crate) buffer: wgpu::Buffer,
    /// Elements the buffer holds.
    capacity: u32,
    /// Free ranges, sorted by start, never touching.
    free: Vec<Range>,
    pub(crate) generation: u64,
}

impl Arena {
    pub(crate) fn new(device: &wgpu::Device, label: &'static str, unit: u64, capacity: u32) -> Self {
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let capacity = capacity.max(1);
        Self {
            label,
            usage,
            unit,
            buffer: buffer(device, label, usage, unit, capacity),
            capacity,
            free: vec![Range {
                start: 0,
                len: capacity,
            }],
            generation: 0,
        }
    }

    /// Bytes the buffer holds.
    pub(crate) fn bytes(&self) -> u64 {
        self.unit.saturating_mul(u64::from(self.capacity))
    }

    /// `len` elements, the buffer grown when it has no room.
    pub(crate) fn alloc(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, len: u32) -> Range {
        let len = len.max(1);
        if let Some(r) = self.take(len) {
            return r;
        }
        let need = self.capacity.saturating_add(len);
        let capacity = need.max(self.capacity.saturating_mul(2)).next_power_of_two();
        let bigger = buffer(device, self.label, self.usage, self.unit, capacity);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gui arena growth"),
        });
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &bigger, 0, self.bytes());
        queue.submit([encoder.finish()]);
        self.release(Range {
            start: self.capacity,
            len: capacity.saturating_sub(self.capacity),
        });
        self.buffer = bigger;
        self.capacity = capacity;
        self.generation = self.generation.saturating_add(1);
        self.take(len).unwrap_or(Range { start: 0, len })
    }

    fn take(&mut self, len: u32) -> Option<Range> {
        let i = self.free.iter().position(|r| r.len >= len)?;
        let r = self.free.get_mut(i)?;
        let out = Range { start: r.start, len };
        r.start = r.start.saturating_add(len);
        r.len = r.len.saturating_sub(len);
        if r.len == 0 {
            self.free.remove(i);
        }
        Some(out)
    }

    /// Give `r` back.
    pub(crate) fn release(&mut self, r: Range) {
        if r.len == 0 {
            return;
        }
        let i = self.free.partition_point(|f| f.start < r.start);
        self.free.insert(i, r);
        // Merge with the next, then with the one before.
        if let (Some(a), Some(b)) = (self.free.get(i).copied(), self.free.get(i.saturating_add(1)).copied())
            && a.start.saturating_add(a.len) == b.start
        {
            if let Some(m) = self.free.get_mut(i) {
                m.len = m.len.saturating_add(b.len);
            }
            self.free.remove(i.saturating_add(1));
        }
        if let Some(before) = i.checked_sub(1)
            && let (Some(a), Some(b)) = (self.free.get(before).copied(), self.free.get(i).copied())
            && a.start.saturating_add(a.len) == b.start
        {
            if let Some(m) = self.free.get_mut(before) {
                m.len = m.len.saturating_add(b.len);
            }
            self.free.remove(i);
        }
    }

    /// The byte offset of element `i`.
    pub(crate) fn offset(&self, i: u32) -> u64 {
        self.unit.saturating_mul(u64::from(i))
    }

    /// The bytes of range `r`.
    pub(crate) fn size(&self, r: Range) -> u64 {
        self.unit.saturating_mul(u64::from(r.len))
    }
}

fn buffer(device: &wgpu::Device, label: &str, usage: wgpu::BufferUsages, unit: u64, capacity: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: unit.saturating_mul(u64::from(capacity)).max(16),
        usage,
        mapped_at_creation: false,
    })
}

/// A buffer the CPU rewrites, made larger (and anew) when it must be.
pub(crate) struct Scratch {
    label: &'static str,
    usage: wgpu::BufferUsages,
    pub(crate) buffer: wgpu::Buffer,
    pub(crate) generation: u64,
    /// What [`Scratch::update`] wrote last.
    shown: Vec<u8>,
}

impl Scratch {
    pub(crate) fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages, size: u64) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        Self {
            label,
            usage,
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size.max(16),
                usage,
                mapped_at_creation: false,
            }),
            generation: 0,
            shown: Vec::new(),
        }
    }

    /// At least `size` bytes (contents not kept).
    pub(crate) fn ensure(&mut self, device: &wgpu::Device, size: u64) {
        if self.buffer.size() >= size {
            return;
        }
        self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(self.label),
            size: size.next_power_of_two().max(16),
            usage: self.usage,
            mapped_at_creation: false,
        });
        self.generation = self.generation.saturating_add(1);
    }

    /// Write `data` from the start, growing first.
    pub(crate) fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        self.ensure(device, u64::try_from(data.len()).unwrap_or(u64::MAX));
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, data);
        }
    }

    /// Write `data` unless it is what was written last; the bytes written.
    pub(crate) fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) -> u64 {
        let generation = self.generation;
        self.ensure(device, u64::try_from(data.len()).unwrap_or(u64::MAX));
        if generation == self.generation && self.shown == data {
            return 0;
        }
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, data);
        }
        data.clone_into(&mut self.shown);
        u64::try_from(data.len()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena() -> Option<(wgpu::Device, wgpu::Queue, Arena)> {
        let (device, queue) = super::super::tests::device()?;
        let a = Arena::new(&device, "test", 4, 8);
        Some((device, queue, a))
    }

    #[test]
    fn freed_ranges_merge_and_are_reused() {
        let Some((device, queue, mut a)) = arena() else { return };
        let x = a.alloc(&device, &queue, 3);
        let y = a.alloc(&device, &queue, 3);
        a.release(x);
        a.release(y);
        let z = a.alloc(&device, &queue, 8);
        assert_eq!((z.start, a.generation), (0, 0), "the whole buffer, unmoved");
        let w = a.alloc(&device, &queue, 4);
        assert_eq!(a.generation, 1, "grown");
        assert!(w.start >= 8);
    }
}
