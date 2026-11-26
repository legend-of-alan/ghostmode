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
    let docstring = step.docstring.as_deref().expect("a script docstring");
    let code = docstring.strip_prefix('\n').unwrap_or(docstring).to_owned();
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

#[then("the status is finished")]
fn status_is_finished(world: &mut World) {
    assert_eq!(world.status["status"], "finished", "{}", world.status);
}

#[then(regex = r#"^the status's stdout (is|starts with) (\d+) "(.)" characters and a newline$"#)]
fn stdout_matches_repeated_line(
    world: &mut World,
    comparison: String,
    char_count: usize,
    character: char,
) {
    let stdout = stdout_text(world);
    let expected = format!("{}\n", character.to_string().repeat(char_count));
    let matches = match comparison.as_str() {
        "is" => stdout == expected,
        _ => stdout.starts_with(&expected),
    };
    assert!(matches, "stdout is {} characters", stdout.chars().count());
}

#[then(regex = r"^the status's stdout is at most (\d+) bytes$")]
fn stdout_is_at_most(world: &mut World, max_bytes: usize) {
    let byte_count = stdout_text(world).len();
    assert!(byte_count <= max_bytes, "stdout is {byte_count} bytes");
}

#[then(regex = r#"^the status's stdout doesn't contain "(.+)"$"#)]
fn stdout_does_not_contain(world: &mut World, text: String) {
    assert!(!stdout_text(world).contains(&text));
}

fn stdout_text(world: &World) -> &str {
    world.status["stdout"].as_str().expect("stdout text")
}
