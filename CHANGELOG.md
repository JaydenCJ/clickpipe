# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-12

### Added

- Streaming stdin→stdout filter that wraps detected targets in OSC 8 hyperlinks, flushing per line; `--when auto|always|never` with grep-style terminal detection, and byte-identical passthrough for disabled output and non-UTF-8 lines.
- ANSI-aware line model: detection runs on the visible text, SGR/OSC/DCS sequences pass through verbatim, matches spanning color changes are wrapped correctly, and regions already inside an OSC 8 hyperlink are never double-wrapped.
- File-path detector: `path:line:col` (rustc/gcc/tsc), `path(line,col)` (MSVC/swiftc), Python `File "...", line N` tracebacks, absolute/relative/`~/` paths, and bare `file.ext:line` names — with on-disk existence checks by default and shape-based `--no-check` for foreign logs.
- Link targets: `file://host/path` URIs (percent-encoded, hostname from `--host`/the machine) or editor deep links via `--editor` presets (`vscode`, `vscode-insiders`, `cursor`, `zed`, `idea`, `subl`, `txmt`) and custom `{path}`/`{line}`/`{col}` templates.
- URL detector for `http(s)://`, `file://` and `www.` targets with prose-punctuation trimming and paren-balance handling; non-ASCII IRI bytes percent-encoded in the href only.
- Issue-ID detector: bare `#123` via a configured template, `--repo owner/name` shorthand, cross-repo `owner/repo#123` against `--forge`, and opt-in Jira `KEY-123` via `--jira`.
- Offline git-remote discovery: `#123` templates derived from the repository's `origin` remote (scp/ssh/https syntax, nested GitLab groups, `/-/issues/` layout, linked-worktree `.git` files), disabled with `--no-git`.
- `--dump` (tab-separated `kind text target` rows for scripting), `--stats` (stderr summary), file arguments, `--cwd`, exit codes 0/1/2, and BrokenPipe-safe writes for `| head` pipelines.
- Test suite: 70 unit tests, 19 CLI integration tests, and `scripts/smoke.sh`.

[0.1.0]: https://github.com/JaydenCJ/clickpipe/releases/tag/v0.1.0
