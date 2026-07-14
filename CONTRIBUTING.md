# Contributing to clickpipe

Thanks for your interest in improving clickpipe. Issues, discussions and pull requests are all welcome.

## Getting started

Prerequisites: Rust 1.75 or newer (stable toolchain).

```bash
git clone https://github.com/JaydenCJ/clickpipe.git
cd clickpipe
cargo build
cargo test
bash scripts/smoke.sh
```

`scripts/smoke.sh` builds the binary and pipes a real colored compiler log through it, asserting on the exact OSC 8 byte sequences, the editor and issue-tracker links, the passthrough guarantees and the exit codes. It finishes in well under a minute and must print `SMOKE OK`.

## Before you open a pull request

1. `cargo fmt` — formatting is enforced.
2. `cargo clippy --all-targets -- -D warnings` — clippy must be clean.
3. `cargo test` — unit tests and the CLI integration tests must pass.
4. `bash scripts/smoke.sh` — the smoke test must print `SMOKE OK`.
5. Add tests for behavior changes. Detection logic lives in pure modules (`ansi`, `urls`, `paths`, `issues`, `giturl`, `scan`) that are easy to unit-test; please keep it that way.

## Ground rules

- Keep dependencies at zero. clickpipe is std-only by design; adding a dependency needs a very strong justification in the PR description.
- No network calls, no telemetry, ever. The filter reads stdin, the paths it checks and `.git/config`; it writes stdout. Nothing else.
- Passthrough is sacred: with links disabled (or on non-UTF-8 input) the output must be byte-identical to the input. Any change to the line loop needs a test proving this still holds.
- False positives are worse than false negatives. A new detection pattern must come with negative tests showing what it does *not* match in ordinary prose.
- Code comments and doc comments are written in English.

## Reporting bugs

Please include the `clickpipe --version` output, the exact flags used, and a minimal input line that reproduces the problem — ideally as `printf '...' | clickpipe --dump` so the detected spans and targets are visible. For rendering issues, name the terminal and attach the raw bytes (`... | clickpipe --when always | cat -v`).

## Security

If you find a security issue (e.g. an escape-sequence injection that corrupts terminal state), please do not open a public issue. Use GitHub's private vulnerability reporting on this repository instead.
