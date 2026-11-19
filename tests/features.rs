//! The feature scenarios: one `World`, run once per executor against `backend::Monty`, and once
//! more against `backend::Pool` when the `pool` feature is on.

#[path = "steps/caller.rs"]
mod caller;
#[path = "steps/limits.rs"]
mod limits;
#[cfg(feature = "rmcp")]
#[path = "steps/mcp.rs"]
mod mcp;
#[path = "steps/model.rs"]
mod model;
#[cfg(feature = "pool")]
#[path = "steps/pool.rs"]
mod pool;
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
use futures::future::{self, BoxFuture, FutureExt};
use ghostmode::backend::{Backend, Monty};
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
    /// The backend this pass runs the scenario against.
    backend: TestBackend,
    sandbox: Option<Box<dyn AnySandbox>>,
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

/// The backend that a pass runs every scenario against.
#[derive(Clone, Default)]
enum TestBackend {
    #[default]
    Monty,
    #[cfg(feature = "pool")]
    Pool(ghostmode::backend::Pool),
}

/// A sandbox over any backend, which the When steps send each [`Request`] to.
trait AnySandbox: Send {
    /// The sandbox's [`Reply`] to `request`.
    fn reply(&mut self, request: Request) -> BoxFuture<'_, Reply>;
}

impl<B: Backend + Send + 'static> AnySandbox for Sandbox<test_host::TestHost, B> {
    fn reply(&mut self, request: Request) -> BoxFuture<'_, Reply> {
        async move {
            match request {
                Request::Execute { code, restart } => {
                    Reply::Status(self.execute(&code, restart).await)
                }
                Request::Poll(run_id) => Reply::Status(self.poll(run_id).await),
                Request::Cancel(run_id) => Reply::Cancel(self.cancel(run_id).await),
                Request::Reset => {
                    self.reset().await;
                    Reply::Reset
                }
                Request::Dump => Reply::Dump(self.dump().await),
            }
        }
        .boxed()
    }
}

impl World {
    /// A new sandbox over this pass's backend, with the host, waits, limits and signing key the
    /// scenario's steps set. It holds an empty session, or the one in `dump` if there is one.
    async fn new_sandbox(
        &self,
        dump: Option<&[u8]>,
    ) -> Result<Box<dyn AnySandbox>, ghostmode::Error> {
        match &self.backend {
            TestBackend::Monty => build(self.builder(Monty), dump).await,
            #[cfg(feature = "pool")]
            TestBackend::Pool(pool) => build(self.builder(pool.clone()), dump).await,
        }
    }

    /// A builder for a sandbox over `backend`, with the host, waits, limits and signing key the
    /// scenario's steps set.
    fn builder<B: Backend>(&self, backend: B) -> Builder<test_host::TestHost, B> {
        let mut builder = Builder::new(self.host.clone(), backend);
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
            None => self.new_sandbox(None).await.expect("a new sandbox"),
        };
        let answer = async move {
            let reply = sandbox.reply(request).await;
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
        match self.new_sandbox(Some(bytes)).await {
            Ok(sandbox) => self.sandbox = Some(sandbox),
            Err(error) => self.caller_error = Some(error),
        }
    }
}

/// The sandbox that `builder` builds, with an empty session, or the one in `dump` if there is
/// one.
async fn build<B: Backend + Send + 'static>(
    builder: Builder<test_host::TestHost, B>,
    dump: Option<&[u8]>,
) -> Result<Box<dyn AnySandbox>, ghostmode::Error> {
    let sandbox = match dump {
        Some(bytes) => builder.restore(bytes).await?,
        None => builder.build().await,
    };
    Ok(Box::new(sandbox))
}

/// Runs the whole feature tree once against `backend` and says whether any scenario failed. A
/// feature tagged `@rmcp` needs the `rmcp` feature and a Tokio runtime, so it runs only when it
/// has both. A feature or scenario tagged `@monty` or `@pool` runs only against that backend.
async fn run_scenarios(on_tokio: bool, backend: TestBackend) -> bool {
    let run_rmcp = on_tokio && cfg!(feature = "rmcp");
    let backend_tag = match backend {
        TestBackend::Monty => "monty",
        #[cfg(feature = "pool")]
        TestBackend::Pool(_) => "pool",
    };
    World::cucumber()
        .max_concurrent_scenarios(1)
        .fail_on_skipped()
        .before(move |_, _, _, world| {
            world.backend = backend.clone();
            future::ready(()).boxed_local()
        })
        .filter_run("tests/features", move |feature, _, scenario| {
            let runs_rmcp = run_rmcp || !feature.tags.iter().any(|tag| tag == "rmcp");
            let runs_here = (feature.tags.iter().chain(&scenario.tags))
                .all(|tag| !["monty", "pool"].contains(&tag.as_str()) || tag == backend_tag);
            runs_rmcp && runs_here
        })
        .await
        .execution_has_failed()
}

/// The path of the `monty` worker binary in the first `PATH` directory that holds one. monty-pool
/// starts workers with an empty environment, so it needs the full path.
#[cfg(feature = "pool")]
fn find_worker_binary() -> std::path::PathBuf {
    let file_name = format!("monty{}", std::env::consts::EXE_SUFFIX);
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(&file_name))
        .find(|binary| binary.is_file())
        .expect("the `monty` worker binary on PATH")
}

fn main() {
    println!("executor: futures::executor::block_on");
    let mut failed = futures::executor::block_on(run_scenarios(false, TestBackend::Monty));
    println!("executor: Tokio current-thread runtime");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime");
    failed |= runtime.block_on(run_scenarios(true, TestBackend::Monty));
    println!("executor: Tokio multi-thread runtime");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("multi-thread runtime");
    failed |= runtime.block_on(run_scenarios(true, TestBackend::Monty));
    #[cfg(feature = "pool")]
    {
        println!("executor: Tokio multi-thread runtime, against monty-pool workers");
        failed |= runtime.block_on(async {
            let config = monty_pool::PoolConfig::subprocess(find_worker_binary());
            let workers = monty_pool::Pool::new(config)
                .await
                .expect("monty-pool starts the `monty` worker");
            run_scenarios(
                true,
                TestBackend::Pool(ghostmode::backend::Pool::new(workers)),
            )
            .await
        });
    }
    if failed {
        std::process::exit(1);
    }
}
