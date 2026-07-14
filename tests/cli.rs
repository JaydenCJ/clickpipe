//! End-to-end tests that exercise the compiled `clickpipe` binary: stdin
//! filtering, OSC 8 byte output, passthrough guarantees, `--dump`, editor
//! and issue-tracker flags, and error handling. Everything runs against
//! temporary directories with fully controlled environments — no network,
//! no reliance on the host's git config or hostname.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_clickpipe")
}

/// Run clickpipe with `args`, feeding `input` on stdin, in directory `dir`.
fn run_in(dir: &std::path::Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(bin())
        .args(args)
        .current_dir(dir)
        .env_remove("HOME")
        .env_remove("HOSTNAME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run clickpipe binary");
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn run(args: &[&str], input: &[u8]) -> Output {
    run_in(&std::env::temp_dir(), args, input)
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clickpipe-cli-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn help_documents_the_flags_and_version_matches_the_manifest() {
    let help = run(&["--help"], b"");
    assert!(help.status.success());
    let text = stdout(&help);
    for flag in [
        "--when",
        "--editor",
        "--issues",
        "--jira",
        "--dump",
        "--no-check",
    ] {
        assert!(text.contains(flag), "help must mention '{flag}'");
    }

    let version = run(&["--version"], b"");
    assert!(version.status.success());
    assert_eq!(
        stdout(&version).trim(),
        format!("clickpipe {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn bad_flags_and_values_are_usage_errors_with_exit_code_2() {
    let out = run(&["--frobnicate"], b"");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown option"));
    assert_eq!(run(&["--when", "sometimes"], b"").status.code(), Some(2));
    assert_eq!(
        run(&["--issues", "https://x.test/no-placeholder"], b"")
            .status
            .code(),
        Some(2)
    );
    assert_eq!(run(&["--repo", "not-a-slug"], b"").status.code(), Some(2));
    assert_eq!(run(&["--editor", "nano"], b"").status.code(), Some(2));
    // Flags that take no value must reject an inline one instead of
    // silently ignoring it.
    let out = run(&["--dump=yes"], b"");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not take a value"));
}

#[test]
fn never_and_piped_auto_modes_are_byte_identical_passthrough() {
    let input = b"error at src/main.rs:3 see https://example.test/a\n";
    let out = run(&["--when", "never"], input);
    assert!(out.status.success());
    assert_eq!(out.stdout, input);
    // The test harness captures stdout through a pipe, so auto must not
    // emit escape sequences either — exactly like grep --color=auto.
    let out = run(&[], input);
    assert!(out.status.success());
    assert_eq!(out.stdout, input);
}

#[test]
fn always_mode_wraps_urls_in_osc8_bytes() {
    let out = run(
        &["--when", "always", "--no-git"],
        b"see https://example.test/a now\n",
    );
    assert!(out.status.success());
    assert_eq!(
        stdout(&out),
        "see \x1b]8;;https://example.test/a\x1b\\https://example.test/a\x1b]8;;\x1b\\ now\n"
    );
}

#[test]
fn existing_file_is_linked_and_missing_file_is_not() {
    let dir = tempdir("exists");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    let out = run_in(
        &dir,
        &["--when", "always", "--host", "testhost", "--no-git"],
        b"warn src/lib.rs:1:5 and src/ghost.rs:9\n",
    );
    let text = stdout(&out);
    assert!(
        text.contains(&format!(
            "\x1b]8;;file://testhost{}/src/lib.rs\x1b\\src/lib.rs:1:5\x1b]8;;\x1b\\",
            dir.display()
        )),
        "missing file link in: {text:?}"
    );
    assert!(
        !text.contains("ghost.rs\x1b]8"),
        "ghost.rs must stay unlinked: {text:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn no_check_links_paths_from_other_machines() {
    let out = run(
        &["--when", "always", "--no-check", "--host", "h", "--no-git"],
        b"crash in /srv/app/worker.py:88\n",
    );
    assert!(stdout(&out).contains("\x1b]8;;file://h/srv/app/worker.py\x1b\\"));
}

#[test]
fn ansi_colored_compiler_output_keeps_its_colors() {
    let dir = tempdir("colors");
    fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
    // rustc-style: bold-blue arrow, plain path.
    let input = b"\x1b[1m\x1b[38;5;12m--> \x1b[0mmain.rs:1:1\n";
    let out = run_in(
        &dir,
        &["--when", "always", "--host", "h", "--no-git"],
        input,
    );
    let text = stdout(&out);
    assert!(
        text.starts_with("\x1b[1m\x1b[38;5;12m--> \x1b[0m"),
        "colors lost: {text:?}"
    );
    assert!(text.contains(&format!(
        "\x1b]8;;file://h{}/main.rs\x1b\\main.rs:1:1\x1b]8;;\x1b\\",
        dir.display()
    )));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn existing_hyperlinks_are_never_double_wrapped() {
    let input = b"\x1b]8;;file://x/etc/hosts\x1b\\hosts\x1b]8;;\x1b\\ https://example.test\n";
    let out = run(&["--when", "always", "--no-check", "--no-git"], input);
    let text = stdout(&out);
    // The pre-linked "hosts" region is untouched; the URL after it links.
    assert!(text.starts_with("\x1b]8;;file://x/etc/hosts\x1b\\hosts\x1b]8;;\x1b\\ "));
    assert!(text.contains("\x1b]8;;https://example.test\x1b\\https://example.test\x1b]8;;\x1b\\"));
    assert_eq!(text.matches("file://x/etc/hosts").count(), 1);
}

#[test]
fn editor_flag_rewrites_file_links_to_the_editor_scheme() {
    let dir = tempdir("editor");
    fs::write(dir.join("app.py"), "x = 1\n").unwrap();
    let out = run_in(
        &dir,
        &["--when", "always", "--editor", "vscode", "--no-git"],
        b"E999 app.py:7:12 bad syntax\n",
    );
    assert!(stdout(&out).contains(&format!(
        "\x1b]8;;vscode://file{}/app.py:7:12\x1b\\app.py:7:12\x1b]8;;\x1b\\",
        dir.display()
    )));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn repo_flag_links_bare_issue_refs_to_github() {
    let out = run(
        &["--when", "always", "--repo", "acme/widgets", "--no-git"],
        b"fixes #42\n",
    );
    assert!(stdout(&out)
        .contains("\x1b]8;;https://github.com/acme/widgets/issues/42\x1b\\#42\x1b]8;;\x1b\\"));
}

#[test]
fn issue_template_is_discovered_from_the_git_remote() {
    let dir = tempdir("gitrepo");
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(
        dir.join(".git/config"),
        "[remote \"origin\"]\n\turl = git@example.test:acme/widgets.git\n",
    )
    .unwrap();
    let out = run_in(&dir, &["--when", "always"], b"closes #7\n");
    assert!(
        stdout(&out)
            .contains("\x1b]8;;https://example.test/acme/widgets/issues/7\x1b\\#7\x1b]8;;\x1b\\"),
        "git-derived template missing: {:?}",
        stdout(&out)
    );
    // --no-git disables the discovery.
    let out = run_in(&dir, &["--when", "always", "--no-git"], b"closes #7\n");
    assert_eq!(stdout(&out), "closes #7\n");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn jira_flag_links_uppercase_keys() {
    let out = run(
        &[
            "--when",
            "always",
            "--jira",
            "https://tracker.example.test/",
            "--no-git",
        ],
        b"deploying PAY-991 today\n",
    );
    assert!(stdout(&out).contains(
        "\x1b]8;;https://tracker.example.test/browse/PAY-991\x1b\\PAY-991\x1b]8;;\x1b\\"
    ));
}

#[test]
fn dump_prints_kind_text_href_lines() {
    let dir = tempdir("dump");
    fs::write(dir.join("mod.rs"), "// hi\n").unwrap();
    let out = run_in(
        &dir,
        &[
            "--dump",
            "--host",
            "h",
            "--repo",
            "acme/widgets",
            "--no-git",
        ],
        b"err mod.rs:2:1 see https://example.test/doc (#5)\n",
    );
    assert_eq!(
        stdout(&out),
        format!(
            "path\tmod.rs:2:1\tfile://h{}/mod.rs\n\
             url\thttps://example.test/doc\thttps://example.test/doc\n\
             issue\t#5\thttps://github.com/acme/widgets/issues/5\n",
            dir.display()
        )
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn file_arguments_are_read_in_order_and_missing_files_exit_1() {
    let dir = tempdir("files");
    fs::write(dir.join("a.log"), "one https://example.test/1\n").unwrap();
    fs::write(dir.join("b.log"), "two https://example.test/2\n").unwrap();
    let out = run_in(
        &dir,
        &["--when", "always", "--no-git", "a.log", "b.log"],
        b"",
    );
    let text = stdout(&out);
    let one = text.find("example.test/1").unwrap();
    let two = text.find("example.test/2").unwrap();
    assert!(one < two);
    fs::remove_dir_all(&dir).unwrap();

    let out = run(&["--when", "always", "--no-git", "/nonexistent/x.log"], b"");
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("/nonexistent/x.log"));
}

#[test]
fn invalid_utf8_lines_pass_through_byte_identically() {
    let input: &[u8] =
        b"ok https://example.test\n\xff\xfe broken \x80 line\nback https://example.test/z\n";
    let out = run(&["--when", "always", "--no-check", "--no-git"], input);
    let raw = &out.stdout;
    // The middle line is forwarded untouched.
    assert!(raw
        .windows(b"\xff\xfe broken \x80 line\n".len())
        .any(|w| w == b"\xff\xfe broken \x80 line\n"));
    // Lines around it still get linkified.
    assert_eq!(
        stdout(&out).matches("\x1b]8;;https://example.test").count(),
        2
    );
}

#[test]
fn line_terminators_crlf_and_missing_final_newline_are_preserved() {
    let out = run(
        &["--when", "always", "--no-git"],
        b"see https://example.test/a\r\n",
    );
    assert!(stdout(&out).ends_with("\x1b]8;;\x1b\\\r\n"));

    let out = run(
        &["--when", "always", "--no-git"],
        b"tail https://example.test/end",
    );
    let text = stdout(&out);
    assert!(text.ends_with("\x1b]8;;\x1b\\"));
    assert!(!text.ends_with('\n'));
}

#[test]
fn stats_summary_goes_to_stderr_not_stdout() {
    let dir = tempdir("stats");
    fs::write(dir.join("x.rs"), "fn x() {}\n").unwrap();
    let out = run_in(
        &dir,
        &["--when", "always", "--stats", "--repo", "a/b", "--no-git"],
        b"x.rs:1 https://example.test #9\nplain line\n",
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        err.trim(),
        "clickpipe: 2 lines, 3 links (1 path, 1 url, 1 issue)"
    );
    assert!(!stdout(&out).contains("links ("));
}

#[test]
fn python_traceback_lines_link_with_the_line_number() {
    let dir = tempdir("pytb");
    fs::create_dir_all(dir.join("app")).unwrap();
    fs::write(dir.join("app/tool.py"), "raise SystemExit\n").unwrap();
    let out = run_in(
        &dir,
        &["--dump", "--editor", "vscode", "--no-git"],
        b"  File \"app/tool.py\", line 1, in <module>\n",
    );
    assert_eq!(
        stdout(&out),
        format!(
            "path\tapp/tool.py\tvscode://file{}/app/tool.py:1\n",
            dir.display()
        )
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn disabled_detectors_are_respected_end_to_end() {
    let out = run(
        &[
            "--when",
            "always",
            "--no-urls",
            "--no-files",
            "--no-issues",
            "--no-git",
        ],
        b"see https://example.test and /etc/hosts\n",
    );
    assert_eq!(stdout(&out), "see https://example.test and /etc/hosts\n");
}
