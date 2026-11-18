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
                return reject(format!("run {} is still running", run.id));
            }
            self.last_run_id += 1;
            Span::current().record("run_id", self.last_run_id);
            let progress = self.repl.start(&request.code);
            match self.park_or_finish(self.last_run_id, Vec::new(), progress) {
                Some(status) => status,
                None => self.wait(self.execute_wait).await,
            }
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
                self.wait(self.poll_wait).await
            } else {
                reject(format!("no run {}", request.run_id))
            }
        }
        .instrument(span.clone())
        .await;
        record_status(&span, &status, started);
        status
    }

    /// Waits up to `wait` for the parked run, answering its tool calls as the host answers
    /// them. The run stays parked in the sandbox while it waits, so dropping this future
    /// loses nothing.
    async fn wait(&mut self, wait: Duration) -> tool::Status {
        let mut timer = Delay::new(wait);
        loop {
            let run = self.run.as_mut().expect("a parked run");
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
            } = self.run.take().expect("a parked run");
            answered_calls.push(call);
            let progress = self.repl.resume(answer);
            if let Some(status) = self.park_or_finish(id, answered_calls, progress) {
                return status;
            }
        }
    }

    /// Parks the run at the tool call its script waits on, or finishes it with its status.
    fn park_or_finish(
        &mut self,
        id: u64,
        answered_calls: Vec<host::Call>,
        progress: Progress,
    ) -> Option<tool::Status> {
        Span::current().record("call_count", answered_calls.len());
        match progress {
            Progress::Call(call) => {
                let answer = self.call_host(call.clone());
                self.run = Some(Run {
                    id,
                    answered_calls,
                    call,
                    answer,
                });
                None
            }
            Progress::Finished(value) => Some(tool::Status::Finished { value }),
            Progress::Failed(error) => Some(tool::Status::Failed {
                error,
                calls: answered_calls,
            }),
        }
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

/// A `failed` status for a request the sandbox turned down before any script ran.
fn reject(error: String) -> tool::Status {
    tool::Status::Failed {
        error,
        calls: Vec::new(),
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
