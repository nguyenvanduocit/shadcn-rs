//! Randomness, the one way a component gets it (ADR-0028, "What lands at
//! K4a"): as time comes only through [`crate::clock`], a random number
//! comes only through here, so a test, a Scenario or a replay can fix it
//! ([`seed`]) and a check draws without moving the live sequence.
//!
//! The generator is SplitMix64 (Steele, Lea and Flood, "Fast splittable
//! pseudorandom number generators", OOPSLA 2014): one `u64` of state, every
//! value of it a valid state, equidistributed over its period of 2^64. It is
//! not for secrets.

use std::cell::Cell;

crate::engine::engine_local! {
    /// The generator's state; `None` until the first draw or [`seed`]. A
    /// fork draws from a copy and the live sequence is as before it.
    [Fork] static STATE: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Start the sequence of this thread's engine at `seed`: the same seed
/// gives the same draws, in every build.
pub fn seed(seed: u64) {
    STATE.with(|s| s.set(Some(seed)));
}

/// A seed no run shares: the process's hasher keys, which the standard
/// library takes from the system once per thread.
fn fresh() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(0x9e37_79b9_7f4a_7c15);
    h.finish()
}

/// The next 64 random bits.
#[must_use]
pub fn next_u64() -> u64 {
    STATE.with(|s| {
        let state = s.get().unwrap_or_else(fresh).wrapping_add(0x9e37_79b9_7f4a_7c15);
        s.set(Some(state));
        let mut z = state;
        z = (z ^ z.wrapping_shr(30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ z.wrapping_shr(27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ z.wrapping_shr(31)
    })
}

/// A number in `0..n`, each equally likely (Lemire's multiply-shift, its
/// rare biased draws rejected); `0` for `n == 0`.
#[must_use]
pub fn below(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    // `n` is not 0 here: the remainder is defined.
    let threshold = n.wrapping_neg().checked_rem(n).unwrap_or(0);
    loop {
        let wide = u128::from(next_u64()).wrapping_mul(u128::from(n));
        let low = u64::try_from(wide & u128::from(u64::MAX)).unwrap_or(u64::MAX);
        if low >= threshold {
            return u64::try_from(wide.wrapping_shr(64)).unwrap_or(0);
        }
    }
}

/// A number in `[0, 1)`, its 53 bits uniform.
#[must_use]
pub fn unit() -> f64 {
    /// 2^-53.
    const STEP: f64 = 1.0 / 9_007_199_254_740_992.0;
    // The top 53 bits of one draw, as a high 32 and a low 21.
    let draw = next_u64();
    let high = u32::try_from(draw.wrapping_shr(32)).unwrap_or(0);
    let low = u32::try_from(draw.wrapping_shr(11) & 0x1f_ffff).unwrap_or(0);
    (f64::from(high) * 2_097_152.0 + f64::from(low)) * STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_gives_the_same_draws() {
        seed(7);
        let a: Vec<u64> = (0..4).map(|_| next_u64()).collect();
        seed(7);
        let b: Vec<u64> = (0..4).map(|_| next_u64()).collect();
        assert_eq!(a, b);
        seed(8);
        assert_ne!(next_u64(), a[0]);
    }

    /// SplitMix64's published first output for seed 0.
    #[test]
    fn it_is_splitmix64() {
        seed(0);
        assert_eq!(next_u64(), 0xe220_a839_7b1d_cdaf);
    }

    #[test]
    fn below_and_unit_stay_in_range() {
        seed(3);
        for n in [1_u64, 2, 3, 10, u64::MAX] {
            for _ in 0..200 {
                assert!(below(n) < n);
            }
        }
        assert_eq!(below(0), 0);
        for _ in 0..1000 {
            let u = unit();
            assert!((0.0..1.0).contains(&u), "{u}");
        }
    }

    /// A check draws from a copy: the live sequence goes on as if it had
    /// not drawn.
    #[test]
    fn a_check_leaves_the_live_sequence() {
        seed(11);
        let live = next_u64();
        seed(11);
        let checked = crate::testing::in_check(next_u64);
        assert_eq!(checked, live, "the check drew the next value");
        assert_eq!(next_u64(), live, "and the live sequence did not move");
    }
}
