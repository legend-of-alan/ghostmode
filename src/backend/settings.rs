//! The settings that both Monty backends give each session.

use monty_types::{OsPolicy, ResourceLimits, SleepMode};

use crate::sandbox::Limits;

/// The name each session gives its scripts.
pub(crate) const SCRIPT_NAME: &str = "script.py";

/// Monty's limits that stop a script at `limits`' run time, memory and recursion depth. They
/// put no bound on suspensions, so only [`Limits::calls_per_execute`] limits tool calls.
pub(crate) fn to_resource_limits(limits: &Limits) -> ResourceLimits {
    ResourceLimits {
        max_feed_duration: Some(limits.run_time),
        max_memory: Some(limits.memory_bytes),
        max_recursion_depth: limits.recursion_depth,
        max_suspensions: usize::MAX,
        ..ResourceLimits::default()
    }
}

/// The OS policy under which `time.sleep` returns at once.
pub(crate) fn os_policy() -> OsPolicy {
    OsPolicy {
        sleep: SleepMode::Zero,
        ..OsPolicy::default()
    }
}
