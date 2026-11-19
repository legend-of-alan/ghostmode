//! Code Mode for Rust agents. The model writes a script, ghostmode runs it in a sandbox, and
//! the host provides the tools the script calls. The model sees two tools, `execute` and `poll`.
//!
//! A sandbox runs scripts in a [`backend::Backend`]. The default backend, `backend::Monty`, runs
//! them inside the caller's process. It isn't crash-isolated: a script can panic, abort or hang
//! the caller's process. Untrusted scripts belong in `backend::Pool`, behind the `pool` feature,
//! which runs each session in its own Monty worker process.

pub mod backend;
pub mod host;
#[cfg(feature = "rmcp")]
pub mod mcp;
mod sandbox;
pub mod schema;

pub use sandbox::{Builder, Error, Limits, Sandbox, Status};
