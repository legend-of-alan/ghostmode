//! Code Mode for Rust agents. The model writes a script, ghostmode runs it in a sandbox, and
//! the host provides the tools the script calls. The model sees two tools, `execute` and `poll`.

pub mod host;
mod repl;
mod sandbox;
mod value;

pub use sandbox::{Builder, Limits, Sandbox, Status};
