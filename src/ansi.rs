//! ANSI-aware line model.
//!
//! Real command output is full of SGR color codes, and colored text is
//! exactly where links matter most (compiler errors). This module splits a
//! line into escape-sequence and plain-text pieces so the detectors can run
//! over the *visible* text, then re-emits the original bytes with OSC 8
//! wrappers inserted at the right plain-text offsets. Escape sequences —
//! including color changes in the middle of a matched path — pass through
//! untouched, and regions already inside an OSC 8 hyperlink (e.g. from
//! `ls --hyperlink`) are reported as taken so they are never double-wrapped.

use crate::osc8;
use crate::scan::Link;

enum Piece<'a> {
    /// Visible text; `plain_start` is its byte offset in the plain string.
    Text { s: &'a str, plain_start: usize },
    /// Any escape sequence, passed through verbatim.
    Esc(&'a str),
}

/// A parsed line: original pieces, the concatenated visible text, and the
/// plain-text ranges already covered by pre-existing hyperlinks.
pub struct AnsiLine<'a> {
    pieces: Vec<Piece<'a>>,
    plain: String,
    taken: Vec<(usize, usize)>,
}

impl<'a> AnsiLine<'a> {
    pub fn parse(line: &'a str) -> AnsiLine<'a> {
        let bytes = line.as_bytes();
        let mut pieces = Vec::new();
        let mut plain = String::new();
        let mut taken = Vec::new();
        let mut link_open_at: Option<usize> = None;
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b {
                let end = esc_end(bytes, i);
                let seq = &line[i..end];
                if let Some(uri) = osc8_uri(seq) {
                    if uri.is_empty() {
                        if let Some(start) = link_open_at.take() {
                            if plain.len() > start {
                                taken.push((start, plain.len()));
                            }
                        }
                    } else {
                        link_open_at = Some(plain.len());
                    }
                }
                pieces.push(Piece::Esc(seq));
                i = end;
            } else {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != 0x1b {
                    j += 1;
                }
                let s = &line[i..j];
                pieces.push(Piece::Text {
                    s,
                    plain_start: plain.len(),
                });
                plain.push_str(s);
                i = j;
            }
        }
        // An unclosed link still owns the rest of the line.
        if let Some(start) = link_open_at {
            if plain.len() > start {
                taken.push((start, plain.len()));
            }
        }
        AnsiLine {
            pieces,
            plain,
            taken,
        }
    }

    /// The visible text of the line (what detectors scan).
    pub fn plain(&self) -> &str {
        &self.plain
    }

    /// Plain-text ranges already inside a hyperlink in the input.
    pub fn taken(&self) -> &[(usize, usize)] {
        &self.taken
    }

    /// Re-emit the original line with OSC 8 wrappers around `links`.
    /// `links` must be sorted by start and non-overlapping (scan::resolve
    /// guarantees both). A link may span escape sequences: the wrapper opens
    /// before its first visible byte and closes after its last, so colors
    /// keep working inside the link.
    pub fn render(&self, links: &[Link]) -> String {
        if links.is_empty() {
            // Fast path: reassemble the original bytes exactly.
            return self
                .pieces
                .iter()
                .map(|p| match p {
                    Piece::Text { s, .. } => *s,
                    Piece::Esc(s) => *s,
                })
                .collect();
        }
        let mut out = String::with_capacity(self.plain.len() + links.len() * 32);
        let mut li = 0usize;
        let mut open = false;
        for piece in &self.pieces {
            match piece {
                Piece::Esc(s) => out.push_str(s),
                Piece::Text { s, plain_start } => {
                    let base = *plain_start;
                    let mut cur = 0usize;
                    while cur < s.len() {
                        let abs = base + cur;
                        if open {
                            let stop = links[li].end.min(base + s.len()) - base;
                            out.push_str(&s[cur..stop]);
                            cur = stop;
                            if base + cur == links[li].end {
                                out.push_str(osc8::CLOSE);
                                open = false;
                                li += 1;
                            }
                        } else if li < links.len() && links[li].start < base + s.len() {
                            if abs < links[li].start {
                                let stop = links[li].start - base;
                                out.push_str(&s[cur..stop]);
                                cur = stop;
                            } else {
                                out.push_str(&osc8::open(&links[li].href));
                                open = true;
                            }
                        } else {
                            out.push_str(&s[cur..]);
                            cur = s.len();
                        }
                    }
                }
            }
        }
        if open {
            out.push_str(osc8::CLOSE);
        }
        out
    }
}

/// Byte offset one past the end of the escape sequence starting at `i`
/// (which must point at ESC). Only ASCII bytes are consumed for CSI-style
/// sequences, so the returned offset always lands on a UTF-8 boundary.
fn esc_end(bytes: &[u8], i: usize) -> usize {
    if i + 1 >= bytes.len() {
        return i + 1; // lone trailing ESC
    }
    match bytes[i + 1] {
        b'[' => {
            // CSI: parameter/intermediate bytes 0x20..=0x3F, final 0x40..=0x7E.
            let mut j = i + 2;
            while j < bytes.len() && (0x20..=0x3f).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && (0x40..=0x7e).contains(&bytes[j]) {
                j + 1
            } else {
                j
            }
        }
        b']' | b'P' | b'X' | b'^' | b'_' => {
            // OSC / DCS / SOS / PM / APC: run to BEL or ST (ESC \).
            let mut j = i + 2;
            while j < bytes.len() {
                if bytes[j] == 0x07 {
                    return j + 1;
                }
                if bytes[j] == 0x1b {
                    if j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                        return j + 2;
                    }
                    return j; // a new sequence starts; this one was cut short
                }
                j += 1;
            }
            j
        }
        0x20..=0x2f => {
            // ESC + intermediates + final (e.g. charset selection ESC ( B).
            let mut j = i + 1;
            while j < bytes.len() && (0x20..=0x2f).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && (0x30..=0x7e).contains(&bytes[j]) {
                j + 1
            } else {
                j
            }
        }
        0x30..=0x7e => i + 2, // two-byte escape (ESC 7, ESC =, ...)
        _ => i + 1,
    }
}

/// If `seq` is an OSC 8 sequence, return its URI part ("" for a closer).
fn osc8_uri(seq: &str) -> Option<&str> {
    let body = seq.strip_prefix("\x1b]8;")?;
    let body = body
        .strip_suffix("\x1b\\")
        .or_else(|| body.strip_suffix('\x07'))
        .unwrap_or(body);
    // Skip the params field (between the two semicolons).
    let (_params, uri) = body.split_once(';')?;
    Some(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{Kind, Link};

    fn link(start: usize, end: usize, href: &str) -> Link {
        Link {
            start,
            end,
            kind: Kind::Url,
            href: href.to_string(),
        }
    }

    #[test]
    fn plain_text_of_an_uncolored_line_is_the_line_itself() {
        let l = AnsiLine::parse("error in src/main.rs:3");
        assert_eq!(l.plain(), "error in src/main.rs:3");
        assert!(l.taken().is_empty());
    }

    #[test]
    fn sgr_sequences_are_stripped_from_plain_but_kept_in_render() {
        let raw = "\x1b[1m\x1b[31merror\x1b[0m: boom";
        let l = AnsiLine::parse(raw);
        assert_eq!(l.plain(), "error: boom");
        assert_eq!(l.render(&[]), raw);
    }

    #[test]
    fn render_wraps_a_match_in_the_middle_of_plain_text() {
        let l = AnsiLine::parse("see https://example.test now");
        let out = l.render(&[link(4, 24, "https://example.test")]);
        assert_eq!(
            out,
            "see \x1b]8;;https://example.test\x1b\\https://example.test\x1b]8;;\x1b\\ now"
        );
    }

    #[test]
    fn render_keeps_color_changes_inside_the_wrapped_span() {
        // grep --color output: the middle of the path is highlighted. The
        // link must open before the first path byte and close after the
        // last, with the SGR bytes preserved in between.
        let raw = "src/\x1b[01;31mmain\x1b[0m.rs";
        let l = AnsiLine::parse(raw);
        assert_eq!(l.plain(), "src/main.rs");
        let out = l.render(&[link(0, 11, "file:///w/src/main.rs")]);
        assert_eq!(
            out,
            "\x1b]8;;file:///w/src/main.rs\x1b\\src/\x1b[01;31mmain\x1b[0m.rs\x1b]8;;\x1b\\"
        );
    }

    #[test]
    fn render_handles_adjacent_links_and_line_edges() {
        let l = AnsiLine::parse("a.rs b.rs");
        let out = l.render(&[link(0, 4, "file:///a.rs"), link(5, 9, "file:///b.rs")]);
        assert_eq!(
            out,
            "\x1b]8;;file:///a.rs\x1b\\a.rs\x1b]8;;\x1b\\ \x1b]8;;file:///b.rs\x1b\\b.rs\x1b]8;;\x1b\\"
        );
    }

    #[test]
    fn existing_hyperlink_regions_are_reported_as_taken() {
        // `ls --hyperlink` style input: text between open and close is taken.
        let raw = "x \x1b]8;;file:///etc/hosts\x1b\\hosts\x1b]8;;\x1b\\ y";
        let l = AnsiLine::parse(raw);
        assert_eq!(l.plain(), "x hosts y");
        assert_eq!(l.taken(), &[(2, 7)]);
        // Round-trips byte-identically when nothing new is linked.
        assert_eq!(l.render(&[]), raw);
    }

    #[test]
    fn osc8_variants_bel_terminated_unclosed_and_with_params_are_taken() {
        // BEL-terminated form.
        let l = AnsiLine::parse("\x1b]8;;https://example.test\x07here\x1b]8;;\x07 tail");
        assert_eq!(l.plain(), "here tail");
        assert_eq!(l.taken(), &[(0, 4)]);
        // An unclosed link owns the rest of the line.
        let l = AnsiLine::parse("\x1b]8;;https://example.test\x1b\\tail text");
        assert_eq!(l.taken(), &[(0, 9)]);
        // A params field (id=...) is skipped when reading the URI.
        let l = AnsiLine::parse("\x1b]8;id=xyz;https://example.test\x1b\\t\x1b]8;;\x1b\\");
        assert_eq!(l.taken(), &[(0, 1)]);
    }

    #[test]
    fn malformed_and_non_link_escapes_round_trip_unharmed() {
        // Lone trailing ESC must neither panic nor vanish.
        let raw = "half\x1b";
        let l = AnsiLine::parse(raw);
        assert_eq!(l.plain(), "half");
        assert_eq!(l.render(&[]), raw);
        // A window-title OSC is not a hyperlink.
        let raw = "\x1b]0;my title\x07visible";
        let l = AnsiLine::parse(raw);
        assert_eq!(l.plain(), "visible");
        assert!(l.taken().is_empty());
        assert_eq!(l.render(&[]), raw);
    }

    #[test]
    fn multibyte_text_around_escapes_keeps_byte_offsets_consistent() {
        // "héllo " is 7 bytes; the link offsets are byte offsets into the
        // plain string and must slice cleanly around the escape.
        let raw = "héllo \x1b[32mwörld\x1b[0m a.rs";
        let l = AnsiLine::parse(raw);
        assert_eq!(l.plain(), "héllo wörld a.rs");
        let start = l.plain().find("a.rs").unwrap();
        let out = l.render(&[link(start, start + 4, "file:///a.rs")]);
        assert!(out.ends_with("\x1b]8;;file:///a.rs\x1b\\a.rs\x1b]8;;\x1b\\"));
        assert!(out.starts_with("héllo \x1b[32mwörld\x1b[0m "));
    }
}
