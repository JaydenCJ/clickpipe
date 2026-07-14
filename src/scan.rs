//! Detection orchestrator: runs the URL, file-path and issue-ID detectors
//! over the visible text of one line, then resolves overlaps so the output
//! never contains nested or interleaved hyperlinks.
//!
//! Overlap policy: leftmost span wins; at the same start the longest span
//! wins (so `owner/repo#123` beats the `owner/repo` path candidate); at the
//! same start and length, URLs beat paths beat issues.

use crate::uri::Editor;
use crate::{issues, paths, urls};
use std::path::PathBuf;

/// What kind of thing a link points at (also the `--dump` label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Url,
    Path,
    Issue,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Url => "url",
            Kind::Path => "path",
            Kind::Issue => "issue",
        }
    }

    fn priority(self) -> u8 {
        match self {
            Kind::Url => 0,
            Kind::Path => 1,
            Kind::Issue => 2,
        }
    }
}

/// One detected link: a byte span in the plain text plus its target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    pub href: String,
}

/// Detection settings, fully explicit so tests are deterministic.
#[derive(Debug, Clone)]
pub struct Options {
    /// Enable the file-path detector.
    pub files: bool,
    /// Enable the URL detector.
    pub urls: bool,
    /// Enable the issue-ID detector.
    pub issues: bool,
    /// Require path candidates to exist on disk (kills false positives;
    /// `--no-check` disables for logs produced on another machine).
    pub check: bool,
    /// Base directory for resolving relative paths.
    pub cwd: PathBuf,
    /// Home directory for `~/` expansion (None disables it).
    pub home: Option<PathBuf>,
    /// Hostname embedded in `file://` URIs ("" for the bare form).
    pub host: String,
    /// Where file links open; None means plain `file://` URIs.
    pub editor: Option<Editor>,
    /// Template with `{id}` for bare `#123` references; None disables them.
    pub issue_template: Option<String>,
    /// Base URL for `owner/repo#123` cross-repo references.
    pub forge: String,
    /// Base URL that turns `KEY-123` into `<base>/browse/KEY-123`; opt-in.
    pub jira: Option<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            files: true,
            urls: true,
            issues: true,
            check: true,
            cwd: PathBuf::from("."),
            home: None,
            host: String::new(),
            editor: None,
            issue_template: None,
            forge: "https://github.com".to_string(),
            jira: None,
        }
    }
}

/// Run all enabled detectors over `plain` and resolve overlaps.
pub fn scan(plain: &str, opts: &Options) -> Vec<Link> {
    let mut candidates = Vec::new();
    if opts.urls {
        candidates.extend(urls::find(plain));
    }
    if opts.files {
        candidates.extend(paths::find(plain, opts));
    }
    if opts.issues {
        candidates.extend(issues::find(plain, opts));
    }
    resolve(candidates)
}

/// Sort candidates leftmost-longest and drop everything that overlaps an
/// earlier winner.
fn resolve(mut candidates: Vec<Link>) -> Vec<Link> {
    candidates.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(a.kind.priority().cmp(&b.kind.priority()))
    });
    let mut out: Vec<Link> = Vec::with_capacity(candidates.len());
    for c in candidates {
        if out.last().map_or(true, |prev| c.start >= prev.end) {
            out.push(c);
        }
    }
    out
}

/// Drop links that overlap ranges already hyperlinked in the input.
pub fn drop_taken(links: Vec<Link>, taken: &[(usize, usize)]) -> Vec<Link> {
    if taken.is_empty() {
        return links;
    }
    links
        .into_iter()
        .filter(|l| !taken.iter().any(|&(s, e)| l.start < e && l.end > s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts_no_check() -> Options {
        Options {
            check: false,
            ..Options::default()
        }
    }

    fn texts<'a>(plain: &'a str, links: &[Link]) -> Vec<&'a str> {
        links.iter().map(|l| &plain[l.start..l.end]).collect()
    }

    #[test]
    fn url_wins_over_the_path_candidate_inside_it() {
        // "example.test/x/y.rs" is path-shaped, but it sits inside a URL:
        // only one link may be emitted and it must be the URL.
        let plain = "get https://example.test/x/y.rs now";
        let links = scan(plain, &opts_no_check());
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, Kind::Url);
        assert_eq!(texts(plain, &links), vec!["https://example.test/x/y.rs"]);
    }

    #[test]
    fn longer_issue_ref_beats_the_path_candidate_at_the_same_start() {
        let plain = "fixed in tokio-rs/tokio#6120 yesterday";
        let links = scan(plain, &opts_no_check());
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, Kind::Issue);
        assert_eq!(texts(plain, &links), vec!["tokio-rs/tokio#6120"]);
    }

    #[test]
    fn disabled_detectors_produce_nothing() {
        let plain = "https://example.test and src/lib.rs:1 and a/b#12";
        let opts = Options {
            urls: false,
            files: false,
            issues: false,
            ..opts_no_check()
        };
        assert!(scan(plain, &opts).is_empty());
    }

    #[test]
    fn links_come_back_sorted_and_non_overlapping() {
        let plain = "err at src/a.rs:1:2 see https://example.test/doc";
        let links = scan(plain, &opts_no_check());
        assert_eq!(links.len(), 2);
        assert!(links[0].start < links[1].start);
        assert!(links[0].end <= links[1].start);
        assert_eq!(links[0].kind, Kind::Path);
        assert_eq!(links[1].kind, Kind::Url);
    }

    #[test]
    fn drop_taken_removes_overlaps_but_keeps_touching_ranges() {
        let links = vec![
            Link {
                start: 0,
                end: 4,
                kind: Kind::Path,
                href: "a".into(),
            },
            Link {
                start: 10,
                end: 20,
                kind: Kind::Url,
                href: "b".into(),
            },
        ];
        let kept = drop_taken(links, &[(3, 5)]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].href, "b");
        // Ranges that only touch the link's edges do not swallow it.
        let links = vec![Link {
            start: 5,
            end: 9,
            kind: Kind::Path,
            href: "a".into(),
        }];
        assert_eq!(drop_taken(links, &[(0, 5), (9, 12)]).len(), 1);
    }

    #[test]
    fn empty_line_scans_to_nothing() {
        assert!(scan("", &Options::default()).is_empty());
    }
}
