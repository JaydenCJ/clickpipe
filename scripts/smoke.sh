#!/usr/bin/env bash
# Smoke test: builds clickpipe, then exercises the real end-to-end path a
# user takes — piping compiler-style output through the filter and checking
# the OSC 8 bytes, the editor and issue-tracker flags, --dump, passthrough
# guarantees and exit codes. Self-contained: temp dirs only, no network.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() { echo "SMOKE FAIL: $*" >&2; exit 1; }

echo "[smoke] building..."
cargo build --quiet
BIN=$(pwd)/target/debug/clickpipe

WORK=$(mktemp -d "${TMPDIR:-/tmp}/clickpipe-smoke.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

ESC=$(printf '\033')
OSC_OPEN="${ESC}]8;;"
OSC_CLOSE="${ESC}]8;;${ESC}\\"

# --- 1. version/help sanity --------------------------------------------------
"$BIN" --version | grep -q '^clickpipe 0\.1\.0$' || fail "--version mismatch"
"$BIN" --help | grep -q -- '--editor' || fail "--help missing --editor"

# --- 2. a fake project with real files and a git remote ----------------------
mkdir -p "$WORK/proj/src" "$WORK/proj/.git"
printf 'fn main() { let x: u32 = -1; }\n' > "$WORK/proj/src/main.rs"
printf '[remote "origin"]\n\turl = git@example.test:acme/widgets.git\n' \
  > "$WORK/proj/.git/config"

# rustc-style colored diagnostics referencing a file that exists.
printf '%b' \
  "${ESC}[1m${ESC}[38;5;9merror[E0600]${ESC}[0m: cannot apply unary operator\n" \
  " ${ESC}[1m${ESC}[38;5;12m-->${ESC}[0m src/main.rs:1:26\n" \
  "note: see https://example.test/error-index#E0600 (fixes #42)\n" \
  > "$WORK/build.log"

echo "[smoke] filter: file/URL/issue links from a compiler log"
(cd "$WORK/proj" && "$BIN" --when always --host smokehost < "$WORK/build.log") > "$WORK/out.txt"
grep -qF "${OSC_OPEN}file://smokehost$WORK/proj/src/main.rs${ESC}\\src/main.rs:1:26${OSC_CLOSE}" \
  "$WORK/out.txt" || fail "file link missing or wrong"
grep -qF "${OSC_OPEN}https://example.test/error-index#E0600${ESC}\\" "$WORK/out.txt" \
  || fail "URL link missing"
grep -qF "${OSC_OPEN}https://example.test/acme/widgets/issues/42${ESC}\\#42${OSC_CLOSE}" \
  "$WORK/out.txt" || fail "git-derived issue link missing"
grep -qF "${ESC}[38;5;9m" "$WORK/out.txt" || fail "input colors were not preserved"

# --- 3. --editor rewrites file links ------------------------------------------
echo "[smoke] filter: --editor vscode"
(cd "$WORK/proj" && "$BIN" --when always --editor vscode < "$WORK/build.log") > "$WORK/ed.txt"
grep -qF "${OSC_OPEN}vscode://file$WORK/proj/src/main.rs:1:26${ESC}\\" "$WORK/ed.txt" \
  || fail "vscode editor link missing"

# --- 4. --dump is scriptable --------------------------------------------------
echo "[smoke] --dump"
(cd "$WORK/proj" && "$BIN" --dump --host smokehost < "$WORK/build.log") > "$WORK/dump.txt"
grep -q "^path	src/main.rs:1:26	file://smokehost$WORK/proj/src/main.rs$" "$WORK/dump.txt" \
  || fail "--dump path row missing"
grep -q "^url	https://example.test/error-index#E0600	" "$WORK/dump.txt" \
  || fail "--dump url row missing"
grep -q "^issue	#42	https://example.test/acme/widgets/issues/42$" "$WORK/dump.txt" \
  || fail "--dump issue row missing"

# --- 5. passthrough guarantees ------------------------------------------------
echo "[smoke] passthrough: --when never and non-terminal auto are byte-identical"
(cd "$WORK/proj" && "$BIN" --when never < "$WORK/build.log") > "$WORK/never.txt"
cmp -s "$WORK/build.log" "$WORK/never.txt" || fail "--when never modified bytes"
(cd "$WORK/proj" && "$BIN" < "$WORK/build.log") > "$WORK/auto.txt"
cmp -s "$WORK/build.log" "$WORK/auto.txt" || fail "auto mode modified bytes in a pipe"

# --- 6. jira + stats ----------------------------------------------------------
echo "[smoke] --jira and --stats"
printf 'deploying PAY-991 today\n' \
  | "$BIN" --when always --no-git --jira https://tracker.example.test --stats \
    > "$WORK/jira.txt" 2> "$WORK/stats.txt"
grep -qF "${OSC_OPEN}https://tracker.example.test/browse/PAY-991${ESC}\\PAY-991${OSC_CLOSE}" \
  "$WORK/jira.txt" || fail "jira link missing"
grep -q '1 line, 1 link (0 paths, 0 urls, 1 issue)' "$WORK/stats.txt" || fail "--stats summary missing"

# --- 7. errors exit non-zero with a message -----------------------------------
if "$BIN" --when sometimes < /dev/null 2> "$WORK/err.txt"; then
  fail "invalid --when accepted"
fi
grep -q "invalid --when" "$WORK/err.txt" || fail "usage error lacks message"
if "$BIN" --when always "$WORK/does-not-exist.log" 2>> "$WORK/err.txt"; then
  fail "missing input file accepted"
fi

echo "SMOKE OK"
