use std::time::Duration;

use cucumber::gherkin::Step;
use cucumber::{then, when};
use ghostmode::tool;

use crate::test_host::parse_json;
use crate::{Request, World};

#[when(regex = r"^the model executes (.*[^:])$")]
async fn execute_code(world: &mut World, code: String) {
    let code = code.replace("; ", "\n");
    world.send(Request::Execute(tool::Execute { code })).await;
}

#[when(regex = r"^the model executes this script:$")]
async fn execute_docstring(world: &mut World, step: &Step) {
    let code = step.docstring.clone().expect("a script docstring");
    world.send(Request::Execute(tool::Execute { code })).await;
}

#[when(regex = r"^the model polls run (\d+)$")]
async fn poll_run(world: &mut World, run_id: u64) {
    world.send(Request::Poll(tool::Poll { run_id })).await;
}

#[then(regex = r"^the status is (\{.+\})$")]
fn status_is(world: &mut World, status: String) {
    assert_eq!(world.status, parse_json(&status));
}

#[then(regex = r"^the status is failed with an error naming (.+)$")]
fn status_is_failed(world: &mut World, cause: String) {
    assert_eq!(world.status["status"], "failed", "{}", world.status);
    let error = world.status["error"].as_str().expect("error text");
    assert!(error.contains(&cause), "{error}");
}

#[then(regex = r"^the failed status lists the calls (.+)$")]
fn failed_status_lists_calls(world: &mut World, calls: String) {
    assert_eq!(world.status["calls"], parse_json(&calls));
}

#[then(regex = r"^the model waited at least (\d+) ms but under (\d+) seconds$")]
fn wait_lasted_between(world: &mut World, least_ms: u64, under_secs: u64) {
    let elapsed = world.elapsed;
    assert!(elapsed >= Duration::from_millis(least_ms), "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(under_secs), "{elapsed:?}");
}
