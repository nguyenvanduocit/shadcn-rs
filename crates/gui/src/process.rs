//! What the whole process uses, as the operating system counts it: CPU time
//! and resident memory. The heap ([`crate::heap`]) and the caches
//! ([`crate::inspect::Footprint`]) say what the app holds; this says what
//! that costs the machine, every thread and library included.

use std::time::{Duration, Instant};

/// The process's use of the machine at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// CPU time spent since the process started, every thread's.
    pub cpu: Option<Duration>,
    /// Bytes of physical memory the process holds (its resident set).
    pub resident: Option<u64>,
}

/// The process's use of the machine now; a part the platform does not report is `None`.
#[must_use]
pub fn usage() -> Usage {
    Usage {
        cpu: cpu_time::ProcessTime::try_now().ok().map(|t| t.as_duration()),
        resident: memory_stats::memory_stats().and_then(|m| u64::try_from(m.physical_mem).ok()),
    }
}

/// How often [`CpuMeter`] takes a new share: often enough to follow a hover,
/// seldom enough that a single slow frame does not make it jump.
const CPU_WINDOW: Duration = Duration::from_millis(500);

/// The process's CPU use as a share of one core, over the last half second.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuMeter {
    since: Option<(Instant, Duration)>,
    /// Percent of one core (above 100 when several threads run at once).
    percent: Option<f32>,
}

impl CpuMeter {
    /// The share over the last whole window, sampling the clock when the
    /// window has passed; `None` until a window has passed or when the
    /// platform does not report CPU time.
    pub fn sample(&mut self) -> Option<f32> {
        let now = Instant::now();
        let cpu = usage().cpu?;
        match self.since {
            Some((at, before)) if now.duration_since(at) >= CPU_WINDOW => {
                let wall = now.duration_since(at).as_secs_f32();
                let used = cpu.saturating_sub(before).as_secs_f32();
                self.percent = Some(used / wall * 100.0);
                self.since = Some((now, cpu));
            }
            Some(_) => {}
            None => self.since = Some((now, cpu)),
        }
        self.percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_process_reports_its_cpu_time_and_memory() {
        let u = usage();
        assert!(u.cpu.is_some(), "cpu time is reported on the platforms gui runs on");
        assert!(u.resident.is_some_and(|r| r > 0));
    }
}
