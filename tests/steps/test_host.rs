use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cucumber::{given, then, when};
use futures::channel::oneshot;
use futures::future::{self, BoxFuture, FutureExt};
use ghostmode::Sandbox;
use ghostmode::host::{self, Host};
use serde_json::Value;

use crate::World;

/// What a test host's tool does when the script calls it.
#[derive(Clone)]
pub(crate) enum Reply {
    Answer(Value),
    Refuse(String),
    Fail(String),
    /// Answers once a step sends the answer through `TestHost::held`.
    Hold,
}

/// A host whose tools reply as the scenario's Given steps set up.
#[derive(Clone, Default)]
pub(crate) struct TestHost {
    pub(crate) replies: HashMap<String, Reply>,
    pub(crate) held: Arc<Mutex<Vec<oneshot::Sender<Value>>>>,
}

impl Host for TestHost {
    fn tools(&self) -> Vec<host::Tool> {
        self.replies
            .keys()
            .map(|id| host::Tool {
                id: id.clone(),
                description: format!("The scenario's {id} tool."),
                input_schema: serde_json::json!({"type": "object"}),
            })
            .collect()
    }

    fn call(&self, call: host::Call) -> BoxFuture<'static, Result<Value, host::Error>> {
        match self.replies[&call.tool_id].clone() {
            Reply::Answer(value) => future::ready(Ok(value)).boxed(),
            Reply::Refuse(reason) => future::ready(Err(host::Error::Refused(reason))).boxed(),
            Reply::Fail(message) => future::ready(Err(host::Error::Failed(message))).boxed(),
            Reply::Hold => {
                let (sender, receiver) = oneshot::channel();
                self.held.lock().expect("held answers").push(sender);
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
    let reply = Reply::Answer(parse_json(&answer));
    world.host.replies.insert(tool_id, reply);
}

#[given(regex = r#"^a host that refuses `(\w+)` with "(.*)"$"#)]
fn host_refuses(world: &mut World, tool_id: String, reason: String) {
    world.host.replies.insert(tool_id, Reply::Refuse(reason));
}

#[given(regex = r#"^a host whose `(\w+)` fails with "(.*)"$"#)]
fn host_fails(world: &mut World, tool_id: String, message: String) {
    world.host.replies.insert(tool_id, Reply::Fail(message));
}

#[given(regex = r"^a host whose `(\w+)` answers only when told to$")]
fn host_holds(world: &mut World, tool_id: String) {
    world.host.replies.insert(tool_id, Reply::Hold);
}

#[given(regex = r"^a sandbox whose execute wait is (\d+) ms and poll wait is (\d+) ms$")]
fn sandbox_waits(world: &mut World, execute_ms: u64, poll_ms: u64) {
    world.waits = Some((
        Duration::from_millis(execute_ms),
        Duration::from_millis(poll_ms),
    ));
}

#[when(regex = r"^`(\w+)` answers (.+)$")]
fn held_call_answers(world: &mut World, _tool_id: String, answer: String) {
    let sender = world.host.held.lock().expect("held answers").pop();
    let sender = sender.expect("a call waiting for its answer");
    sender
        .send(parse_json(&answer))
        .expect("the run still waits");
}

#[then(regex = r"^a default sandbox waits (\d+) seconds in execute and (\d+) seconds in poll$")]
fn default_waits(_world: &mut World, execute_secs: u64, poll_secs: u64) {
    let sandbox = Sandbox::builder(TestHost::default()).build();
    assert_eq!(sandbox.execute_wait(), Duration::from_secs(execute_secs));
    assert_eq!(sandbox.poll_wait(), Duration::from_secs(poll_secs));
}
