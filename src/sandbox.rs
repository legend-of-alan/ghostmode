//! The sandbox that answers the model's `execute` and `poll` tool calls.

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

use crate::backend::{Backend, Progress, Streams};
use crate::host::{self, Host};

/// One Python session that runs the model's scripts in backend `B`, and the host whose tools
/// they call. Globals persist from one `execute` to the next, and one run is in flight at a time.
pub struct Sandbox<H, B: Backend> {
    host: H,
    backend: B,
    state: Option<State<B>>,
    last_run_id: u64,
    unreported_restart: bool,
    execute_wait: Duration,
    poll_wait: Duration,
    limits: Limits,
    signing_key: Option<[u8; 32]>,
}

/// Sets up a [`Sandbox`]. Made by [`Sandbox::builder`] or [`Builder::new`].
pub struct Builder<H, B> {
    host: H,
    backend: B,
    execute_wait: Duration,
    poll_wait: Duration,
    limits: Limits,
    signing_key: Option<[u8; 32]>,
}

/// How many bytes the HMAC-SHA256 tag takes that a signing key appends to a dump.
const TAG_BYTES: usize = 32;

/// What a script may use before the sandbox stops it with a `failed` status. A stopped script
/// leaves the session's globals as it had set them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// How long the script itself may run, summed over `execute` and every `poll` of its run
    /// and not counting time spent waiting on the host. The default is 30 s.
    pub run_time: Duration,
    /// The most memory the script may use. In-process this does not cap the script's total
    /// memory. It only refuses one value bigger than the limit, such as one huge string, with
    /// `MemoryError`. The default is 256 MiB.
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

/// What `execute` or `poll` answers, from the sandbox to the model. It serializes as the JSON
/// the model sees, such as `{"status": "running", "run_id": 1}`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    /// The script ended, and this is the value of its last expression.
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
    Failed {
        /// What went wrong, such as the script's traceback.
        error: String,
        /// The tool calls the host answered before the failure. Each argument is cut to its
        /// share of [`Limits::output_chars`], which is `output_chars / calls_per_execute`.
        calls: Vec<host::Call>,
        /// What the script printed to stdout before the failure, if it printed anything there.
        /// A print that would take the script's prints on stdout and stderr together past
        /// 10 MiB raises `MemoryError` in the script.
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
pub enum Error {
    /// The session is held by the run with this id, which `execute` left running.
    RunOpen {
        /// The open run's id.
        run_id: u64,
    },
    /// The backend can't load these bytes as a session, for this reason.
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

/// A script suspended at a tool call the host has not answered yet.
struct Run<B: Backend> {
    id: u64,
    answered_calls: Vec<host::Call>,
    call: host::Call,
    suspension: B::Suspension,
    /// The host's answer to `call`, or `None` once the call timeout passes first.
    answer: BoxFuture<'static, Option<Result<Value, host::Error>>>,
    idle_since: Instant,
}

impl<B: Backend> Run<B> {
    /// Whether the host has answered `call` or the call timeout has passed. A ready answer stays
    /// in the run for the next wait.
    fn is_answer_ready(&mut self) -> bool {
        let Some(answer) = (&mut self.answer).now_or_never() else {
            return false;
        };
        self.answer = future::ready(answer).boxed();
        true
    }
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

impl<H: Host, B: Backend> Sandbox<H, B> {
    /// Runs `code`, waiting up to the execute wait for its tool calls. The wait bounds only the
    /// time spent waiting on the host; [`Limits::run_time`] bounds the script's own time. A
    /// script that fails is a `failed` status, as is a request while another run is still
    /// running. If a run waited past its idle timeout with no answer from the host, it expires
    /// and `execute` answers its `failed` status without running `code`. With `restart`, an
    /// idle session starts fresh before `code` runs, so the script sees no globals.
    pub async fn execute(&mut self, code: &str, restart: bool) -> Status {
        let span = tracing::info_span!(
            "execute",
            run_id = Empty,
            status = Empty,
            call_count = Empty,
            duration = Empty
        );
        let started = Instant::now();
        let status = async {
            if let Some(status) = self.expire_idle_run().await {
                return status;
            }
            if restart && matches!(self.state, Some(State::Idle(_))) {
                self.reset().await;
            }
            let session = match self.state.take() {
                Some(State::Idle(session)) => session,
                suspended => {
                    self.state = suspended;
                    let run_id = self.last_run_id;
                    let error = Error::RunOpen { run_id }.to_string();
                    return Status::Failed {
                        error,
                        calls: Vec::new(),
                        stdout: None,
                        stderr: None,
                        restarted: false,
                    };
                }
            };
            self.last_run_id += 1;
            let id = self.last_run_id;
            Span::current().record("run_id", id).record("call_count", 0);
            let progress = self.backend.feed_start(session, code).await;
            if let Some(status) = self.suspend_or_end(id, Vec::new(), progress).await {
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
                    stdout: None,
                    stderr: None,
                    restarted: false,
                };
            }
            match self.expire_idle_run().await {
                Some(status) => status,
                None => self.wait_timeout(self.poll_wait).await,
            }
        }
        .instrument(span.clone())
        .await;
        record_end(&span, status.as_str(), started);
        status
    }

    /// Cancels the run with `run_id` that `execute` left running, and says whether that run was
    /// open. The session is idle again with the globals the script had set, and the host's
    /// answer to the call the script waits on is dropped. Cancelling any other run changes
    /// nothing.
    pub async fn cancel(&mut self, run_id: u64) -> bool {
        let Some(run) = self.take_run_if(|run| run.id == run_id) else {
            return false;
        };
        let (session, _) = self.backend.abandon(run.suspension).await;
        self.state = Some(State::Idle(session));
        true
    }

    /// Replaces the session with an empty one, and marks the next finished or failed status
    /// `restarted`. An open run is dropped with the host's answer to its call, so polling it
    /// answers `failed`.
    pub async fn reset(&mut self) {
        self.state = Some(State::Idle(self.backend.new_session(&self.limits).await));
        self.unreported_restart = true;
    }

    /// The idle session as bytes that [`Builder::restore`] loads into a new sandbox. With a
    /// signing key, the bytes end in its HMAC-SHA256 tag. While a run is open this is
    /// [`Error::RunOpen`].
    pub async fn dump(&mut self) -> Result<Vec<u8>, Error> {
        match &mut self.state {
            Some(State::Idle(session)) => {
                let mut bytes = self.backend.dump(session).await;
                if let Some(key) = self.signing_key {
                    let tag = compute_hmac(&key, &bytes).finalize().into_bytes();
                    bytes.extend_from_slice(&tag);
                }
                Ok(bytes)
            }
            _ => Err(Error::RunOpen {
                run_id: self.last_run_id,
            }),
        }
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
                suspension,
                ..
            } = run;
            let progress = match answer {
                Some(answer) => {
                    answered_calls.push(self.cut_args(call));
                    Span::current().record("call_count", answered_calls.len());
                    self.backend.resume(suspension, answer).await
                }
                None => {
                    let message = format!("{} passed the call timeout", call.tool_id);
                    self.backend.abort(suspension, message).await
                }
            };
            if let Some(status) = self.suspend_or_end(id, answered_calls, progress).await {
                return status;
            }
        }
    }

    /// Suspends run `id` at the tool call `progress` reached, or ends it with its status. A call
    /// past the call limit ends the run instead of reaching the host.
    async fn suspend_or_end(
        &mut self,
        id: u64,
        answered_calls: Vec<host::Call>,
        mut progress: Progress<B>,
    ) -> Option<Status> {
        let call_limit = self.limits.calls_per_execute;
        if answered_calls.len() >= call_limit
            && let Progress::Call { suspension, .. } = progress
        {
            let message = format!("the script reached the call limit of {call_limit}");
            progress = self.backend.abort(suspension, message).await;
        }
        match progress {
            Progress::Call { call, suspension } => {
                let answer = self.call_host(id, &call);
                self.state = Some(State::Suspended(Run {
                    id,
                    answered_calls,
                    call,
                    suspension,
                    answer,
                    idle_since: Instant::now(),
                }));
                None
            }
            Progress::Ended {
                result,
                streams,
                session,
            } => Some(self.end_run(session, result, answered_calls, streams)),
        }
    }

    /// Ends the suspended run if it has waited past the idle timeout and the host's answer is not
    /// ready, and says it expired. A run whose answer is ready stays for the next wait to resume.
    async fn expire_idle_run(&mut self) -> Option<Status> {
        let idle_timeout = self.limits.idle_timeout?;
        if let Some(State::Suspended(run)) = &mut self.state
            && run.is_answer_ready()
        {
            return None;
        }
        let run = self.take_run_if(|run| run.idle_since.elapsed() >= idle_timeout)?;
        let (session, streams) = self.backend.abandon(run.suspension).await;
        let error = format!("run {} expired", run.id);
        Some(self.end_run(session, Err(error), run.answered_calls, streams))
    }

    /// Ends the run: `session` is idle again, and the script's result and what it printed to
    /// stdout and stderr become the status. The result gets its share of
    /// [`Limits::output_chars`] against both streams together, and the two streams then share
    /// the rest. The status is `restarted` if the session started fresh since the last status
    /// that was.
    fn end_run(
        &mut self,
        session: B::Session,
        result: Result<Value, String>,
        answered_calls: Vec<host::Call>,
        streams: Streams,
    ) -> Status {
        self.state = Some(State::Idle(session));
        let result_chars = match &result {
            Ok(value) => count_chars(value),
            Err(error) => error.chars().count(),
        };
        let Streams { stdout, stderr } = streams;
        let (stdout_chars, stderr_chars) = (stdout.chars().count(), stderr.chars().count());
        let (result_share, streams_share) = split_budget(
            self.limits.output_chars,
            result_chars,
            stdout_chars + stderr_chars,
        );
        let (stdout_share, stderr_share) = split_budget(streams_share, stdout_chars, stderr_chars);
        let stdout = (!stdout.is_empty()).then(|| cut_text(stdout, stdout_share));
        let stderr = (!stderr.is_empty()).then(|| cut_text(stderr, stderr_share));
        let restarted = mem::take(&mut self.unreported_restart);
        match result {
            Ok(value) => Status::Finished {
                value: cut_value(value, result_share),
                stdout,
                stderr,
                restarted,
            },
            Err(error) => Status::Failed {
                error: cut_text(error, result_share),
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
        let share = self.limits.output_chars / self.limits.calls_per_execute.max(1);
        for value in call.args.values_mut() {
            *value = cut_value(mem::take(value), share);
        }
        call
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

impl<H: Host, B: Backend> Builder<H, B> {
    /// A builder for a sandbox whose scripts call `host`'s tools and run in `backend`.
    pub fn new(host: H, backend: B) -> Self {
        Self {
            host,
            backend,
            execute_wait: Duration::from_secs(30),
            poll_wait: Duration::from_secs(10),
            limits: Limits::default(),
            signing_key: None,
        }
    }

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
        self.build_with_session(session)
    }

    /// The sandbox, with the session that `bytes` from [`Sandbox::dump`] hold, running under
    /// this builder's limits. Bytes the backend can't load are [`Error::Restore`].
    ///
    /// With a [signing key](Builder::signing_key), bytes that don't end in the key's tag for
    /// them are [`Error::SignatureMismatch`], checked in constant time before the backend reads
    /// a byte. Without a key, restore trusts the bytes, so restore only bytes you made.
    pub async fn restore(self, bytes: &[u8]) -> Result<Sandbox<H, B>, Error> {
        let session_bytes = match self.signing_key {
            Some(key) => {
                let (session_bytes, tag) = bytes
                    .split_last_chunk::<TAG_BYTES>()
                    .ok_or(Error::SignatureMismatch)?;
                compute_hmac(&key, session_bytes)
                    .verify_slice(tag)
                    .map_err(|_| Error::SignatureMismatch)?;
                session_bytes
            }
            None => bytes,
        };
        let session = self
            .backend
            .restore(session_bytes, &self.limits)
            .await
            .map_err(Error::Restore)?;
        Ok(self.build_with_session(session))
    }

    /// The sandbox, with `session` as its idle Python session.
    fn build_with_session(self, session: B::Session) -> Sandbox<H, B> {
        Sandbox {
            host: self.host,
            backend: self.backend,
            state: Some(State::Idle(session)),
            last_run_id: 0,
            unreported_restart: false,
            execute_wait: self.execute_wait,
            poll_wait: self.poll_wait,
            limits: self.limits,
            signing_key: self.signing_key,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunOpen { run_id } => write!(f, "run {run_id} is still running"),
            Self::Restore(error) => write!(f, "can't restore the session: {error}"),
            Self::SignatureMismatch => f.write_str("the signing key didn't sign these bytes"),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::RunOpen { .. } | Self::SignatureMismatch => None,
            Self::Restore(error) => Some(error.as_ref()),
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

/// How many characters `value` counts for against [`Limits::output_chars`]: a string's own
/// text, or any other value's JSON text.
fn count_chars(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        other => other.to_string().chars().count(),
    }
}

/// `value`, or if [`count_chars`] finds it longer than `max_chars`, its text cut by [`cut_text`]
/// as a string.
fn cut_value(value: Value, max_chars: usize) -> Value {
    match value {
        Value::String(text) => Value::String(cut_text(text, max_chars)),
        other => {
            let text = other.to_string();
            if text.chars().count() <= max_chars {
                other
            } else {
                Value::String(cut_text(text, max_chars))
            }
        }
    }
}

/// `text`, or if it is longer than `max_chars` characters, its first and last characters, up to
/// `max_chars` in all, around a marker such as `[… 60,000 of 100,000 characters cut …]`.
fn cut_text(text: String, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text;
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
    let cut_count = group_thousands(total - max_chars);
    let total_count = group_thousands(total);
    format!("{head}[… {cut_count} of {total_count} characters cut …]{tail}")
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
