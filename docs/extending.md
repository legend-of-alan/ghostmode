# Writing a host or a backend

A `Sandbox` works with two parts that you can replace:

- **The host** decides which tools a script can call, and answers each call. Write one to offer your own tools. It implements the `Host` trait in `src/host.rs`.
- **The backend** runs the script. Write one to run scripts in another interpreter or another process. It implements the `Backend` trait in `src/backend.rs`.

If you only want to offer tools, you only need a host. The `monty` feature, on by default, adds `backend::Monty`, and the `monty-pool` feature adds `backend::MontyPool`. [Where scripts run](#where-scripts-run) compares them.

The README and this guide use these words:

- The model is the LLM at the model provider. It never runs code and only sees the definitions of `execute` and `poll`, the two functions the model calls.
- The caller is your app, the agent loop. It owns a `Sandbox`, passes each `execute` or `poll` call from the model into it, and sends the `Status` JSON back.
- The `Sandbox` is a library object in the caller's process, not a separate service.
- The script is the Python the model wrote.
- Monty is the Python interpreter that runs the script.
- The host is the value you pass to `Builder::new`, along with the backend. Its type implements `Host`.
- A tool is one function the host offers. The script calls it like a Python function, and the `Sandbox` turns the call into `Host::call`. The host can do anything to answer, network calls included.
- `mcp::Server` is a ready-made host behind the `rmcp` feature. It wraps an rmcp client connection to an MCP server that runs somewhere else, and its tools are that server's tools.

## Where scripts run

`Monty` runs the script inside the caller's process, on the thread that polls `Sandbox::execute` or `Sandbox::poll`. That thread runs nothing else until the script calls a tool or ends, for up to `Limits::run_time` in total, 30 s by default. On Tokio, call `execute`, `poll`, `Sandbox::dump` and `Builder::restore` inside `spawn_blocking` or `block_in_place`, so a script doesn't stall the runtime's other tasks.

`Monty` isn't crash-isolated. A script, or a dump restored without a signing key, can panic, abort or hang the caller's process.

`MontyPool`, behind the `monty-pool` feature, runs each session in its own `monty` worker process from monty-pool. A crash ends only that worker, and the next status says the session restarted. It needs a Tokio multi-thread runtime. Its rustdoc says what still runs on the polling thread and how the pool counts workers.

## What happens during one execute

When the caller passes the model's code to `Sandbox::execute`, the `Sandbox` has the backend run the script. The backend runs the script until it calls a tool. The `Sandbox` asks the host for the answer and hands it back to the backend. This repeats until the script ends.

```mermaid
sequenceDiagram
    participant Caller
    participant Sandbox
    participant Backend
    participant Host
    Caller->>Sandbox: execute(code, restart)
    Sandbox->>Backend: feed_start(session, code, tool_ids)
    Backend-->>Sandbox: Progress::Call with the call and the suspension
    Sandbox->>Host: call(call)
    Host-->>Sandbox: a future of Ok(value) or Err(host::Error)
    Sandbox->>Backend: resume(suspension, answer, tool_ids)
    Backend-->>Sandbox: Progress::Ended with the result, the streams and the session
    Sandbox-->>Caller: Status, finished or failed
```

A script can call tools many times. Each tool call adds one more round of `Progress::Call`, the host's answer and `resume` before the script ends with `Progress::Ended`.

The `Sandbox` waits for the host's answer only up to the wait set by `Builder::execute_wait`. If the wait ends first, `execute` returns a `running` status, and the run stays suspended. A later `Sandbox::poll` waits on the same answer and runs the script on from there.

## Write a host

A host is any type that implements `Host`. The trait has two methods.

`tools` returns the tools a script may call, as a `Vec<Tool>`. Each `Tool` has three fields:

- `id` is the name the script calls the tool by, as a Python function. Use a Python identifier that is not a Python keyword, such as `add_note`, and check it with `schema::is_python_identifier`. The `execute` description that `schema::list_mcp_tools` writes leaves out any tool whose id is not one.
- `description` tells the model what the tool does.
- `input_schema` is the JSON Schema of the tool's keyword arguments.

`call` answers one tool call. It gets a `Call` with the `tool_id` and the keyword `args` as a JSON object. It returns a future of `Result<Value, host::Error>`:

- An Ok value is the tool's return value. The script gets it as a Python value.
- A Refused error means the host would not run the call. The script sees a PermissionError with the reason.
- A Failed error means the call ran and failed. The script sees a RuntimeError with the message.

This host keeps a list of notes. It offers `add_note` and `list_notes`, and it refuses `add_note` when it is read-only:

```rust
use std::sync::{Arc, Mutex};

use futures_util::future::{self, BoxFuture, FutureExt};
use ghostmode::Builder;
use ghostmode::backend::Monty;
use ghostmode::host::{self, Call, Host, Tool};
use serde_json::{Value, json};

struct Notes {
    notes: Arc<Mutex<Vec<String>>>,
    read_only: bool,
}

impl Host for Notes {
    fn tools(&self) -> Vec<Tool> {
        let note_schema = json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"],
        });
        vec![
            Tool::new("add_note", "Saves a note.", note_schema),
            Tool::new("list_notes", "Returns every saved note.", json!({ "type": "object" })),
        ]
    }

    fn call(&self, call: &Call) -> BoxFuture<'static, Result<Value, host::Error>> {
        let answer = match call.tool_id.as_str() {
            "add_note" if self.read_only => Err(host::Error::Refused("notes are read-only".into())),
            "add_note" => match call.args.get("text").and_then(Value::as_str) {
                Some(text) => {
                    self.notes.lock().unwrap().push(text.to_string());
                    Ok(Value::Null)
                }
                None => Err(host::Error::Failed("text must be a string".into())),
            },
            "list_notes" => Ok(json!(*self.notes.lock().unwrap())),
            other => Err(host::Error::Failed(format!("no tool has the id {other}"))),
        };
        future::ready(answer).boxed()
    }
}

async fn add_and_list_notes() {
    let host = Notes { notes: Arc::default(), read_only: false };
    let mut sandbox = Builder::new(host, Monty).build().await;
    let status = sandbox.execute("add_note(text='hi')\nlist_notes()", false).await;
}
```

The `status` serializes as `{"status": "finished", "value": ["hi"]}`.

For a host that does real I/O, see `mcp::Server` in `src/mcp.rs`. It clones what it needs out of `self` and `call`, then sends the call to an MCP server inside an `async move` block.

### Rules for a host

- **The future owns its data.** `call` returns a `'static` future, so it can't borrow `self` or `call`. Clone what the future needs before you build it, as `mcp::Server` clones its rmcp peer.
- **The future may be dropped.** The `Sandbox` drops the future when the call passes `call_timeout`, when the caller calls `Sandbox::cancel` or `Sandbox::reset`, and when the run passes `idle_timeout`. Don't rely on the future running to the end.
- **The tool list is read once.** `Builder::new` calls `tools` once, and the `Sandbox` hands the tool ids to the backend. A tool that `tools` adds later is not callable in that `Sandbox`, so build a new one.
- **Arguments are keywords.** Monty raises TypeError for a positional argument before the call reaches the host, so `call.args` holds every argument.
- **Keys come sorted.** Objects in `call.args`, the host's answers and the script's results come with their keys sorted. To keep insertion order, turn on serde_json's `preserve_order` feature in your own `Cargo.toml`.
- **Answer every call.** Use Refused for a call the host won't run, such as one without permission. Use Failed for a call that ran and went wrong. Answer an unknown `tool_id` with Failed, as `mcp::Server` does.

## Write a backend

A backend is any type that implements `Backend`. The backend holds no session itself. The `Sandbox` keeps the idle session, or the suspended script, between calls. `feed_start` and `resume` take it by value and hand it back inside a `Progress`, and `abort` and `abandon` hand it back inside an `Ending`.

The trait has two associated types, and both must be Send:

- `Session` is a Python session between scripts. It holds the globals that scripts set.
- `Suspension` is a script stopped at a tool call. It also holds what the script has printed so far.

It has seven methods. Each returns a future that must be Send.

- `new_session(limits)` makes an empty idle session. The session must stop a script that goes past the run time, memory or recursion depth in `limits`.
- `feed_start(session, code, tool_ids)` runs `code` in the idle session. It returns `Progress::Call` when the script calls one of the host's tools, whose ids are `tool_ids`, and `Progress::Ended` when the script ends.
- `resume(suspension, answer, tool_ids)` gives the host's answer to the suspended call and runs on. An Ok value becomes the call's return value. A Refused error raises PermissionError, and a Failed error raises RuntimeError.
- `abort(suspension, message)` ends the suspended script with a RuntimeError that carries `message`, and returns its `Ending`. The script can't catch this error.
- `abandon(suspension, message)` drops the suspended script without running it further. It returns an `Ending` whose `result` is Err with `message`, whose `session` is idle with the globals the script set, and whose `streams` hold what the script printed.
- `dump(session)` returns the idle session as bytes, or an error for a session it can't write, together with whether it lost the session's globals while it tried.
- `restore(bytes, limits)` loads the bytes from `dump` as an idle session, or returns an error for bytes it can't load.

Every `Progress` is one of two variants:

- `Progress::Call` carries the `host::Call` and the `Suspension`. The `Sandbox` sends the call to the host and passes the answer to `resume`.
- `Progress::Ended` carries an `Ending`, which holds the `result`, the `streams`, the idle `session` and whether the backend lost the session's globals during the run, `lost_globals`. The `result` is Ok with the value of the script's last expression as JSON, or Err with the script's traceback as text.

The toy backend below treats the whole script as one tool id. It calls that tool with no arguments, and the script's value is the tool's answer. It has no globals, so its `Session` and `Suspension` are `()`. The snippet runs it with any host, such as the notes host from above.

```rust
use std::error;

use ghostmode::backend::{Backend, Ending, Progress, Streams};
use ghostmode::host::{self, Call, Host};
use ghostmode::{Builder, Limits};
use serde_json::{Map, Value};

struct OneCall;

impl Backend for OneCall {
    type Session = ();
    type Suspension = ();

    async fn new_session(&self, _limits: &Limits) {}

    async fn feed_start(&self, session: (), code: &str, tool_ids: &[String]) -> Progress<Self> {
        let tool_id = code.trim().to_string();
        if !tool_ids.contains(&tool_id) {
            let result = Err(format!("NameError: name '{tool_id}' is not defined"));
            let streams = Streams::default();
            return Progress::Ended(Ending { result, streams, session, lost_globals: false });
        }
        let call = Call::new(tool_id, Map::new());
        Progress::Call { call, suspension: () }
    }

    async fn resume(
        &self,
        _suspension: (),
        answer: Result<Value, host::Error>,
        _tool_ids: &[String],
    ) -> Progress<Self> {
        let result = answer.map_err(|error| match error {
            host::Error::Refused(reason) => format!("PermissionError: {reason}"),
            host::Error::Failed(message) => format!("RuntimeError: {message}"),
        });
        Progress::Ended(Ending { result, streams: Streams::default(), session: (), lost_globals: false })
    }

    async fn abort(&self, _suspension: (), message: String) -> Ending<Self> {
        let result = Err(format!("RuntimeError: {message}"));
        Ending { result, streams: Streams::default(), session: (), lost_globals: false }
    }

    async fn abandon(&self, _suspension: (), message: String) -> Ending<Self> {
        Ending { result: Err(message), streams: Streams::default(), session: (), lost_globals: false }
    }

    async fn dump(&self, _session: &mut ()) -> (Result<Vec<u8>, Box<dyn error::Error + Send + Sync>>, bool) {
        (Ok(Vec::new()), false)
    }

    async fn restore(
        &self,
        _bytes: &[u8],
        _limits: &Limits,
    ) -> Result<(), Box<dyn error::Error + Send + Sync>> {
        Ok(())
    }
}

async fn build_one_call_sandbox(host: impl Host) {
    let mut sandbox = Builder::new(host, OneCall).build().await;
}
```

For the real reference, see `Monty` in `src/backend/monty/in_process.rs`. Its `run` method shows how to turn every way Monty can stop into either a `Progress::Call` or an answer that Monty gets at once. `MontyPool` in `src/backend/monty/pool.rs` does the same over a worker process, and shows a backend that can lose its session.

### Rules for a backend

- **Stop only for the host's tools.** Return `Progress::Call` only for a call to one of the host's tools, with keyword arguments only. The `Sandbox` passes the host's tool ids to `feed_start` and `resume`. Answer everything else inside the backend. `Monty` raises NameError for an unknown name and TypeError for a positional argument, and it gives every OS call the no-handler answer. A tool call hands the script a plain value, so a script that awaits one gets a TypeError such as `'dict' object can't be awaited`, and the session keeps its globals.
- **An `Ending` carries everything the run produced.** The `session` is idle and keeps the globals the script set, even when the script failed. A backend that loses the globals, as `MontyPool` does when its worker crashes, gives back an empty session and sets `lost_globals` on that `Ending`, or returns true from `dump`. The backend reports each loss once, when it hands the session back. The `Sandbox` keeps the notice, in its dumps too, and puts `restarted` on the next finished or failed status. The `streams` hold everything the script printed during the whole run, across every `resume`. The `Sandbox` puts the Err text in the `failed` status's `error`, so make it the traceback you want the model to read.
- **`abort` ends the script.** The script can't catch the error, so `abort` returns an `Ending`, never a tool call. The `Sandbox` calls it when the script reaches `calls_per_execute` and when a call passes `call_timeout`.
- **The backend enforces three limits.** `new_session` and `restore` get the `Limits`. The session stops a script at `run_time`, `memory_bytes` and `recursion_depth`. `run_time` counts only the script's own time, summed over the whole run, and not the time it waits on the host. The `Sandbox` enforces `calls_per_execute`, `call_timeout`, `idle_timeout` and `output_chars` itself.
- **`dump` and `restore` work on idle sessions only.** The `Sandbox` calls `dump` only when no run is open, and hands an error from `dump` to the caller as Error::Dump. `restore` uses the `limits` it gets, not the limits the session was dumped with. When the caller sets `Builder::signing_key`, the `Sandbox` adds and checks the signature, so the backend sees only its own bytes.
- **Without a signing key, `restore` gets bytes it can't trust.** Return an error for bytes you can't load. If a bad dump can crash or hang your backend, say so in its docs, as `Monty` does.
