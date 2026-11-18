//! The feature scenarios: one `World`, run once per executor.

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
use ghostmode::{Limits, Sandbox};
use tokio::runtime::{Handle, RuntimeFlavor};
use tracing::Dispatch;
use tracing::instrument::WithSubscriber;

#[derive(Default, cucumber::World)]
struct World {
    host: test_host::TestHost,
    execute_wait: Option<Duration>,
    poll_wait: Option<Duration>,
    limits: Option<Limits>,
    sandbox: Option<Sandbox<test_host::TestHost>>,
    status: serde_json::Value,
    elapsed: Duration,
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

/// One of the model's two tool calls, from a When step to the sandbox.
enum Request {
    Execute(String),
    Poll(u64),
}

impl World {
    /// Sends `request` to the scenario's sandbox, building it on first use, and keeps the status
    /// JSON and how long the model waited for it. On a Tokio multi-thread runtime the call runs
    /// in `tokio::spawn`, so a sandbox future that is not `Send` fails the build.
    async fn send(&mut self, request: Request) {
        let mut sandbox = self.sandbox.take().unwrap_or_else(|| {
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
            builder.build()
        });
        let answer = async move {
            let status = match request {
                Request::Execute(code) => sandbox.execute(&code).await,
                Request::Poll(run_id) => sandbox.poll(run_id).await,
            };
            (sandbox, status)
        }
        .with_subscriber(self.dispatch.clone());
        let started = Instant::now();
        let multi_thread = Handle::try_current()
            .is_ok_and(|handle| handle.runtime_flavor() == RuntimeFlavor::MultiThread);
        let (sandbox, status) = if multi_thread {
            tokio::spawn(answer).await.expect("sandbox task")
        } else {
            answer.await
        };
        self.elapsed = started.elapsed();
        self.sandbox = Some(sandbox);
        self.status = serde_json::to_value(status).expect("status serializes");
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
