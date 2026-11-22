//! Retired instructions, counted beside time (ADR-0023): a frame's time
//! moves 35% between runs of unchanged code on a laptop (ADR-0008), its
//! instructions a few tenths of a percent, so a bench compares by them when
//! both runs have them.
//!
//! What is counted depends on what the OS lets a process read without
//! privileges or `unsafe` in this workspace:
//! - **Linux**: the calling thread, user space only, through `perf_event_open`
//!   (`perf-event`). It needs `kernel.perf_event_paranoid` at 2 or below
//!   and a CPU whose counters the kernel exposes (a VM may not).
//! - **macOS**: the whole process (every thread, user and kernel), from
//!   `proc_pid_rusage`'s `ri_instructions` (`libproc`). Apple silicon
//!   counts per thread only in private interfaces that need root.
//! - elsewhere: not counted.

/// A source of retired-instruction counts; [`Instructions::read`] is the
/// count since it was made.
pub struct Instructions {
    inner: Inner,
    start: u64,
}

#[cfg(target_os = "linux")]
struct Inner(perf_event::Counter);

#[cfg(target_os = "macos")]
struct Inner(i32);

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct Inner;

impl Instructions {
    /// Start counting; why not, when this OS or machine cannot.
    pub fn start() -> Result<Self, String> {
        let mut inner = Inner::open()?;
        let start = inner.read()?;
        Ok(Self { inner, start })
    }

    /// Instructions retired since [`Self::start`], `None` when the counter
    /// could not be read.
    pub fn read(&mut self) -> Option<u64> {
        self.inner.read().ok().map(|n| n.saturating_sub(self.start))
    }

    /// What the count covers.
    #[must_use]
    pub fn scope(&self) -> &'static str {
        Inner::SCOPE
    }
}

#[cfg(target_os = "linux")]
impl Inner {
    const SCOPE: &'static str = "this thread, user space";

    fn open() -> Result<Self, String> {
        let mut builder = perf_event::Builder::new().kind(perf_event::events::Hardware::INSTRUCTIONS);
        builder.exclude_kernel(true).exclude_hv(true);
        let mut counter = builder.build().map_err(|e| {
            format!("perf_event_open: {e} (kernel.perf_event_paranoid above 2, or no hardware counters in this VM)")
        })?;
        counter.enable().map_err(|e| format!("perf_event enable: {e}"))?;
        Ok(Self(counter))
    }

    fn read(&mut self) -> Result<u64, String> {
        self.0.read().map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
impl Inner {
    const SCOPE: &'static str = "the whole process";

    fn open() -> Result<Self, String> {
        let pid = i32::try_from(std::process::id()).map_err(|e| e.to_string())?;
        Ok(Self(pid))
    }

    fn read(&mut self) -> Result<u64, String> {
        libproc::pid_rusage::pidrusage::<libproc::pid_rusage::RUsageInfoV4>(self.0)
            .map(|r| r.ri_instructions)
            .map_err(|e| format!("proc_pid_rusage: {e}"))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl Inner {
    const SCOPE: &'static str = "nothing";

    fn open() -> Result<Self, String> {
        Err("retired instructions are counted on Linux and macOS only".to_owned())
    }

    fn read(&mut self) -> Result<u64, String> {
        Err("not counted".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(n: u64) -> u64 {
        (0..n).fold(0_u64, |x, i| std::hint::black_box(x.wrapping_add(i ^ (x >> 3))))
    }

    /// Where the counter can be read, ten times the work is about ten times
    /// the instructions, and the same work about the same count.
    #[test]
    fn instructions_follow_the_work() {
        let Ok(mut c) = Instructions::start() else {
            eprintln!(
                "instructions not counted here: {}",
                Instructions::start().err().unwrap_or_default()
            );
            return;
        };
        let mut take = |n| {
            let before = c.read().unwrap_or(0);
            std::hint::black_box(work(n));
            c.read().unwrap_or(0).saturating_sub(before)
        };
        let small = take(1_000_000);
        let again = take(1_000_000);
        let large = take(10_000_000);
        assert!(small > 1_000_000, "{small} for a million steps");
        assert!(large > small);
        if c.scope() == "the whole process" {
            // Other tests run beside this one in the same process.
            return;
        }
        let ratio = crate::convert::u64_to_f64(large) / crate::convert::u64_to_f64(small.max(1));
        assert!((8.0..12.0).contains(&ratio), "{large} / {small} = {ratio}");
        let drift = crate::convert::u64_to_f64(small.abs_diff(again)) / crate::convert::u64_to_f64(small);
        assert!(drift < 0.05, "the same work counted {small} then {again}");
    }
}
