use std::time::Duration;

use cucumber::gherkin::Step;
use cucumber::{then, when};

use crate::test_host::parse_json;
use crate::{Request, World};

#[when(regex = r"^the model executes (.*?[^:])( with restart)?$")]
async fn execute_code(world: &mut World, code: String, with_restart: String) {
    let code = code.replace("; ", "\n");
    let restart = !with_restart.is_empty();
    world.send(Request::Execute { code, restart }).await;
}

#[when(regex = r"^the model executes this script:$")]
async fn execute_docstring(world: &mut World, step: &Step) {
    let code = step.docstring.clone().expect("a script docstring");
    let restart = false;
    world.send(Request::Execute { code, restart }).await;
}

#[when(regex = r"^the model polls run (\d+)$")]
async fn poll_run(world: &mut World, run_id: u64) {
    world.send(Request::Poll(run_id)).await;
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
    let duration = world.request_duration;
    assert!(duration >= Duration::from_millis(least_ms), "{duration:?}");
    assert!(duration < Duration::from_secs(under_secs), "{duration:?}");
}

#[then(regex = r#"^the status's (stdout|stderr) is (".*")$"#)]
fn stream_text_is(world: &mut World, stream: String, text: String) {
    assert_eq!(world.status[&stream], parse_json(&text));
}

#[then(regex = r"^the status says the session restarted$")]
fn status_says_restarted(world: &mut World) {
    assert_eq!(world.status["restarted"], true, "{}", world.status);
}
