//! The backend that runs scripts in Monty inside the caller's process. This is the only module
//! that talks to Monty.
//!
//! In-process Monty is not crash-isolated: a script, or a dump restored without a signing key,
//! can panic, abort or hang the caller's process.

#![cfg(feature = "monty")]

use std::{error, mem};

pub use ::monty::DumpError;
use ::monty::{
    DUMP_VERSION, Dump, MontyRepl, ReplFunctionCall, ReplProgress, ReplStartError, Session,
    SessionRef,
};
use monty_types::{
    CollectedStreams, CompileOptions, ExcType, ExtFunctionResult, MontyException, NameLookupResult,
    OsPolicy, PrintStream, PrintWriter, ResourceLimits, ResourceTracker, SleepMode,
};
use serde_json::Value;

use crate::backend::{Backend, Ending, Progress, Streams};
use crate::host;
use crate::sandbox::Limits;

mod value;

/// The file name a script's traceback names.
const SCRIPT_NAME: &str = "script.py";

/// Runs scripts in a Monty session inside the caller's process. [`Sandbox::builder`] uses it.
///
/// [`Sandbox::builder`]: crate::Sandbox::builder
pub struct Monty;

/// A script suspended at a tool call in [`Monty`], with what it has printed so far.
pub struct Suspension {
    function_call: Box<ReplFunctionCall>,
    streams: CollectedStreams,
}

impl Backend for Monty {
    type Session = Box<MontyRepl>;
    type Suspension = Suspension;

    async fn new_session(&self, limits: &Limits) -> Box<MontyRepl> {
        let tracker = ResourceTracker::default();
        let mut session = Box::new(MontyRepl::new(
            SCRIPT_NAME,
            tracker,
            CompileOptions::default(),
        ));
        set_limits_and_os_policy(&mut session, limits);
        session
    }

    async fn feed_start(
        &self,
        session: Box<MontyRepl>,
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
            Ok(answer) => ExtFunctionResult::Return(value::from_json(&answer)),
            Err(host::Error::Refused(reason)) => ExtFunctionResult::Error(MontyException::new(
                ExcType::PermissionError,
                Some(reason),
            )),
            Err(host::Error::Failed(message)) => {
                ExtFunctionResult::Error(MontyException::new(ExcType::RuntimeError, Some(message)))
            }
        };
        let monty_progress =
            function_call.resume(answer, PrintWriter::collect_streams(&mut streams));
        self.run(monty_progress, streams, tool_ids)
    }

    async fn abort(
        &self,
        suspension: Suspension,
        message: String,
        tool_ids: &[String],
    ) -> Ending<Self> {
        let Suspension {
            function_call,
            mut streams,
        } = suspension;
        let error = MontyException::new(ExcType::RuntimeError, Some(message));
        let monty_progress = function_call.abort(error, PrintWriter::collect_streams(&mut streams));
        match self.run(monty_progress, streams, tool_ids) {
            Progress::Ended(ending) => ending,
            Progress::Call { .. } => unreachable!("Monty's abort always ends the script"),
        }
    }

    async fn abandon(&self, suspension: Suspension) -> (Box<MontyRepl>, Streams) {
        let session = Box::new(suspension.function_call.into_repl());
        (session, split_streams(suspension.streams))
    }

    async fn dump(&self, session: &mut Box<MontyRepl>) -> Vec<u8> {
        ::monty::dump(SCRIPT_NAME, None, &[], SessionRef::Idle(session))
            .expect("Monty serializes every idle session")
    }

    /// Bytes Monty can't load are a [`DumpError`]. A dump of a suspended or one-shot run is
    /// [`DumpError::Unsupported`], whose `found` is this build's dump version.
    async fn restore(
        &self,
        bytes: &[u8],
        limits: &Limits,
    ) -> Result<Box<MontyRepl>, Box<dyn error::Error + Send + Sync>> {
        let Session::Idle(mut session) = Dump::load(bytes)?.state else {
            return Err(Box::new(DumpError::Unsupported {
                found: DUMP_VERSION,
                reason: "the dump holds a run, and ghostmode restores only an idle session"
                    .to_string(),
            }));
        };
        set_limits_and_os_policy(&mut session, limits);
        Ok(session)
    }
}

impl Monty {
    /// Runs the script until it calls one of the host's tools, whose ids are `tool_ids`, with
    /// keyword arguments, or ends. Every other suspension is answered here: an unknown name
    /// raises `NameError`, an OS call gets Monty's no-handler answer, and awaiting a tool raises
    /// `RuntimeError`. What the script prints is added to `streams`.
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
                        session: Box::new(failure.repl),
                    });
                }
                Ok(ReplProgress::Complete { repl, value }) => {
                    return Progress::Ended(Ending {
                        result: Ok(value::to_json(value.as_ref())),
                        streams: split_streams(streams),
                        session: Box::new(repl),
                    });
                }
                Ok(ReplProgress::FunctionCall(mut function_call))
                    if tool_ids.contains(&function_call.function_name) =>
                {
                    let positional = function_call.args.args().len();
                    if positional > 0 {
                        let message = format!(
                            "{}() takes keyword arguments only",
                            function_call.function_name
                        );
                        let error = MontyException::new(ExcType::TypeError, Some(message));
                        function_call.resume(
                            ExtFunctionResult::Error(error),
                            PrintWriter::collect_streams(&mut streams),
                        )
                    } else {
                        let args = function_call.args.kwargs().map(|(key, value)| {
                            let key = key.as_str().expect("kwargs keys are str").to_string();
                            (key, value::to_json(value))
                        });
                        let args = args.collect();
                        let call = host::Call {
                            tool_id: mem::take(&mut function_call.function_name),
                            args,
                        };
                        let suspension = Suspension {
                            function_call: Box::new(function_call),
                            streams,
                        };
                        return Progress::Call { call, suspension };
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
                    let error = MontyException::runtime_error("tools are not awaitable");
                    futures.abort(error, PrintWriter::collect_streams(&mut streams))
                }
            };
        }
    }
}

/// Makes `session` stop a script at `limits`' run time, memory and recursion depth, give every
/// OS call Monty's no-handler answer, and return from `time.sleep` at once.
fn set_limits_and_os_policy(session: &mut MontyRepl, limits: &Limits) {
    let resource_limits = ResourceLimits {
        max_feed_duration: Some(limits.run_time),
        max_memory: Some(limits.memory_bytes),
        max_recursion_depth: limits.recursion_depth,
        ..ResourceLimits::default()
    };
    *session.tracker_mut() = ResourceTracker::new(resource_limits);
    session.set_os_policy(OsPolicy {
        sleep: SleepMode::Zero,
        ..OsPolicy::default()
    });
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
