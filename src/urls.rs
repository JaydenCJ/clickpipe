//! URL detection in plain text.
//!
//! Finds `http://`, `https://`, `file://` and bare `www.` targets, extends
//! them to the longest plausible span, then trims the punctuation that prose
//! and Markdown wrap around URLs — including the classic unbalanced-paren
//! problem: `(see https://example.test/a)` must not link the closing paren,
//! while `https://en.wikipedia.org/wiki/Pipe_(Unix)` must keep it.

use crate::scan::{Kind, Link};

const SCHEMES: [&str; 3] = ["https://", "http://", "file://"];

/// Characters that terminate a URL span outright.
fn is_terminator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '|') || c.is_control()
}

/// Find all URL links in `plain`.
pub fn find(plain: &str) -> Vec<Link> {
    let mut out = Vec::new();
    let bytes = plain.as_bytes();
    let mut i = 0;
    while i < plain.len() {
        let Some(hit) = match_start(plain, i) else {
            i += char_len(bytes[i]);
            continue;
        };
        let (skip, is_www) = hit;
        // Word boundary: the previous character must not be part of a
        // longer token ("xhttp://", "foo.www.").
        if let Some(prev) = plain[..i].chars().next_back() {
            let scheme_tail = prev.is_ascii_alphanumeric() || matches!(prev, '+' | '-' | '.');
            let www_tail = prev.is_ascii_alphanumeric() || matches!(prev, '.' | '@' | '/' | '-');
            if (is_www && www_tail) || (!is_www && scheme_tail) {
                i += char_len(bytes[i]);
                continue;
            }
        }
        let mut end = i + skip;
        for c in plain[end..].chars() {
            if is_terminator(c) {
                break;
            }
            end += c.len_utf8();
        }
        let end = trim_end(plain, i, end);
        // Require some substance after the scheme marker.
        if plain[i + skip..end]
            .chars()
            .any(|c| c.is_ascii_alphanumeric())
        {
            let text = &plain[i..end];
            let href = if is_www {
                format!("https://{text}")
            } else {
                text.to_string()
            };
            out.push(Link {
                start: i,
                end,
                kind: Kind::Url,
                href: encode_href(&href),
            });
            i = end;
        } else {
            i += skip;
        }
    }
    out
}

/// Does a URL start at byte `i`? Returns (marker length, is_www).
fn match_start(plain: &str, i: usize) -> Option<(usize, bool)> {
    let rest = &plain.as_bytes()[i..];
    for scheme in SCHEMES {
        if rest.len() >= scheme.len()
            && rest[..scheme.len()].eq_ignore_ascii_case(scheme.as_bytes())
        {
            return Some((scheme.len(), false));
        }
    }
    if rest.len() >= 5 && rest[..4].eq_ignore_ascii_case(b"www.") && rest[4].is_ascii_alphanumeric()
    {
        return Some((4, true));
    }
    None
}

/// Trim trailing prose punctuation and unbalanced closing brackets.
fn trim_end(plain: &str, start: usize, mut end: usize) -> usize {
    loop {
        let Some(last) = plain[start..end].chars().next_back() else {
            return end;
        };
        match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '*' => end -= last.len_utf8(),
            ')' | ']' | '}' => {
                let (open, close) = match last {
                    ')' => ('(', ')'),
                    ']' => ('[', ']'),
                    _ => ('{', '}'),
                };
                let span = &plain[start..end];
                let opens = span.matches(open).count();
                let closes = span.matches(close).count();
                if closes > opens {
                    end -= 1;
                } else {
                    return end;
                }
            }
            _ => return end,
        }
    }
}

/// Percent-encode the bytes that cannot travel inside an OSC 8 payload
/// (spaces never appear here, but non-ASCII IRI characters do).
fn encode_href(href: &str) -> String {
    if href.is_ascii() {
        return href.to_string();
    }
    let mut out = String::with_capacity(href.len() + 8);
    for b in href.bytes() {
        if b.is_ascii() {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn char_len(b: u8) -> usize {
    match b {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str) -> Vec<(String, String)> {
        find(text)
            .into_iter()
            .map(|l| (text[l.start..l.end].to_string(), l.href))
            .collect()
    }

    #[test]
    fn plain_https_url_is_found_verbatim_and_case_insensitively() {
        assert_eq!(
            spans("docs at https://example.test/guide today"),
            vec![(
                "https://example.test/guide".into(),
                "https://example.test/guide".into()
            )]
        );
        assert_eq!(
            spans("HTTPS://EXAMPLE.TEST/A")[0].0,
            "HTTPS://EXAMPLE.TEST/A"
        );
    }

    #[test]
    fn query_and_fragment_are_part_of_the_link() {
        let url = "https://example.test/search?q=a+b&lang=en#results";
        assert_eq!(spans(url), vec![(url.into(), url.into())]);
    }

    #[test]
    fn trailing_sentence_punctuation_is_trimmed() {
        assert_eq!(
            spans("see https://example.test/a.")[0].0,
            "https://example.test/a"
        );
        assert_eq!(
            spans("see https://example.test/a,")[0].0,
            "https://example.test/a"
        );
        assert_eq!(
            spans("really https://example.test/a?!")[0].0,
            "https://example.test/a"
        );
    }

    #[test]
    fn unbalanced_closing_paren_is_trimmed_but_balanced_is_kept() {
        // Markdown/prose wrapper: the paren belongs to the sentence.
        assert_eq!(
            spans("(then https://example.test/a)")[0].0,
            "https://example.test/a"
        );
        // Wikipedia-style URL: the paren belongs to the URL.
        let wiki = "https://en.wikipedia.org/wiki/Pipe_(Unix)";
        assert_eq!(spans(wiki)[0].0, wiki);
    }

    #[test]
    fn brackets_quotes_backticks_and_pipes_delimit_the_url() {
        assert_eq!(
            spans("<https://example.test/a>")[0].0,
            "https://example.test/a"
        );
        assert_eq!(
            spans("href=\"https://example.test/a\"")[0].0,
            "https://example.test/a"
        );
        assert_eq!(
            spans("`https://example.test/a`")[0].0,
            "https://example.test/a"
        );
        assert_eq!(
            spans("https://example.test/a|next")[0].0,
            "https://example.test/a"
        );
    }

    #[test]
    fn www_prefix_gets_an_https_href_but_keeps_its_text() {
        let links = find("visit www.example.test/x now");
        assert_eq!(links.len(), 1);
        assert_eq!(
            &"visit www.example.test/x now"[links[0].start..links[0].end],
            "www.example.test/x"
        );
        assert_eq!(links[0].href, "https://www.example.test/x");
    }

    #[test]
    fn scheme_must_start_at_a_word_boundary_and_have_substance() {
        assert!(spans("xhttps://example.test").is_empty());
        assert!(spans("foo.www.example").is_empty());
        // A bare scheme marker with nothing behind it is prose, not a URL.
        assert!(spans("the https:// prefix and http://. too").is_empty());
        // ...but punctuation before the scheme is fine.
        assert_eq!(
            spans("[https://example.test/a]")[0].0,
            "https://example.test/a"
        );
    }

    #[test]
    fn file_urls_are_linked_as_is() {
        assert_eq!(
            spans("wrote file:///tmp/out.txt")[0],
            ("file:///tmp/out.txt".into(), "file:///tmp/out.txt".into())
        );
    }

    #[test]
    fn multiple_urls_on_one_line_are_all_found_in_order() {
        let got = spans("a https://example.test/1 b http://example.test/2 c");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "https://example.test/1");
        assert_eq!(got[1].0, "http://example.test/2");
    }

    #[test]
    fn non_ascii_iri_bytes_are_percent_encoded_in_the_href_only() {
        let text = "read https://example.test/café now";
        let links = find(text);
        assert_eq!(
            &text[links[0].start..links[0].end],
            "https://example.test/café"
        );
        assert_eq!(links[0].href, "https://example.test/caf%C3%A9");
    }
}
