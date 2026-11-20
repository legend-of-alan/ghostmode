<p align="center"><img src="assets/ghostmode-logo.png" alt="Ghost Mode: Code Mode for Rust agents."></p>
<p align="center"><a href="https://github.com/legend-of-alan/ghostmode/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/legend-of-alan/ghostmode/ci.yml?branch=main&label=CI" alt="CI"></a> <a href="#license"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License: MIT"></a> <a href="https://github.com/sponsors/legend-of-alan"><img src="https://img.shields.io/badge/sponsor-%E2%99%A5-db61a2?logo=githubsponsors" alt="Sponsor"></a></p>

# Ghost Mode

Ghost Mode is a Rust library that brings Code Mode to agents. With normal tool calls, every result goes back to the model before it can ask for the next one. Ghost Mode lets the model write a short Python script that chains the calls itself, so intermediate results never go back to the model.

Think about it like this: normal tool calling is like going back to the store 30 times to get 30 items. Code Mode is one trip with a shopping list.

Say the model needs to know why a pull request's checks failed. Each step needs the one before it. The pull request gives the commit, the commit gives the check runs, and each failed run gives a log. With normal tool calls, that's a round trip to the model per step, and every full result goes through the model. With Ghost Mode, the model sends one script:

```python
pr = get_pull_request(number=42)
runs = list_check_runs(commit=pr["head_sha"])
failed = [run for run in runs if run["conclusion"] == "failure"]
[{"check": run["name"], "log": get_log(run_id=run["id"])[-500:]} for run in failed]
```

The script runs inside your process. The model only gets the last 500 characters of each failed log, not the pull request, every check run, and every full log.

## Why Ghost Mode

- **Fewer tokens.** Anthropic cut one workflow from 150,000 tokens to 2,000 (98.7% less) by having the agent write code against its tools instead of loading every tool definition up front ([post](https://www.anthropic.com/engineering/code-execution-with-mcp)). Cloudflare fits its whole API, 2,500+ endpoints, into about 1,000 tokens, where plain MCP tools would take 1.17 million ([post](https://blog.cloudflare.com/code-mode-mcp/)).
- **API bills up to 70% lower.** Across eight models, code mode cut cost by 70.6% for GPT-5.4 and 40.7% on average ([benchmark](https://www.ijcaonline.org/archives/volume187/number136/mbat-ijca-2026-e10508249e55.pdf)). Intermediate results never reach the model, so you aren't billed for them as input on every later step.
- **Fewer steps, better results.** Models have read way more code than tool-call JSON. Across 17 models, CodeAct's code actions needed up to 30% fewer actions and scored up to 20 points higher success than JSON or text tool calls ([paper](https://arxiv.org/abs/2402.01030)).

> Savings depend on the model. In the eight-model benchmark, weaker models retried failed scripts until some runs cost more than plain tool calls, and Gemini models passed fewer tasks. A [pre-registered study](https://arxiv.org/abs/2611.19123) measured 70% to 90% fewer tokens with three open-weight models. In Ghost Mode, the `Limits` you build the `Sandbox` with cap how much any one result can add (40,000 characters by default).

Those are other teams' Code Mode numbers. Ghost Mode brings the same idea to Rust.

What it takes: give a `Sandbox` your tools, or point it at an MCP server, and hand the model only two function definitions, `execute` and `poll`. The Monty backend runs the script inside your process, so there's no container or extra service. The script can only reach the outside world through your tools.

## How it works

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/how-it-works-dark.png">
  <img src="assets/how-it-works.png" alt="The model at the model provider writes a script and sends it with execute, or asks for a run with poll. The caller, your agent loop, passes the call to the Sandbox, where Monty runs the script. Each tool call goes to the host with Host::call, and the answer goes back to the script. The caller sends the model a Status JSON with only the script's result. The host reaches an MCP server only if it is mcp::Server.">
</picture>

Every arrow inside the caller's process is a plain Rust function call. The model never runs code and only sees the definitions of `execute` and `poll`. [Writing a host or a backend](docs/extending.md) names each part and follows one `execute` from start to end.

## Usage

Write a host, build a `Sandbox`, and give the model the `execute` and `poll` definitions. When the model calls `execute`, pass its code to `Sandbox::execute` and send the `Status` JSON back. If the run is still going, the model calls `poll` with the `run_id`, and you pass that to `Sandbox::poll`.

The [crate docs on docs.rs](https://docs.rs/ghostmode) open with a full example: a host with one tool, and a `Sandbox` that runs one script.

`schema::list_mcp_tools` gives you the pair in MCP shape, and `schema::list_openai_functions` gives them as OpenAI functions. With the `rmcp` feature on, pass an `mcp::Server` to `Builder::new` instead of writing your own host.

The default backend, `Monty`, runs the script inside your process and isn't crash-isolated. For untrusted scripts, turn on the `monty-pool` feature and pass a `backend::MontyPool` to `Builder::new`, which runs each session in its own worker process. On Tokio, read [Where scripts run](docs/extending.md#where-scripts-run) first.

`Sandbox::dump` saves an idle session as bytes, and `Builder::restore` loads them into a new sandbox. Before you store dumps where others can write, set `Builder::signing_key`. Restore then refuses bytes the key didn't sign.

To offer tools some other way, or to run scripts in something other than Monty, see [Writing a host or a backend](docs/extending.md).

## Build from source

You need [rustup](https://rustup.rs). It installs the Rust version that `rust-toolchain.toml` pins.

```sh
git clone https://github.com/legend-of-alan/ghostmode.git
cd ghostmode
cargo build --locked
cargo test --locked
```

The default build includes Monty. Add `--all-features` to also build `mcp::Server`, which the `rmcp` feature turns on, and `backend::MontyPool`, which the `monty-pool` feature turns on. The `monty-pool` feature's dependencies need a C compiler. [CONTRIBUTING.md](CONTRIBUTING.md) lists every check that CI runs, and how to install the worker binary that the `monty-pool` tests need.

## Contributing

For now, contributions are issues only. Open an issue and pick the form that fits. [CONTRIBUTING.md](CONTRIBUTING.md) has the rest.

## License

Ghost Mode is under the [MIT license](LICENSE).
