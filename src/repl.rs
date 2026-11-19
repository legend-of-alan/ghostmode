//! The Python session a sandbox runs scripts in. This is the only module that talks to Monty.

use std::mem;

use monty::{DUMP_VERSION, Dump, ReplProgress, ReplStartError, Session, SessionRef};
pub(crate) use monty::{DumpError, MontyRepl, ReplFunctionCall};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyException, NameLookupResult, OsPolicy,
    PrintWriter, ResourceLimits, ResourceTracker, SleepMode,
};
use serde_json::Value;

use crate::sandbox::Limits;
use crate::{host, value};

/// The file name a script's traceback names.
const SCRIPT_NAME: &str = "script.py";

/// Runs scripts in a Monty session whose globals persist from one script to the next. It holds
/// no session: each call takes the idle session, or the call a script is suspended at, by value,
/// and its [`Progress`] hands that on.
pub(crate) struct Repl {
    tool_ids: Vec<String>,
}

/// How far a script got, from the repl to the sandbox.
pub(crate) enum Progress {
    /// The script is suspended at this tool call until [`Repl::resume`] gives the host's answer.
    Call {
        call: host::Call,
        function_call: Box<ReplFunctionCall>,
    },
    /// The script ended with its value or its error, and the session is idle again.
    Ended {
        result: Result<Value, String>,
        session: Box<MontyRepl>,
    },
}

impl Repl {
    /// A repl whose scripts may call the tools with these ids.
    pub(crate) fn new(tool_ids: Vec<String>) -> Self {
        Self { tool_ids }
    }

    /// An empty idle session that runs under [`Repl::set_limits_and_os_policy`].
    pub(crate) fn new_session(limits: &Limits) -> Box<MontyRepl> {
        let tracker = ResourceTracker::default();
        let mut session = Box::new(MontyRepl::new(
            SCRIPT_NAME,
            tracker,
            CompileOptions::default(),
        ));
        Self::set_limits_and_os_policy(&mut session, limits);
        session
    }

    /// The bytes of the idle `session`, in Monty's dump format, which [`Repl::load_session`]
    /// reads back.
    pub(crate) fn dump_session(session: &MontyRepl) -> Vec<u8> {
        monty::dump(SCRIPT_NAME, None, &[], SessionRef::Idle(session))
            .expect("Monty serializes every idle session")
    }

    /// The idle session that `bytes` from [`Repl::dump_session`] hold, running under
    /// [`Repl::set_limits_and_os_policy`] whatever limits it was dumped with. A dump of a
    /// suspended or one-shot run is [`DumpError::Unsupported`].
    pub(crate) fn load_session(bytes: &[u8], limits: &Limits) -> Result<Box<MontyRepl>, DumpError> {
        let Session::Idle(mut session) = Dump::load(bytes)?.state else {
            return Err(DumpError::Unsupported {
                found: DUMP_VERSION,
                reason: "ghostmode restores only an idle session".to_string(),
            });
        };
        Self::set_limits_and_os_policy(&mut session, limits);
        Ok(session)
    }

    /// Makes `session` stop a script at `limits`' run time, memory and recursion depth, give
    /// every OS call Monty's no-handler answer, and return from `time.sleep` at once.
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

    /// Feeds `code` to the idle `session` and runs it, adding what it prints to `printed`.
    pub(crate) fn feed_start(
        &self,
        session: Box<MontyRepl>,
        code: &str,
        printed: &mut String,
    ) -> Progress {
        let monty_progress =
            session.feed_start(code, Vec::new(), PrintWriter::collect_string(printed));
        self.run(monty_progress, printed)
    }

    /// Answers the tool call a script is suspended at, and runs on, adding what it prints to
    /// `printed`.
    pub(crate) fn resume(
        &self,
        function_call: Box<ReplFunctionCall>,
        answer: Result<Value, host::Error>,
        printed: &mut String,
    ) -> Progress {
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
        let monty_progress = function_call.resume(answer, PrintWriter::collect_string(printed));
        self.run(monty_progress, printed)
    }

    /// Ends the script suspended at `function_call` with a `RuntimeError` carrying `message`,
    /// which the script can't catch, adding what it prints to `printed`.
    pub(crate) fn abort(
        &self,
        function_call: Box<ReplFunctionCall>,
        message: String,
        printed: &mut String,
    ) -> Progress {
        let error = MontyException::new(ExcType::RuntimeError, Some(message));
        let monty_progress = function_call.abort(error, PrintWriter::collect_string(printed));
        self.run(monty_progress, printed)
    }

    /// Drops the script suspended at `function_call` without running it further, and gives back
    /// its idle session with the globals the script had set.
    pub(crate) fn abandon(function_call: Box<ReplFunctionCall>) -> Box<MontyRepl> {
        Box::new(function_call.into_repl())
    }

    /// Runs the script until it calls one of the host's tools with keyword arguments, or ends.
    /// Every other suspension is answered here: an unknown name raises `NameError`, an OS call
    /// gets Monty's no-handler answer, and awaiting a tool raises `RuntimeError`. What the script
    /// prints is added to `printed`.
    fn run(
        &self,
        mut monty_progress: Result<ReplProgress, Box<ReplStartError>>,
        printed: &mut String,
    ) -> Progress {
        loop {
            monty_progress = match monty_progress {
                Err(failure) => {
                    return Progress::Ended {
                        result: Err(failure.error.to_string()),
                        session: Box::new(failure.repl),
                    };
                }
                Ok(ReplProgress::Complete { repl, value }) => {
                    return Progress::Ended {
                        result: Ok(value::to_json(value.as_ref())),
                        session: Box::new(repl),
                    };
                }
                Ok(ReplProgress::FunctionCall(mut function_call))
                    if self.tool_ids.contains(&function_call.function_name) =>
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
                            PrintWriter::collect_string(printed),
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
                        return Progress::Call {
                            call,
                            function_call: Box::new(function_call),
                        };
                    }
                }
                Ok(ReplProgress::FunctionCall(mut function_call)) => {
                    let answer =
                        ExtFunctionResult::NotFound(mem::take(&mut function_call.function_name));
                    function_call.resume(answer, PrintWriter::collect_string(printed))
                }
                Ok(ReplProgress::NameLookup(lookup)) => lookup.resume(
                    NameLookupResult::Undefined,
                    PrintWriter::collect_string(printed),
                ),
                Ok(ReplProgress::OsCall(os_call)) => {
                    let answer = os_call.function_call.no_handler_result();
                    os_call.resume(answer, PrintWriter::collect_string(printed))
                }
                Ok(ReplProgress::ResolveFutures(futures)) => {
                    let error = MontyException::runtime_error("tools are not awaitable");
                    futures.abort(error, PrintWriter::collect_string(printed))
                }
            };
        }
    }
}
