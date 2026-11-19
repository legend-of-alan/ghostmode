![ghostmode: Code Mode for Rust agents.](assets/ghostmode-logo.png)

# ghostmode

Code Mode for Rust agents. The model writes a script, ghostmode runs it in a sandbox, and the host provides the tools the script calls. Only the result comes back to the model. The core is async without tying you to a runtime.

## Why Code Mode

Your agent calls tools one at a time, and every result takes a trip through the model, even when the model just hands it to the next tool. You pay for those tokens, you wait on those round trips, and one big result can blow the context window.

Code Mode fixes that. The model writes a short Python script that calls your tools, loops, filters and handles errors, and only the answer comes back.

What you get:

- **Fewer tokens.** Big intermediate results stay in the script.
- **Fewer round trips.** One `execute` can make many tool calls.
- **Smarter tool use.** Models have seen way more code than tool-call JSON, so they chain tools better in code.

What it takes: give a `Sandbox` your tools, or point it at an MCP server, and hand the model two tools, `execute` and `poll`. The script runs in Monty, a Python interpreter inside your process, so there's no container or extra service to run. The script can only reach the outside world through your tools.

The evidence:

- [Executable Code Actions Elicit Better LLM Agents](https://arxiv.org/abs/2402.01030) (CodeAct) tested 17 models and found code actions beat JSON and text tool calls, with up to 20% higher success.
- [Cloudflare's Code Mode post](https://blog.cloudflare.com/code-mode/) found agents handle more tools, and more complex ones, when the tools look like an API to write code against.
- [Anthropic's code execution with MCP post](https://www.anthropic.com/engineering/code-execution-with-mcp) walks through how tool definitions and intermediate results fill the context, and how running code fixes both.

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

Early days: the design is underway and the first real release is coming. The `0.0.1` crate on crates.io only reserves the name.

Licensed under MIT.
