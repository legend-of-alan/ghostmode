//! The backend that runs each session's scripts in its own Monty worker process, checked out
//! from a monty-pool pool. Only this module talks to monty-pool.
//!
//! A worker that crashes takes down only itself, never the caller's process. The script's run
//! fails with an error that says so, and the next status reports `restarted`, with no globals.
//! The same holds when a script goes past [`Limits::memory_bytes`], which a worker enforces on
//! its whole memory, and when the sandbox stops a run at a tool call: at
//! [`Limits::calls_per_execute`], [`Limits::call_timeout`] or [`Limits::idle_timeout`], or with
//! [`Sandbox::cancel`]. A stopped run ends its worker, and the session's globals with it.
//!
//! Each session holds one worker from its first script until the session drops, as
//! [`Sandbox::reset`] and dropping the sandbox do. Dropping a session ends its worker.
//!
//! [`Sandbox::cancel`]: crate::Sandbox::cancel
//! [`Sandbox::reset`]: crate::Sandbox::reset

use std::error;
use std::path::PathBuf;
use std::sync::Arc;

use monty_pool::{
    Checkout, Pool, PoolConfig, PoolError, PrintFuture, ReplConfig, ResumeValue, TurnEvent,
    on_print_sync,
};
use monty_types::{DEFAULT_MAX_PRINT_COLLECT_BYTES, MontyException, NameLookupResult, PrintStream};
use serde_json::Value;

use super::{SCRIPT_NAME, value};
use crate::backend::{Backend, Ending, Progress, Streams};
use crate::host;
use crate::sandbox::Limits;

/// Runs each session's scripts in its own worker process from a pool of `monty` workers that
/// [`WorkerOptions`] describe. Clones share the pool's workers. Build it, and so the sandbox,
/// inside a Tokio multi-thread runtime.
///
/// ```no_run
/// use ghostmode::backend::{MontyPool, WorkerOptions};
///
/// # async fn build() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// let mut options = WorkerOptions::new("/usr/local/bin/monty");
/// options.min_workers = 0;
/// let backend = MontyPool::new(options).await?;
/// # Ok(())
/// # }
/// ```
///
/// The worker runs the script, but the thread that polls `execute` or `poll` still converts
/// values between JSON and Python and cuts output to [`Limits::output_chars`]. The thread that
/// polls `Builder::restore` checks a dump's signature.
#[derive(Clone)]
pub struct MontyPool(Arc<Pool>); // repeats module: pairs with Monty, the in-process backend

/// The `monty` worker processes that a [`MontyPool`] starts. Build it with [`WorkerOptions::new`],
/// then set its fields.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct WorkerOptions {
    /// The full path of the `monty` worker binary. Workers start with an empty environment, so
    /// a bare file name isn't looked up on `PATH`.
    pub binary_path: PathBuf,
    /// How many workers the pool keeps running, counting the ones that open sessions hold. With
    /// one session open and `min_workers` at 1, a new session's first script or a restore
    /// starts a new worker. The default is 1.
    pub min_workers: usize,
}

/// A Python session in [`MontyPool`], whose globals live in one worker process.
pub struct Session {
    config: ReplConfig,
    /// The session's worker, or `None` before its first script and after its worker ended.
    checkout: Option<Checkout>,
}

/// A script suspended at a tool call in [`MontyPool`], with what it has printed so far.
pub struct Suspension {
    config: ReplConfig,
    checkout: Checkout,
    streams: Streams,
}

impl WorkerOptions {
    /// Workers started from the `monty` binary at `binary_path`, with every other setting at its
    /// default.
    pub fn new(binary_path: impl Into<PathBuf>) -> Self {
        Self {
            binary_path: binary_path.into(),
            min_workers: 1,
        }
    }
}

impl MontyPool {
    /// A backend whose sessions each check out one of the workers that `options` describe. It
    /// starts [`WorkerOptions::min_workers`] workers before it returns. It fails if a worker
    /// can't start from [`WorkerOptions::binary_path`], or if `min_workers` is more than the pool
    /// allows, which is the machine's [available parallelism].
    ///
    /// [available parallelism]: std::thread::available_parallelism
    pub async fn new(options: WorkerOptions) -> Result<Self, Box<dyn error::Error + Send + Sync>> {
        let config = PoolConfig {
            min_processes: options.min_workers,
            ..PoolConfig::subprocess(options.binary_path)
        };
        Ok(Self(Arc::new(Pool::new(config).await?)))
    }

    /// The worker in `checkout`, or a new one checked out under `config` if there is none.
    async fn take_worker(
        &self,
        checkout: Option<Checkout>,
        config: &ReplConfig,
    ) -> Result<Checkout, PoolError> {
        match checkout {
            Some(checkout) => Ok(checkout),
            None => self.0.checkout(config).await,
        }
    }

    /// Runs the script on from `turn` until it calls one of the host's tools, whose ids are
    /// `tool_ids`, with keyword arguments, or ends. Every other suspension is answered here as
    /// `backend::Monty` answers it. What the script prints is added to the suspension's streams.
    async fn run(
        &self,
        mut script: Suspension,
        mut turn: Result<TurnEvent, PoolError>,
        tool_ids: &[String],
    ) -> Progress<Self> {
        loop {
            let event = match turn {
                Ok(event) => event,
                Err(error) => return Progress::Ended(script.into_ending(Err(error.to_string()))),
            };
            let checkout = &mut script.checkout;
            let mut on_print = print_into(&mut script.streams);
            turn = match event {
                TurnEvent::Complete(value) => {
                    drop(on_print);
                    return Progress::Ended(script.into_ending(Ok(value::to_json(value.as_ref()))));
                }
                TurnEvent::FunctionCall {
                    function_name,
                    args,
                    ..
                } if tool_ids.contains(&function_name) => {
                    match value::to_call(function_name, &args) {
                        Ok(call) => {
                            drop(on_print);
                            return Progress::Call {
                                call,
                                suspension: script,
                            };
                        }
                        Err(error) => checkout.resume(ResumeValue::Error(error), &mut on_print),
                    }
                    .await
                }
                TurnEvent::FunctionCall { .. } => {
                    checkout.resume(ResumeValue::NotFound, &mut on_print).await
                }
                TurnEvent::NameLookup { .. } => {
                    let answer = NameLookupResult::Undefined;
                    checkout.resume_name_lookup(answer, &mut on_print).await
                }
                TurnEvent::OsCall { .. } => {
                    checkout
                        .resume(ResumeValue::NotHandled, &mut on_print)
                        .await
                }
                TurnEvent::ResolveFutures { .. } => {
                    drop(on_print);
                    let message = super::TOOLS_NOT_AWAITABLE.to_string();
                    return Progress::Ended(self.abort(script, message).await);
                }
            };
        }
    }
}

impl Backend for MontyPool {
    type Session = Session;
    type Suspension = Suspension;

    async fn new_session(&self, limits: &Limits) -> Session {
        Session {
            config: to_repl_config(limits),
            checkout: None,
        }
    }

    async fn feed_start(
        &self,
        session: Session,
        code: &str,
        tool_ids: &[String],
    ) -> Progress<Self> {
        let Session { config, checkout } = session;
        let mut checkout = match self.take_worker(checkout, &config).await {
            Ok(checkout) => checkout,
            Err(error) => {
                return Progress::Ended(Ending {
                    result: Err(error.to_string()),
                    streams: Streams::default(),
                    session: Session {
                        config,
                        checkout: None,
                    },
                    lost_globals: false,
                });
            }
        };
        let mut streams = Streams::default();
        let turn = checkout
            .feed(
                code,
                Vec::new(),
                Vec::new(),
                true,
                &mut print_into(&mut streams),
            )
            .await;
        let script = Suspension {
            config,
            checkout,
            streams,
        };
        self.run(script, turn, tool_ids).await
    }

    async fn resume(
        &self,
        mut suspension: Suspension,
        answer: Result<Value, host::Error>,
        tool_ids: &[String],
    ) -> Progress<Self> {
        let answer = match answer {
            Ok(answer) => ResumeValue::Return(value::from_json(&answer)),
            Err(error) => ResumeValue::Error(value::to_exception(error)),
        };
        let mut on_print = print_into(&mut suspension.streams);
        let turn = suspension.checkout.resume(answer, &mut on_print).await;
        drop(on_print);
        self.run(suspension, turn, tool_ids).await
    }

    /// Ends the script by ending its worker, so the session loses its globals.
    async fn abort(&self, suspension: Suspension, message: String) -> Ending<Self> {
        suspension.kill_worker(Err(MontyException::runtime_error(message).to_string()))
    }

    /// Ends the script's worker, so the session loses its globals.
    async fn abandon(&self, suspension: Suspension, message: String) -> Ending<Self> {
        suspension.kill_worker(Err(message))
    }

    /// A session with no worker yet checks one out to dump. Bytes the worker can't write are an
    /// `Err` that says why. A worker that ends while it dumps takes the session's globals with it.
    async fn dump(
        &self,
        session: &mut Session,
    ) -> (Result<Vec<u8>, Box<dyn error::Error + Send + Sync>>, bool) {
        let mut checkout = match self
            .take_worker(session.checkout.take(), &session.config)
            .await
        {
            Ok(checkout) => checkout,
            Err(error) => return (Err(error.into()), false),
        };
        let bytes = checkout.dump().await;
        let alive = checkout.worker_id().is_some();
        if alive {
            session.checkout = Some(checkout);
        }
        (bytes.map_err(Into::into), !alive)
    }

    /// Bytes the worker can't load are an `Err` that says why. Bytes that crash or hang the
    /// worker harm only that worker.
    async fn restore(
        &self,
        bytes: &[u8],
        limits: &Limits,
    ) -> Result<Session, Box<dyn error::Error + Send + Sync>> {
        let config = to_repl_config(limits);
        let mut checkout = self.0.checkout(&config).await?;
        let mut on_print = on_print_sync(|_, _| {});
        let (suspension, _) = checkout
            .restore(bytes.to_vec(), Vec::new(), &mut on_print)
            .await?;
        if suspension.is_some() {
            return Err(super::DUMP_HOLDS_RUN.into());
        }
        Ok(Session {
            config,
            checkout: Some(checkout),
        })
    }
}

impl Suspension {
    /// How the script ended with `result`. The session keeps the worker if it is still alive,
    /// and otherwise has none and says the session lost its globals.
    fn into_ending(self, result: Result<Value, String>) -> Ending<MontyPool> {
        let alive = self.checkout.worker_id().is_some();
        Ending {
            result,
            streams: self.streams,
            session: Session {
                config: self.config,
                checkout: alive.then_some(self.checkout),
            },
            lost_globals: !alive,
        }
    }

    /// Ends the script with `result` by ending its worker, so the session it gives back has no
    /// worker and says it lost its globals. Dropping the checkout kills the worker process and
    /// frees its place in the pool.
    fn kill_worker(self, result: Result<Value, String>) -> Ending<MontyPool> {
        drop(self.checkout);
        Ending {
            result,
            streams: self.streams,
            session: Session {
                config: self.config,
                checkout: None,
            },
            lost_globals: true,
        }
    }
}

/// The worker session settings that stop a script at `limits`' run time, memory and recursion
/// depth, name the session [`SCRIPT_NAME`] and return from `time.sleep` at once.
fn to_repl_config(limits: &Limits) -> ReplConfig {
    ReplConfig {
        script_name: SCRIPT_NAME.to_string(),
        limits: Some(super::resource_limits(limits)),
        os_policy: super::os_policy(),
        ..ReplConfig::default()
    }
}

/// A print callback that adds what the script prints to `streams`, and drops a print that would
/// take them together past 10 MiB.
fn print_into(streams: &mut Streams) -> impl FnMut(PrintStream, &str) -> PrintFuture + Send + '_ {
    on_print_sync(|stream, text| {
        let printed_bytes = streams.stdout.len() + streams.stderr.len();
        if printed_bytes + text.len() > DEFAULT_MAX_PRINT_COLLECT_BYTES {
            return;
        }
        match stream {
            PrintStream::Stdout => streams.stdout.push_str(text),
            PrintStream::Stderr => streams.stderr.push_str(text),
        }
    })
}
