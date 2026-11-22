//! Fractional z keys: an object goes between two others by a key made
//! between theirs, so inserting or raising one never renumbers the rest
//! and the draw order is never sorted again whole (ADR-0017).

/// A place in the draw order: byte strings compared as such, the last
/// byte never 0, so a key always exists between two different ones.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZKey(Vec<u8>);

impl ZKey {
    /// A key between `below` and `above` (either end open), as short as
    /// the two allow. `below` must be less than `above`.
    #[must_use]
    pub fn between(below: Option<&Self>, above: Option<&Self>) -> Self {
        let lo: &[u8] = below.map_or(&[], |k| &k.0);
        let hi: Option<&[u8]> = above.map(|k| k.0.as_slice());
        let mut out = Vec::new();
        // Once a byte is below `hi`'s, any byte after it is.
        let mut under_hi = hi.is_none();
        for i in 0.. {
            // The key made so far equals `lo`'s start (or `lo` and zeros).
            let a = u16::from(lo.get(i).copied().unwrap_or(0));
            let b = if under_hi {
                256
            } else {
                hi.and_then(|h| h.get(i)).map_or(256, |v| u16::from(*v))
            };
            if b > a.saturating_add(1) {
                out.push(u8::try_from(a.saturating_add((b.saturating_sub(a)) / 2)).unwrap_or(u8::MAX));
                break;
            }
            out.push(u8::try_from(a).unwrap_or(u8::MAX));
            under_hi |= a < b;
        }
        Self(out)
    }

    /// The key after `last` (the top of the order). Appended keys count
    /// up as integers whose head byte holds their length (`0x80 + n`, `n`
    /// digits), so a million appends stay four bytes long where midpoints
    /// would grow a byte every eight.
    #[must_use]
    pub fn after(last: Option<&Self>) -> Self {
        let first = Self(vec![0x81, 0x80]);
        let Some(k) = last else { return first };
        let head = k.0.first().copied().unwrap_or(0);
        let n = usize::from(head.saturating_sub(0x80));
        let digits = k.0.get(1..n.saturating_add(1));
        let Some(digits) = digits.filter(|_| (0x81..0xff).contains(&head)) else {
            return if *k < first {
                first
            } else {
                Self::between(Some(k), None)
            };
        };
        let mut next = digits.to_vec();
        // Count up, carrying; past every digit, one more digit.
        let mut carried = true;
        for d in next.iter_mut().rev() {
            let (v, over) = d.overflowing_add(1);
            *d = v;
            if !over {
                carried = false;
                break;
            }
        }
        let mut out = if carried {
            let mut out = vec![head.saturating_add(1), 1];
            out.extend(std::iter::repeat_n(0, n));
            out
        } else {
            let mut out = vec![head];
            out.extend(next);
            out
        };
        // The last byte never 0, so a key always fits below it.
        if let Some(l) = out.last_mut()
            && *l == 0
        {
            *l = 1;
        }
        Self(out)
    }

    /// Its bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_between_any_two_and_stay_short_when_appended() {
        let mut keys = vec![ZKey::after(None)];
        for _ in 0..1000 {
            let k = ZKey::after(keys.last());
            assert!(k > *keys.last().unwrap());
            keys.push(k);
        }
        assert!(keys.last().unwrap().bytes().len() <= 3, "{:?}", keys.last());
        let mut k = keys.last().unwrap().clone();
        for _ in 0..1_000_000 {
            let n = ZKey::after(Some(&k));
            assert!(n > k);
            k = n;
        }
        assert!(k.bytes().len() <= 4, "{k:?}");
        // Always into the same gap, the worst case.
        let (mut lo, hi) = (keys[0].clone(), keys[1].clone());
        for _ in 0..2000 {
            let k = ZKey::between(Some(&lo), Some(&hi));
            assert!(lo < k && k < hi, "{lo:?} < {k:?} < {hi:?}");
            lo = k;
        }
        let mut rng = crate::testing::Rng::new(3);
        let mut order = keys.clone();
        for _ in 0..2000 {
            let i = usize::try_from(rng.below(u64::try_from(order.len() - 1).unwrap())).unwrap();
            let k = ZKey::between(Some(&order[i]), Some(&order[i + 1]));
            assert!(order[i] < k && k < order[i + 1]);
            order.insert(i + 1, k);
        }
        let first = ZKey::between(None, Some(&order[0]));
        assert!(first < order[0]);
    }
}
