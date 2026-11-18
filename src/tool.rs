//! The model's two tools, `execute` and `poll`: what the model sends and the status it reads.

use serde::Serialize;
use serde_json::Value;

use crate::host;

/// The model's `execute` tool call: run this script in the sandbox's session.
#[derive(Clone, Debug, PartialEq)]
pub struct Execute {
    /// The Python script.
    pub code: String,
}

/// The model's `poll` tool call: wait again for a run that `execute` left running.
#[derive(Clone, Debug, PartialEq)]
pub struct Poll {
    /// The run id a `running` status gave.
    pub run_id: u64,
}

/// What the model reads back from `execute` or `poll`. It serializes as the JSON the model
/// sees, such as `{"status": "running", "run_id": 1}`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    /// The script ended, and this is the value of its last expression.
    Finished {
        /// The value as JSON.
        value: Value,
    },
    /// A tool call was still unanswered when the wait ended. Poll this run id for the rest.
    Running {
        /// The id to poll.
        run_id: u64,
    },
    /// The script or the request failed.
    Failed {
        /// What went wrong, such as the script's traceback.
        error: String,
        /// The tool calls the host answered before the failure.
        calls: Vec<host::Call>,
    },
}
