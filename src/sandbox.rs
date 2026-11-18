//! The sandbox that answers the model's `execute` and `poll` tool calls.

use std::time::{Duration, Instant};

use futures_timer::Delay;
use futures_util::future::{self, BoxFuture, Either, FutureExt};
use serde_json::Value;
use tracing::field::{self, Empty};
use tracing::{Instrument, Span};

use crate::host::{self, Host};
use crate::repl::{MontyRepl, Progress, Repl, ReplFunctionCall};
use crate::tool;

/// One Python session that runs the model's scripts, and the host whose tools they call.
/// Globals persist from one `execute` to the next, and one run is in flight at a time.
pub struct Sandbox<H> {
    host: H,
    repl: Repl,
    idle_session: Option<Box<MontyRepl>>,
    run: Option<Run>,
    last_run_id: u64,
    execute_wait: Duration,
    poll_wait: Duration,
}

/// Sets up a [`Sandbox`]. Made by [`Sandbox::builder`].
pub struct Builder<H> {
    host: H,
    execute_wait: Duration,
    poll_wait: Duration,
}

/// A script suspended at a tool call the host has not answered yet.
struct Run {
    id: u64,
    answered_calls: Vec<host::Call>,
    call: host::Call,
    function_call: Box<ReplFunctionCall>,
    answer: BoxFuture<'static, Result<Value, host::Error>>,
}

impl<H: Host> Sandbox<H> {
    /// A builder for a sandbox whose scripts call `host`'s tools.
    pub fn builder(host: H) -> Builder<H> {
        Builder {
            host,
            execute_wait: Duration::from_secs(30),
            poll_wait: Duration::from_secs(10),
        }
    }

    /// Runs `code`, waiting up to the execute wait for its tool calls. The wait bounds only the
    /// time spent waiting on the host, not the script's own running time. A script that fails
    /// is a `failed` status, as is a request while another run is still running.
    pub async fn execute(&mut self, code: &str) -> tool::Status {
        let span = tracing::info_span!(
            "execute",
            run_id = Empty,
            status = Empty,
            call_count = Empty,
            duration = Empty
        );
        let started = Instant::now();
        let status = async {
            let Some(session) = self.idle_session.take() else {
                let error = format!("run {} is still running", self.last_run_id);
                return tool::Status::Failed {
                    error,
                    calls: Vec::new(),
                };
            };
            self.last_run_id += 1;
            let id = self.last_run_id;
            Span::current().record("run_id", id).record("call_count", 0);
            match self.repl.feed_start(session, code) {
                Progress::Call {
                    call,
                    function_call,
                } => self.suspend(id, Vec::new(), call, function_call),
                Progress::Ended { result, session } => {
                    return self.end_run(session, result, Vec::new());
                }
            }
            self.wait_timeout(self.execute_wait).await
        }
        .instrument(span.clone())
        .await;
        record_status(&span, &status, started);
        status
    }

    /// Waits up to the poll wait for the run with `run_id` that `execute` left running.
    pub async fn poll(&mut self, run_id: u64) -> tool::Status {
        let span = tracing::info_span!(
            "poll",
            run_id,
            status = Empty,
            call_count = Empty,
            duration = Empty
        );
        let started = Instant::now();
        let status = async {
            if self.run.as_ref().is_some_and(|run| run.id == run_id) {
                self.wait_timeout(self.poll_wait).await
            } else {
                let error = format!("no run {run_id}");
                tool::Status::Failed {
                    error,
                    calls: Vec::new(),
                }
            }
        }
        .instrument(span.clone())
        .await;
        record_status(&span, &status, started);
        status
    }

    /// Waits up to `wait` for the suspended run, answering its tool calls as the host answers
    /// them. The run stays suspended in the sandbox while it waits, so dropping this future
    /// loses nothing.
    async fn wait_timeout(&mut self, wait: Duration) -> tool::Status {
        let mut timer = Delay::new(wait);
        loop {
            let run = self.run.as_mut().expect("a suspended run");
            let answer = match future::select(&mut run.answer, &mut timer).await {
                Either::Left((answer, _)) => answer,
                Either::Right(_) => {
                    Span::current().record("call_count", run.answered_calls.len());
                    return tool::Status::Running { run_id: run.id };
                }
            };
            let Run {
                id,
                mut answered_calls,
                call,
                function_call,
                ..
            } = self.run.take().expect("a suspended run");
            answered_calls.push(call);
            Span::current().record("call_count", answered_calls.len());
            match self.repl.resume(function_call, answer) {
                Progress::Call {
                    call,
                    function_call,
                } => self.suspend(id, answered_calls, call, function_call),
                Progress::Ended { result, session } => {
                    return self.end_run(session, result, answered_calls);
                }
            }
        }
    }

    /// Suspends the run at `call` until the host's answer arrives.
    fn suspend(
        &mut self,
        id: u64,
        answered_calls: Vec<host::Call>,
        call: host::Call,
        function_call: Box<ReplFunctionCall>,
    ) {
        let answer = self.call_host(&call);
        self.run = Some(Run {
            id,
            answered_calls,
            call,
            function_call,
            answer,
        });
    }

    /// Ends the run: `session` is idle again, and the script's result becomes the status.
    fn end_run(
        &mut self,
        session: Box<MontyRepl>,
        result: Result<Value, String>,
        answered_calls: Vec<host::Call>,
    ) -> tool::Status {
        self.idle_session = Some(session);
        match result {
            Ok(value) => tool::Status::Finished { value },
            Err(error) => tool::Status::Failed {
                error,
                calls: answered_calls,
            },
        }
    }

    /// Sends `call` to the host and traces its answer in a `call` span with no parent.
    fn call_host(&self, call: &host::Call) -> BoxFuture<'static, Result<Value, host::Error>> {
        let span = tracing::info_span!(parent: None, "call", tool_id = %call.tool_id, status = Empty, duration = Empty);
        let answer = self.host.call(call);
        let started = Instant::now();
        async move {
            let answer = answer.await;
            let status = match &answer {
                Ok(_) => "answered",
                Err(host::Error::Refused(_)) => "refused",
                Err(host::Error::Failed(_)) => "failed",
            };
            let span = Span::current();
            span.record("status", status);
            span.record("duration", field::debug(started.elapsed()));
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

    /// The sandbox, with an empty Python session.
    pub fn build(self) -> Sandbox<H> {
        let tool_ids = self.host.tools().into_iter().map(|tool| tool.id).collect();
        Sandbox {
            host: self.host,
            repl: Repl::new(tool_ids),
            idle_session: Some(Repl::new_session()),
            run: None,
            last_run_id: 0,
            execute_wait: self.execute_wait,
            poll_wait: self.poll_wait,
        }
    }
}

/// Records a tool call's status and how long it took on its span.
fn record_status(span: &Span, status: &tool::Status, started: Instant) {
    let status = match status {
        tool::Status::Finished { .. } => "finished",
        tool::Status::Running { .. } => "running",
        tool::Status::Failed { .. } => "failed",
    };
    span.record("status", status);
    span.record("duration", field::debug(started.elapsed()));
}
