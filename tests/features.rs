//! The feature scenarios: one `World`, run once per executor.

#[path = "steps/caller.rs"]
mod caller;
#[path = "steps/limits.rs"]
mod limits;
#[cfg(feature = "rmcp")]
#[path = "steps/mcp.rs"]
mod mcp;
#[path = "steps/model.rs"]
mod model;
#[path = "steps/schema.rs"]
mod schema;
#[path = "steps/spans.rs"]
mod spans;
#[path = "steps/test_host.rs"]
mod test_host;

use std::fmt;
use std::time::{Duration, Instant};

use cucumber::World as _;
use cucumber::writer::Stats as _;
use ghostmode::repl::Repl;
use ghostmode::{Builder, Limits, Sandbox, Status};
use tokio::runtime::{Handle, RuntimeFlavor};
use tracing::Dispatch;
use tracing::instrument::WithSubscriber;

#[derive(Default, cucumber::World)]
struct World {
    host: test_host::TestHost,
    execute_wait: Option<Duration>,
    poll_wait: Option<Duration>,
    limits: Option<Limits>,
    signing_key: Option<[u8; 32]>,
    sandbox: Option<Sandbox<test_host::TestHost, Repl>>,
    status: serde_json::Value,
    cancel_returned: Option<bool>,
    dump: Option<Vec<u8>>,
    caller_error: Option<ghostmode::Error>,
    /// How long the last [`Request`] from the test (standing in for the caller) to the
    /// `Sandbox` took to return, including time the sandbox spent waiting on the script
    /// and the host's tool calls.
    request_duration: Duration,
    dispatch: Dispatch,
    span_capture: spans::SpanCapture,
    #[cfg(feature = "rmcp")]
    mcp: mcp::Connection,
}

impl fmt::Debug for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("World")
    }
}

/// One of the model's two tool calls or a call of the caller's, from a When step to the sandbox.
enum Request {
    Execute { code: String, restart: bool },
    Poll(u64),
    Cancel(u64),
    Reset,
    Dump,
}

/// What the sandbox answers a [`Request`], from the sandbox to a When step.
enum Reply {
    Status(Status),
    Cancel(bool),
    Reset,
    Dump(Result<Vec<u8>, ghostmode::Error>),
}

impl World {
    /// A builder for the scenario's sandbox, with the host, waits, limits and signing key its
    /// steps set.
    fn builder(&self) -> Builder<test_host::TestHost, Repl> {
        let mut builder = Sandbox::builder(self.host.clone());
        if let Some(wait) = self.execute_wait {
            builder = builder.execute_wait(wait);
        }
        if let Some(wait) = self.poll_wait {
            builder = builder.poll_wait(wait);
        }
        if let Some(limits) = self.limits {
            builder = builder.limits(limits);
        }
        if let Some(key) = self.signing_key {
            builder = builder.signing_key(key);
        }
        builder
    }

    /// Sends `request` to the scenario's sandbox, building it on first use, and keeps what it
    /// answers: the status JSON, what cancel returned, or the dump or the caller's error. It also
    /// keeps how long the request waited. On a Tokio multi-thread runtime the call runs in
    /// `tokio::spawn`, so a sandbox future that is not `Send` fails the build.
    async fn send(&mut self, request: Request) {
        let mut sandbox = match self.sandbox.take() {
            Some(sandbox) => sandbox,
            None => self.builder().build().await,
        };
        let answer = async move {
            let reply = match request {
                Request::Execute { code, restart } => {
                    Reply::Status(sandbox.execute(&code, restart).await)
                }
                Request::Poll(run_id) => Reply::Status(sandbox.poll(run_id).await),
                Request::Cancel(run_id) => Reply::Cancel(sandbox.cancel(run_id).await),
                Request::Reset => {
                    sandbox.reset().await;
                    Reply::Reset
                }
                Request::Dump => Reply::Dump(sandbox.dump().await),
            };
            (sandbox, reply)
        }
        .with_subscriber(self.dispatch.clone());
        let started = Instant::now();
        let multi_thread = Handle::try_current()
            .is_ok_and(|handle| handle.runtime_flavor() == RuntimeFlavor::MultiThread);
        let (sandbox, reply) = if multi_thread {
            tokio::spawn(answer).await.expect("sandbox task")
        } else {
            answer.await
        };
        self.request_duration = started.elapsed();
        self.sandbox = Some(sandbox);
        match reply {
            Reply::Status(status) => {
                self.status = serde_json::to_value(status).expect("status serializes");
            }
            Reply::Cancel(was_open) => self.cancel_returned = Some(was_open),
            Reply::Reset => {}
            Reply::Dump(Ok(bytes)) => self.dump = Some(bytes),
            Reply::Dump(Err(error)) => self.caller_error = Some(error),
        }
    }

    /// Replaces the scenario's sandbox with a new one restored from `bytes`, or keeps the
    /// caller's error if restoring fails.
    async fn restore(&mut self, bytes: &[u8]) {
        match self.builder().restore(bytes).await {
            Ok(sandbox) => self.sandbox = Some(sandbox),
            Err(error) => self.caller_error = Some(error),
        }
    }
}

/// Runs the whole feature tree once and says whether any scenario failed. A feature tagged
/// `@rmcp` needs the `rmcp` feature and a Tokio runtime, so it runs only when it has both.
async fn run_scenarios(on_tokio: bool) -> bool {
    let run_rmcp = on_tokio && cfg!(feature = "rmcp");
    World::cucumber()
        .max_concurrent_scenarios(1)
        .fail_on_skipped()
        .filter_run("tests/features", move |feature, _, _| {
            run_rmcp || !feature.tags.iter().any(|tag| tag == "rmcp")
        })
        .await
        .execution_has_failed()
}

fn main() {
    println!("executor: futures::executor::block_on");
    let mut failed = futures::executor::block_on(run_scenarios(false));
    println!("executor: Tokio current-thread runtime");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime");
    failed |= runtime.block_on(run_scenarios(true));
    println!("executor: Tokio multi-thread runtime");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .build()
        .expect("multi-thread runtime");
    failed |= runtime.block_on(run_scenarios(true));
    if failed {
        std::process::exit(1);
    }
}
