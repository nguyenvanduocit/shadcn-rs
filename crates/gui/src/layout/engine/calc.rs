//! `calc()` values for Taffy (ADR-0014, L4): Taffy holds a `calc()` as an
//! opaque, 8-byte-aligned handle and asks the tree to resolve it against a
//! basis (`LayoutPartialTree::resolve_calc_value`). Our handles index a
//! process-wide table of interned `fraction × basis + px` sums, so a style
//! carries a calc as one word and every thread resolves it alike. Built
//! without `unsafe`: a handle is an address with no provenance, never read
//! through.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// `fraction × basis + px`: a percentage plus a length, the `calc()` shadcn
/// writes (`max-w-[calc(100%-2rem)]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalcSum {
    pub fraction: f32,
    pub px: f32,
}

#[derive(Default)]
struct Table {
    sums: Vec<CalcSum>,
    index: HashMap<(u32, u32), usize>,
}

fn table() -> &'static RwLock<Table> {
    static TABLE: OnceLock<RwLock<Table>> = OnceLock::new();
    TABLE.get_or_init(RwLock::default)
}

/// The index of `sum`, interned: equal sums share one, so styles holding
/// them compare equal. Distinct sums stay for the process's life; layout
/// writes few (they come from style classes, not from animation).
#[must_use]
pub fn intern(sum: CalcSum) -> usize {
    let key = (sum.fraction.to_bits(), sum.px.to_bits());
    let found = table().read().ok().and_then(|t| t.index.get(&key).copied());
    found.unwrap_or_else(|| {
        let Ok(mut t) = table().write() else { return 0 };
        if let Some(&i) = t.index.get(&key) {
            return i;
        }
        t.sums.push(sum);
        let i = t.sums.len().saturating_sub(1);
        t.index.insert(key, i);
        i
    })
}

/// The handle Taffy holds for interned sum `index`.
#[must_use]
pub fn handle(index: usize) -> *const () {
    // Index 0 would be a null pointer, and Taffy keeps its tag in the low 3 bits.
    std::ptr::without_provenance(index.saturating_add(1).checked_shl(3).unwrap_or(8))
}

/// `handle`'s value against `basis`.
#[must_use]
pub fn resolve(handle: *const (), basis: f32) -> f32 {
    let index = (handle.addr() >> 3).saturating_sub(1);
    table()
        .read()
        .ok()
        .and_then(|t| t.sums.get(index).copied())
        .map_or(0.0, |s| s.fraction.mul_add(basis, s.px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_sums_share_a_handle_and_resolve_against_their_basis() {
        let a = handle(intern(CalcSum {
            fraction: 1.0,
            px: -32.0,
        }));
        let b = handle(intern(CalcSum {
            fraction: 1.0,
            px: -32.0,
        }));
        assert_eq!(a, b);
        assert_eq!(a.addr() & 7, 0);
        assert_eq!(resolve(a, 400.0), 368.0);
        let c = handle(intern(CalcSum { fraction: 0.5, px: 2.0 }));
        assert_ne!(a, c);
        assert_eq!(resolve(c, 100.0), 52.0);
    }
}
