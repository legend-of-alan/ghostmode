//! The live scenarios: Claude reads the `execute` and `poll` definitions and answers a question
//! through the sandbox, over the Claude Messages API, which `curl` calls.

use std::fs;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use cucumber::{then, when};
use ghostmode::host::Tool;
use ghostmode::schema;
use serde_json::{Value, json};

use crate::{Request, World};

const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";
const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
const REQUESTS_PER_TRY: usize = 6;
const MAX_TOKENS: u32 = 2048;
/// The priced usage past which the run sends no more requests.
const RUN_BUDGET_USD: f64 = 0.75;

/// The tokens the run's requests used so far, across every scenario.
static RUN_USAGE: Mutex<Usage> = Mutex::new(Usage {
    requests: 0,
    input_tokens: 0,
    cache_write_tokens: 0,
    cache_read_tokens: 0,
    output_tokens: 0,
});

/// The tokens that one try or the whole run used, as the Messages API's `usage` reports them.
#[derive(Clone, Copy, Default)]
struct Usage {
    requests: u64,
    input_tokens: u64,
    cache_write_tokens: u64,
    cache_read_tokens: u64,
    output_tokens: u64,
}

impl Usage {
    /// Adds one response's `usage` object.
    fn add(&mut self, usage: &Value) {
        let tokens = |key: &str| usage[key].as_u64().unwrap_or(0);
        self.requests += 1;
        self.input_tokens += tokens("input_tokens");
        self.cache_write_tokens += tokens("cache_creation_input_tokens");
        self.cache_read_tokens += tokens("cache_read_input_tokens");
        self.output_tokens += tokens("output_tokens");
    }

    /// The price in US dollars at Sonnet 5.5's rates, whatever model ran: $2 per million input
    /// tokens, $2.50 per million cache writes, $0.10 per million cache reads and $10 per million
    /// output tokens.
    fn cost_usd(&self) -> f64 {
        let dollars = self.input_tokens as f64 * 2.0
            + self.cache_write_tokens as f64 * 2.5
            + self.cache_read_tokens as f64 * 0.1
            + self.output_tokens as f64 * 10.0;
        dollars / 1_000_000.0
    }
}

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} requests, {} input tokens, {} cache writes, {} cache reads, {} output tokens, ${:.4}",
            self.requests,
            self.input_tokens,
            self.cache_write_tokens,
            self.cache_read_tokens,
            self.output_tokens,
            self.cost_usd()
        )
    }
}

/// The credential the live scenarios send, from `GHOSTMODE_LIVE_KEY`: a federated access token
/// or an API key. Without it, no live scenario runs.
pub(crate) fn credential() -> Option<String> {
    std::env::var("GHOSTMODE_LIVE_KEY")
        .ok()
        .filter(|key| !key.is_empty())
}

/// Prints the tokens and price of every request the run sent.
pub(crate) fn print_run_usage() {
    let usage = *RUN_USAGE.lock().expect("run usage");
    println!("live run: {usage}");
}

#[when(regex = r#"^Claude answers "(.+)" in (\d+) tries$"#)]
async fn claude_answers(world: &mut World, question: String, tries: usize) {
    let key = credential().expect("GHOSTMODE_LIVE_KEY");
    let model = std::env::var("GHOSTMODE_LIVE_MODEL")
        .ok()
        .filter(|model| !model.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
    for try_number in 1..=tries {
        println!("    Try {try_number} of {tries}");
        world.sandbox = None;
        let mut usage = Usage::default();
        let reply = ask_claude(world, &key, &model, &question, &mut usage).await;
        match &reply {
            Ok(text) => println!("    Claude replied: {text}"),
            Err(error) => println!("    The try failed: {error}"),
        }
        println!("    Try {try_number} used {usage}");
        world.live_replies.push(reply);
    }
}

#[then(regex = r#"^at least (\d+) of the (\d+) replies contain "(.+)"$"#)]
fn replies_contain(world: &mut World, least: usize, tries: usize, token: String) {
    assert_eq!(world.live_replies.len(), tries);
    let token = token.to_lowercase();
    let passed = (world.live_replies.iter().flatten())
        .filter(|reply| reply.to_lowercase().contains(&token))
        .count();
    assert!(passed >= least, "{passed} of {tries} replies passed");
}

/// Claude's final reply to `question` in a fresh sandbox, or why the try failed. Each request's
/// usage is added to `usage` and to the run's.
async fn ask_claude(
    world: &mut World,
    key: &str,
    model: &str,
    question: &str,
    usage: &mut Usage,
) -> Result<String, String> {
    let tools = to_claude_tools(world.sandbox_tools().await);
    let mut messages = vec![json!({"role": "user", "content": question})];
    for _ in 0..REQUESTS_PER_TRY {
        let run_cost = RUN_USAGE.lock().expect("run usage").cost_usd();
        if run_cost > RUN_BUDGET_USD {
            return Err(format!(
                "the run's priced usage, ${run_cost:.4}, passed ${RUN_BUDGET_USD}"
            ));
        }
        let body = json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "output_config": {"effort": "low"},
            "cache_control": {"type": "ephemeral"},
            "tools": tools,
            "messages": messages,
        });
        let response = post_messages(key, &body)?;
        usage.add(&response["usage"]);
        RUN_USAGE.lock().expect("run usage").add(&response["usage"]);
        let content = response["content"].as_array().cloned().unwrap_or_default();
        match response["stop_reason"].as_str() {
            Some("tool_use") => {
                let mut results = Vec::new();
                for block in content.iter().filter(|block| block["type"] == "tool_use") {
                    let status = send_to_sandbox(world, &block["name"], &block["input"]).await;
                    results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": block["id"],
                        "content": status,
                    }));
                }
                messages.push(json!({"role": "assistant", "content": content}));
                messages.push(json!({"role": "user", "content": results}));
            }
            Some("end_turn") => {
                let text = content.iter().rev().find(|block| block["type"] == "text");
                return Ok(text
                    .map_or("", |block| block["text"].as_str().unwrap_or(""))
                    .to_owned());
            }
            other => return Err(format!("the response stopped with {other:?}")),
        }
    }
    Err(format!(
        "Claude didn't finish in {REQUESTS_PER_TRY} requests"
    ))
}

/// The tools for the Claude Messages API: `schema::mcp_definitions` with `inputSchema` renamed
/// to `input_schema`.
fn to_claude_tools(host_tools: &[Tool]) -> Vec<Value> {
    let mut definitions = schema::mcp_definitions(host_tools);
    for definition in &mut definitions {
        let definition = definition.as_object_mut().expect("a definition object");
        let input_schema = definition.remove("inputSchema").expect("an inputSchema");
        definition.insert("input_schema".to_owned(), input_schema);
    }
    definitions
}

/// Sends Claude's call of `execute` or `poll` to the sandbox, printing it as a When step and the
/// status as a Then step, and answers the status JSON. A poll first sends each held call its
/// answer from `TestHost::answers_on_poll`.
async fn send_to_sandbox(world: &mut World, function_id: &Value, input: &Value) -> String {
    let request = match function_id.as_str() {
        Some("execute") => {
            let code = input["code"].as_str().unwrap_or_default().to_owned();
            let restart = input["restart"].as_bool().unwrap_or(false);
            println!("      When Claude executes:\n        \"\"\"");
            for line in code.lines() {
                println!("        {line}");
            }
            println!("        \"\"\"");
            Request::Execute { code, restart }
        }
        Some("poll") => {
            let run_id = input["run_id"].as_u64().unwrap_or_default();
            println!("      When Claude polls run {run_id}");
            let mut held_calls = world.host.held_calls.lock().expect("held calls");
            for (tool_id, answer) in &world.host.answers_on_poll {
                if let Some(sender) = held_calls.remove(tool_id) {
                    let _ = sender.send(answer.clone());
                }
            }
            drop(held_calls);
            Request::Poll(run_id)
        }
        _ => return format!("no function {function_id}"),
    };
    world.send(request).await;
    let status = world.status.to_string();
    println!("      Then the status is {status}");
    status
}

/// The Messages API's response to `body`. The credential goes to `curl` on stdin: a federated
/// access token, which starts with `sk-ant-oat`, as `authorization: Bearer`, and any other value
/// as `x-api-key`.
fn post_messages(key: &str, body: &Value) -> Result<Value, String> {
    let body_path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("live-request.json");
    fs::write(&body_path, body.to_string()).expect("the request body is written");
    let credential = if key.starts_with("sk-ant-oat") {
        format!("authorization: Bearer {key}")
    } else {
        format!("x-api-key: {key}")
    };
    let headers =
        format!("{credential}\nanthropic-version: {API_VERSION}\ncontent-type: application/json\n");
    let mut curl = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--header",
            "@-",
            "--data-binary",
        ])
        .arg(format!("@{}", body_path.display()))
        .args(["--write-out", "\n%{http_code}", MESSAGES_URL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("curl starts");
    let mut stdin = curl.stdin.take().expect("curl's stdin");
    stdin
        .write_all(headers.as_bytes())
        .expect("curl reads the headers");
    drop(stdin);
    let output = curl.wait_with_output().expect("curl finishes");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (response, http_code) = stdout.rsplit_once('\n').unwrap_or(("", &stdout));
    match http_code {
        "200" => serde_json::from_str(response).map_err(|err| format!("{err}: {response}")),
        "401" => Err(format!(
            "HTTP 401: the access token expired or the key was refused: {response}"
        )),
        _ => Err(format!("HTTP {http_code}: {response}")),
    }
}
