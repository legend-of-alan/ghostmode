//! The Python session a sandbox runs scripts in. This is the only module that talks to Monty.

use monty::{MontyRepl, ReplFunctionCall, ReplProgress, ReplStartError};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyException, NameLookupResult, OsPolicy,
    PrintWriter, ResourceTracker, SleepMode,
};
use serde_json::Value;

use crate::{host, value};

/// What Monty's `feed_start` and `resume` return.
type MontyProgress = Result<ReplProgress, Box<ReplStartError>>;

/// A Python session whose globals persist from one script to the next. A script that calls a
/// tool suspends at the call until [`Repl::resume`] gives the host's answer.
pub(crate) struct Repl {
    tool_ids: Vec<String>,
    session: Option<Session>,
}

/// Where the session is: idle between scripts, or suspended at a tool call.
enum Session {
    Idle(Box<MontyRepl>),
    Suspended(Box<ReplFunctionCall>),
}

/// How far a script got, from the repl to the sandbox: to a tool call it waits on, or to its end.
pub(crate) enum Progress {
    Call(host::Call),
    Finished(Value),
    Failed(String),
}

impl Repl {
    /// An empty session whose scripts may call the tools with these ids. Every OS call gets
    /// Monty's no-handler answer, `time.sleep` returns at once and prints are discarded.
    pub(crate) fn new(tool_ids: Vec<String>) -> Self {
        let os_policy = OsPolicy {
            sleep: SleepMode::Zero,
            ..OsPolicy::default()
        };
        let repl = MontyRepl::new(
            "script.py",
            ResourceTracker::default(),
            CompileOptions::default(),
        )
        .with_os_policy(os_policy);
        Self {
            tool_ids,
            session: Some(Session::Idle(Box::new(repl))),
        }
    }

    /// Feeds `code` to the session and runs it.
    ///
    /// # Panics
    /// If a script is still waiting on a tool call.
    pub(crate) fn feed_start(&mut self, code: &str) -> Progress {
        let Some(Session::Idle(repl)) = self.session.take() else {
            panic!("feed_start while a script waits on a tool call");
        };
        let progress = (*repl).feed_start(code, Vec::new(), PrintWriter::Disabled);
        self.run(progress)
    }

    /// Answers the tool call the script waits on, and runs on.
    ///
    /// # Panics
    /// If no script waits on a tool call.
    pub(crate) fn resume(&mut self, answer: Result<Value, host::Error>) -> Progress {
        let Some(Session::Suspended(call)) = self.session.take() else {
            panic!("resume while no script waits on a tool call");
        };
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
        self.run((*call).resume(answer, PrintWriter::Disabled))
    }

    /// Runs the script until it calls one of the host's tools with keyword arguments, or ends.
    fn run(&mut self, mut progress: MontyProgress) -> Progress {
        loop {
            progress = match progress {
                Err(failure) => {
                    self.session = Some(Session::Idle(Box::new(failure.repl)));
                    return Progress::Failed(failure.error.to_string());
                }
                Ok(ReplProgress::Complete { repl, value }) => {
                    self.session = Some(Session::Idle(Box::new(repl)));
                    return Progress::Finished(value::to_json(value.as_ref()));
                }
                Ok(ReplProgress::FunctionCall(call))
                    if self.tool_ids.contains(&call.function_name) =>
                {
                    if call.args.args().len() > 0 {
                        let message =
                            format!("{}() takes keyword arguments only", call.function_name);
                        let error = MontyException::new(ExcType::TypeError, Some(message));
                        call.resume(ExtFunctionResult::Error(error), PrintWriter::Disabled)
                    } else {
                        let args = call.args.kwargs().map(|(key, value)| {
                            let key = key.as_str().unwrap_or_default().to_string();
                            (key, value::to_json(value))
                        });
                        let tool_call = host::Call {
                            tool_id: call.function_name.clone(),
                            args: args.collect(),
                        };
                        self.session = Some(Session::Suspended(Box::new(call)));
                        return Progress::Call(tool_call);
                    }
                }
                Ok(suspension) => resume_with_no_handler(suspension),
            };
        }
    }
}

/// Resumes a suspension that no host tool handles with the answer Monty gives when no handler
/// accepts it: `NameError` for an unknown name and Monty's own answer for an OS call.
fn resume_with_no_handler(suspension: ReplProgress) -> MontyProgress {
    match suspension {
        ReplProgress::NameLookup(lookup) => {
            lookup.resume(NameLookupResult::Undefined, PrintWriter::Disabled)
        }
        ReplProgress::FunctionCall(call) => {
            let answer = ExtFunctionResult::NotFound(call.function_name.clone());
            call.resume(answer, PrintWriter::Disabled)
        }
        ReplProgress::OsCall(call) => {
            let answer = call.function_call.no_handler_result();
            call.resume(answer, PrintWriter::Disabled)
        }
        ReplProgress::ResolveFutures(futures) => {
            let error = MontyException::runtime_error("tools are not awaitable");
            futures.abort(error, PrintWriter::Disabled)
        }
        ReplProgress::Complete { repl, value } => Ok(ReplProgress::Complete { repl, value }),
    }
}
