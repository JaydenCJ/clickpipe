//! OSC 8 hyperlink emission.
//!
//! The wire format is `ESC ] 8 ; params ; URI ST` to open a link and
//! `ESC ] 8 ; ; ST` to close it, where ST is `ESC \`. clickpipe emits no
//! params and always terminates with ST (never BEL), which is what every
//! OSC 8 capable terminal accepts. Terminals without OSC 8 support ignore
//! the sequence entirely, so the output degrades to the unmodified text.

/// Closes the most recently opened hyperlink.
pub const CLOSE: &str = "\x1b]8;;\x1b\\";

/// Open a hyperlink to `uri`. The wrapped text follows, then [`CLOSE`].
pub fn open(uri: &str) -> String {
    format!("\x1b]8;;{}\x1b\\", sanitize(uri))
}

/// Make `uri` safe to embed in an OSC payload: the spec allows only
/// printable ASCII, so control bytes, spaces and non-ASCII bytes are
/// percent-encoded. Already-encoded input passes through unchanged because
/// `%` is printable ASCII.
pub fn sanitize(uri: &str) -> String {
    if uri.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return uri.to_string();
    }
    let mut out = String::with_capacity(uri.len() + 8);
    for b in uri.bytes() {
        if (0x21..=0x7e).contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_close_emit_the_documented_byte_sequences() {
        assert_eq!(
            open("https://example.test/a"),
            "\x1b]8;;https://example.test/a\x1b\\"
        );
        assert_eq!(CLOSE, "\x1b]8;;\x1b\\");
    }

    #[test]
    fn sanitize_percent_encodes_spaces_controls_and_non_ascii() {
        // A raw ESC or BEL inside the URI would terminate the OSC payload
        // early and corrupt the terminal state; it must never pass through.
        assert_eq!(sanitize("file:///a b\x1b\x07"), "file:///a%20b%1B%07");
        // UTF-8 is encoded byte-by-byte, matching RFC 3986.
        assert_eq!(
            sanitize("https://example.test/é"),
            "https://example.test/%C3%A9"
        );
    }

    #[test]
    fn sanitize_leaves_clean_uris_untouched() {
        let uri = "vscode://file/tmp/a.rs:3:7";
        assert_eq!(sanitize(uri), uri);
    }
}
