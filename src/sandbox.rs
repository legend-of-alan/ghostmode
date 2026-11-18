//! The sandbox that answers the model's `execute` and `poll` tool calls.

use std::time::{Duration, Instant};

use futures_timer::Delay;
use futures_util::future::{self, BoxFuture, Either, FutureExt};
use serde_json::Value;
use tracing::field::{self, Empty};
use tracing::{Instrument, Span};

use crate::host::{self, Host};
use crate::repl::{Progress, Repl};
use crate::tool;

/// One Python session that runs the model's scripts, and the host whose tools they call.
/// Globals persist from one `execute` to the next, and one run is in flight at a time.
pub struct Sandbox<H> {
    host: H,
    repl: Repl,
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

/// A script waiting on a tool call the host has not answered yet.
struct Run {
    id: u64,
    answered_calls: Vec<host::Call>,
    call: host::Call,
    answer: BoxFuture<'static, Result<Value, host::Error>>,
}

impl<H: Host> Sandbox<H> {
    /// A builder for a sandbox whose scripts call `host`'s tools. `execute` waits 30 s and
    /// `poll` waits 10 s unless the builder sets them.
    pub fn builder(host: H) -> Builder<H> {
        Builder {
            host,
            execute_wait: Duration::from_secs(30),
            poll_wait: Duration::from_secs(10),
        }
    }

    /// How long `execute` waits for a script before it answers `running`.
    pub fn execute_wait(&self) -> Duration {
        self.execute_wait
    }

    /// How long `poll` waits for a running script before it answers `running` again.
    pub fn poll_wait(&self) -> Duration {
        self.poll_wait
    }

    /// Runs a script for up to the execute wait. A script that fails is a `failed` status,
    /// as is a request while another run is still running.
    pub async fn execute(&mut self, request: tool::Execute) -> tool::Status {
        let span = tracing::info_span!(
            "execute",
            run_id = Empty,
            status = Empty,
            call_count = Empty,
            duration = Empty
        );
        let started = Instant::now();
        let status = async {
            if let Some(run) = &self.run {
                let error = format!("run {} is still running", run.id);
                return tool::Status::Failed {
                    error,
                    calls: Vec::new(),
                };
            }
            self.last_run_id += 1;
            let id = self.last_run_id;
            Span::current().record("run_id", id).record("call_count", 0);
            match self.repl.feed_start(&request.code) {
                Progress::Call(call) => self.suspend(id, Vec::new(), call),
                Progress::Finished(value) => return tool::Status::Finished { value },
                Progress::Failed(error) => {
                    return tool::Status::Failed {
                        error,
                        calls: Vec::new(),
                    };
                }
            }
            self.wait_timeout(self.execute_wait).await
        }
        .instrument(span.clone())
        .await;
        record_status(&span, &status, started);
        status
    }

    /// Waits up to the poll wait for the run that `execute` left running.
    pub async fn poll(&mut self, request: tool::Poll) -> tool::Status {
        let span = tracing::info_span!(
            "poll",
            run_id = request.run_id,
            status = Empty,
            call_count = Empty,
            duration = Empty
        );
        let started = Instant::now();
        let status = async {
            if self
                .run
                .as_ref()
                .is_some_and(|run| run.id == request.run_id)
            {
                self.wait_timeout(self.poll_wait).await
            } else {
                let error = format!("no run {}", request.run_id);
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
                ..
            } = self.run.take().expect("a suspended run");
            answered_calls.push(call);
            Span::current().record("call_count", answered_calls.len());
            match self.repl.resume(answer) {
                Progress::Call(call) => self.suspend(id, answered_calls, call),
                Progress::Finished(value) => return tool::Status::Finished { value },
                Progress::Failed(error) => {
                    return tool::Status::Failed {
                        error,
                        calls: answered_calls,
                    };
                }
            }
        }
    }

    /// Suspends the run at `call`, its script's tool call, until the host's answer arrives.
    fn suspend(&mut self, id: u64, answered_calls: Vec<host::Call>, call: host::Call) {
        let answer = self.call_host(call.clone());
        self.run = Some(Run {
            id,
            answered_calls,
            call,
            answer,
        });
    }

    /// Sends `call` to the host, in a `call` span with no parent so that the span of the
    /// `execute` or `poll` that made it closes when that tool call answers.
    fn call_host(&self, call: host::Call) -> BoxFuture<'static, Result<Value, host::Error>> {
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
    /// How long `execute` waits for a script before it answers `running`.
    pub fn execute_wait(mut self, wait: Duration) -> Self {
        self.execute_wait = wait;
        self
    }

    /// How long `poll` waits for a running script before it answers `running` again.
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
