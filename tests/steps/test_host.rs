use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cucumber::{given, when};
use futures::channel::oneshot;
use futures::future::{self, BoxFuture, FutureExt};
use ghostmode::Limits;
use ghostmode::host::{self, Host};
use serde_json::Value;

use crate::World;

/// What a test host's tool answers when the script calls it, from the test host to the script.
#[derive(Clone)]
pub(crate) enum Answer {
    Value(Value),
    Refused(String),
    Failed(String),
    /// Answers once a When step sends the answer through `TestHost::held_calls`.
    Held,
}

/// A host whose tools answer as the scenario's Given steps set up.
#[derive(Clone, Default)]
pub(crate) struct TestHost {
    /// Each tool's answer, by tool id.
    pub(crate) answers: HashMap<String, Answer>,
    /// The sending halves that answer `Answer::Held` calls by tool id, from a When step to the
    /// call's future.
    pub(crate) held_calls: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
}

impl Host for TestHost {
    fn tools(&self) -> Vec<host::Tool> {
        self.answers
            .keys()
            .map(|id| host::Tool {
                id: id.clone(),
                description: format!("The scenario's {id} tool."),
                input_schema: serde_json::json!({"type": "object"}),
            })
            .collect()
    }

    fn call(&self, call: &host::Call) -> BoxFuture<'static, Result<Value, host::Error>> {
        match self.answers[&call.tool_id].clone() {
            Answer::Value(value) => future::ready(Ok(value)).boxed(),
            Answer::Refused(reason) => future::ready(Err(host::Error::Refused(reason))).boxed(),
            Answer::Failed(message) => future::ready(Err(host::Error::Failed(message))).boxed(),
            Answer::Held => {
                let (sender, receiver) = oneshot::channel();
                let held_calls = &mut self.held_calls.lock().expect("held calls");
                held_calls.insert(call.tool_id.clone(), sender);
                receiver
                    .map(|answer| answer.map_err(|_| host::Error::Failed("dropped".into())))
                    .boxed()
            }
        }
    }
}

/// The JSON a step wrote.
pub(crate) fn parse_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|err| panic!("{text} is not JSON: {err}"))
}

#[given(regex = r#"^a host whose `(\w+)` answers ([\[{"].*)$"#)]
fn host_answers(world: &mut World, tool_id: String, answer: String) {
    world
        .host
        .answers
        .insert(tool_id, Answer::Value(parse_json(&answer)));
}

#[given(regex = r#"^a host that refuses `(\w+)` with "(.*)"$"#)]
fn host_refuses(world: &mut World, tool_id: String, reason: String) {
    world.host.answers.insert(tool_id, Answer::Refused(reason));
}

#[given(regex = r#"^a host whose `(\w+)` fails with "(.*)"$"#)]
fn host_fails(world: &mut World, tool_id: String, message: String) {
    world.host.answers.insert(tool_id, Answer::Failed(message));
}

#[given(regex = r"^a host whose `(\w+)` answers only when told to$")]
fn host_holds(world: &mut World, tool_id: String) {
    world.host.answers.insert(tool_id, Answer::Held);
}

#[given(regex = r"^a sandbox whose execute wait is (\d+) ms and poll wait is (\d+) ms$")]
fn sandbox_waits(world: &mut World, execute_ms: u64, poll_ms: u64) {
    world.execute_wait = Some(Duration::from_millis(execute_ms));
    world.poll_wait = Some(Duration::from_millis(poll_ms));
}

#[given(regex = r"^a sandbox whose `(\w+)` is (\d+)( ms| MiB|)$")]
fn sandbox_limit(world: &mut World, field: String, number: usize, unit: String) {
    let limits = world.limits.get_or_insert_with(Limits::default);
    let millis = Duration::from_millis(number as u64);
    match (field.as_str(), unit.as_str()) {
        ("run_time", " ms") => limits.run_time = millis,
        ("memory_bytes", " MiB") => limits.memory_bytes = number * 1024 * 1024,
        ("calls_per_execute", "") => limits.calls_per_execute = number,
        ("call_timeout", " ms") => limits.call_timeout = Some(millis),
        ("idle_timeout", " ms") => limits.idle_timeout = Some(millis),
        _ => panic!("no limit `{field}` in{unit}"),
    }
}

#[when(regex = r"^`(\w+)` answers (.+)$")]
fn held_call_answers(world: &mut World, tool_id: String, answer: String) {
    let sender = world
        .host
        .held_calls
        .lock()
        .expect("held calls")
        .remove(&tool_id);
    let sender = sender.unwrap_or_else(|| panic!("no {tool_id} call waits for its answer"));
    sender
        .send(parse_json(&answer))
        .expect("the run still waits");
}
