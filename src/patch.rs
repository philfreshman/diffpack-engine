use crate::core::get_diff_content;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    pub data: String,
    pub is_diff: bool,
}

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

    #[test]
    fn a_file_without_a_trailing_newline_renders_one_line() {
        let result = build_patch("a.rs", None, Some("only"), false);
        assert_eq!(result.data, "--- /dev/null\n+++ to/a.rs\n+ only");
    }
}
