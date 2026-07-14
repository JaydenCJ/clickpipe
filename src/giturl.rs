//! Git remote discovery: turn the repository around the current directory
//! into an issue-URL template, so `make 2>&1 | clickpipe` links `#123` to
//! the right tracker with zero flags.
//!
//! Everything here is offline file reading: walk up from the working
//! directory to `.git/config` (following `gitdir:` indirection for
//! worktrees), take the `origin` remote (or the first remote), and
//! normalize scp/ssh/https remote syntax to an `https://` web base.

use std::fs;
use std::path::{Path, PathBuf};

/// Discover the issue template (`https://host/owner/repo/issues/{id}`) for
/// the repository containing `dir`, if any.
pub fn issue_template_from(dir: &Path) -> Option<String> {
    let config = find_git_config(dir)?;
    let text = fs::read_to_string(config).ok()?;
    let url = remote_url(&text)?;
    let base = normalize_remote(&url)?;
    Some(template_for(&base))
}

/// Walk up from `dir` looking for `.git/config`. A `.git` *file* (linked
/// worktree) contains `gitdir: <path>`; the shared config then lives above
/// the `worktrees/<name>` directory it points at.
fn find_git_config(dir: &Path) -> Option<PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        let dotgit = d.join(".git");
        if dotgit.is_dir() {
            let config = dotgit.join("config");
            if config.is_file() {
                return Some(config);
            }
        } else if dotgit.is_file() {
            if let Ok(text) = fs::read_to_string(&dotgit) {
                if let Some(gitdir) = text.trim().strip_prefix("gitdir:") {
                    let gitdir = d.join(gitdir.trim());
                    let root = gitdir
                        .ancestors()
                        .find(|a| a.file_name().is_some_and(|n| n == "worktrees"))
                        .and_then(Path::parent)
                        .map(Path::to_path_buf)
                        .unwrap_or(gitdir);
                    let config = root.join("config");
                    if config.is_file() {
                        return Some(config);
                    }
                }
            }
        }
        cur = d.parent();
    }
    None
}

/// Extract the `origin` remote URL from git config text, falling back to
/// the first remote if `origin` is absent.
fn remote_url(config: &str) -> Option<String> {
    let mut in_remote = false;
    let mut in_origin = false;
    let mut first: Option<String> = None;
    let mut origin: Option<String> = None;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_remote = line.starts_with("[remote ");
            in_origin = line == "[remote \"origin\"]";
            continue;
        }
        if !in_remote {
            continue;
        }
        if let Some(rest) = line.strip_prefix("url") {
            if let Some(value) = rest.trim_start().strip_prefix('=') {
                let value = value.trim();
                if in_origin && origin.is_none() {
                    origin = Some(value.to_string());
                }
                if first.is_none() {
                    first = Some(value.to_string());
                }
            }
        }
    }
    origin.or(first)
}

/// Normalize a git remote URL to an `https://host/path` web base.
/// Handles `https://`, `http://`, `git://`, `ssh://[user@]host[:port]/path`
/// and scp-style `user@host:path`; strips a trailing `.git`.
pub fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = strip_any_scheme(url) {
        // ssh://git@example.test:2222/owner/repo — split host[:port]/path.
        let (host_port, path) = rest.split_once('/')?;
        let host = host_port.rsplit_once('@').map_or(host_port, |(_, h)| h);
        let host = host.split_once(':').map_or(host, |(h, _)| h);
        (host, path)
    } else if let Some((user_host, path)) = url.split_once(':') {
        // scp syntax: git@example.test:owner/repo(.git)
        if user_host.contains('/') || path.starts_with("//") {
            return None;
        }
        let host = user_host.rsplit_once('@').map_or(user_host, |(_, h)| h);
        (host, path)
    } else {
        return None;
    };
    if host.is_empty() || path.is_empty() {
        return None;
    }
    let path = path.strip_suffix(".git").unwrap_or(path);
    let path = path.trim_matches('/');
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

fn strip_any_scheme(url: &str) -> Option<&str> {
    for scheme in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            return Some(rest);
        }
    }
    None
}

/// Issue path layout per forge family: GitLab nests issues under `/-/`.
fn template_for(base: &str) -> String {
    let host = base.strip_prefix("https://").unwrap_or(base);
    let host = host.split('/').next().unwrap_or(host);
    if host.contains("gitlab") {
        format!("{base}/-/issues/{{id}}")
    } else {
        format!("{base}/issues/{{id}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clickpipe-git-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scp_and_ssh_remotes_normalize_to_https() {
        assert_eq!(
            normalize_remote("git@example.test:acme/widgets.git").as_deref(),
            Some("https://example.test/acme/widgets")
        );
        assert_eq!(
            normalize_remote("ssh://git@example.test:2222/acme/widgets.git").as_deref(),
            Some("https://example.test/acme/widgets")
        );
    }

    #[test]
    fn https_remote_keeps_nested_gitlab_groups() {
        assert_eq!(
            normalize_remote("https://gitlab.example.test/group/sub/proj.git").as_deref(),
            Some("https://gitlab.example.test/group/sub/proj")
        );
    }

    #[test]
    fn malformed_remotes_are_rejected() {
        assert_eq!(normalize_remote("/local/bare/repo.git"), None);
        assert_eq!(normalize_remote("example.test"), None);
        assert_eq!(normalize_remote(""), None);
    }

    #[test]
    fn gitlab_hosts_get_the_dash_issues_layout() {
        assert_eq!(
            template_for("https://gitlab.example.test/g/p"),
            "https://gitlab.example.test/g/p/-/issues/{id}"
        );
        assert_eq!(
            template_for("https://example.test/a/b"),
            "https://example.test/a/b/issues/{id}"
        );
    }

    #[test]
    fn origin_wins_over_other_remotes() {
        let config = "[remote \"upstream\"]\n\turl = git@example.test:up/stream.git\n\
                      [remote \"origin\"]\n\turl = git@example.test:me/mine.git\n";
        assert_eq!(
            remote_url(config).as_deref(),
            Some("git@example.test:me/mine.git")
        );
    }

    #[test]
    fn first_remote_is_the_fallback_without_origin() {
        let config =
            "[core]\n\tbare = false\n[remote \"fork\"]\n\turl = https://example.test/f/k\n";
        assert_eq!(
            remote_url(config).as_deref(),
            Some("https://example.test/f/k")
        );
    }

    #[test]
    fn template_is_discovered_from_a_nested_directory() {
        let dir = tempdir("walkup");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join("src/deep")).unwrap();
        fs::write(
            dir.join(".git/config"),
            "[remote \"origin\"]\n\turl = git@example.test:acme/widgets.git\n",
        )
        .unwrap();
        assert_eq!(
            issue_template_from(&dir.join("src/deep")).as_deref(),
            Some("https://example.test/acme/widgets/issues/{id}")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn linked_worktree_gitdir_file_resolves_to_the_shared_config() {
        let dir = tempdir("worktree");
        let main = dir.join("main/.git");
        fs::create_dir_all(main.join("worktrees/wt")).unwrap();
        fs::write(
            main.join("config"),
            "[remote \"origin\"]\n\turl = https://example.test/acme/widgets.git\n",
        )
        .unwrap();
        let wt = dir.join("wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join("worktrees/wt").display()),
        )
        .unwrap();
        assert_eq!(
            issue_template_from(&wt).as_deref(),
            Some("https://example.test/acme/widgets/issues/{id}")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn directories_outside_any_repository_yield_none() {
        let dir = tempdir("norepo");
        assert_eq!(issue_template_from(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
