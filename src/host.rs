//! What the host provides: the tools a script calls, and the answers to those calls.

use std::{error, fmt};

use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::{Map, Value};

/// The tools a script may call, and the answers to its calls. The host lives in the caller's
/// process.
pub trait Host {
    /// The tools a script may call, by id.
    fn tools(&self) -> Vec<Tool>;

    /// Answers one tool call from a script. The future clones what it needs from `call`. The
    /// run waits for the future, and a run that outlives its wait keeps the future until a
    /// later `poll` finds it ready. The sandbox polls the future only while the caller awaits
    /// `execute` or `poll`, so work that must go on between them belongs in a task the host
    /// spawns.
    fn call(&self, call: &Call) -> BoxFuture<'static, Result<Value, Error>>;
}

/// A tool the host provides. A script calls it by its id, with keyword arguments.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Tool {
    /// The id a script calls the tool by, as a Python function. It must be a Python identifier
    /// that is neither a keyword nor a builtin's name, as [`crate::schema::is_tool_id`] checks.
    /// [`crate::schema::mcp_definitions`] leaves out any tool whose id isn't one.
    pub id: String,
    /// What the tool does, for the model.
    pub description: String,
    /// The JSON Schema of the tool's keyword arguments.
    pub input_schema: Value,
}

impl Tool {
    /// A tool a script calls by `id`, which the model reads about in `description`, and whose
    /// keyword arguments follow the JSON Schema `input_schema`.
    pub fn new(id: impl Into<String>, description: impl Into<String>, input_schema: Value) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// A tool call from a script to the host, and an entry in a failed status's `calls`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Call {
    /// The id of the tool the script called.
    pub tool_id: String,
    /// The call's keyword arguments.
    pub args: Map<String, Value>,
    /// The arguments a failed status dropped from `args` to fit the output budget, as their JSON
    /// text truncated around a marker, serialized as the `truncated_args` key in the call's JSON.
    /// `None`, and left out of the JSON, if none were dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) truncated_args: Option<String>,
}

impl Call {
    /// A call to the tool `tool_id` with the keyword arguments `args`.
    pub fn new(tool_id: impl Into<String>, args: Map<String, Value>) -> Self {
        Self {
            tool_id: tool_id.into(),
            args,
            truncated_args: None,
        }
    }
}

/// Why the host answered a call with no value, from the host to the script.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The host would not run the call. The script sees a `PermissionError` with this reason.
    Refused(String),
    /// The call ran and failed. The script sees a `RuntimeError` with this message.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(reason) => f.write_str(reason),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl error::Error for Error {}
