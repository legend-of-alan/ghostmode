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
}

/// A host whose tools answer as the scenario's Given steps set up.
#[derive(Clone, Default)]
pub(crate) struct TestHost {
    /// Each tool's answer, by tool id.
    pub(crate) answers: HashMap<String, Answer>,
    /// The sending halves that answer `Answer::Held` calls by tool id, from a When step to the
    /// call's future.
    pub(crate) held_calls: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
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
        let tools = self.answers.keys().map(|id| host::Tool {
            id: id.clone(),
            description: self
                .descriptions
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("The scenario's {id} tool.")),
            input_schema: self
                .input_schemas
                .get(id)
                .cloned()
                .unwrap_or_else(|| serde_json::json!({"type": "object"})),
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
    let call = host::Call {
        tool_id,
        args: serde_json::Map::new(),
    };
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
