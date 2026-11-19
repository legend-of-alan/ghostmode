![ghostmode: Code Mode for Rust agents.](assets/ghostmode-logo.png)

# ghostmode

Code Mode for Rust agents. The model writes a script, ghostmode runs it in a sandbox, and the host provides the tools the script calls. Only the result comes back to the model. The core is async without tying you to a runtime.

## Why Code Mode

Agents usually call tools one at a time. Every result goes back through the model, even when the model only needs to hand it to the next tool. That costs tokens and time, and big results can blow past the context window.

With Code Mode the model writes a short script instead. The script calls the tools, loops, filters and handles errors on its own, and the model only reads what the script returns.

- **Fewer tokens.** Big intermediate results stay in the script and never hit the model's context.
- **Fewer round trips.** One `execute` can make many tool calls.
- **Better tool use.** Models have seen way more code than tool-call JSON, so they chain tools better in code.

The evidence:

- [Executable Code Actions Elicit Better LLM Agents](https://arxiv.org/abs/2402.01030) (CodeAct) tested 17 models and found code actions beat JSON and text tool calls, with up to 20% higher success.
- [Cloudflare's Code Mode post](https://blog.cloudflare.com/code-mode/) found agents handle more tools, and more complex ones, when the tools look like an API to write code against.
- [Anthropic's code execution with MCP post](https://www.anthropic.com/engineering/code-execution-with-mcp) walks through how tool definitions and intermediate results fill the context, and how running code fixes both.

## How it works

```mermaid
flowchart LR
    subgraph Provider["Model provider, remote"]
        Model["The model"]
    end
    subgraph App["Caller's process"]
        Caller["Caller: agent loop"]
        subgraph GM["ghostmode Sandbox"]
            Monty["Monty, runs the script"]
        end
        Host["Host: your type, or mcp::Server"]
    end
    MCPS["MCP server, separate process"]
    Model -->|"tool call: execute or poll"| Caller
    Caller -->|"Status JSON"| Model
    Caller -->|"execute, poll"| GM
    GM -->|"Host::call"| Host
    Host -.->|"only if it is mcp::Server"| MCPS
```

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
