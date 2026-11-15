//! When to look for a GPU again, and when Blender may be replaced to use it.
//!
//! Cycles lists its devices once per process, so a GPU that was not ready when
//! Blender started only shows up in a new process.
use crate::blender::{ProcState, Status};
use std::time::Duration;

/// Pacing of the checks made while Blender runs without a GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuRetry {
    /// Wait before the first check; later waits double.
    pub first: Duration,
    /// Longest wait between checks.
    pub max: Duration,
    /// Time without requests before a session counts as left alone.
    pub quiet: Duration,
}

impl Default for GpuRetry {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(5),
            max: Duration::from_mins(10),
            quiet: Duration::from_mins(10),
        }
    }
}

impl GpuRetry {
    /// The wait after a check that found no GPU.
    #[must_use]
    pub fn after(&self, delay: Duration) -> Duration {
        delay.saturating_mul(2).min(self.max)
    }

    /// Whether replacing the process now loses nothing: no request is running,
    /// the scene matches its file, and nobody has used the session for a while.
    /// `idle` is the time since the last request, `None` when there was none.
    #[must_use]
    pub fn replaceable(&self, status: &Status, idle: Option<Duration>) -> bool {
        status.state == ProcState::Ready
            && !status.unsaved
            && idle.is_none_or(|idle| idle >= self.quiet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RETRY: GpuRetry = GpuRetry {
        first: Duration::from_secs(5),
        max: Duration::from_secs(30),
        quiet: Duration::from_mins(1),
    };

    fn status(state: ProcState, unsaved: bool) -> Status {
        Status {
            state,
            unsaved,
            ..Status::default()
        }
    }

    #[test]
    fn checks_back_off_up_to_the_longest_wait() {
        let mut delay = RETRY.first;
        let mut waits = Vec::new();
        for _ in 0..5 {
            delay = RETRY.after(delay);
            waits.push(delay.as_secs());
        }
        assert_eq!(waits, [10, 20, 30, 30, 30]);
    }

    #[test]
    fn a_fresh_idle_session_can_be_replaced() {
        assert!(
            RETRY.replaceable(&status(ProcState::Ready, false), None),
            "an unused session was kept"
        );
    }

    #[test]
    fn unsaved_changes_are_never_replaced() {
        for idle in [None, Some(Duration::from_hours(1))] {
            assert!(
                !RETRY.replaceable(&status(ProcState::Ready, true), idle),
                "unsaved scene replaced after {idle:?}"
            );
        }
    }

    #[test]
    fn a_session_in_use_waits_for_the_quiet_time() {
        let saved = status(ProcState::Ready, false);
        assert!(
            !RETRY.replaceable(&saved, Some(Duration::from_secs(59))),
            "replaced between two requests"
        );
        assert!(
            RETRY.replaceable(&saved, Some(Duration::from_mins(1))),
            "a session left alone was kept"
        );
    }

    #[test]
    fn only_a_ready_process_is_replaced() {
        for state in [
            ProcState::Starting,
            ProcState::Busy,
            ProcState::Restarting,
            ProcState::Failed,
        ] {
            assert!(
                !RETRY.replaceable(&status(state, false), None),
                "replaced while {state:?}"
            );
        }
    }
}
