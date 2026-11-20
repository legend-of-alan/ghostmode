//! The backend that runs scripts in Monty inside the caller's process. This is the only module
//! that calls the `monty` crate.
//!
//! In-process Monty is not crash-isolated: a script, or a dump restored without a signing key,
//! can panic, abort or hang the caller's process. Untrusted scripts belong in `backend::MontyPool`,
//! which runs each session in a worker process.

use std::{error, fmt, mem};

use ::monty::{Dump, MontyRepl, ReplFunctionCall, ReplProgress, ReplStartError, SessionRef};
use monty_types::{
    CollectedStreams, CompileOptions, ExtFunctionResult, MontyException, NameLookupResult,
    PrintStream, PrintWriter, ResourceTracker,
};
use serde_json::Value;

use super::{SCRIPT_NAME, value};
use crate::backend::{Backend, Ending, Progress, Streams};
use crate::host;
use crate::sandbox::Limits;

/// Runs scripts in a Monty session inside the caller's process.
#[derive(Clone, Copy, Debug, Default)]
pub struct Monty;

/// A Python session in [`Monty`], holding the globals its scripts set.
pub struct Session(Box<MontyRepl>);

/// A script suspended at a tool call in [`Monty`], with what it has printed so far.
pub struct Suspension {
    function_call: Box<ReplFunctionCall>,
    streams: CollectedStreams,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

impl fmt::Debug for Suspension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Suspension")
            .field("streams", &self.streams)
            .finish_non_exhaustive()
    }
}

impl Backend for Monty {
    type Session = Session;
    type Suspension = Suspension;

    async fn new_session(&self, limits: &Limits) -> Session {
        let tracker = ResourceTracker::default();
        let mut repl = MontyRepl::new(SCRIPT_NAME, tracker, CompileOptions::default());
        set_limits_and_os_policy(&mut repl, limits);
        Session(Box::new(repl))
    }

    async fn feed_start(
        &self,
        Session(session): Session,
        code: &str,
        tool_ids: &[String],
    ) -> Progress<Self> {
        let mut streams = CollectedStreams::default();
        let monty_progress =
            session.feed_start(code, Vec::new(), PrintWriter::collect_streams(&mut streams));
        self.run(monty_progress, streams, tool_ids)
    }

    async fn resume(
        &self,
        suspension: Suspension,
        answer: Result<Value, host::Error>,
        tool_ids: &[String],
    ) -> Progress<Self> {
        let Suspension {
            function_call,
            mut streams,
        } = suspension;
        let answer = match answer {
            Ok(answer) => ExtFunctionResult::Return(value::from_json(answer)),
            Err(error) => ExtFunctionResult::Error(value::to_exception(error)),
        };
        let monty_progress =
            function_call.resume(answer, PrintWriter::collect_streams(&mut streams));
        self.run(monty_progress, streams, tool_ids)
    }

    async fn abort(&self, suspension: Suspension, message: String) -> Ending<Self> {
        let Suspension {
            function_call,
            mut streams,
        } = suspension;
        let error = MontyException::runtime_error(message);
        let monty_progress = function_call.abort(error, PrintWriter::collect_streams(&mut streams));
        match self.run(monty_progress, streams, &[]) {
            Progress::Ended(ending) => ending,
            Progress::Call { .. } => unreachable!("Monty's abort always ends the script"),
        }
    }

    async fn abandon(&self, suspension: Suspension, message: String) -> Ending<Self> {
        Ending {
            result: Err(message),
            streams: split_streams(suspension.streams),
            session: Session(Box::new(suspension.function_call.into_repl())),
            lost_globals: false,
        }
    }

    async fn dump(
        &self,
        Session(session): &mut Session,
    ) -> (Result<Vec<u8>, Box<dyn error::Error + Send + Sync>>, bool) {
        let bytes = ::monty::dump(SCRIPT_NAME, None, &[], SessionRef::Idle(session));
        (bytes.map_err(Into::into), false)
    }

    async fn restore(
        &self,
        bytes: &[u8],
        limits: &Limits,
    ) -> Result<Session, Box<dyn error::Error + Send + Sync>> {
        let ::monty::Session::Idle(mut session) = Dump::load(bytes)?.state else {
            return Err(super::DUMP_HOLDS_RUN.into());
        };
        set_limits_and_os_policy(&mut session, limits);
        Ok(Session(session))
    }
}

impl Monty {
    /// Runs the script until it calls one of the host's tools, whose ids are `tool_ids`, with
    /// keyword arguments, or ends. Every other suspension is answered here: an unknown name
    /// raises `NameError`, and an OS call gets Monty's no-handler answer. What the script prints
    /// is added to `streams`.
    fn run(
        &self,
        mut monty_progress: Result<ReplProgress, Box<ReplStartError>>,
        mut streams: CollectedStreams,
        tool_ids: &[String],
    ) -> Progress<Self> {
        loop {
            monty_progress = match monty_progress {
                Err(failure) => {
                    return Progress::Ended(Ending {
                        result: Err(failure.error.to_string()),
                        streams: split_streams(streams),
                        session: Session(Box::new(failure.repl)),
                        lost_globals: false,
                    });
                }
                Ok(ReplProgress::Complete { repl, value }) => {
                    return Progress::Ended(Ending {
                        result: Ok(value::to_json(value.as_ref())),
                        streams: split_streams(streams),
                        session: Session(Box::new(repl)),
                        lost_globals: false,
                    });
                }
                Ok(ReplProgress::FunctionCall(mut function_call))
                    if tool_ids.contains(&function_call.function_name) =>
                {
                    let tool_id = mem::take(&mut function_call.function_name);
                    match value::to_call(tool_id, &function_call.args) {
                        Ok(call) => {
                            let suspension = Suspension {
                                function_call: Box::new(function_call),
                                streams,
                            };
                            return Progress::Call { call, suspension };
                        }
                        Err(error) => function_call.resume(
                            ExtFunctionResult::Error(error),
                            PrintWriter::collect_streams(&mut streams),
                        ),
                    }
                }
                Ok(ReplProgress::FunctionCall(mut function_call)) => {
                    let answer =
                        ExtFunctionResult::NotFound(mem::take(&mut function_call.function_name));
                    function_call.resume(answer, PrintWriter::collect_streams(&mut streams))
                }
                Ok(ReplProgress::NameLookup(lookup)) => lookup.resume(
                    NameLookupResult::Undefined,
                    PrintWriter::collect_streams(&mut streams),
                ),
                Ok(ReplProgress::OsCall(os_call)) => {
                    let answer = os_call.function_call.no_handler_result();
                    os_call.resume(answer, PrintWriter::collect_streams(&mut streams))
                }
                Ok(ReplProgress::ResolveFutures(futures)) => {
                    let error = MontyException::runtime_error(super::TOOLS_NOT_AWAITABLE);
                    futures.abort(error, PrintWriter::collect_streams(&mut streams))
                }
            };
        }
    }
}

/// Makes `session` stop a script at `limits`' run time, memory and recursion depth, and return
/// from `time.sleep` at once.
fn set_limits_and_os_policy(session: &mut MontyRepl, limits: &Limits) {
    *session.tracker_mut() = ResourceTracker::new(super::to_resource_limits(limits));
    session.set_os_policy(super::build_os_policy());
}

/// What a script printed into `streams`, split into stdout and stderr.
fn split_streams(streams: CollectedStreams) -> Streams {
    let mut split = Streams::default();
    for (stream, text) in streams.into_entries() {
        match stream {
            PrintStream::Stdout => split.stdout.push_str(&text),
            PrintStream::Stderr => split.stderr.push_str(&text),
        }
    }
    split
}
