# Contributing

Contributions are issues only for now. Open an issue, and pick the form that fits.

A pull request from outside this repository is closed with thanks.

A contributor license agreement (CLA) and a developer certificate of origin (DCO) are deferred.

`rust-toolchain.toml` pins the Rust version, and rustup installs it for you. CI runs these, and all must be clean:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked --no-default-features -- -D warnings
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --locked --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --no-default-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --all-features
cargo test --locked
cargo test --locked --all-features
```

The `--all-features` tests run every scenario against the `monty-pool` feature's `backend::MontyPool` too, which needs the `monty` worker binary on your `PATH`. Install it with `cargo install monty-runtime --version =1.6.0 --locked`.

CI also fails on a runtime call or a print in `src`, tokio in the default build's dependency tree, a plain comment in `src` or `tests`, a touched name that breaks Naming rule 8, and a dependency without a permissive license. Pull requests only run CI on Linux. Pushes to main also run it on macOS and Windows.

## Issues and PRs

Issue titles are plain sentences. Every issue gets an area label and a size label (`size: S` is about one sitting, `size: M` a focused PR, `size: L` likely split into sub-issues).

A PR squashes into one commit titled after the PR, so the title starts with the first letter of its issue's area label and ends with its size label, like `T: Return the script's result from execute [S]`. CI checks it.

Every issue gets a priority label from `P0` (most urgent) to `P3` (low). It also gets a type label, which is `enhancement`, `bug` or `documentation`.

## Design principles

- **Keep it simple.** The best fix is often deleting code, not adding it. Before you write something new, check whether you can remove, reuse or derive what you need from what's already there. Build only as much as the job needs. CI shows a `Net: +N production, +M tests` notice on its check run for the lines it adds or removes (files under a `tests/` directory count as tests, and `Cargo.lock`, which is generated, is not counted). If it grows the code, say why in the PR body.
- **Say it once.** Each message, rule, check or format should live in one place. If you find the same thing in two places, merge them instead of adding a flag to keep them in sync.
- **No side doors.** Tests should use the same public API a caller uses. If a test needs a shortcut, fix the test or the design.
- **Use the words people already know.** Good code reads the way people expect, so it needs no tutorial. Names follow [Naming](#naming) below.
- **Fix the cause.** When something fails, find out why. Don't paper over it with sleeps, retries or longer timeouts.
- **Test first.** Write the test that shows what a caller sees, then the code. Tests check behavior, not how the code does it, so a refactor shouldn't break them.
- **Test it once.** Each behavior gets one test, at the layer where a caller sees it. [Testing](#testing) lists what that rules out.
- **No plain comments.** Rust source has no `//` or `/* */` comments, except the `// repeats module: <reason>` marker from Naming rule 8. Doc comments (`///` and `//!`) say what an item does and what it guarantees its callers, never why a workaround exists or why odd behavior is fine. If code needs explaining, fix the code.
- **Bring no runtime.** The core returns plain futures, and the caller picks Tokio, smol or a blocking executor. It never builds, enters or blocks on a runtime.
- **The caller owns output.** Library code never prints. It hands results and errors back to its caller, and nothing it logs carries a secret, a prompt or a script's data. The crate records `tracing` spans with ids, statuses, durations and counts only, and never installs a subscriber.

## Testing

Behavior is tested with cucumber scenarios. Feature files live in `tests/features/<area>/`, shared Given/When/Then steps live in `tests/steps/`, and `tests/features.rs` holds the one `World` and runs the whole tree once per executor. Run one scenario with `cargo test --test features -- --name "<regex>"`. A plain `#[test]` is only for what a scenario can't express, such as `tests/doc_code_names.rs`.

A scenario earns its place by failing when something a caller would notice changes. Check a new scenario against this list before you add it.

- **Extend, don't duplicate.** If a scenario already shows the behavior, extend or rename it instead of adding another. One scenario can cover several acceptance criteria, so you don't need one per item.
- **Don't guard the past.** Don't test that a removed flag, field or behavior stays gone, and don't re-run a fixed bug's old steps to prove it stays fixed. The scenario for the new behavior already fails if the old code comes back.
- **Leave other scenarios alone.** Don't add setup or checks to a scenario about something else just to re-test your change. Extend the scenario whose subject is your change.
- **Assert what callers see.** Check return values, errors and what a script can observe. Don't check private fields, call counts or the order of internal steps when a visible outcome already shows the result. A refactor that keeps behavior should break no scenario.
- **Plain tests only where a scenario can't reach.** A parser edge case is a good one. A parser test for an argument that doesn't exist is not.
- **Name it for what it proves.** A scenario's name says what its steps check. If they disagree, fix one of them.

## Naming

Default to the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html) (casing, `as_`/`to_`/`into_` conversions, getters, constructors, typed units and the rest), with these exceptions and additions:

1. Choose names that let a human newcomer follow a file top to bottom from its names and types alone, and easily trace the code without already knowing its implementation.
   A short name is useful only when its meaning is clear where it appears. Review names from the caller's perspective, not the author's.
2. Types are nouns and functions are verbs. A getter can be a noun. A pair constructor that mirrors `mpsc::channel()` can be a noun when its doc comment says so.
   Ask "a value of this type is a ___". If the answer is a noun, that noun is the name, so not `Evaluated` but `Verdict`. A past participle or adjective is a name only for an enum variant that is a state or past event of its enum's noun (`Cow::Borrowed`, `Bound::Excluded`, `ErrorKind::TimedOut`), for the type a same-named method returns (`Iterator::copied()` returns `Copied`), or under rule 15.
   A past participle or adjective is not a function name (`suffixed`, `parsed`), except an adapter that copies a std name, such as `cloned` or `copied`. So not `suffixed(path)` but `add_suffix(path)`.
3. Borrow words from Rust, std, the crates we use and this codebase before inventing new ones. If std says `try_lock`, say that.
4. No vague words. A name says what the thing shows, sends, reads or refuses.
5. Two names that look alike must do alike jobs.
6. Use the README's words in the README's sense. The model writes a script, ghostmode runs it in a sandbox, and the host provides the tools the script calls.
7. A type that carries messages between layers, such as a request, reply, event or channel half, says in its doc comment which two layers it connects. Similar names get that comment, not a rename.
8. Don't repeat the module name inside a name. A name that is the module name, such as `query::Query`, is fine. CI checks the files a PR touches; a name kept on purpose, under rule 15 or as a pair, ends its line with `// repeats module: <reason>`.
9. A comment shouldn't have to explain a name. If it does, fix the name.
10. Names collide only within one namespace, so `tokio::fs` and `std::fs` are fine. When a name exists in two namespaces anywhere in the project, a `use` keeps the path prefix (`fs::read`, never a bare `read`).
11. Fix a bad name in the code you touch, even if it is older than your change. A rename-only commit changes no logic and updates the docs that mention the name.
12. Prefer a typed unit such as `Duration`, which carries its own unit. A bare number carries the unit as a suffix, such as `_secs` or `_bytes`.
13. Name a module after the protocol or job it handles, never a metaphor.
14. End a stream with `None` and report a failure with `Err`. Don't add `Eof` or `Unreadable` variants for them.
15. A name that mirrors a tool or wire name uses that exact name, so the `execute` and `poll` tools keep those names in code.
16. Use one word per concept across docs, code and the tools' JSON.
17. An identifier is `id` or ends in `_id`, whatever type backs it. `name` is only ever text a person reads. A map, set or lookup is keyed by id, never by name. An outside format's keys keep their spelling, such as the `name` that OpenAI and MCP tool calls use for a tool's id.
18. Every lifetime, single or one of several, has a name of at most 3 letters that says what it borrows, such as `'de` for deserializer input. Generic `'a` and `'b` are for non-specific wrappers such as a collection or a plain borrow. Names that std or well-known crates already use keep their spelling even if they are longer (`'static`, and `'scope` and `'env` from `std::thread::scope`). A single type parameter is `T`, or a usual letter (`K`/`V` for a map key and value, `E` for an error, `F` for a closure, `I` for an iterator). Several type parameters, or one whose meaning is not obvious, get a short meaningful name.

Items in the guidelines about stability and future-proofing don't apply until the first real release. Until then a breaking change is fine.
