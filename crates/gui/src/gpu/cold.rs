//! Data chunks checked against ones built cold (ADR-0018, ADR-0023), and
//! each frame's plan against one made whole (ADR-0016): with
//! `GUI_COLD_CHECK` set, or [`GpuRenderer::check_cold`] on, every data
//! chunk a frame's scene draws is drawn alone twice after the frame, from
//! the state kept across frames (its uploads appended, its pages grown on
//! the GPU, its parameters changed in place) and by a runtime that never
//! saw it, and the two must be the same to the bit, as layout's cold check
//! asks of the node tree.

use super::GpuRenderer;

impl GpuRenderer {
    /// Check every data chunk drawn from now on against one built cold,
    /// panicking where they differ (or stop checking).
    pub fn check_cold(&mut self, on: bool) {
        self.data.set_cold_check(on);
        self.cold_plan = on;
    }

    /// The frame's data chunks against ones built cold, when checking.
    pub(super) fn check_chunks_cold(&mut self) {
        let pairs = self.data.cold_pairs(&self.device, &self.queue);
        for (key, warm, cold, size) in pairs {
            let (Some(a), Some(b)) = (self.read_back(&warm, size), self.read_back(&cold, size)) else {
                continue;
            };
            let differ = a
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .zip(b.data().as_chunks::<4>().0)
                .filter(|(p, q)| p != q)
                .count();
            assert!(
                differ == 0,
                "data chunk {key:#x}: {differ} of {} pixels differ from the chunk built cold",
                u64::from(size.0).saturating_mul(u64::from(size.1))
            );
        }
    }

    /// Data chunks checked against cold ones so far (tests).
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn chunks_checked_cold(&self) -> usize {
        self.data.checked
    }
}
