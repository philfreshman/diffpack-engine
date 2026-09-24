use crate::core::get_diff_content;
use serde::{Deserialize, Serialize};

/// What one file's view shows: either a unified diff (`is_diff`) or text the
/// viewer renders as it is — the file itself when both versions hold the same
/// bytes, or a sentence when neither holds it at all.
///
/// Supported API. The field names are plain Rust on purpose: this is the shape
/// diffpack-server stores in `patches.json`, so it can re-export this type in
/// place of its own without renaming a stored field. The browser's
/// `{ data, isDiff }` is a separate, private shape in `lib.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    pub data: String,
    pub is_diff: bool,
}

/// Renders one file's view from its content in each version, `None` where that
/// version has no file at the path.
///
/// Takes contents rather than file maps: what a version holds at a path —
/// including whether a directory there counts as nothing — is the caller's
/// lookup, not this function's. The browser and diffpack-server both render
/// through here, so the two cannot drift apart on the four cases:
///
/// - in neither version: a sentence, not a diff;
/// - added or removed: every line against `/dev/null`, split on `\n`, so a file
///   ending in `\n` gets an empty last `+`/`-` line;
/// - byte-identical: the file's own content, not a diff;
/// - changed: [`get_diff_content`].
pub fn build_patch(
    filename: &str,
    from: Option<&str>,
    to: Option<&str>,
    ignore_whitespace: bool,
) -> Patch {
    match (from, to) {
        (None, None) => Patch {
            data: "File not present in either version.".to_string(),
            is_diff: false,
        },
        (None, Some(to)) => {
            let header = format!("--- /dev/null\n+++ to/{filename}");
            let mut lines = Vec::new();
            lines.push(header);
            for line in to.split('\n') {
                lines.push(format!("+ {line}"));
            }
            Patch {
                data: lines.join("\n"),
                is_diff: true,
            }
        }
        (Some(from), None) => {
            let header = format!("--- from/{filename}\n+++ /dev/null");
            let mut lines = Vec::new();
            lines.push(header);
            for line in from.split('\n') {
                lines.push(format!("- {line}"));
            }
            Patch {
                data: lines.join("\n"),
                is_diff: true,
            }
        }
        (Some(from), Some(to)) => {
            if from == to {
                Patch {
                    data: to.to_string(),
                    is_diff: false,
                }
            } else {
                Patch {
                    data: get_diff_content(filename, from, to, ignore_whitespace),
                    is_diff: true,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path is in neither version — a stale link, or a file that only
    /// ever existed as a rename's source. The viewer gets a sentence, not a
    /// diff, so it does not try to render one.
    #[test]
    fn a_file_in_neither_version_is_not_a_diff() {
        let result = build_patch("gone.rs", None, None, false);
        assert!(!result.is_diff);
        assert_eq!(result.data, "File not present in either version.");
    }

    #[test]
    fn an_added_file_is_rendered_against_dev_null() {
        let result = build_patch("a.rs", None, Some("one\ntwo"), false);
        assert!(result.is_diff);
        assert_eq!(result.data, "--- /dev/null\n+++ to/a.rs\n+ one\n+ two");
    }

    #[test]
    fn a_removed_file_is_rendered_against_dev_null() {
        let result = build_patch("a.rs", Some("one\ntwo"), None, false);
        assert!(result.is_diff);
        assert_eq!(result.data, "--- from/a.rs\n+++ /dev/null\n- one\n- two");
    }

    /// Byte-identical: the file itself, marked as not a diff, so the viewer
    /// renders it as a file rather than as a hunk of all-context lines.
    #[test]
    fn an_unchanged_file_is_returned_as_its_own_content() {
        let result = build_patch("a.rs", Some("same\n"), Some("same\n"), false);
        assert!(!result.is_diff);
        assert_eq!(result.data, "same\n");
    }

    #[test]
    fn a_changed_file_is_rendered_as_a_diff() {
        let result = build_patch("a.rs", Some("one\n"), Some("two\n"), false);
        assert!(result.is_diff);
        assert_eq!(result.data, "--- from/a.rs\n+++ to/a.rs\n- one\n+ two");
    }

    /// Whitespace-only changes still reach the diff renderer — the file is
    /// not byte-identical — but in `ignore_whitespace` mode every line comes
    /// back as context.
    #[test]
    fn a_reformat_is_all_context_when_whitespace_is_ignored() {
        let from = "fn main() {\n\tlet x=1;\n}\n";
        let to = "fn main() {\n    let x = 1;\n}\n";

        let ignoring = build_patch("a.rs", Some(from), Some(to), true);
        assert!(ignoring.is_diff);
        assert!(
            !ignoring
                .data
                .lines()
                .any(|line| line.starts_with('-') && !line.starts_with("---")),
            "no line should read as removed: {}",
            ignoring.data
        );

        let exact = build_patch("a.rs", Some(from), Some(to), false);
        assert!(exact.data.contains("- \tlet x=1;"));
    }

    /// A one-line file has no trailing newline to split on; the renderer must
    /// still produce a header and exactly one line.
    #[test]
    fn a_file_without_a_trailing_newline_renders_one_line() {
        let result = build_patch("a.rs", None, Some("only"), false);
        assert_eq!(result.data, "--- /dev/null\n+++ to/a.rs\n+ only");
    }
}
