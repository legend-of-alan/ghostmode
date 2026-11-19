//! The backends that run scripts in Monty: `repl` in the caller's process, and `pool` in
//! monty-pool worker processes, with what they share. Only this module and its children name
//! Monty's types.

#![cfg(any(feature = "monty", feature = "pool"))]

use monty_types::{OsPolicy, ResourceLimits, SleepMode};

use crate::sandbox::Limits;

#[cfg(feature = "pool")]
pub mod pool;
#[cfg(feature = "monty")]
pub mod repl;
mod value;

/// The name each session gives its scripts.
const SCRIPT_NAME: &str = "script.py";

/// The error text for restoring a dump that holds a suspended or one-shot run, not an idle
/// session.
const DUMP_HOLDS_RUN: &str = "the dump holds a run, and ghostmode restores only an idle session";

/// Monty's limits that stop a script at `limits`' run time, memory and recursion depth. They
/// put no bound on suspensions, so only [`Limits::calls_per_execute`] limits tool calls.
impl From<&Limits> for ResourceLimits {
    fn from(limits: &Limits) -> Self {
        Self {
            max_feed_duration: Some(limits.run_time),
            max_memory: Some(limits.memory_bytes),
            max_recursion_depth: limits.recursion_depth,
            max_suspensions: usize::MAX,
            ..Self::default()
        }
    }
}

/// The OS policy under which `time.sleep` returns at once.
fn os_policy() -> OsPolicy {
    OsPolicy {
        sleep: SleepMode::Zero,
        ..OsPolicy::default()
    }
}
