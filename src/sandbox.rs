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

use crate::host::{self, Host};
use crate::repl::{DumpError, MontyRepl, Progress, Repl, ReplFunctionCall};

/// One Python session that runs the model's scripts, and the host whose tools they call.
/// Globals persist from one `execute` to the next, and one run is in flight at a time.
pub struct Sandbox<H> {
    host: H,
    repl: Repl,
    state: Option<State>,
    last_run_id: u64,
    unreported_restart: bool,
    execute_wait: Duration,
    poll_wait: Duration,
    limits: Limits,
    signing_key: Option<Vec<u8>>,
}

/// Sets up a [`Sandbox`]. Made by [`Sandbox::builder`].
pub struct Builder<H> {
    host: H,
    execute_wait: Duration,
    poll_wait: Duration,
    limits: Limits,
    signing_key: Option<Vec<u8>>,
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
    /// `poll` before it expires. The default is unset, no limit.
    pub idle_timeout: Option<Duration>,
    /// How many characters of output a status carries: a finished value or an error, together
    /// with what the script printed. Output over it keeps its head and tail around a marker
    /// such as `[… 60,000 of 100,000 characters cut …]`, and a value cut this way becomes a
    /// string. A value is counted as its JSON text, except a string, which is counted as its
    /// own text. The default is 40,000.
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
        /// What the script printed, if it printed anything.
        #[serde(skip_serializing_if = "Option::is_none")]
        printed: Option<String>,
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
        /// What the script printed before the failure, if it printed anything. A print that
        /// would take the script's prints past 10 MiB raises `MemoryError` in the script.
        #[serde(skip_serializing_if = "Option::is_none")]
        printed: Option<String>,
        /// Whether the session started fresh before this script, with no globals. Serialized
        /// only when true.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        restarted: bool,
    },
}

/// Why the caller's [`Sandbox::dump`] or [`Builder::restore`] failed.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The session is held by the run with this id, which `execute` left running.
    RunOpen {
        /// The open run's id.
        run_id: u64,
    },
    /// Monty can't load these bytes as a session.
    Restore(DumpError),
    /// The bytes don't end in the tag that the builder's signing key makes for them: the key
    /// didn't sign them, or they changed since.
    SignatureMismatch,
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
    printed: String,
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
            output_chars: 40_000,
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
            signing_key: None,
        }
    }

    /// Runs `code`, waiting up to the execute wait for its tool calls. The wait bounds only the
    /// time spent waiting on the host; [`Limits::run_time`] bounds the script's own time. A
    /// script that fails is a `failed` status, as is a request while another run is still
    /// running. If a run waited past its idle timeout, it expires and `execute` answers its
    /// `failed` status without running `code`. With `restart`, an idle session starts fresh
    /// before `code` runs, so the script sees no globals.
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
            if let Some(status) = self.expire_idle_run() {
                return status;
            }
            if restart && matches!(self.state, Some(State::Idle(_))) {
                self.reset();
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
                        printed: None,
                        restarted: false,
                    };
                }
            };
            self.last_run_id += 1;
            let id = self.last_run_id;
            Span::current().record("run_id", id).record("call_count", 0);
            let mut printed = String::new();
            let progress = self.repl.feed_start(session, code, &mut printed);
            if let Some(status) = self.suspend_or_end(id, Vec::new(), printed, progress) {
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
                    printed: None,
                    restarted: false,
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

    /// Cancels the run with `run_id` that `execute` left running, and says whether that run was
    /// open. The session is idle again with the globals the script had set, and the host's
    /// answer to the call the script waits on is dropped. Cancelling any other run changes
    /// nothing.
    pub fn cancel(&mut self, run_id: u64) -> bool {
        let Some(run) = self.take_run_if(|run| run.id == run_id) else {
            return false;
        };
        self.state = Some(State::Idle(Repl::abandon(run.function_call)));
        true
    }

    /// Replaces the session with an empty one, and marks the next finished or failed status
    /// `restarted`. An open run is dropped with the host's answer to its call, so polling it
    /// answers `failed`.
    pub fn reset(&mut self) {
        self.state = Some(State::Idle(Repl::new_session(&self.limits)));
        self.unreported_restart = true;
    }

    /// The idle session as bytes that [`Builder::restore`] loads into a new sandbox. With a
    /// signing key, the bytes end in its HMAC-SHA256 tag. While a run is open this is
    /// [`Error::RunOpen`].
    pub fn dump(&self) -> Result<Vec<u8>, Error> {
        match &self.state {
            Some(State::Idle(session)) => {
                let mut bytes = Repl::dump_session(session);
                if let Some(key) = &self.signing_key {
                    let tag = compute_hmac(key, &bytes).finalize().into_bytes();
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
    fn take_run_if(&mut self, condition: impl FnOnce(&Run) -> bool) -> Option<Run> {
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
                mut printed,
                call,
                function_call,
                ..
            } = run;
            let progress = match answer {
                Some(answer) => {
                    answered_calls.push(self.cut_args(call));
                    Span::current().record("call_count", answered_calls.len());
                    self.repl.resume(function_call, answer, &mut printed)
                }
                None => {
                    let message = format!("{} passed the call timeout", call.tool_id);
                    self.repl.abort(function_call, message, &mut printed)
                }
            };
            if let Some(status) = self.suspend_or_end(id, answered_calls, printed, progress) {
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
        mut printed: String,
        mut progress: Progress,
    ) -> Option<Status> {
        let call_limit = self.limits.calls_per_execute;
        if answered_calls.len() >= call_limit
            && let Progress::Call { function_call, .. } = progress
        {
            let message = format!("the script reached the call limit of {call_limit}");
            progress = self.repl.abort(function_call, message, &mut printed);
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
                    printed,
                    call,
                    function_call,
                    answer,
                    idle_since: Instant::now(),
                }));
                None
            }
            Progress::Ended { result, session } => {
                Some(self.end_run(session, result, answered_calls, printed))
            }
        }
    }

    /// Ends the suspended run if it has waited past the idle timeout, and says it expired.
    fn expire_idle_run(&mut self) -> Option<Status> {
        let idle_timeout = self.limits.idle_timeout?;
        let run = self.take_run_if(|run| run.idle_since.elapsed() >= idle_timeout)?;
        let session = Repl::abandon(run.function_call);
        let error = format!("run {} expired", run.id);
        Some(self.end_run(session, Err(error), run.answered_calls, run.printed))
    }

    /// Ends the run: `session` is idle again, and the script's result and what it printed become
    /// the status, cut to share [`Limits::output_chars`]. The status is `restarted` if the
    /// session started fresh since the last status that was.
    fn end_run(
        &mut self,
        session: Box<MontyRepl>,
        result: Result<Value, String>,
        answered_calls: Vec<host::Call>,
        printed: String,
    ) -> Status {
        self.state = Some(State::Idle(session));
        let result_chars = match &result {
            Ok(value) => count_chars(value),
            Err(error) => error.chars().count(),
        };
        let printed_chars = printed.chars().count();
        let (result_share, printed_share) =
            split_budget(self.limits.output_chars, result_chars, printed_chars);
        let printed = (!printed.is_empty()).then(|| cut_text(printed, printed_share));
        let restarted = mem::take(&mut self.unreported_restart);
        match result {
            Ok(value) => Status::Finished {
                value: cut_value(value, result_share),
                printed,
                restarted,
            },
            Err(error) => Status::Failed {
                error: cut_text(error, result_share),
                calls: answered_calls,
                printed,
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

    /// The secret key that signs dumps. With it, [`Sandbox::dump`] appends an HMAC-SHA256 tag,
    /// and [`Builder::restore`] checks the tag before Monty reads a byte. The default is unset,
    /// no signing.
    ///
    /// The tag proves the key signed the bytes. It doesn't hide them, and any dump the key
    /// signed restores, including an older one. Use a random key of at least 32 bytes, and only
    /// for dumps.
    pub fn signing_key(mut self, key: impl Into<Vec<u8>>) -> Self {
        self.signing_key = Some(key.into());
        self
    }

    /// The sandbox, with an empty Python session.
    pub fn build(self) -> Sandbox<H> {
        let session = Repl::new_session(&self.limits);
        self.build_with_session(session)
    }

    /// The sandbox, with the session that `bytes` from [`Sandbox::dump`] hold, running under
    /// this builder's limits. Bytes Monty can't load are [`Error::Restore`].
    ///
    /// With a [signing key](Builder::signing_key), bytes that don't end in the key's tag for
    /// them are [`Error::SignatureMismatch`], checked in constant time before Monty reads a
    /// byte. Without a key, restore trusts the bytes. Monty doesn't authenticate dumps, and
    /// bytes from anyone else can panic, abort or hang the process, so restore only bytes you
    /// made.
    pub fn restore(self, bytes: &[u8]) -> Result<Sandbox<H>, Error> {
        let session_bytes = match &self.signing_key {
            Some(key) => {
                let (session_bytes, tag) = bytes
                    .split_last_chunk::<TAG_BYTES>()
                    .ok_or(Error::SignatureMismatch)?;
                compute_hmac(key, session_bytes)
                    .verify_slice(tag)
                    .map_err(|_| Error::SignatureMismatch)?;
                session_bytes
            }
            None => bytes,
        };
        let session = Repl::load_session(session_bytes, &self.limits).map_err(Error::Restore)?;
        Ok(self.build_with_session(session))
    }

    /// The sandbox, with `session` as its idle Python session.
    fn build_with_session(self, session: Box<MontyRepl>) -> Sandbox<H> {
        let tool_ids = self.host.tools().into_iter().map(|tool| tool.id).collect();
        Sandbox {
            host: self.host,
            repl: Repl::new(tool_ids),
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
            Self::Restore(error) => Some(error),
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

/// How many of `budget` characters a run's result and its prints each get, as
/// `(result_share, printed_share)`. Each gets what it needs, and when both need more, they
/// split the budget evenly.
fn split_budget(budget: usize, result_chars: usize, printed_chars: usize) -> (usize, usize) {
    let printed_share = budget
        .saturating_sub(result_chars)
        .max(budget / 2)
        .min(printed_chars);
    (budget - printed_share, printed_share)
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
