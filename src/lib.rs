//! Code Mode for Rust agents. The model writes a script, ghostmode runs it in a sandbox, and
//! the host provides the tools the script calls. The model sees two tools, `execute` and `poll`.

pub mod host;
#[cfg(feature = "rmcp")]
pub mod mcp;
mod repl;
mod sandbox;
pub mod schema;
mod value;

pub use monty::DumpError;
pub use sandbox::{Builder, Error, Limits, Sandbox, Status};
