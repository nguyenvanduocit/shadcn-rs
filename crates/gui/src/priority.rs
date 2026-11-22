//! How each of the app's threads is scheduled (ADR-0015), after Chromium's
//! `PlatformThread::SetCurrentThreadType`, through `gdt-cpus`' safe API.
//!
//! | | macOS | Windows | Linux |
//! | --- | --- | --- | --- |
//! | [`ThreadType::Render`] | fixed priority 47 (`SCHED_RR`) | `THREAD_PRIORITY_TIME_CRITICAL` | nice −20 |
//! | [`ThreadType::Platform`] | `QOS_CLASS_USER_INTERACTIVE` | `THREAD_PRIORITY_HIGHEST` | nice −10 |
//! | [`ThreadType::Ui`] | `QOS_CLASS_USER_INITIATED` | `THREAD_PRIORITY_NORMAL` | nice 0 |
//!
//! On Linux a nice value below 0 goes through rtkit when the process may not
//! set it itself, and is left as it was when neither may.
//!
//! The platform and UI threads are scheduled as Chromium schedules its
//! browser UI thread (`kDisplayCritical`) and a renderer's main thread
//! (`kDefault`). The render thread differs on purpose. Chromium gives its
//! compositor the user-interactive class, which is a timeshare priority that
//! decays while the thread runs. Measured on a loaded machine (stress
//! `threads`), that class still woke the render thread up to 4 ms late and
//! missed up to 8 ticks in 180; at the top fixed priority of the user band it
//! missed none. The render thread sleeps on the vblank or its channel and
//! never spins (invariant 8), so a fixed priority cannot starve the rest.
//! macOS's time-constraint policy, which Chromium keeps for audio, is not used:
//! a thread that overruns its declared computation is demoted below every
//! other.

use std::fmt;

/// What a thread does, for how the OS should schedule it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadType {
    /// Draws at every vblank, whatever else runs.
    Render,
    /// Stamps input and feeds the render thread.
    Platform,
    /// The app's own work.
    Ui,
}

/// What a thread asked for and what it got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub asked: ThreadType,
    /// The OS's answer: the class or priority it runs at, or why it kept the one it had.
    pub got: String,
    /// Whether the OS gave less than asked.
    pub degraded: bool,
}

impl fmt::Display for Applied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.asked, self.got)?;
        if self.degraded {
            f.write_str(" (degraded)")?;
        }
        Ok(())
    }
}

/// Schedule the calling thread as `kind`.
pub fn set_current(kind: ThreadType) -> Applied {
    let level = match kind {
        ThreadType::Render => gdt_cpus::ThreadPriority::TimeCritical,
        ThreadType::Platform => gdt_cpus::ThreadPriority::Highest,
        ThreadType::Ui => gdt_cpus::ThreadPriority::Normal,
    };
    match gdt_cpus::set_thread_priority(level) {
        Ok(applied) => Applied {
            asked: kind,
            got: applied.to_string(),
            degraded: applied.degraded(),
        },
        Err(e) => Applied {
            asked: kind,
            got: format!("unchanged: {e}"),
            degraded: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{ThreadType, set_current};

    #[test]
    fn the_render_and_platform_threads_are_raised() {
        for kind in [ThreadType::Render, ThreadType::Platform] {
            let applied = std::thread::spawn(move || set_current(kind)).join().ok();
            let applied = applied.as_ref();
            assert!(applied.is_some_and(|a| a.asked == kind));
            // macOS and Windows let any thread take these.
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            assert!(applied.is_some_and(|a| !a.degraded), "{applied:?}");
        }
    }
}
