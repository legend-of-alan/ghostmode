# AGENTS.md

Guidance for coding agents working in this repository. [CONTRIBUTING.md](CONTRIBUTING.md) is the authority: its checks, design principles, testing rules, naming rules and PR title format apply to every change, so read it before you write code or name anything.

These rules apply to agents only:

- Never merge a PR. Never force-push or rebase a pushed branch; merge `main` into it instead.
- In PR bodies, use `Closes #N` only for the closing keyword; elsewhere write "issue #N" / "PR #N", never a bare `#N`.
- Run every check CONTRIBUTING.md lists before you push, and fix what fails at its cause.
- Every backticked file path, Rust path and CamelCase name in this file, `README.md` and `docs/*.md` must exist (`tests/doc_code_names.rs`), so a rename updates the docs in the same PR.
- Never commit a key, token or other secret. CI scans every commit for them.
