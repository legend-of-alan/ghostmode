//! The sandbox that answers the model's `execute` and `poll` tool calls.

use std::time::{Duration, Instant};

use futures_timer::Delay;
use futures_util::future::{self, BoxFuture, Either, FutureExt};
use serde::Serialize;
use serde_json::Value;
use tracing::field::{self, Empty};
use tracing::{Instrument, Span};

use crate::host::{self, Host};
use crate::repl::{MontyRepl, Progress, Repl, ReplFunctionCall};

/// One Python session that runs the model's scripts, and the host whose tools they call.
/// Globals persist from one `execute` to the next, and one run is in flight at a time.
pub struct Sandbox<H> {
    host: H,
    repl: Repl,
    state: Option<State>,
    last_run_id: u64,
    execute_wait: Duration,
    poll_wait: Duration,
    limits: Limits,
}

/// Sets up a [`Sandbox`]. Made by [`Sandbox::builder`].
pub struct Builder<H> {
    host: H,
    execute_wait: Duration,
    poll_wait: Duration,
    limits: Limits,
}

/// What a script may use before the sandbox stops it with a `failed` status. A stopped script
/// leaves the session's globals as it had set them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// How long the script itself may run, summed over `execute` and every `poll` of its run
    /// and not counting time spent waiting on the host. The default is 30 s.
    pub run_time: Duration,
    /// The most memory the script may use. In-process this refuses only a single allocation
    /// over the limit, such as one huge string, with `MemoryError`. The default is 256 MiB.
    pub memory_bytes: usize,
    /// How deep the script's calls may nest before it raises `RecursionError`. The default is
    /// 1,000.
    pub recursion_depth: usize,
    /// How many tool calls the script of one `execute` may make, over all its polls. The
    /// default is 100.
    pub calls_per_execute: usize,
    /// How long one tool call may wait for the host's answer. The default is unset, no limit.
    pub call_timeout: Option<Duration>,
    /// How long a run that `execute` or `poll` left `running` may wait for the next `execute` or
    /// `poll` before it expires. The default is unset, no limit.
    pub idle_timeout: Option<Duration>,
}

/// What `execute` or `poll` answers, from the sandbox to the model. It serializes as the JSON
/// the model sees, such as `{"status": "running", "run_id": 1}`.
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

/// Where the sandbox's session is: idle between scripts, or held by a suspended run.
enum State {
    Idle(Box<MontyRepl>),
    Suspended(Run),
}

/// A script suspended at a tool call the host has not answered yet.
struct Run {
    id: u64,
    answered_calls: Vec<host::Call>,
    call: host::Call,
    function_call: Box<ReplFunctionCall>,
    /// The host's answer to `call`, or `None` once the call timeout passes first.
    answer: BoxFuture<'static, Option<Result<Value, host::Error>>>,
    idle_since: Instant,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            run_time: Duration::from_secs(30),
            memory_bytes: 256 * 1024 * 1024,
            recursion_depth: 1000,
            calls_per_execute: 100,
            call_timeout: None,
            idle_timeout: None,
        }
    }
}

impl<H: Host> Sandbox<H> {
    /// A builder for a sandbox whose scripts call `host`'s tools.
    pub fn builder(host: H) -> Builder<H> {
        Builder {
            host,
            execute_wait: Duration::from_secs(30),
            poll_wait: Duration::from_secs(10),
            limits: Limits::default(),
        }
    }

    /// Runs `code`, waiting up to the execute wait for its tool calls. The wait bounds only the
    /// time spent waiting on the host; [`Limits::run_time`] bounds the script's own time. A
    /// script that fails is a `failed` status, as is a request while another run is still
    /// running. If a run waited past its idle timeout, it expires and `execute` answers its
    /// `failed` status without running `code`.
    pub async fn execute(&mut self, code: &str) -> Status {
        let span = tracing::info_span!(
            "execute",
            run_id = Empty,
            status = Empty,
            call_count = Empty,
            duration = Empty
        );
        let started = Instant::now();
        let status = async {
            if let Some(status) = self.expire_idle_run() {
                return status;
            }
            let session = match self.state.take() {
                Some(State::Idle(session)) => session,
                suspended => {
                    self.state = suspended;
                    let error = format!("run {} is still running", self.last_run_id);
                    return Status::Failed {
                        error,
                        calls: Vec::new(),
                    };
                }
            };
            self.last_run_id += 1;
            let id = self.last_run_id;
            Span::current().record("run_id", id).record("call_count", 0);
            let progress = self.repl.feed_start(session, code);
            if let Some(status) = self.suspend_or_end(id, Vec::new(), progress) {
                return status;
            }
            self.wait_timeout(self.execute_wait).await
        }
        .instrument(span.clone())
        .await;
        record_end(&span, status.as_str(), started);
        status
    }

    /// Waits up to the poll wait for the run with `run_id` that `execute` left running.
    pub async fn poll(&mut self, run_id: u64) -> Status {
        let span = tracing::info_span!(
            "poll",
            run_id,
            status = Empty,
            call_count = Empty,
            duration = Empty
        );
        let started = Instant::now();
        let status = async {
            if !matches!(&self.state, Some(State::Suspended(run)) if run.id == run_id) {
                let error = format!("no run {run_id}");
                return Status::Failed {
                    error,
                    calls: Vec::new(),
                };
            }
            match self.expire_idle_run() {
                Some(status) => status,
                None => self.wait_timeout(self.poll_wait).await,
            }
        }
        .instrument(span.clone())
        .await;
        record_end(&span, status.as_str(), started);
        status
    }

    /// Waits up to `wait` for the suspended run, answering its tool calls as the host answers
    /// them. The run stays suspended in the sandbox while it waits, so dropping this future
    /// loses nothing.
    async fn wait_timeout(&mut self, wait: Duration) -> Status {
        let mut timer = Delay::new(wait);
        loop {
            let Some(State::Suspended(run)) = &mut self.state else {
                panic!("wait_timeout without a suspended run");
            };
            let answer = match future::select(&mut run.answer, &mut timer).await {
                Either::Left((answer, _)) => answer,
                Either::Right(_) => {
                    run.idle_since = Instant::now();
                    Span::current().record("call_count", run.answered_calls.len());
                    return Status::Running { run_id: run.id };
                }
            };
            let Some(State::Suspended(run)) = self.state.take() else {
                panic!("wait_timeout without a suspended run");
            };
            let Run {
                id,
                mut answered_calls,
                call,
                function_call,
                ..
            } = run;
            let progress = match answer {
                Some(answer) => {
                    answered_calls.push(call);
                    Span::current().record("call_count", answered_calls.len());
                    self.repl.resume(function_call, answer)
                }
                None => {
                    let message = format!("{} passed the call timeout", call.tool_id);
                    self.repl.abort(function_call, message)
                }
            };
            if let Some(status) = self.suspend_or_end(id, answered_calls, progress) {
                return status;
            }
        }
    }

    /// Suspends run `id` at the tool call `progress` reached, or ends it with its status. A call
    /// past the call limit ends the run instead of reaching the host.
    fn suspend_or_end(
        &mut self,
        id: u64,
        answered_calls: Vec<host::Call>,
        mut progress: Progress,
    ) -> Option<Status> {
        let call_limit = self.limits.calls_per_execute;
        if answered_calls.len() >= call_limit
            && let Progress::Call { function_call, .. } = progress
        {
            let message = format!("the script reached the call limit of {call_limit}");
            progress = self.repl.abort(function_call, message);
        }
        match progress {
            Progress::Call {
                call,
                function_call,
            } => {
                let answer = self.call_host(id, &call);
                self.state = Some(State::Suspended(Run {
                    id,
                    answered_calls,
                    call,
                    function_call,
                    answer,
                    idle_since: Instant::now(),
                }));
                None
            }
            Progress::Ended { result, session } => {
                Some(self.end_run(session, result, answered_calls))
            }
        }
    }

    /// Ends the suspended run if it has waited past the idle timeout, and says it expired.
    fn expire_idle_run(&mut self) -> Option<Status> {
        let idle_timeout = self.limits.idle_timeout?;
        let expired = matches!(&self.state, Some(State::Suspended(run)) if run.idle_since.elapsed() >= idle_timeout);
        if !expired {
            return None;
        }
        let Some(State::Suspended(run)) = self.state.take() else {
            panic!("expire_idle_run without a suspended run");
        };
        let session = Repl::abandon(run.function_call);
        let error = format!("run {} expired", run.id);
        Some(self.end_run(session, Err(error), run.answered_calls))
    }

    /// Ends the run: `session` is idle again, and the script's result becomes the status.
    fn end_run(
        &mut self,
        session: Box<MontyRepl>,
        result: Result<Value, String>,
        answered_calls: Vec<host::Call>,
    ) -> Status {
        self.state = Some(State::Idle(session));
        match result {
            Ok(value) => Status::Finished { value },
            Err(error) => Status::Failed {
                error,
                calls: answered_calls,
            },
        }
    }

    /// Sends `call`, made by run `run_id`, to the host and traces its answer in a `call` span
    /// with no parent. The answer is `None` if the call timeout passes first.
    fn call_host(
        &self,
        run_id: u64,
        call: &host::Call,
    ) -> BoxFuture<'static, Option<Result<Value, host::Error>>> {
        let span = tracing::info_span!(parent: None, "call", run_id, tool_id = %call.tool_id, status = Empty, duration = Empty);
        let answer = self.host.call(call);
        let timeout = self.limits.call_timeout.map(Delay::new);
        let started = Instant::now();
        async move {
            let answer = match timeout {
                Some(timeout) => match future::select(answer, timeout).await {
                    Either::Left((answer, _)) => Some(answer),
                    Either::Right(_) => None,
                },
                None => Some(answer.await),
            };
            let status = match &answer {
                Some(Ok(_)) => "answered",
                Some(Err(host::Error::Refused(_))) => "refused",
                Some(Err(host::Error::Failed(_))) => "failed",
                None => "timed_out",
            };
            record_end(&Span::current(), status, started);
            answer
        }
        .instrument(span)
        .boxed()
    }
}

impl<H: Host> Builder<H> {
    /// How long `execute` waits for a script before it answers `running`. The default is 30 s.
    pub fn execute_wait(mut self, wait: Duration) -> Self {
        self.execute_wait = wait;
        self
    }

    /// How long `poll` waits for a running script before it answers `running` again. The
    /// default is 10 s.
    pub fn poll_wait(mut self, wait: Duration) -> Self {
        self.poll_wait = wait;
        self
    }

    /// What a script may use before the sandbox stops it. The default is [`Limits::default`].
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The sandbox, with an empty Python session.
    pub fn build(self) -> Sandbox<H> {
        let tool_ids = self.host.tools().into_iter().map(|tool| tool.id).collect();
        Sandbox {
            host: self.host,
            repl: Repl::new(tool_ids),
            state: Some(State::Idle(Repl::new_session(&self.limits))),
            last_run_id: 0,
            execute_wait: self.execute_wait,
            poll_wait: self.poll_wait,
            limits: self.limits,
        }
    }
}

impl Status {
    /// The status's `status` tag in the model's JSON, such as `"running"`.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Finished { .. } => "finished",
            Self::Running { .. } => "running",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Records on `span` the status a traced step ended with and how long it took since `started`.
fn record_end(span: &Span, status: &'static str, started: Instant) {
    span.record("status", status);
    span.record("duration", field::debug(started.elapsed()));
}
