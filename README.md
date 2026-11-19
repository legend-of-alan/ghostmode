<p align="center"><img src="assets/ghostmode-logo.png" alt="ghostmode: Code Mode for Rust agents."></p>
<p align="center"><a href="https://github.com/legend-of-alan/ghostmode/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/legend-of-alan/ghostmode/ci.yml?branch=main&label=CI" alt="CI"></a> <a href="#license"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License: MIT"></a> <a href="https://github.com/sponsors/legend-of-alan"><img src="https://img.shields.io/badge/sponsor-%E2%99%A5-db61a2?logo=githubsponsors" alt="Sponsor"></a></p>

# ghostmode

> Early days. The first real release is coming. The `0.0.1` crate on crates.io only reserves the name.

Code Mode for Rust agents. The model writes a script, ghostmode runs it in a sandbox, and the host provides the tools the script calls. Only the result comes back to the model. The core is async without tying you to a runtime.

## Why Code Mode

Your agent calls tools one at a time, and every result takes a trip through the model, even when the model just hands it to the next tool. You pay for those tokens, you wait on those round trips, and one big result can blow the context window.

Code Mode fixes that. The model writes a short Python script that calls your tools, loops, filters and handles errors, and only the answer comes back.

What you get:

- **Way fewer tokens.** Big intermediate results stay in the script. Anthropic cut one workflow from 150,000 tokens to 2,000, and Cloudflare fits its whole 2,500-endpoint API in about 1,000 tokens instead of 1.17 million.
- **Fewer round trips.** One `execute` can make many tool calls. CodeAct needed up to 30% fewer actions.
- **Smarter tool use.** Models have seen way more code than tool-call JSON, so they chain tools better in code. CodeAct raised success by up to 20 points.

What it takes: give a `Sandbox` your tools, or point it at an MCP server, and hand the model two tools, `execute` and `poll`. The script runs in Monty, a Python interpreter inside your process, so there's no container or extra service to run. The script can only reach the outside world through your tools.

Where those numbers come from:

- [Executable Code Actions Elicit Better LLM Agents](https://arxiv.org/abs/2402.01030) (CodeAct) tested 17 models on tool tasks. Code actions beat JSON and text tool calls by up to 20 points of success while needing up to 30% fewer actions.
- [Anthropic's code execution with MCP post](https://www.anthropic.com/engineering/code-execution-with-mcp) cut one example from 150,000 tokens to 2,000 (98.7% less) by having the agent write code against its tools instead of loading every tool definition up front.
- [Cloudflare's Code Mode MCP post](https://blog.cloudflare.com/code-mode-mcp/) puts the whole Cloudflare API, over 2,500 endpoints, behind two code tools in about 1,000 tokens. The same API as plain MCP tools would take 1.17 million.

## How it works

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/how-it-works-dark.png">
  <img src="assets/how-it-works.png" alt="The model at the provider sends execute or poll to the caller. The caller passes it to the Sandbox, where Monty runs the script. Tool calls go to the host, which reaches an MCP server only if it is mcp::Server.">
</picture>

The model is the LLM. It runs at the model provider, not on your machine. It never runs code. It only sees two tools, `execute` and `poll`, and it sends back tool calls with `code` or a `run_id`.

The caller is your application, the agent loop. It owns a `Sandbox`. It passes each `execute` or `poll` call from the model into the `Sandbox`, then sends the resulting `Status` JSON back to the model.

The `Sandbox` is a ghostmode library object in the caller's process, not a separate service.

The script is the Python the model wrote. It runs inside Monty.

Monty is the Python interpreter. Today it runs inside the caller's process, in the same address space.

The host is the value the caller passes to `Sandbox::builder`. Its type implements `Host`, and it lives in the caller's process. It offers tools and answers calls to them.

A tool is one function the host offers. The script calls it as a Python function, and the `Sandbox` turns that into `Host::call`. Whatever the host does to answer runs in the caller's process, and it may reach out over the network.

`mcp::Server` is one ready-made host, offered when the `rmcp` feature is on. It wraps an rmcp client connection to an MCP server that runs elsewhere, in another process or on another machine. Its tools are that server's tools.

Every arrow inside the caller's process is a plain Rust function call. The only network hops are caller to model provider, and host to MCP server when the host is `mcp::Server`.

## Usage

The caller writes a host that offers tools, builds a `Sandbox`, and hands the model two tool definitions, `execute` and `poll`. When the model calls `execute`, the caller passes the code to `Sandbox::execute` and sends the `Status` JSON back. If that status is running, the model calls `poll` with the `run_id`, and the caller does the same with `Sandbox::poll`.

```rust
use futures_util::future::{BoxFuture, FutureExt};
use ghostmode::host::{Call, Error, Host, Tool};
use ghostmode::{schema, Sandbox};
use serde_json::{json, Value};

struct Echo;

impl Host for Echo {
    fn tools(&self) -> Vec<Tool> {
        vec![Tool {
            id: "echo".into(),
            description: "Returns the text you pass it.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
            }),
        }]
    }

    fn call(&self, call: &Call) -> BoxFuture<'static, Result<Value, Error>> {
        let text = call.args["text"].clone();
        async move { Ok(text) }.boxed()
    }
}

let host = Echo;
let tools = schema::list_mcp_tools(&host.tools());
let mut sandbox = Sandbox::builder(host).build();
// Hand `tools` to the model. When it calls execute, run the code and send the Status JSON back.
let status = sandbox.execute("echo(text='hello')").await;
let reply = serde_json::to_value(&status).unwrap();
```

`schema::list_mcp_tools` is the MCP shape. `schema::list_openai_functions` is the same pair as OpenAI functions. With the `rmcp` feature, you can pass an `mcp::Server` to `Sandbox::builder` instead of writing a host, and the script calls that server's tools.

## Build from source

You need [rustup](https://rustup.rs). It installs the Rust version pinned in `rust-toolchain.toml`.

```sh
git clone https://github.com/legend-of-alan/ghostmode.git
cd ghostmode
cargo build --locked
cargo test --locked
```

`cargo test --locked --all-features` also builds the `rmcp` feature, which offers `mcp::Server`. Fmt, clippy and the rest of CI are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributing

Contributions are issues only for now. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under [MIT](LICENSE).
