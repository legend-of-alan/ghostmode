//! The Python interpreters a sandbox can run its scripts in.

#[cfg(feature = "monty")]
pub use self::monty::Monty;
#[cfg(feature = "monty-pool")]
pub use self::monty::{MontyPool, WorkerOptions};

pub(crate) use self::sealed::{Ending, Progress, Sealed, Streams};

mod monty;
mod sealed;

/// Runs a sandbox's scripts in Python sessions whose globals persist from one script to the
/// next. Only this crate's backends, `Monty` and `MontyPool`, implement it.
pub trait Backend: Sealed {}

impl<B: Sealed> Backend for B {}
