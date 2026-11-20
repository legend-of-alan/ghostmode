//! Code Mode for Rust agents. The model writes a script, ghostmode runs it in a sandbox, and
//! the host provides the tools the script calls. The model calls only `execute` and `poll`.
//!
//! Write a host, build a [`Sandbox`], and give the model the `execute` and `poll` definitions.
//! Pass each `execute` call's code to [`Sandbox::execute`] and send the [`Status`] JSON back:
//!
//! ```no_run
//! use futures_util::future::{BoxFuture, FutureExt};
//! use ghostmode::host::{self, Call, Host, Tool};
//! # #[cfg(feature = "monty")]
//! use ghostmode::backend::Monty;
//! use ghostmode::{Builder, schema};
//! use serde_json::{Value, json};
//!
//! struct Echo;
//!
//! impl Host for Echo {
//!     fn tools(&self) -> Vec<Tool> {
//!         let input_schema = json!({
//!             "type": "object",
//!             "properties": { "text": { "type": "string" } },
//!             "required": ["text"],
//!         });
//!         vec![Tool::new("echo", "Returns the text you pass it.", input_schema)]
//!     }
//!
//!     fn call(&self, call: &Call) -> BoxFuture<'static, Result<Value, host::Error>> {
//!         let text = call.args.get("text").cloned().ok_or_else(|| host::Error::Failed("text is required".into()));
//!         async move { text }.boxed()
//!     }
//! }
//!
//! # #[cfg(feature = "monty")]
//! # async fn execute_echo() {
//! let host = Echo;
//! let definitions = schema::list_mcp_tools(&host.tools());
//! let mut sandbox = Builder::new(host, Monty).build().await;
//! let status = sandbox.execute("echo(text='hello')", false).await;
//! let reply = serde_json::to_value(&status).unwrap();
//! # }
//! ```
//!
//! A sandbox runs scripts in a [`backend::Backend`]. Each feature adds a backend or a host:
//!
//! - `monty`, on by default, adds `backend::Monty`, which runs scripts inside the caller's
//!   process. It isn't crash-isolated: a script can panic, abort or hang the caller's process.
//! - `monty-pool` adds `backend::MontyPool`, which runs each session in its own Monty worker
//!   process. Untrusted scripts belong there.
//! - `rmcp` adds `mcp::Server`, a host that offers a connected MCP server's tools to scripts.

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod backend;
pub mod host;
#[cfg(feature = "rmcp")]
pub mod mcp;
mod sandbox;
pub mod schema;

pub use sandbox::{Builder, Error, Limits, Sandbox, Status};

#[cfg(all(doctest, feature = "monty"))]
#[doc = include_str!("../docs/extending.md")]
struct ExtendingGuideDoctests;
