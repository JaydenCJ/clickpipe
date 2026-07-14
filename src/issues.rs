//! Issue-ID detection: `#123`, cross-repo `owner/repo#123`, and opt-in
//! Jira-style `KEY-123` references.
//!
//! Bare `#123` is ambiguous without a tracker, so it only links when an
//! issue template is configured — explicitly via `--issues`/`--repo`, or
//! discovered from the repository's git remote (see [`crate::giturl`]).
//! `owner/repo#123` names its repository itself and links against the
//! configured forge base. `KEY-123` collides with too many real-world
//! strings (`UTF-8`, `SHA-256`) to be safe by default, so it requires
//! `--jira <base-url>`.

use crate::scan::{Kind, Link, Options};

/// Find all issue-reference links in `plain`.
pub fn find(plain: &str, opts: &Options) -> Vec<Link> {
    let mut out = Vec::new();
    find_hash_refs(plain, opts, &mut out);
    if let Some(base) = &opts.jira {
        find_jira_keys(plain, base, &mut out);
    }
    out
}

/// `#123` and `owner/repo#123`.
fn find_hash_refs(plain: &str, opts: &Options, out: &mut Vec<Link>) {
    for (i, _) in plain.match_indices('#') {
        let Some((id, id_len)) = number_after(plain, i + 1) else {
            continue;
        };
        let end = i + 1 + id_len;
        // The ref must end at a word boundary: "#123abc" is a color hash
        // or an identifier, never an issue.
        if plain[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '#')
        {
            continue;
        }
        if let Some((start, owner_repo)) = slug_before(plain, i) {
            out.push(Link {
                start,
                end,
                kind: Kind::Issue,
                href: format!("{}/{}/issues/{}", opts.forge, owner_repo, id),
            });
            continue;
        }
        // Bare ref: needs a configured tracker and a clean left boundary.
        let Some(template) = &opts.issue_template else {
            continue;
        };
        let boundary = plain[..i].chars().next_back().map_or(true, |c| {
            c.is_whitespace() || matches!(c, '(' | '[' | '{' | '<' | ',' | ':')
        });
        if boundary {
            out.push(Link {
                start: i,
                end,
                kind: Kind::Issue,
                href: template.replace("{id}", &id.to_string()),
            });
        }
    }
}

/// Read 1-7 digits starting at byte `at`.
fn number_after(plain: &str, at: usize) -> Option<(u32, usize)> {
    let digits: String = plain[at..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() || digits.len() > 7 {
        return None;
    }
    Some((digits.parse().ok()?, digits.len()))
}

/// Walk backwards from the `#` at `hash` looking for exactly
/// `owner/repo` — two slug segments, one slash, clean left boundary.
fn slug_before(plain: &str, hash: usize) -> Option<(usize, &str)> {
    let is_slug = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.');
    let before = &plain[..hash];
    let repo_start = before.rfind(|c: char| !is_slug(c)).map_or(0, |p| p + 1);
    let repo = &before[repo_start..];
    if repo.is_empty() || !before[..repo_start].ends_with('/') {
        return None;
    }
    let owner_end = repo_start - 1;
    let owner_start = before[..owner_end]
        .rfind(|c: char| !is_slug(c))
        .map_or(0, |p| p + 1);
    let owner = &before[owner_start..owner_end];
    if owner.is_empty() {
        return None;
    }
    // The character before the owner must not extend the token ("a/b/c#1"
    // or "v1.2/x#3" are paths or fragments, not repo slugs).
    if plain[..owner_start]
        .chars()
        .next_back()
        .is_some_and(|c| is_slug(c) || matches!(c, '/' | '~' | '#'))
    {
        return None;
    }
    Some((owner_start, &plain[owner_start..hash]))
}

/// Jira-style keys: `ABC-123` with an uppercase project key.
fn find_jira_keys(plain: &str, base: &str, out: &mut Vec<Link>) {
    let bytes = plain.as_bytes();
    let mut i = 0;
    while i < plain.len() {
        if !bytes[i].is_ascii_uppercase() {
            i += 1;
            continue;
        }
        // Left boundary: previous char must not be part of a bigger token.
        if plain[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
        {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < plain.len() && (bytes[j].is_ascii_uppercase() || bytes[j].is_ascii_digit()) {
            j += 1;
        }
        let key_len = j - i;
        if !(2..=10).contains(&key_len) || bytes.get(j) != Some(&b'-') {
            i = j;
            continue;
        }
        let Some((_, digits)) = number_after(plain, j + 1) else {
            i = j + 1;
            continue;
        };
        let end = j + 1 + digits;
        // Right boundary: "PROJ-123x" or "PROJ-123-foo" is an identifier.
        if plain[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            i = end;
            continue;
        }
        out.push(Link {
            start: i,
            end,
            kind: Kind::Issue,
            href: format!("{}/browse/{}", base, &plain[i..end]),
        });
        i = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::Options;

    fn with_template() -> Options {
        Options {
            issue_template: Some("https://forge.example.test/t/p/issues/{id}".into()),
            ..Options::default()
        }
    }

    fn spans(plain: &str, opts: &Options) -> Vec<(String, String)> {
        find(plain, opts)
            .into_iter()
            .map(|l| (plain[l.start..l.end].to_string(), l.href))
            .collect()
    }

    #[test]
    fn bare_ref_links_through_the_configured_template() {
        assert_eq!(
            spans("fixes #142 for good", &with_template()),
            vec![(
                "#142".into(),
                "https://forge.example.test/t/p/issues/142".into()
            )]
        );
        // Parenthesized and line-leading refs match too.
        assert_eq!(spans("(#7)", &with_template()).len(), 1);
        assert_eq!(spans("#7 first thing", &with_template()).len(), 1);
    }

    #[test]
    fn bare_ref_without_a_template_stays_plain_text() {
        assert!(spans("fixes #142", &Options::default()).is_empty());
    }

    #[test]
    fn hex_colors_and_identifiers_do_not_match() {
        let opts = with_template();
        assert!(spans("color: #142abc;", &opts).is_empty());
        assert!(spans("hash #142142142 overflow", &opts).is_empty());
        assert!(spans("word#142 glued", &opts).is_empty());
    }

    #[test]
    fn cross_repo_ref_links_to_the_forge_without_any_template() {
        assert_eq!(
            spans("ported from tokio-rs/tokio#6120", &Options::default()),
            vec![(
                "tokio-rs/tokio#6120".into(),
                "https://github.com/tokio-rs/tokio/issues/6120".into()
            )]
        );
        // The forge base is configurable for self-hosted forges.
        let opts = Options {
            forge: "https://git.example.test".into(),
            ..Options::default()
        };
        assert_eq!(
            spans("see org/proj#9", &opts)[0].1,
            "https://git.example.test/org/proj/issues/9"
        );
    }

    #[test]
    fn deep_paths_are_not_cross_repo_refs() {
        // "a/b/c#1" has three segments: that is a path fragment, not a slug.
        assert!(spans("see a/b/c#1", &Options::default()).is_empty());
    }

    #[test]
    fn jira_keys_only_match_when_a_base_is_configured() {
        assert!(spans("work on PROJ-123", &Options::default()).is_empty());
        let opts = Options {
            jira: Some("https://jira.example.test".into()),
            ..Options::default()
        };
        assert_eq!(
            spans("work on PROJ-123 now", &opts),
            vec![(
                "PROJ-123".into(),
                "https://jira.example.test/browse/PROJ-123".into()
            )]
        );
    }

    #[test]
    fn jira_key_boundaries_reject_identifier_like_strings() {
        let opts = Options {
            jira: Some("https://jira.example.test".into()),
            ..Options::default()
        };
        assert!(spans("id XPROJ-123x tail", &opts).is_empty());
        assert!(spans("branch PROJ-123-fix-crash", &opts).is_empty());
        assert!(spans("var MY_PROJ-1", &opts).is_empty());
        // Single-letter keys are below the Jira minimum of two characters.
        assert!(spans("grade A-1 beef", &opts).is_empty());
    }

    #[test]
    fn multiple_refs_on_one_line_all_link() {
        let got = spans("closes #1, #2 and org/other#3", &with_template());
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].0, "#1");
        assert_eq!(got[1].0, "#2");
        assert_eq!(got[2].0, "org/other#3");
    }
}
