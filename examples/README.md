# clickpipe examples

Two captured logs to pipe through clickpipe without having to break a build
first. Run everything from the repository root.

## build.log — a rustc error, with its original colors

Real `cargo build` diagnostics (ANSI SGR codes included) referencing
`src/main.rs`, which exists in this repository — so the default
existence check passes and the path lights up:

```bash
cargo run --quiet -- --when always < examples/build.log
```

In an OSC 8 capable terminal, `src/main.rs:2:22` is now clickable and the
colors are untouched. To see what was detected without squinting at
escape bytes:

```bash
cargo run --quiet -- --dump < examples/build.log
```

Add `--editor vscode` (or `zed`, `idea`, `subl`, ...) to make the click
land on line 2, column 22 in your editor instead of just opening the file.

## ci.log — a polyglot CI failure from another machine

Python traceback, Node stack frames, gcc and MSVC diagnostics, a Jira key,
a cross-repo issue reference and two URLs. The paths do not exist locally,
so this is what `--no-check` is for:

```bash
cargo run --quiet -- --when always --no-check --jira https://tracker.example.test < examples/ci.log
```

`PAY-1204` links to the tracker, `acme/gateway#392` links to the forge, and
every stack frame becomes a `file://` link (point `--host` at the builder to
make them resolvable over your file manager's network support).
