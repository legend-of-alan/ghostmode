use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use futures::channel::oneshot;
use futures::future::{self, BoxFuture, FutureExt};
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
    /// A list nested `depth` levels, built when the script calls the tool.
    NestedList(usize),
    /// Answers `value` when the call gives the argument `arg`, and fails otherwise.
    ValueIfGiven {
        arg: String,
        value: Value,
    },
}

/// A host whose tools answer as the scenario's Given steps set up.
#[derive(Clone, Default)]
pub(crate) struct TestHost {
    /// Each tool's answer, by tool id.
    pub(crate) answers: HashMap<String, Answer>,
    /// The sending halves that answer `Answer::Held` calls by tool id, from a When step to the
    /// call's future.
    pub(crate) held_calls: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    /// The answer each `Answer::Held` call gets when the model polls, by tool id.
    pub(crate) answers_on_poll: HashMap<String, Value>,
    /// The host over the scenario's MCP server, whose tools this host offers beside its own.
    pub(crate) mcp_client: Option<Arc<dyn Host + Send + Sync>>,
    /// The input schema of each tool a Given step set one for, by tool id. Other tools take
    /// `{"type": "object"}`.
    pub(crate) input_schemas: HashMap<String, Value>,
    /// The description of each tool a Given step set one for, by tool id. Other tools say
    /// "The scenario's <id> tool."
    pub(crate) descriptions: HashMap<String, String>,
}

impl Host for TestHost {
    fn tools(&self) -> Vec<host::Tool> {
        let tools = self.answers.keys().map(|id| {
            let description = self
                .descriptions
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("The scenario's {id} tool."));
            let input_schema = self
                .input_schemas
                .get(id)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({"type": "object"}));
            host::Tool::new(id, description, input_schema)
        });
        let mcp_tools = self
            .mcp_client
            .iter()
            .flat_map(|mcp_client| mcp_client.tools());
        tools.chain(mcp_tools).collect()
    }

    fn call(&self, call: &host::Call) -> BoxFuture<'static, Result<Value, host::Error>> {
        let Some(answer) = self.answers.get(&call.tool_id).cloned() else {
            let mcp_client = self
                .mcp_client
                .as_ref()
                .expect("an MCP client offers the tool");
            return mcp_client.call(call);
        };
        match answer {
            Answer::Value(value) => future::ready(Ok(value)).boxed(),
            Answer::Refused(reason) => future::ready(Err(host::Error::Refused(reason))).boxed(),
            Answer::Failed(message) => future::ready(Err(host::Error::Failed(message))).boxed(),
            Answer::NestedList(depth) => {
                let mut value = Value::from(1);
                for _ in 0..depth {
                    value = Value::Array(vec![value]);
                }
                future::ready(Ok(value)).boxed()
            }
            Answer::ValueIfGiven { arg, value } => {
                let answer = if call.args.contains_key(&arg) {
                    Ok(value)
                } else {
                    Err(host::Error::Failed(format!("missing argument `{arg}`")))
                };
                future::ready(answer).boxed()
            }
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

#[given(regex = r"^a host whose `([^`]+)` answers a list nested (\d+) levels$")]
fn host_answers_nested_list(world: &mut World, tool_id: String, depth: usize) {
    world
        .host
        .answers
        .insert(tool_id, Answer::NestedList(depth));
}

#[given(regex = r#"^a host whose `([^`]+)` answers ([\[{"].*)$"#)]
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

#[given(regex = r"^a host whose `(\w+)` holds its answer (\{.*\}) until the model polls$")]
fn host_holds_until_poll(world: &mut World, tool_id: String, answer: String) {
    let host = &mut world.host;
    host.answers.insert(tool_id.clone(), Answer::Held);
    host.answers_on_poll.insert(tool_id, parse_json(&answer));
}

#[given(regex = r"^a host whose `(\w+)` needs `([^`]+)` and then answers (\{.*\})$")]
fn host_needs_arg(world: &mut World, tool_id: String, arg: String, answer: String) {
    let value = parse_json(&answer);
    let answer = Answer::ValueIfGiven { arg, value };
    world.host.answers.insert(tool_id, answer);
}

#[given(
    regex = r#"^a host whose `(\w+)` answers a (\d+) character log with "(.+)" on its middle line$"#
)]
fn host_answers_log(world: &mut World, tool_id: String, chars: usize, middle_line: String) {
    let mut log = String::new();
    let mut line_number = 0;
    let mut push_lines_until = |log: &mut String, len: usize| {
        while log.len() < len {
            line_number += 1;
            log.push_str(&format!("line {line_number:05}: INFO request served\n"));
        }
    };
    push_lines_until(&mut log, chars / 2);
    log.push_str(&format!("{middle_line}\n"));
    push_lines_until(&mut log, chars);
    log.truncate(chars);
    world
        .host
        .answers
        .insert(tool_id, Answer::Value(log.into()));
}

#[given(regex = r"^a host whose `(\w+)` input schema is (\{.*\})$")]
fn host_input_schema(world: &mut World, tool_id: String, input_schema: String) {
    let host = &mut world.host;
    host.answers
        .entry(tool_id.clone())
        .or_insert(Answer::Value(Value::Null));
    host.input_schemas
        .insert(tool_id, parse_json(&input_schema));
}

#[given(regex = r#"^a host whose `(\w+)` description is "(.*)"$"#)]
fn host_description(world: &mut World, tool_id: String, description: String) {
    let host = &mut world.host;
    host.answers
        .entry(tool_id.clone())
        .or_insert(Answer::Value(Value::Null));
    host.descriptions.insert(tool_id, description);
}

#[given(regex = r"^a host whose `(\w+)` description is:$")]
fn host_description_block(world: &mut World, tool_id: String, step: &Step) {
    let description = step.docstring.as_ref().expect("a docstring").trim();
    host_description(world, tool_id, description.to_owned());
}

#[when(regex = r"^`(\w+)` answers (.+)$")]
fn held_call_answers(world: &mut World, tool_id: String, answer: String) {
    let sender = take_held_call(world, &tool_id);
    sender
        .send(parse_json(&answer))
        .expect("the run still waits");
}

#[when(regex = r"^the caller calls `(\w+)` on the host$")]
async fn caller_calls_host(world: &mut World, tool_id: String) {
    let call = host::Call::new(tool_id, serde_json::Map::new());
    let error = world
        .host
        .call(&call)
        .await
        .expect_err("the host answers an error");
    world.host_error = Some(error.into());
}

#[then(regex = r#"^the host's error reads "(.*)"$"#)]
fn host_error_reads(world: &mut World, text: String) {
    let error = world
        .host_error
        .as_ref()
        .expect("the host answered an error");
    assert_eq!(error.to_string(), text);
}

#[then(regex = r"^answering `(\w+)` finds no run waiting$")]
fn held_call_finds_no_run(world: &mut World, tool_id: String) {
    let sender = take_held_call(world, &tool_id);
    assert!(sender.send(Value::Null).is_err(), "a run still waits");
}

/// Takes the sending half that answers the held call to `tool_id`, failing if there is none.
fn take_held_call(world: &mut World, tool_id: &str) -> oneshot::Sender<Value> {
    let sender = world
        .host
        .held_calls
        .lock()
        .expect("held calls")
        .remove(tool_id);
    sender.unwrap_or_else(|| panic!("no {tool_id} call waits for its answer"))
}
