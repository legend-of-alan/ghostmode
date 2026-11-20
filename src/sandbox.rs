//! The sandbox that answers each `execute` or `poll` call the caller passes in from the model.

use std::borrow::Cow;
use std::time::{Duration, Instant};
use std::{error, fmt, mem};

use futures_timer::Delay;
use futures_util::future::{self, BoxFuture, Either, FutureExt};
use hmac::{Hmac, KeyInit, Mac};
use serde::Serialize;
use serde_json::Value;
use sha2::Sha256;
use tracing::field::{self, Empty};
use tracing::{Instrument, Span};

use crate::backend::{Backend, Ending, Progress, Streams};
use crate::host::{self, Host, Tool};

/// One Python session that runs the model's scripts in backend `B`, and the host whose tools
/// they call. Globals persist from one `execute` to the next, and one run is in flight at a time.
///
/// In-process `backend::Monty` runs the script on the thread that polls `execute` or `poll`,
/// while `backend::MontyPool` runs it in a worker process. A wait ends only while the script
/// waits on the host: a script that runs past it holds `execute` or `poll` until it calls a
/// tool or ends, and [`Limits::run_time`] bounds the script's own time. Its `Debug` output never
/// shows the signing key.
pub struct Sandbox<H, B: Backend> {
    builder: Builder<H, B>,
    /// The session, or `None` once a backend call that held it was dropped, which loses it.
    state: Option<State<B>>,
    /// The id of the last run `execute` started, which is the suspended run's id while one is
    /// open.
    last_run_id: u64,
    /// Whether the session lost its globals since the last finished or failed status, so the
    /// next one says `restarted`.
    unreported_restart: bool,
}

/// Sets up a [`Sandbox`]. Made by [`Builder::new`], which names the backend. The built sandbox
/// keeps its builder and runs with its host, backend, waits, limits and signing key. Its `Debug`
/// output never shows the signing key.
pub struct Builder<H, B> {
    host: H,
    tools: Vec<Tool>,
    backend: B,
    execute_wait: Duration,
    poll_wait: Duration,
    limits: Limits,
    signing_key: Option<[u8; 32]>,
}

/// How many bytes the HMAC-SHA256 tag takes that a signing key appends to a dump.
const TAG_BYTES: usize = 32;

/// The format version that starts every dump. [`Builder::restore`] refuses any other version.
const DUMP_FORMAT_VERSION: u8 = 1;

/// How many bytes the header takes that starts a dump: the [format version](DUMP_FORMAT_VERSION),
/// one byte for whether the session owes the model a `restarted` status, then the last run id as
/// 8 little-endian bytes.
const HEADER_BYTES: usize = 10;

/// What a script may use before the sandbox stops it with a `failed` status. A stopped script
/// leaves the session's globals as it had set them, unless the backend lost them, which the next
/// status reports as `restarted`.
///
/// Start from [`Limits::default`] and set the fields you want:
///
/// ```
/// # use std::time::Duration;
/// let mut limits = ghostmode::Limits::default();
/// limits.run_time = Duration::from_secs(5);
/// limits.output_chars = 8_000;
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// How long the script itself may run, summed over `execute` and every `poll` of its run
    /// and not counting time spent waiting on the host. The default is 30 s.
    pub run_time: Duration,
    /// The most memory the script may use. In-process this does not cap the script's total
    /// memory. It only refuses one value bigger than the limit, such as one huge string, with
    /// `MemoryError`. A `backend::MontyPool` worker caps the script's total memory. The default is
    /// 256 MiB.
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
    /// `poll` before it expires. A run whose host has answered by then doesn't expire, and the
    /// next `poll` resumes it. The default is unset, no limit.
    pub idle_timeout: Option<Duration>,
    /// How many characters of output a status carries: a finished value or an error, together
    /// with what the script printed to stdout and stderr. Output over it keeps its head and
    /// tail around a marker such as `[… 60,000 of 100,000 characters cut …]`, and a value cut
    /// this way becomes a string. A value is counted as its JSON text, except a string, which
    /// is counted as its own text. The default is 40,000.
    pub output_chars: usize,
}

/// What `execute` or `poll` answers, from the sandbox to the caller. It serializes as the JSON
/// the caller sends to the model, such as `{"status": "running", "run_id": 1}`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    /// The script ended, and this is the value of its last expression.
    #[non_exhaustive]
    Finished {
        /// The value as JSON.
        value: Value,
        /// What the script printed to stdout, if it printed anything there.
        #[serde(skip_serializing_if = "Option::is_none")]
        stdout: Option<String>,
        /// What the script printed to stderr with `print(..., file=sys.stderr)`, if anything.
        #[serde(skip_serializing_if = "Option::is_none")]
        stderr: Option<String>,
        /// Whether the session started fresh before this script, with no globals. Serialized
        /// only when true.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        restarted: bool,
    },
    /// A tool call was still unanswered when the wait ended. Poll this run id for the rest.
    Running {
        /// The id to poll.
        run_id: u64,
    },
    /// The script or the request failed.
    #[non_exhaustive]
    Failed {
        /// What went wrong, such as the script's traceback.
        error: String,
        /// Every tool call the host answered in the run, refused and failed ones too. Each
        /// argument is cut to its share of [`Limits::output_chars`], which is
        /// `output_chars / calls_per_execute`.
        calls: Vec<host::Call>,
        /// What the script printed to stdout before the failure, if it printed anything there.
        /// A print that would take the script's prints on stdout and stderr together past
        /// 10 MiB raises `MemoryError` in the script, or in `backend::MontyPool` is dropped.
        #[serde(skip_serializing_if = "Option::is_none")]
        stdout: Option<String>,
        /// What the script printed to stderr with `print(..., file=sys.stderr)` before the
        /// failure, if anything. The traceback is in `error`, not here.
        #[serde(skip_serializing_if = "Option::is_none")]
        stderr: Option<String>,
        /// Whether the session started fresh before this script, with no globals. Serialized
        /// only when true.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        restarted: bool,
    },
}

/// Why the caller's [`Sandbox::dump`] or [`Builder::restore`] failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The session is held by the run with this id, which `execute` left running.
    RunOpen {
        /// The open run's id.
        run_id: u64,
    },
    /// The backend can't write the session as bytes, for this reason.
    Dump(Box<dyn error::Error + Send + Sync>),
    /// The bytes are too short to be a dump, start with another format version, or the backend
    /// can't load them as a session, for this reason.
    Restore(Box<dyn error::Error + Send + Sync>),
    /// The bytes don't end in the tag that the builder's signing key makes for them: the key
    /// didn't sign them, or they changed since.
    SignatureMismatch,
}

/// Where the sandbox's session is: idle between scripts, or held by a suspended run.
enum State<B: Backend> {
    Idle(B::Session),
    Suspended(Run<B>),
}

/// A script suspended at a tool call, with the host's answer still to be picked up.
struct Run<B: Backend> {
    answered_calls: Vec<host::Call>,
    call: host::Call,
    suspension: B::Suspension,
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
            output_chars: 40_000,
        }
    }
}

/// Awaits the `Status` future `step` in an info span named `name`, which records the run id, the
/// status, the call count and the duration.
macro_rules! trace_step {
    ($name:literal, $step:expr) => {{
        let span = tracing::info_span!(
            $name,
            run_id = Empty,
            status = Empty,
            call_count = Empty,
            duration = Empty
        );
        let started = Instant::now();
        let status: Status = $step.instrument(span.clone()).await;
        record_end(&span, status.as_str(), started);
        status
    }};
}

impl<H: Host, B: Backend> Sandbox<H, B> {
    /// The tools that [`Builder::new`] read from the host, which are the tools scripts call.
    /// Pass them to [`crate::schema::mcp_definitions`] or [`crate::schema::openai_definitions`]
    /// for the definitions the model sees.
    pub fn tools(&self) -> &[Tool] {
        &self.builder.tools
    }

    /// Runs `code`, waiting up to the execute wait for its tool calls. A script that fails is a
    /// `failed` status, as is a request while another run is still running. If a run waited
    /// past its idle timeout with no answer from the host, it expires and `execute` answers its
    /// `failed` status without running `code`. With `restart`, an idle session starts fresh
    /// before `code` runs, so the script sees no globals. A session lost to a dropped
    /// `execute`, `poll`, `cancel` or `dump` future also starts fresh, and the next finished or
    /// failed status says `restarted`.
    pub async fn execute(&mut self, code: &str, restart: bool) -> Status {
        trace_step!("execute", async {
            if let Some(status) = self.expire_idle_run().await {
                return status;
            }
            if self.state.is_none() || (restart && matches!(self.state, Some(State::Idle(_)))) {
                self.reset().await;
            }
            let session = match self.take_session() {
                Ok(session) => session,
                Err(error) => return Status::failed(error.to_string()),
            };
            self.last_run_id += 1;
            Span::current()
                .record("run_id", self.last_run_id)
                .record("call_count", 0);
            let progress = self
                .builder
                .backend
                .feed_start(session, code, &self.builder.tool_ids())
                .await;
            if let Some(status) = self.suspend_or_end(Vec::new(), progress).await {
                return status;
            }
            self.wait_timeout(self.builder.execute_wait).await
        })
    }

    /// Continues the run with `run_id` that `execute` left running, and answers `running` again
    /// if the host still owes it an answer when the poll wait ends.
    pub async fn poll(&mut self, run_id: u64) -> Status {
        trace_step!("poll", async {
            Span::current().record("run_id", run_id);
            if !self.is_open_run(run_id) {
                return Status::failed(format!("no run {run_id}"));
            }
            match self.expire_idle_run().await {
                Some(status) => status,
                None => self.wait_timeout(self.builder.poll_wait).await,
            }
        })
    }

    /// Cancels the run with `run_id` that `execute` left running, and says whether that run was
    /// open. The session is idle again with the globals the script had set, unless the backend
    /// lost them, which the next status reports as `restarted`. The host's answer to the call the
    /// script waits on is dropped. Cancelling any other run changes nothing.
    pub async fn cancel(&mut self, run_id: u64) -> bool {
        let is_open = self.is_open_run(run_id);
        let Some(run) = self.take_run_if(|_| is_open) else {
            return false;
        };
        let message = format!("run {run_id} was cancelled");
        let ending = self.builder.backend.abandon(run.suspension, message).await;
        self.state = Some(State::Idle(ending.session));
        self.unreported_restart |= ending.lost_globals;
        true
    }

    /// Replaces the session with an empty one, and marks the next finished or failed status
    /// `restarted`. An open run is dropped with the host's answer to its call, so polling it
    /// answers `failed`.
    pub async fn reset(&mut self) {
        self.state = Some(State::Idle(
            self.builder.backend.new_session(&self.builder.limits).await,
        ));
        self.unreported_restart = true;
    }

    /// The idle session as bytes that [`Builder::restore`] loads into a new sandbox, with the
    /// last run id and whether the next finished or failed status owes the model `restarted`.
    /// With a signing key, the bytes end in its HMAC-SHA256 tag. While a run is open this is
    /// [`Error::RunOpen`], and bytes the backend can't write are [`Error::Dump`]. A session lost
    /// to a dropped `execute`, `poll`, `cancel` or `dump` future starts fresh first.
    pub async fn dump(&mut self) -> Result<Vec<u8>, Error> {
        if self.state.is_none() {
            self.reset().await;
        }
        let mut session = self.take_session()?;
        let (dump, lost_globals) = self.builder.backend.dump(&mut session).await;
        self.state = Some(State::Idle(session));
        self.unreported_restart |= lost_globals;
        let mut bytes = vec![DUMP_FORMAT_VERSION, u8::from(self.unreported_restart)];
        bytes.extend(self.last_run_id.to_le_bytes());
        bytes.extend(dump.map_err(Error::Dump)?);
        if let Some(key) = self.builder.signing_key {
            let tag = compute_hmac(&key, &bytes).finalize().into_bytes();
            bytes.extend_from_slice(&tag);
        }
        Ok(bytes)
    }

    /// Takes the idle session out of the sandbox, whose state stays `None` until the session is
    /// put back. While a run is open this is [`Error::RunOpen`], and the run stays in the sandbox.
    fn take_session(&mut self) -> Result<B::Session, Error> {
        match self.state.take() {
            Some(State::Idle(session)) => Ok(session),
            suspended => {
                self.state = suspended;
                Err(Error::RunOpen {
                    run_id: self.last_run_id,
                })
            }
        }
    }

    /// Whether `run_id` is the id of the run `execute` left running.
    fn is_open_run(&self, run_id: u64) -> bool {
        matches!(self.state, Some(State::Suspended(_))) && run_id == self.last_run_id
    }

    /// Takes the suspended run out of the sandbox if `condition` holds for it.
    fn take_run_if(&mut self, condition: impl FnOnce(&Run<B>) -> bool) -> Option<Run<B>> {
        match self.state.take() {
            Some(State::Suspended(run)) if condition(&run) => Some(run),
            state => {
                self.state = state;
                None
            }
        }
    }

    /// Waits up to `wait` for the suspended run, answering its tool calls as the host answers
    /// them. The run stays suspended in the sandbox while it waits on the host, so dropping this
    /// future then loses nothing. Dropping it while the backend resumes or aborts the run loses
    /// the session.
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
                    return Status::Running {
                        run_id: self.last_run_id,
                    };
                }
            };
            let Some(State::Suspended(run)) = self.state.take() else {
                panic!("wait_timeout without a suspended run");
            };
            let Run {
                mut answered_calls,
                call,
                suspension,
                ..
            } = run;
            let progress = match answer {
                Some(answer) => {
                    answered_calls.push(self.cut_args(call));
                    Span::current().record("call_count", answered_calls.len());
                    self.builder
                        .backend
                        .resume(suspension, answer, &self.builder.tool_ids())
                        .await
                }
                None => {
                    let message = format!("{} passed the call timeout", call.tool_id);
                    let ending = self.builder.backend.abort(suspension, message).await;
                    return self.end_run(ending, answered_calls);
                }
            };
            if let Some(status) = self.suspend_or_end(answered_calls, progress).await {
                return status;
            }
        }
    }

    /// Suspends the last run at the tool call `progress` reached, or ends it with its status. A
    /// call past the call limit ends the run instead of reaching the host.
    async fn suspend_or_end(
        &mut self,
        answered_calls: Vec<host::Call>,
        progress: Progress<B>,
    ) -> Option<Status> {
        let call_limit = self.builder.limits.calls_per_execute;
        let ending = match progress {
            Progress::Call { call, suspension } if answered_calls.len() < call_limit => {
                let answer = self.call_host(&call);
                self.state = Some(State::Suspended(Run {
                    answered_calls,
                    call,
                    suspension,
                    answer,
                    idle_since: Instant::now(),
                }));
                return None;
            }
            Progress::Call { suspension, .. } => {
                let message = format!("the script reached the call limit of {call_limit}");
                self.builder.backend.abort(suspension, message).await
            }
            Progress::Ended(ending) => ending,
        };
        Some(self.end_run(ending, answered_calls))
    }

    /// Ends the suspended run if it has waited past the idle timeout without an answer from the
    /// host, and says it expired. A call past the call timeout counts as unanswered. A run whose
    /// host answered stays for the next wait to resume.
    async fn expire_idle_run(&mut self) -> Option<Status> {
        let idle_timeout = self.builder.limits.idle_timeout?;
        if let Some(State::Suspended(run)) = &mut self.state
            && let Some(answer) = (&mut run.answer).now_or_never()
        {
            let host_answered = answer.is_some();
            run.answer = future::ready(answer).boxed();
            if host_answered {
                return None;
            }
        }
        let run = self.take_run_if(|run| run.idle_since.elapsed() >= idle_timeout)?;
        let message = format!("run {} expired", self.last_run_id);
        let ending = self.builder.backend.abandon(run.suspension, message).await;
        Some(self.end_run(ending, run.answered_calls))
    }

    /// Ends the run: the `ending`'s session is idle again, and the script's result and what it
    /// printed to stdout and stderr become the status. The result gets its share of
    /// [`Limits::output_chars`] against both streams together, and the two streams then share
    /// the rest. The status says `restarted` if the session lost its globals since the last
    /// finished or failed status, and the next status does if the backend lost them in this run.
    fn end_run(&mut self, ending: Ending<B>, answered_calls: Vec<host::Call>) -> Status {
        let Ending {
            result,
            streams,
            session,
            lost_globals,
        } = ending;
        self.state = Some(State::Idle(session));
        let result_text = match &result {
            Ok(value) => to_text(value),
            Err(error) => Cow::Borrowed(error.as_str()),
        };
        let result_chars = result_text.chars().count();
        let Streams { stdout, stderr } = streams;
        let (stdout_chars, stderr_chars) = (stdout.chars().count(), stderr.chars().count());
        let (result_share, streams_share) = split_budget(
            self.builder.limits.output_chars,
            result_chars,
            stdout_chars + stderr_chars,
        );
        let (stdout_share, stderr_share) = split_budget(streams_share, stdout_chars, stderr_chars);
        let result_cut = cut_text(&result_text, result_chars, result_share);
        drop(result_text);
        let stdout = (!stdout.is_empty())
            .then(|| cut_text(&stdout, stdout_chars, stdout_share).unwrap_or(stdout));
        let stderr = (!stderr.is_empty())
            .then(|| cut_text(&stderr, stderr_chars, stderr_share).unwrap_or(stderr));
        let restarted = mem::replace(&mut self.unreported_restart, lost_globals);
        match result {
            Ok(value) => Status::Finished {
                value: result_cut.map_or(value, Value::String),
                stdout,
                stderr,
                restarted,
            },
            Err(error) => Status::Failed {
                error: result_cut.unwrap_or(error),
                calls: answered_calls,
                stdout,
                stderr,
                restarted,
            },
        }
    }

    /// `call` with each argument cut to its share of [`Limits::output_chars`], which is
    /// `output_chars / calls_per_execute`.
    fn cut_args(&self, mut call: host::Call) -> host::Call {
        let limits = &self.builder.limits;
        let share = limits.output_chars / limits.calls_per_execute.max(1);
        for value in call.args.values_mut() {
            let text = to_text(value);
            let cut = cut_text(&text, text.chars().count(), share);
            drop(text);
            if let Some(cut) = cut {
                *value = Value::String(cut);
            }
        }
        call
    }

    /// Sends `call`, made by the last run, to the host and traces its answer in a `call` span
    /// with no parent. The answer is `None` if the call timeout passes first.
    fn call_host(
        &self,
        call: &host::Call,
    ) -> BoxFuture<'static, Option<Result<Value, host::Error>>> {
        let run_id = self.last_run_id;
        let span = tracing::info_span!(parent: None, "call", run_id, tool_id = %call.tool_id, status = Empty, duration = Empty);
        let answer = self.builder.host.call(call);
        let timeout = self.builder.limits.call_timeout.map(Delay::new);
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

impl<H: Host, B: Backend> Builder<H, B> {
    /// A builder for a sandbox whose scripts call `host`'s tools and run in `backend`. It reads
    /// the host's tool list here, once: a tool the host lists later is not callable in the
    /// sandbox, and the backend gets the tool ids on every call that runs a script.
    pub fn new(host: H, backend: B) -> Self {
        let tools = host.tools();
        Self {
            host,
            tools,
            backend,
            execute_wait: Duration::from_secs(30),
            poll_wait: Duration::from_secs(10),
            limits: Limits::default(),
            signing_key: None,
        }
    }

    /// How long `execute` waits on the host before it answers `running`. The default is 30 s.
    pub fn execute_wait(mut self, wait: Duration) -> Self {
        self.execute_wait = wait;
        self
    }

    /// How long `poll` waits on the host before it answers `running` again. The default is 10 s.
    pub fn poll_wait(mut self, wait: Duration) -> Self {
        self.poll_wait = wait;
        self
    }

    /// What a script may use before the sandbox stops it. The default is [`Limits::default`].
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The secret key that signs dumps. With it, [`Sandbox::dump`] appends an HMAC-SHA256 tag,
    /// and [`Builder::restore`] checks the tag before the backend reads a byte. The default is
    /// unset, no signing.
    ///
    /// The tag proves the key signed the bytes. It doesn't hide them, and any dump the key
    /// signed restores, including an older one. Use the key only for dumps. The key isn't wiped
    /// from memory when the sandbox drops.
    pub fn signing_key(mut self, key: [u8; 32]) -> Self {
        self.signing_key = Some(key);
        self
    }

    /// The sandbox, with an empty Python session.
    pub async fn build(self) -> Sandbox<H, B> {
        let session = self.backend.new_session(&self.limits).await;
        self.build_with_session(session, 0, false)
    }

    /// The sandbox, with the session that `bytes` from [`Sandbox::dump`] hold, running under
    /// this builder's limits. Its run ids go on from the dumped sandbox's last one, and its next
    /// finished or failed status says `restarted` if the dumped sandbox's would have. Bytes too
    /// short to be a dump, that start with another format version, or that the backend can't
    /// load, are [`Error::Restore`].
    ///
    /// With a [signing key](Builder::signing_key), bytes that don't end in the key's tag for
    /// them are [`Error::SignatureMismatch`], checked in constant time before any other byte is
    /// read. Without a key, restore trusts the bytes, so restore only bytes you made.
    pub async fn restore(self, bytes: &[u8]) -> Result<Sandbox<H, B>, Error> {
        let dump = match self.signing_key {
            Some(key) => {
                let (dump, tag) = bytes
                    .split_last_chunk::<TAG_BYTES>()
                    .ok_or(Error::SignatureMismatch)?;
                compute_hmac(&key, dump)
                    .verify_slice(tag)
                    .map_err(|_| Error::SignatureMismatch)?;
                dump
            }
            None => bytes,
        };
        let ([version, restart_byte, last_run_id_bytes @ ..], session_bytes) = dump
            .split_first_chunk::<HEADER_BYTES>()
            .ok_or_else(|| Error::Restore("the bytes are too short to be a dump".into()))?;
        if *version != DUMP_FORMAT_VERSION {
            return Err(Error::Restore(
                format!("the dump is format version {version}, not version {DUMP_FORMAT_VERSION}")
                    .into(),
            ));
        }
        let session = self
            .backend
            .restore(session_bytes, &self.limits)
            .await
            .map_err(Error::Restore)?;
        let last_run_id = u64::from_le_bytes(*last_run_id_bytes);
        Ok(self.build_with_session(session, last_run_id, *restart_byte != 0))
    }

    /// The sandbox, with `session` as its idle Python session, `last_run_id` as the id its next
    /// run counts on from, and `unreported_restart` as whether its next finished or failed status
    /// says `restarted`.
    fn build_with_session(
        self,
        session: B::Session,
        last_run_id: u64,
        unreported_restart: bool,
    ) -> Sandbox<H, B> {
        Sandbox {
            builder: self,
            state: Some(State::Idle(session)),
            last_run_id,
            unreported_restart,
        }
    }

    /// The id of each tool the host listed.
    fn tool_ids(&self) -> Vec<String> {
        self.tools.iter().map(|tool| tool.id.clone()).collect()
    }
}

impl<H, B: Backend> fmt::Debug for Sandbox<H, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sandbox")
            .field("builder", &self.builder)
            .field("last_run_id", &self.last_run_id)
            .finish_non_exhaustive()
    }
}

impl<H, B> fmt::Debug for Builder<H, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("tools", &self.tools)
            .field("execute_wait", &self.execute_wait)
            .field("poll_wait", &self.poll_wait)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunOpen { run_id } => write!(f, "run {run_id} is still running"),
            Self::Dump(error) => write!(f, "can't dump the session: {error}"),
            Self::Restore(error) => write!(f, "can't restore the session: {error}"),
            Self::SignatureMismatch => f.write_str("the signing key didn't sign these bytes"),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::RunOpen { .. } | Self::SignatureMismatch => None,
            Self::Dump(error) | Self::Restore(error) => Some(error.as_ref()),
        }
    }
}

impl Status {
    /// A `failed` status with `error`, no answered calls, no printed output and no restart.
    fn failed(error: String) -> Self {
        Self::Failed {
            error,
            calls: Vec::new(),
            stdout: None,
            stderr: None,
            restarted: false,
        }
    }

    /// The status's `status` tag in the model's JSON, such as `"running"`.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Finished { .. } => "finished",
            Self::Running { .. } => "running",
            Self::Failed { .. } => "failed",
        }
    }
}

/// The text that `value` is measured by against [`Limits::output_chars`], and cut from when it
/// doesn't fit: a string's own text, or any other value's JSON text.
fn to_text(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(text) => Cow::Borrowed(text),
        other => Cow::Owned(serde_json::to_string(other).expect("a Value always serializes")),
    }
}

/// `text`, which is `text_chars` characters long, cut to its first and last characters, up to
/// `max_chars` in all, around a marker such as `[… 60,000 of 100,000 characters cut …]`. `None`
/// if `text` already fits in `max_chars`.
fn cut_text(text: &str, text_chars: usize, max_chars: usize) -> Option<String> {
    if text_chars <= max_chars {
        return None;
    }
    let tail_chars = max_chars / 2;
    let head_end = text
        .char_indices()
        .nth(max_chars - tail_chars)
        .map_or(text.len(), |(byte, _)| byte);
    let tail_start = text
        .char_indices()
        .rev()
        .take(tail_chars)
        .last()
        .map_or(text.len(), |(byte, _)| byte);
    let head = &text[..head_end];
    let tail = &text[tail_start..];
    let cut_count = group_thousands(text_chars - max_chars);
    let total_count = group_thousands(text_chars);
    Some(format!(
        "{head}[… {cut_count} of {total_count} characters cut …]{tail}"
    ))
}

/// `number` in digits with a comma between each group of three, such as `100,000`.
fn group_thousands(number: usize) -> String {
    let digits = number.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// How many of `budget` characters two texts of `first_chars` and `second_chars` each get, as
/// `(first_share, second_share)`. Each gets what it needs, and when both need more, they split
/// the budget evenly, the first taking any odd character.
fn split_budget(budget: usize, first_chars: usize, second_chars: usize) -> (usize, usize) {
    let second_share = budget
        .saturating_sub(first_chars)
        .max(budget / 2)
        .min(second_chars);
    (budget - second_share, second_share)
}

/// The HMAC-SHA256 of `bytes` under `key`, to finalize as a tag or to check a tag against.
fn compute_hmac(key: &[u8], bytes: &[u8]) -> Hmac<Sha256> {
    let mut hmac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC-SHA256 takes a key of any length");
    hmac.update(bytes);
    hmac
}

/// Records on `span` the status a traced step ended with and how long it took since `started`.
fn record_end(span: &Span, status: &'static str, started: Instant) {
    span.record("status", status);
    span.record("duration", field::debug(started.elapsed()));
}
