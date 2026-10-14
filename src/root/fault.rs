//! Deterministic, per-session fault plans for shutdown/recovery qualification.
//!
//! Available only with the test-only `fault-injection` feature. Plans are bound
//! to one [Session][crate::root::Session] and its [Connection][crate::root::Connection],
//! so concurrent tests never share global fault state. Without the feature every
//! hook is a constant `false` and the plan carries no data.

/// Faults injected into one connection's lifecycle.
#[cfg(feature = "fault-injection")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FaultPlan {
    /// Fail watcher thread creation after the native connection exists, which
    /// exercises connect rollback.
    pub fail_watcher_spawn: bool,
    /// Number of explicit disconnect attempts that fail before calling
    /// `CfDisconnectSyncRoot`, leaving the native connection active.
    pub fail_disconnects: u32,
    /// Panic at watcher thread start before any directory I/O is issued.
    pub panic_watcher: bool,
}

/// Private runtime state for a plan; zero-sized without the feature.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Faults {
    #[cfg(feature = "fault-injection")]
    plan: FaultPlan,
}

impl Faults {
    #[cfg(feature = "fault-injection")]
    pub(crate) fn new(plan: FaultPlan) -> Self {
        Self { plan }
    }

    /// Whether watcher creation must fail.
    pub(crate) fn watcher_spawn(&self) -> bool {
        #[cfg(feature = "fault-injection")]
        return self.plan.fail_watcher_spawn;
        #[cfg(not(feature = "fault-injection"))]
        false
    }

    /// Whether the watcher thread must panic before issuing I/O.
    pub(crate) fn watcher_panic(&self) -> bool {
        #[cfg(feature = "fault-injection")]
        return self.plan.panic_watcher;
        #[cfg(not(feature = "fault-injection"))]
        false
    }

    /// Consumes one planned disconnect failure, if any remain.
    pub(crate) fn take_disconnect_failure(&mut self) -> bool {
        #[cfg(feature = "fault-injection")]
        {
            if self.plan.fail_disconnects > 0 {
                self.plan.fail_disconnects -= 1;
                return true;
            }
            false
        }
        #[cfg(not(feature = "fault-injection"))]
        false
    }
}
