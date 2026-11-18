//! What the host provides: the tools a script calls, and the answers to those calls.

use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::{Map, Value};

/// The caller's side of the sandbox: it lists the tools a script may call and answers each
/// call, normally by forwarding it to the client's MCP tools.
pub trait Host {
    /// The tools a script may call, by id.
    fn tools(&self) -> Vec<Tool>;

    /// Answers one tool call from a script. The run waits for the future, and a run that
    /// outlives its wait keeps the future until a later `poll` finds it ready.
    fn call(&self, call: Call) -> BoxFuture<'static, Result<Value, Error>>;
}

/// A tool the host provides. A script calls it by its id, with keyword arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// The name a script calls the tool by.
    pub id: String,
    /// What the tool does, for the model.
    pub description: String,
    /// The JSON Schema of the tool's keyword arguments.
    pub input_schema: Value,
}

/// A tool call from a script to the host, and an entry in a failed status's `calls`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Call {
    /// The id of the tool the script called.
    pub tool_id: String,
    /// The call's keyword arguments.
    pub args: Map<String, Value>,
}

/// Why the host answered a call with no value.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// The host would not run the call. The script sees a `PermissionError` with this reason.
    Refused(String),
    /// The call ran and failed. The script sees a `RuntimeError` with this message.
    Failed(String),
}
