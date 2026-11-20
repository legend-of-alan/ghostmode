//! What a sandbox needs from the Python interpreter that runs its scripts.

use std::error;

use serde_json::Value;

use crate::host;
use crate::sandbox::Limits;

#[cfg(feature = "monty")]
pub use self::monty::{DumpError, Monty};

pub mod monty;

/// Runs a sandbox's scripts in Python sessions whose globals persist from one script to the
/// next. It holds no session: each method takes the idle session, or the script suspended at a
/// tool call, by value, and its [`Progress`] hands that on.
pub trait Backend: Sized {
    /// A Python session between scripts, holding the globals they set.
    type Session: Send;
    /// A script suspended at a tool call, with what it has printed so far.
    type Suspension: Send;

    /// An empty idle session that stops a script at `limits`' run time, memory and recursion
    /// depth.
    fn new_session(&self, limits: &Limits) -> impl Future<Output = Self::Session> + Send;

    /// Feeds `code` to the idle `session` and runs it until it calls one of the host's tools,
    /// whose ids are `tool_ids`, or ends.
    fn feed_start(
        &self,
        session: Self::Session,
        code: &str,
        tool_ids: &[String],
    ) -> impl Future<Output = Progress<Self>> + Send;

    /// Answers the tool call the script is suspended at with `answer`, and runs on until the
    /// next call to one of `tool_ids`, or the end. A [`host::Error::Refused`] raises
    /// `PermissionError` in the script, and a [`host::Error::Failed`] raises `RuntimeError`.
    fn resume(
        &self,
        suspension: Self::Suspension,
        answer: Result<Value, host::Error>,
        tool_ids: &[String],
    ) -> impl Future<Output = Progress<Self>> + Send;

    /// Ends the suspended script with a `RuntimeError` carrying `message`, which the script
    /// can't catch, and gives back how it ended. `tool_ids` are the host's tool ids, as in
    /// [`Backend::feed_start`].
    fn abort(
        &self,
        suspension: Self::Suspension,
        message: String,
        tool_ids: &[String],
    ) -> impl Future<Output = Ending<Self>> + Send;

    /// Drops the suspended script without running it further, and gives back its idle session
    /// with the globals the script had set, and what it printed.
    fn abandon(
        &self,
        suspension: Self::Suspension,
    ) -> impl Future<Output = (Self::Session, Streams)> + Send;

    /// The bytes of the idle `session`, which [`Backend::restore`] reads back.
    fn dump(&self, session: &mut Self::Session) -> impl Future<Output = Vec<u8>> + Send;

    /// The idle session that `bytes` from [`Backend::dump`] hold, running under `limits` like a
    /// [new session](Backend::new_session), whatever limits it was dumped with.
    fn restore(
        &self,
        bytes: &[u8],
        limits: &Limits,
    ) -> impl Future<Output = Result<Self::Session, Box<dyn error::Error + Send + Sync>>> + Send;
}

/// How far a script got, from the backend to the sandbox.
pub enum Progress<B: Backend> {
    /// The script is suspended at this tool call until [`Backend::resume`] gives the host's
    /// answer.
    Call {
        /// The tool call the script made with keyword arguments, to one of the host's tools.
        call: host::Call,
        /// The script, suspended at `call`.
        suspension: B::Suspension,
    },
    /// The script ended, and the session is idle again.
    Ended(Ending<B>),
}

/// How a script ended, from the backend to the sandbox.
pub struct Ending<B: Backend> {
    /// The value of the script's last expression, or its traceback.
    pub result: Result<Value, String>,
    /// What the script printed.
    pub streams: Streams,
    /// The idle session, with the globals the script set.
    pub session: B::Session,
}

/// What a script printed, each stream in the order it was printed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Streams {
    /// What the script printed to stdout.
    pub stdout: String,
    /// What the script printed to stderr.
    pub stderr: String,
}
