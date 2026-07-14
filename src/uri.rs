//! URI construction: RFC 3986 percent-encoding, `file://` URIs with a host,
//! and the editor link schemes (`--editor`) that carry line/column targets.

/// Percent-encode a filesystem path for use inside a URI. Keeps RFC 3986
/// unreserved characters plus `/` (path separators must survive); encodes
/// everything else byte-by-byte, including UTF-8 multibyte sequences.
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Build a `file://` URI. A non-empty `host` lets terminals distinguish
/// local links from links printed over SSH (GNU `ls --hyperlink` does the
/// same); an empty host yields the plain `file:///path` form.
pub fn file_uri(host: &str, abs_path: &str) -> String {
    format!("file://{}{}", host, encode_path(abs_path))
}

/// Where a clicked file link should open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Editor {
    /// `vscode://file/path:line:col` — also used by compatible forks via
    /// the scheme stored alongside (`vscode`, `vscode-insiders`, `cursor`).
    CodeFamily(&'static str),
    /// `zed://file/path:line:col`
    Zed,
    /// `idea://open?file=path&line=n&column=n` (JetBrains IDEs)
    Idea,
    /// `subl://open?url=file://path&line=n&column=n`
    Sublime,
    /// `txmt://open?url=file://path&line=n&column=n`
    TextMate,
    /// User template with `{path}`, `{line}`, `{col}` placeholders.
    Custom(String),
}

impl Editor {
    /// Parse the `--editor` argument: a known preset name, or a custom
    /// template containing `{path}`.
    pub fn from_arg(arg: &str) -> Result<Editor, String> {
        match arg {
            "vscode" => Ok(Editor::CodeFamily("vscode")),
            "vscode-insiders" => Ok(Editor::CodeFamily("vscode-insiders")),
            "cursor" => Ok(Editor::CodeFamily("cursor")),
            "zed" => Ok(Editor::Zed),
            "idea" | "jetbrains" => Ok(Editor::Idea),
            "subl" | "sublime" => Ok(Editor::Sublime),
            "txmt" | "textmate" => Ok(Editor::TextMate),
            other if other.contains("{path}") => Ok(Editor::Custom(other.to_string())),
            other => Err(format!(
                "unknown editor '{other}' (presets: vscode, vscode-insiders, cursor, zed, \
                 idea, subl, txmt; or a template containing {{path}})"
            )),
        }
    }

    /// Build the link target for an absolute path plus optional line/column.
    pub fn href(&self, abs_path: &str, line: Option<u32>, col: Option<u32>) -> String {
        let enc = encode_path(abs_path);
        match self {
            Editor::CodeFamily(scheme) => {
                let mut out = format!("{scheme}://file{enc}");
                if let Some(l) = line {
                    out.push_str(&format!(":{l}"));
                    if let Some(c) = col {
                        out.push_str(&format!(":{c}"));
                    }
                }
                out
            }
            Editor::Zed => {
                let mut out = format!("zed://file{enc}");
                if let Some(l) = line {
                    out.push_str(&format!(":{l}"));
                    if let Some(c) = col {
                        out.push_str(&format!(":{c}"));
                    }
                }
                out
            }
            Editor::Idea => {
                let mut out = format!("idea://open?file={enc}");
                if let Some(l) = line {
                    out.push_str(&format!("&line={l}"));
                    if let Some(c) = col {
                        out.push_str(&format!("&column={c}"));
                    }
                }
                out
            }
            Editor::Sublime => {
                let mut out = format!("subl://open?url=file://{enc}");
                if let Some(l) = line {
                    out.push_str(&format!("&line={l}"));
                    if let Some(c) = col {
                        out.push_str(&format!("&column={c}"));
                    }
                }
                out
            }
            Editor::TextMate => {
                let mut out = format!("txmt://open?url=file://{enc}");
                if let Some(l) = line {
                    out.push_str(&format!("&line={l}"));
                    if let Some(c) = col {
                        out.push_str(&format!("&column={c}"));
                    }
                }
                out
            }
            Editor::Custom(tpl) => tpl
                .replace("{path}", &enc)
                .replace("{line}", &line.unwrap_or(1).to_string())
                .replace("{col}", &col.unwrap_or(1).to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_path_keeps_unreserved_and_encodes_the_rest() {
        assert_eq!(encode_path("/a/b-c_d.e~f"), "/a/b-c_d.e~f");
        // `?` and `#` would be parsed as query/fragment separators by the
        // receiving application if they leaked through raw.
        assert_eq!(encode_path("/a b/%x?y#z"), "/a%20b/%25x%3Fy%23z");
        assert_eq!(encode_path("/répertoire"), "/r%C3%A9pertoire");
    }

    #[test]
    fn file_uri_carries_the_host() {
        assert_eq!(file_uri("devbox", "/tmp/a.rs"), "file://devbox/tmp/a.rs");
        assert_eq!(file_uri("", "/tmp/a.rs"), "file:///tmp/a.rs");
    }

    #[test]
    fn vscode_href_omits_missing_line_and_column() {
        let ed = Editor::from_arg("vscode").unwrap();
        assert_eq!(ed.href("/w/x.rs", None, None), "vscode://file/w/x.rs");
        assert_eq!(ed.href("/w/x.rs", Some(3), None), "vscode://file/w/x.rs:3");
        assert_eq!(
            ed.href("/w/x.rs", Some(3), Some(9)),
            "vscode://file/w/x.rs:3:9"
        );
    }

    #[test]
    fn idea_and_sublime_use_query_parameters() {
        let idea = Editor::from_arg("idea").unwrap();
        assert_eq!(
            idea.href("/w/x.kt", Some(12), Some(4)),
            "idea://open?file=/w/x.kt&line=12&column=4"
        );
        let subl = Editor::from_arg("subl").unwrap();
        assert_eq!(
            subl.href("/w/x.py", Some(7), None),
            "subl://open?url=file:///w/x.py&line=7"
        );
    }

    #[test]
    fn custom_template_expands_placeholders_with_defaults() {
        let ed = Editor::from_arg("myed://o{path}?l={line}&c={col}").unwrap();
        assert_eq!(
            ed.href("/w/a b.c", Some(5), None),
            "myed://o/w/a%20b.c?l=5&c=1"
        );
    }

    #[test]
    fn unknown_editor_name_is_rejected_with_the_preset_list() {
        let err = Editor::from_arg("nano").unwrap_err();
        assert!(err.contains("unknown editor 'nano'"));
        assert!(err.contains("vscode"));
    }
}
