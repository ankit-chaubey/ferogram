# Contributing to ferogram

Thank you for your interest in contributing! All contributions are welcome: bug fixes, new method wrappers, documentation improvements, tests, and ideas.

Please take a few minutes to read this guide before opening a pull request or issue.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Ways to Contribute](#ways-to-contribute)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Coding Guidelines](#coding-guidelines)
- [Submitting a Pull Request](#submitting-a-pull-request)
- [Commit Messages](#commit-messages)
- [Reporting Bugs](#reporting-bugs)
- [Requesting Features](#requesting-features)
- [Security Issues](#security-issues)

---

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you agree to uphold it. Please report unacceptable behaviour to [ankitchaubey.dev@gmail.com](mailto:ankitchaubey.dev@gmail.com).

---

## Ways to Contribute

### Wrapping a new Telegram API method

The most impactful contribution is wrapping a method from the [Telegram API schema](https://core.telegram.org/schema) that is currently only reachable via `client.invoke::<R>()`. Check the [Unsupported Features](README.md#-unsupported-features) table for ideas.

Each wrapper should:
1. Live in the most appropriate module (`ferogram/src/lib.rs`, `participants.rs`, `media.rs`, etc.)
2. Accept ergonomic Rust types as input (not raw TL structs where avoidable)
3. Include a doc comment with a short example

### Writing tests

Unit tests live alongside the source (`#[cfg(test)]` blocks). Integration tests that require a real Telegram connection are kept in `layer-app/`. If you add a new method, a matching test is appreciated.

### Improving documentation

- Fix typos, unclear wording, or outdated examples in any `.md` file
- Improve inline doc comments in the Rust source
- Add a new documentation page under `docs/src/`

### Reporting bugs and requesting features

Use the issue templates:
- [Bug report](.github/ISSUE_TEMPLATE/bug_report.md)
- [Feature request](.github/ISSUE_TEMPLATE/feature_request.md)

---

## Development Setup

**Prerequisites:**
- Rust stable (2024 edition or later): install via [rustup](https://rustup.rs/)
- Cargo

```bash
# Clone the repository
git clone https://github.com/ankit-chaubey/ferogram.git
cd ferogram

# Build the workspace
cargo build --workspace

# Run all tests
cargo test --workspace

# Build with all optional features
cargo build -p ferogram --all-features

# Check formatting
cargo fmt --all -- --check

# Run Clippy
cargo clippy --workspace --all-features -- -D warnings
```

---

## Project Structure

```
ferogram/
├ ferogram-tl-parser/ .tl schema text -> AST
├ ferogram-tl-gen/ AST -> Rust source at build time
├ ferogram-tl-types/ All generated Telegram types
├ ferogram-crypto/ Low-level crypto primitives
├ ferogram-mtproto/ MTProto session and transport
├ ferogram/ High-level Client API <-- most contributions go here
│ └ src/
│ ├ lib.rs Core Client methods
│ ├ participants.rs Admin, ban, reactions, permissions
│ ├ media.rs Upload, download, albums
│ ├ search.rs SearchBuilder, GlobalSearchBuilder
│ ├ keyboard.rs Inline and reply keyboards
│ ├ typing_guard.rs TypingGuard RAII helpers
│ ├ inline_iter.rs InlineQueryIter, InlineResultIter
│ ├ session_backend.rs Session backends
│ ├ parsers.rs Markdown and HTML parsers
│ └ update.rs Update types and IncomingMessage
├ ferogram-connect/ Minimal DH demo
└ ferogram-app/ Interactive demo binary
```

---

## Coding Guidelines

- **Rust edition 2024.** Follow the Rust API guidelines.
- **Async-first.** Public methods that touch the network must be `async`.
- **Error handling.** Return `Result<_, InvocationError>`. Do not panic in library code.
- **No unsafe.** Unless absolutely required and well-justified.
- **Formatting.** Run `cargo fmt` before committing.
- **Clippy.** Fix all warnings before opening a PR.
- **Doc comments.** Every public item needs at least one sentence. Include a short `# Example` where helpful.
- **Naming.** Follow Rust conventions: `snake_case` for functions, `PascalCase` for types.

---

## Submitting a Pull Request

1. **Fork** the repository and create a branch: `git checkout -b feat/my-feature`
2. **Make your changes.** Keep each PR focused on one thing.
3. **Test.** Run `cargo test --workspace` and make sure everything passes.
4. **Format and lint.** Run `cargo fmt --all` and `cargo clippy --workspace --all-features`.
5. **Open a PR** against the `main` branch using the pull request template.
6. **Respond to review feedback.** PRs are usually reviewed within a few days.

### PR checklist

- [ ] Tests pass (`cargo test --workspace`)
- [ ] Code is formatted (`cargo fmt --all`)
- [ ] Clippy is clean (`cargo clippy --workspace --all-features`)
- [ ] Public items have doc comments
- [ ] CHANGELOG entry added under `[Unreleased]` if this is a user-facing change

---

## Commit Messages

Use short, descriptive commit messages in the imperative mood:

```
Add iter_participants with filter support
Fix DownloadIter zero-padding on last chunk
Update docs: correct sqlite-session feature name
```

For larger changes:

```
feat(client): wrap setMyCommands for bot command registration

Adds `set_my_commands(commands, scope, lang_code)` to Client.
Wraps `bots::SetBotCommands` from Layer 224.

Closes #42
```

---

## Reporting Bugs

Use the [bug report template](.github/ISSUE_TEMPLATE/bug_report.md). Please include:

- Your `ferogram` version
- A minimal reproducer if possible
- The error message or unexpected behaviour
- Your OS and Rust version

---

## Requesting Features

Use the [feature request template](.github/ISSUE_TEMPLATE/feature_request.md). Please describe the Telegram API method you want wrapped and the use case.

---

## Security Issues

**Do not open a public issue for security vulnerabilities.** Please follow the process described in [SECURITY.md](SECURITY.md).

---

Thank you for contributing!
