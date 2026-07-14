# Detection rules and the OSC 8 wire format

This document is the precise version of what the README summarizes: exactly
what clickpipe detects, what it refuses to detect, and what it writes to
your terminal.

## The OSC 8 escape sequence

A hyperlink is opened with `ESC ] 8 ; params ; URI ST` and closed with
`ESC ] 8 ; ; ST` (ST = `ESC \`). This is what clickpipe emits around
`src/main.rs:2:22` (shown with `cat -v`):

```text
 --> ^[]8;;file://devbox/work/app/src/main.rs^[\src/main.rs:2:22^[]8;;^[\
```

Three properties make this safe to emit blindly:

- Terminals that do not implement OSC 8 ignore the sequence and render the
  text unchanged — the output degrades to what you had before.
- The URI may contain only printable ASCII; clickpipe percent-encodes
  control bytes, spaces and non-ASCII bytes before emission.
- clickpipe never opens a link inside an existing one: input regions that
  are already hyperlinked (`ls --hyperlink`, another clickpipe) are
  detected and left byte-identical.

By default hyperlinks are only emitted when stdout is a terminal
(`--when auto`, same contract as `grep --color=auto`), so adding clickpipe
to a pipeline that ends in a file or another parser changes nothing.

## File paths

A path candidate is a maximal run of `[A-Za-z0-9_\-./~+@%]`, optionally
followed by a location suffix:

| Shape | Emitted by | Line/col captured |
|---|---|---|
| `src/main.rs:14:9` | rustc, gcc, clang, tsc, eslint, ruff | line + column |
| `src/main.rs:14` | grep -n, make, many linters | line |
| `render.cpp(88,15)` | MSVC, swiftc | line + column |
| `File "/app/tool.py", line 88` | Python tracebacks | line (contextual) |
| `/var/log/app.log`, `./x`, `../x`, `~/x` | anything | — |

The suffix is part of the clickable span (users click the whole
`file.rs:14:9`), but the trailing `:` that gcc puts before the message is
not.

Acceptance is deliberately strict, because a filter sees arbitrary prose:

- **Default (`--check`, on):** the candidate must exist on disk, resolved
  against `--cwd` (relative), `$HOME` (`~/`), or as-is (absolute).
- **`--no-check`:** shape rules only — the token must be anchored
  (`/`, `./`, `../`, `~/`), or contain a slash *and* a letter-led
  extension. `input/output`, `and/or` and `1.2.3` never qualify.
- A bare filename with no slash (`main.c`) additionally needs a line
  suffix in both modes — otherwise every word with a dot would linkify.
- `..`/`.` segments are resolved lexically; symlinks are *not* resolved,
  so the link shows the path the tool printed.

The target is `file://HOST/abs/path` (HOST from `--host`, `$HOSTNAME`, or
the kernel, so terminals can tell local links from SSH ones), or an editor
URI when `--editor` is given:

| `--editor` | Link target |
|---|---|
| `vscode`, `vscode-insiders`, `cursor` | `SCHEME://file/abs/path:line:col` |
| `zed` | `zed://file/abs/path:line:col` |
| `idea` | `idea://open?file=/abs/path&line=N&column=N` |
| `subl` | `subl://open?url=file:///abs/path&line=N&column=N` |
| `txmt` | `txmt://open?url=file:///abs/path&line=N&column=N` |
| custom template | `{path}`, `{line}`, `{col}` placeholders (missing → 1) |

## URLs

`http://`, `https://`, `file://` (case-insensitive) and `www.` at a word
boundary. The span extends to whitespace or a quote/bracket/backtick/pipe,
then trailing sentence punctuation is trimmed. Closing parens/brackets are
trimmed only when unbalanced within the URL — `(see https://a.test/x)`
drops the paren, `https://en.wikipedia.org/wiki/Pipe_(Unix)` keeps it.
`www.` targets get an `https://` href; non-ASCII IRI bytes are
percent-encoded in the href while the visible text stays as printed.

## Issue IDs

| Form | Requires | Link target |
|---|---|---|
| `#123` | a template | template with `{id}` substituted |
| `owner/repo#123` | nothing | `FORGE/owner/repo/issues/123` |
| `PAY-1204` | `--jira BASE` | `BASE/browse/PAY-1204` |

The `#123` template comes from `--issues 'https://.../{id}'`, from
`--repo owner/name` (GitHub shorthand), or — with no flags at all — from
the `origin` remote of the git repository around `--cwd`: scp/ssh/https
remote syntax is normalized, `gitlab` hosts get the `/-/issues/` layout,
and worktree `.git` files are followed. `--no-git` turns discovery off;
nothing else about clickpipe reads your git state. `#123abc` (a color, an
identifier) and nine-digit "refs" never match. Jira keys are opt-in
because the pattern legitimately collides with strings like `UTF-8` — with
`--jira` set, anything key-shaped links, which mirrors how Jira's own
smart-link matching behaves.

When detectors overlap, the leftmost span wins; at the same start the
longest wins (so `owner/repo#123` beats the `owner/repo` path candidate)
and URLs beat paths beat issues.

## Terminal support (as of 2026)

OSC 8 is implemented in iTerm2, WezTerm, kitty, Alacritty (≥0.11), foot,
Konsole, every VTE terminal (GNOME Terminal, Tilix, ...), Windows Terminal
(≥1.4) and Ghostty. tmux passes hyperlinks through from 3.4 on
(`set -ga terminal-features "*:hyperlinks"`); GNU screen strips them —
the text still renders fine.

## Known limitations

- Windows drive-letter paths (`C:\src\main.rs`) are not detected; the
  colon-and-backslash grammar collides with the line-suffix syntax and is
  out of scope for a POSIX filter.
- Matching is per-line: a path hard-wrapped across two lines by the
  emitting tool is not reassembled.
- Lines longer than 1 MiB are split at the limit before scanning; lines
  that are not valid UTF-8 pass through byte-identically, unscanned.
- `file://` links cannot carry a line number (no standard exists); use
  `--editor` when you want clicks to land on the exact line.
