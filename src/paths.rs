//! File-path detection: the part that makes compiler output clickable.
//!
//! Recognized shapes:
//!
//! - `src/main.rs:14:9` / `src/main.rs:14` — rustc, gcc, clang, eslint, tsc
//! - `Sources/App.swift(31,7)` / `file.c(12)` — MSVC and swiftc styles
//! - `File "/app/tool.py", line 88` — Python tracebacks (contextual)
//! - `/var/log/app.log`, `./relative`, `../up`, `~/home` — bare paths
//! - `Makefile:12` style bare filenames with a line suffix
//!
//! False positives are the enemy of a filter that sees arbitrary prose, so
//! by default a candidate only becomes a link if it actually exists on disk
//! (relative to `--cwd`). `--no-check` relaxes that to shape-based rules for
//! logs that were produced on another machine.

use crate::scan::{Kind, Link, Options};
use std::path::{Component, Path, PathBuf};

/// Characters that can be part of a path token. Colons, parens, quotes and
/// commas are deliberately excluded: they delimit tokens in compiler output
/// and are parsed as line/column suffixes afterwards.
fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '~' | '+' | '@' | '%')
}

/// Line/column information parsed from the text after a path token.
struct Suffix {
    line: Option<u32>,
    col: Option<u32>,
    /// Bytes after the token that belong to the clickable span.
    span_extra: usize,
}

/// Find all file-path links in `plain`.
pub fn find(plain: &str, opts: &Options) -> Vec<Link> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < plain.len() {
        let c = match plain[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        if !is_path_char(c) {
            i += c.len_utf8();
            continue;
        }
        let mut end = i;
        for c in plain[i..].chars() {
            if !is_path_char(c) {
                break;
            }
            end += c.len_utf8();
        }
        let token_end = trim_token(plain, i, end);
        if let Some(link) = classify(plain, i, token_end, opts) {
            i = link.end;
            out.push(link);
        } else {
            i = end;
        }
    }
    out
}

/// Trim characters that are almost always sentence punctuation, not path:
/// trailing dots ("see src/main.rs.") and trailing tildes.
fn trim_token(plain: &str, start: usize, mut end: usize) -> usize {
    while end > start {
        let last = plain[start..end].chars().next_back().unwrap();
        if matches!(last, '.' | '~') && !plain[start..end].ends_with("..") {
            end -= last.len_utf8();
        } else {
            break;
        }
    }
    end
}

/// Decide whether the token at `[start, token_end)` is a path, and build
/// the link if so.
fn classify(plain: &str, start: usize, token_end: usize, opts: &Options) -> Option<Link> {
    if token_end <= start {
        return None;
    }
    let token = &plain[start..token_end];
    // Must contain something nameable and must not be a URL remnant.
    if !token.chars().any(|c| c.is_ascii_alphanumeric()) || token.contains("://") {
        return None;
    }
    let has_slash = token.contains('/');
    let anchored = token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/");
    // scp-style remotes ("git@example.test:path") are not local paths.
    if !has_slash && token.contains('@') {
        return None;
    }

    let mut suffix = parse_suffix(plain, token_end);
    if suffix.line.is_none() {
        if let Some(line) = python_traceback_line(plain, start, token_end) {
            suffix = Suffix {
                line: Some(line),
                col: None,
                span_extra: 0,
            };
        }
    }

    // Shape rules: what is allowed to become a link at all.
    let shaped = if anchored {
        token.len() > 1
    } else if has_slash {
        // "input/output" prose needs existence or an extension to qualify.
        opts.check || has_extension(token)
    } else {
        // A bare filename is only plausible with an extension AND a line
        // reference ("main.c:12"): otherwise every word could be a file.
        has_extension(token) && suffix.line.is_some()
    };
    if !shaped {
        return None;
    }

    let resolved = resolve(token, opts)?;
    if opts.check && std::fs::symlink_metadata(&resolved).is_err() {
        return None;
    }

    let abs = lexical_normalize(&resolved);
    let abs_str = abs.to_string_lossy();
    let href = match &opts.editor {
        Some(editor) => editor.href(&abs_str, suffix.line, suffix.col),
        None => crate::uri::file_uri(&opts.host, &abs_str),
    };
    Some(Link {
        start,
        end: token_end + suffix.span_extra,
        kind: Kind::Path,
        href,
    })
}

/// Does the final path segment end in something extension-like
/// (".rs", ".tar.gz", ".c") — a dot followed by a letter-led short suffix?
fn has_extension(token: &str) -> bool {
    let last = token.rsplit('/').next().unwrap_or(token);
    match last.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=10).contains(&ext.len())
                && ext.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

/// Parse `:LINE[:COL]` (gcc/rustc) or `(LINE[,COL])` (MSVC/swiftc) directly
/// after the token. The suffix becomes part of the clickable span.
fn parse_suffix(plain: &str, token_end: usize) -> Suffix {
    let rest = &plain[token_end..];
    let none = Suffix {
        line: None,
        col: None,
        span_extra: 0,
    };
    if rest.as_bytes().first() == Some(&b':') {
        let (line, used) = match read_number(&rest[1..]) {
            Some(v) => v,
            None => return none,
        };
        let mut extra = 1 + used;
        let mut col = None;
        if rest.as_bytes().get(extra) == Some(&b':') {
            if let Some((c, used2)) = read_number(&rest[extra + 1..]) {
                col = Some(c);
                extra += 1 + used2;
            }
        }
        return Suffix {
            line: Some(line),
            col,
            span_extra: extra,
        };
    }
    if rest.as_bytes().first() == Some(&b'(') {
        if let Some((line, used)) = read_number(&rest[1..]) {
            let mut pos = 1 + used;
            let mut col = None;
            if rest.as_bytes().get(pos) == Some(&b',') {
                if let Some((c, used2)) = read_number(&rest[pos + 1..]) {
                    col = Some(c);
                    pos += 1 + used2;
                }
            }
            if rest.as_bytes().get(pos) == Some(&b')') {
                return Suffix {
                    line: Some(line),
                    col,
                    span_extra: pos + 1,
                };
            }
        }
    }
    none
}

/// Read a 1-7 digit number at the head of `s`. Returns (value, bytes used).
fn read_number(s: &str) -> Option<(u32, usize)> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() > 7 {
        return None;
    }
    Some((digits.parse().ok()?, digits.len()))
}

/// Python tracebacks put the line number after the closing quote:
/// `File "/app/x.py", line 88, in main`.
fn python_traceback_line(plain: &str, start: usize, token_end: usize) -> Option<u32> {
    if !plain[..start].ends_with("File \"") {
        return None;
    }
    let rest = plain[token_end..].strip_prefix("\", line ")?;
    read_number(rest).map(|(line, _)| line)
}

/// Expand `~/` and make the token absolute against the configured cwd.
fn resolve(token: &str, opts: &Options) -> Option<PathBuf> {
    if let Some(rest) = token.strip_prefix("~/") {
        return opts.home.as_ref().map(|h| h.join(rest));
    }
    let p = Path::new(token);
    if p.is_absolute() {
        Some(p.to_path_buf())
    } else {
        Some(opts.cwd.join(p))
    }
}

/// Resolve `.` and `..` components lexically (no filesystem access, no
/// symlink resolution — the link should show the path the tool printed).
pub fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                // `/..` is `/`; keep leading `..`s of a relative path.
                Some(Component::RootDir) => {}
                None | Some(Component::ParentDir) => out.push(".."),
                _ => {
                    out.pop();
                }
            },
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uri::Editor;
    use std::fs;

    /// Options with existence checks off — pure shape-rule tests.
    fn loose() -> Options {
        Options {
            check: false,
            cwd: PathBuf::from("/work"),
            ..Options::default()
        }
    }

    fn one(plain: &str, opts: &Options) -> Link {
        let links = find(plain, opts);
        assert_eq!(
            links.len(),
            1,
            "expected exactly one link in {plain:?}: {links:?}"
        );
        links.into_iter().next().unwrap()
    }

    fn span<'a>(plain: &'a str, l: &Link) -> &'a str {
        &plain[l.start..l.end]
    }

    #[test]
    fn rustc_style_line_and_column_are_clickable_and_in_the_href() {
        let plain = " --> src/main.rs:14:9";
        let l = one(plain, &loose());
        assert_eq!(span(plain, &l), "src/main.rs:14:9");
        assert_eq!(l.href, "file:///work/src/main.rs");
    }

    #[test]
    fn editor_scheme_carries_line_and_column() {
        let opts = Options {
            editor: Some(Editor::from_arg("vscode").unwrap()),
            ..loose()
        };
        let l = one("err at src/main.rs:14:9:", &opts);
        assert_eq!(l.href, "vscode://file/work/src/main.rs:14:9");
    }

    #[test]
    fn gcc_style_trailing_colon_stays_outside_the_span() {
        let plain = "lib.c:31:12: warning: unused";
        let l = one(plain, &loose());
        assert_eq!(span(plain, &l), "lib.c:31:12");
    }

    #[test]
    fn msvc_parenthesized_location_is_parsed() {
        let plain = r"main.cpp(31,7): error C2065";
        let opts = Options {
            editor: Some(Editor::from_arg("vscode").unwrap()),
            ..loose()
        };
        let l = one(plain, &opts);
        assert_eq!(span(plain, &l), "main.cpp(31,7)");
        assert_eq!(l.href, "vscode://file/work/main.cpp:31:7");
    }

    #[test]
    fn python_traceback_file_line_is_contextual() {
        let plain = "  File \"/app/tool.py\", line 88, in main";
        let opts = Options {
            editor: Some(Editor::from_arg("vscode").unwrap()),
            ..loose()
        };
        let l = one(plain, &opts);
        // Only the path is wrapped; the quotes and ", line" prose are not.
        assert_eq!(span(plain, &l), "/app/tool.py");
        assert_eq!(l.href, "vscode://file/app/tool.py:88");
    }

    #[test]
    fn absolute_path_without_line_info_links_to_a_file_uri() {
        let opts = Options {
            host: "devbox".into(),
            ..loose()
        };
        let l = one("log at /var/log/app/current.log now", &opts);
        assert_eq!(l.href, "file://devbox/var/log/app/current.log");
    }

    #[test]
    fn relative_dot_prefixes_are_resolved_lexically() {
        let l = one("wrote ./out/../report.txt", &loose());
        assert_eq!(l.href, "file:///work/report.txt");
    }

    #[test]
    fn tilde_expands_against_the_configured_home_or_is_skipped() {
        let opts = Options {
            home: Some(PathBuf::from("/home/dev")),
            ..loose()
        };
        let l = one("config: ~/.config/tool/config.toml", &opts);
        assert_eq!(l.href, "file:///home/dev/.config/tool/config.toml");
        // Without a home directory, `~/` candidates must not guess.
        assert!(find("see ~/notes.txt", &loose()).is_empty());
    }

    #[test]
    fn prose_slash_words_and_version_numbers_are_not_paths() {
        // Without an extension or an anchor, "input/output" and "and/or"
        // must never linkify in --no-check mode.
        assert!(find("the input/output ratio and/or more", &loose()).is_empty());
        // "1.2.3" has a numeric extension; "1.2.4:5" is not file:line.
        assert!(find("upgrade 1.2.3 to 1.2.4:5 now", &loose()).is_empty());
        // scp-style remotes are not local paths.
        assert!(find("pull from git@example.test:org/repo.git", &loose())
            .iter()
            .all(|l| !l.href.contains('@')));
    }

    #[test]
    fn bare_filename_needs_both_extension_and_line_number() {
        assert!(find("compile main.c today", &loose()).is_empty());
        let plain = "main.c:12: undefined reference";
        assert_eq!(span(plain, &one(plain, &loose())), "main.c:12");
    }

    #[test]
    fn trailing_sentence_dot_is_trimmed() {
        let plain = "saved to /tmp/out/result.json.";
        let l = one(plain, &loose());
        assert_eq!(span(plain, &l), "/tmp/out/result.json");
    }

    #[test]
    fn spaces_in_resolved_paths_are_percent_encoded() {
        let opts = Options {
            cwd: PathBuf::from("/work/my dir"),
            ..loose()
        };
        let l = one("see src/a.rs:1", &opts);
        assert_eq!(l.href, "file:///work/my%20dir/src/a.rs");
    }

    #[test]
    fn existence_check_filters_out_missing_files() {
        let dir = std::env::temp_dir().join(format!("clickpipe-paths-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/real.rs"), "fn main() {}\n").unwrap();
        let opts = Options {
            check: true,
            cwd: dir.clone(),
            ..Options::default()
        };
        let links = find("src/real.rs:1 and src/ghost.rs:2", &opts);
        assert_eq!(links.len(), 1);
        assert!(links[0].href.ends_with("/src/real.rs"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn existing_bare_directory_names_stay_unlinked() {
        // Even when "src" exists, the bare word "src" in prose must not
        // become a link — bare tokens require an extension + line.
        let dir = std::env::temp_dir().join(format!("clickpipe-dirs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        let opts = Options {
            check: true,
            cwd: dir.clone(),
            ..Options::default()
        };
        assert!(find("the src directory holds code", &opts).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn node_stack_frame_paths_inside_parens_are_found() {
        let plain = "    at Object.<anonymous> (/app/src/index.js:10:15)";
        let l = one(plain, &loose());
        assert_eq!(span(plain, &l), "/app/src/index.js:10:15");
    }

    #[test]
    fn lexical_normalize_resolves_dots_without_touching_the_fs() {
        assert_eq!(
            lexical_normalize(Path::new("/a/b/../c/./d")),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(lexical_normalize(Path::new("/../x")), PathBuf::from("/x"));
    }

    #[test]
    fn line_numbers_longer_than_seven_digits_are_ignored() {
        // A hash-like ":123456789" is not a line number; the anchored path
        // itself still links, but the digits stay outside the span.
        let plain = "blob /data/objects/pack.idx:123456789";
        let l = one(plain, &loose());
        assert_eq!(span(plain, &l), "/data/objects/pack.idx");
    }
}
