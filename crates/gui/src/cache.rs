//! A cache of what the last frames drew: an entry no frame has used for
//! [`KEEP`] frames is dropped when a frame ends, so a cache holds what the
//! screen shows, however many keys a transition passes through on the way
//! (a colour per frame while a button fades on hover).

use std::collections::HashMap;
use std::hash::Hash;

/// Frames an entry outlives its last use: the frame that drew it and the next,
/// so what one frame drew is there for the next to draw again.
const KEEP: u64 = 2;

pub(crate) struct FrameCache<K, V> {
    entries: HashMap<K, Entry<V>>,
    frame: u64,
}

struct Entry<V> {
    value: V,
    used: u64,
}

impl<K, V> Default for FrameCache<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            frame: 0,
        }
    }
}

impl<K: Eq + Hash, V> FrameCache<K, V> {
    /// The value under `key`, made by `make` when there is none, marked used this frame.
    pub(crate) fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> V) -> &mut V {
        let frame = self.frame;
        let entry = self.entries.entry(key).or_insert_with(|| Entry {
            value: make(),
            used: frame,
        });
        entry.used = frame;
        &mut entry.value
    }

    /// Drop what the last [`KEEP`] frames did not use; call once per frame.
    pub(crate) fn end_frame(&mut self) {
        let frame = self.frame;
        self.entries.retain(|_, e| frame.saturating_sub(e.used) < KEEP);
        self.frame = frame.saturating_add(1);
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.values().map(|e| &e.value)
    }
}
