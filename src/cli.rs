//! Command-line interface: flag parsing and the streaming filter loop.
//! Kept dependency-free on purpose.

use crate::ansi::AnsiLine;
use crate::scan::{self, Kind, Options};
use crate::uri::Editor;
use crate::{giturl, osc8};
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::path::PathBuf;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
clickpipe — make file paths, URLs and issue IDs in any output clickable

USAGE:
    <command> 2>&1 | clickpipe [OPTIONS]
    clickpipe [OPTIONS] [FILE ...]        (read FILEs instead; '-' = stdin)

OPTIONS:
        --when <WHEN>       When to emit hyperlinks: auto, always, never
                            [default: auto — only when stdout is a terminal]
        --editor <EDITOR>   Open file links in an editor instead of file://
                            (vscode, vscode-insiders, cursor, zed, idea,
                             subl, txmt, or a template with {path}/{line}/{col})
        --cwd <DIR>         Resolve relative paths against DIR [default: .]
        --host <NAME>       Hostname for file:// URIs [default: this machine]
        --issues <TPL>      Link bare #123 refs via TPL containing {id}
        --repo <OWNER/NAME> Shorthand for the GitHub issues template
        --jira <BASE>       Link KEY-123 refs to BASE/browse/KEY-123
        --forge <BASE>      Base URL for owner/repo#123 refs
                            [default: https://github.com]
        --no-git            Skip issue-template discovery from .git/config
        --no-check          Link path-shaped tokens even if they do not
                            exist on disk (for logs from other machines)
        --no-files          Disable the file-path detector
        --no-urls           Disable the URL detector
        --no-issues         Disable the issue-ID detector
        --dump              Print detected links as 'kind<TAB>text<TAB>href'
                            lines instead of rewriting the stream
        --stats             Print a summary line to stderr at end of input
    -h, --help              Print this help
    -V, --version           Print version

EXIT CODES:
    0 success, 1 I/O error, 2 usage error

Detection details and OSC 8 background: docs/detection.md";

/// How output mode is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    Auto,
    Always,
    Never,
}

struct Config {
    when: When,
    dump: bool,
    stats: bool,
    inputs: Vec<String>,
    opts: Options,
}

/// Parse argv and run. Returns the process exit code.
pub fn dispatch(argv: Vec<String>) -> i32 {
    let config = match parse_args(argv) {
        Ok(Some(config)) => config,
        Ok(None) => return 0, // --help / --version
        Err(msg) => {
            eprintln!("clickpipe: {msg}");
            eprintln!("Try 'clickpipe --help' for usage.");
            return 2;
        }
    };
    match run(config) {
        Ok(()) => 0,
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(err) => {
            eprintln!("clickpipe: {err}");
            1
        }
    }
}

fn parse_args(argv: Vec<String>) -> Result<Option<Config>, String> {
    let mut when = When::Auto;
    let mut dump = false;
    let mut stats = false;
    let mut no_git = false;
    let mut cwd: Option<PathBuf> = None;
    let mut host: Option<String> = None;
    let mut issues_tpl: Option<String> = None;
    let mut repo: Option<String> = None;
    let mut inputs = Vec::new();
    let mut opts = Options::default();

    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or_else(|| format!("{name} requires a value"))
        };
        let no_value = |name: &str| -> Result<(), String> {
            if inline.is_some() {
                Err(format!("{name} does not take a value"))
            } else {
                Ok(())
            }
        };
        match flag.as_str() {
            "-h" | "--help" => {
                no_value("--help")?;
                println!("{HELP}");
                return Ok(None);
            }
            "-V" | "--version" => {
                no_value("--version")?;
                println!("clickpipe {VERSION}");
                return Ok(None);
            }
            "--when" => {
                when = match value("--when")?.as_str() {
                    "auto" => When::Auto,
                    "always" => When::Always,
                    "never" => When::Never,
                    other => return Err(format!("invalid --when '{other}' (auto|always|never)")),
                }
            }
            "--editor" => opts.editor = Some(Editor::from_arg(&value("--editor")?)?),
            "--cwd" => cwd = Some(PathBuf::from(value("--cwd")?)),
            "--host" => host = Some(value("--host")?),
            "--issues" => {
                let tpl = value("--issues")?;
                if !tpl.contains("{id}") {
                    return Err("--issues template must contain {id}".to_string());
                }
                issues_tpl = Some(tpl);
            }
            "--repo" => {
                let r = value("--repo")?;
                if !r.contains('/') || r.starts_with('/') || r.ends_with('/') {
                    return Err(format!("--repo '{r}' must look like OWNER/NAME"));
                }
                repo = Some(r);
            }
            "--jira" => opts.jira = Some(value("--jira")?.trim_end_matches('/').to_string()),
            "--forge" => opts.forge = value("--forge")?.trim_end_matches('/').to_string(),
            "--no-git" => {
                no_value("--no-git")?;
                no_git = true;
            }
            "--no-check" => {
                no_value("--no-check")?;
                opts.check = false;
            }
            "--no-files" => {
                no_value("--no-files")?;
                opts.files = false;
            }
            "--no-urls" => {
                no_value("--no-urls")?;
                opts.urls = false;
            }
            "--no-issues" => {
                no_value("--no-issues")?;
                opts.issues = false;
            }
            "--dump" => {
                no_value("--dump")?;
                dump = true;
            }
            "--stats" => {
                no_value("--stats")?;
                stats = true;
            }
            "-" => inputs.push("-".to_string()),
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            _ => inputs.push(arg),
        }
    }

    opts.cwd = match cwd {
        Some(dir) => dir,
        None => std::env::current_dir().map_err(|e| format!("cannot resolve cwd: {e}"))?,
    };
    opts.home = std::env::var_os("HOME").map(PathBuf::from);
    opts.host = host.unwrap_or_else(hostname);
    opts.issue_template = issues_tpl
        .or_else(|| repo.map(|r| format!("https://github.com/{r}/issues/{{id}}")))
        .or_else(|| {
            if no_git {
                None
            } else {
                giturl::issue_template_from(&opts.cwd)
            }
        });

    Ok(Some(Config {
        when,
        dump,
        stats,
        inputs,
        opts,
    }))
}

/// Best-effort machine name for `file://` URIs, like GNU `ls --hyperlink`.
fn hostname() -> String {
    if let Some(name) = std::env::var_os("HOSTNAME") {
        let name = name.to_string_lossy().trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    if let Ok(name) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        let name = name.trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "localhost".to_string()
}

/// Should this run rewrite lines with hyperlinks?
fn linkify_enabled(when: When) -> bool {
    match when {
        When::Always => true,
        When::Never => false,
        When::Auto => {
            io::stdout().is_terminal() && std::env::var("TERM").map_or(true, |t| t != "dumb")
        }
    }
}

#[derive(Default)]
struct Stats {
    lines: u64,
    paths: u64,
    urls: u64,
    issues: u64,
}

fn run(config: Config) -> io::Result<()> {
    let active = config.dump || linkify_enabled(config.when);
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut stats = Stats::default();

    let inputs = if config.inputs.is_empty() {
        vec!["-".to_string()]
    } else {
        config.inputs.clone()
    };
    for input in &inputs {
        if input == "-" {
            let stdin = io::stdin();
            filter(stdin.lock(), &mut out, &config, active, &mut stats)?;
        } else {
            let file = std::fs::File::open(input)
                .map_err(|e| io::Error::new(e.kind(), format!("{input}: {e}")))?;
            filter(BufReader::new(file), &mut out, &config, active, &mut stats)?;
        }
    }
    out.flush()?;

    if config.stats {
        let total = stats.paths + stats.urls + stats.issues;
        eprintln!(
            "clickpipe: {}, {} ({}, {}, {})",
            plural(stats.lines, "line"),
            plural(total, "link"),
            plural(stats.paths, "path"),
            plural(stats.urls, "url"),
            plural(stats.issues, "issue")
        );
    }
    Ok(())
}

/// `1 line`, `2 lines` — every noun the stats summary uses pluralizes
/// with a plain `s`.
fn plural(n: u64, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The streaming loop: one line in, one line out, flushed per line so the
/// filter adds no latency to live output (`cargo watch`, `tail -f`).
fn filter<R: BufRead>(
    mut reader: R,
    out: &mut impl Write,
    config: &Config,
    active: bool,
    stats: &mut Stats,
) -> io::Result<()> {
    let mut buf = Vec::with_capacity(4096);
    loop {
        buf.clear();
        if reader.by_ref().take(1 << 20).read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        stats.lines += 1;
        let (body, terminator) = split_terminator(&buf);
        match (active, std::str::from_utf8(body)) {
            (true, Ok(text)) => {
                let line = AnsiLine::parse(text);
                let links = scan::drop_taken(scan::scan(line.plain(), &config.opts), line.taken());
                for link in &links {
                    match link.kind {
                        Kind::Path => stats.paths += 1,
                        Kind::Url => stats.urls += 1,
                        Kind::Issue => stats.issues += 1,
                    }
                }
                if config.dump {
                    for link in &links {
                        out.write_all(
                            format!(
                                "{}\t{}\t{}\n",
                                link.kind.label(),
                                &line.plain()[link.start..link.end],
                                osc8::sanitize(&link.href)
                            )
                            .as_bytes(),
                        )?;
                    }
                } else {
                    out.write_all(line.render(&links).as_bytes())?;
                    out.write_all(terminator)?;
                }
            }
            _ => {
                // Passthrough: binary-ish or linkifying disabled. Bytes are
                // forwarded exactly as read.
                if !config.dump {
                    out.write_all(&buf)?;
                }
            }
        }
        out.flush()?;
    }
}

/// Split a raw line into its body and terminator (`\n`, `\r\n`, or none at
/// EOF), preserving whichever the input used.
fn split_terminator(buf: &[u8]) -> (&[u8], &[u8]) {
    if buf.ends_with(b"\r\n") {
        buf.split_at(buf.len() - 2)
    } else if buf.ends_with(b"\n") {
        buf.split_at(buf.len() - 1)
    } else {
        (buf, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_terminator_handles_lf_crlf_and_eof() {
        assert_eq!(split_terminator(b"abc\n"), (&b"abc"[..], &b"\n"[..]));
        assert_eq!(split_terminator(b"abc\r\n"), (&b"abc"[..], &b"\r\n"[..]));
        assert_eq!(split_terminator(b"abc"), (&b"abc"[..], &b""[..]));
        assert_eq!(split_terminator(b"\n"), (&b""[..], &b"\n"[..]));
    }

    #[test]
    fn hostname_is_never_empty() {
        assert!(!hostname().is_empty());
    }
}
