//! The crate's surface as a dependent sees it.
//!
//! An integration test is its own crate: it links `diffpack-engine` the way
//! `diffpack-server` does and can reach nothing that `lib.rs` has not made
//! public. So a `use` here is the assertion — if an item stops being exported,
//! or its signature moves out from under a caller, this file fails to compile
//! rather than failing at the far end of a version bump.
//!
//! `src/`'s own unit tests already cover what these functions decide. What is
//! covered here is that the decision is reachable from outside, and that the
//! bytes a second implementation would have to match are the bytes that come
//! out.

use diffpack_engine::{
    build_go_zip_url, build_tarball_url, escape_go_module_path, get_diff_content,
    select_pypi_sdist_url, strip_go_module_root, whitespace_mode, FileMapEntry, FileType,
    PyPiResponse, PyPiUrl, WhitespaceMode,
};
use std::collections::HashMap;

/// A file entry as the extractor produces one, for the Go path tests below.
fn file(content: &str) -> FileMapEntry {
    FileMapEntry {
        file_type: FileType::File,
        content: content.to_string(),
    }
}

/// The exact format diffpack-server must reproduce: a `--- from/{f}` /
/// `+++ to/{f}` header, then one line per change as sign, space, content.
/// Written out literally rather than assembled, so it disagrees with the
/// renderer if either side moves.
#[test]
fn the_unified_diff_format_is_reachable_and_unchanged() {
    let from = "one\ntwo\nthree\n";
    let to = "one\n2\nthree\n";

    assert_eq!(
        get_diff_content("src/lib.rs", from, to, false),
        concat!(
            "--- from/src/lib.rs\n",
            "+++ to/src/lib.rs\n",
            "  one\n",
            "- two\n",
            "+ 2\n",
            "  three\n",
        )
        .trim_end_matches('\n'),
    );
}

/// `whitespace_mode` is what keeps the two implementations from drifting on
/// what `-w` means, so the test is the consequence, not the mapping: the same
/// reformat read both ways. `Exact` sees a line rewritten; `IgnoreAll` sees
/// the same line.
#[test]
fn ignoring_whitespace_turns_a_reformat_into_context() {
    let from = "fn main() {\n\tlet x=1;\n}\n";
    let to = "fn main() {\n    let x = 1;\n}\n";

    assert_eq!(
        get_diff_content("a.rs", from, to, true),
        concat!(
            "--- from/a.rs\n",
            "+++ to/a.rs\n",
            "  fn main() {\n",
            "      let x = 1;\n",
            "  }",
        ),
    );
    assert_eq!(
        get_diff_content("a.rs", from, to, false),
        concat!(
            "--- from/a.rs\n",
            "+++ to/a.rs\n",
            "  fn main() {\n",
            "- \tlet x=1;\n",
            "+     let x = 1;\n",
            "  }",
        ),
    );
}

/// The same choice, handed over as a value, for a caller configuring its own
/// `TextDiff`. `IgnoreAll` is Git's `-w`; `Exact` is its default.
#[test]
fn the_whitespace_choice_is_handed_over_as_a_value() {
    assert_eq!(whitespace_mode(true), WhitespaceMode::IgnoreAll);
    assert_eq!(whitespace_mode(false), WhitespaceMode::Exact);
}

/// npm's tarball path repeats the name unscoped after `/-/`, which is the part
/// a second implementation gets wrong. crates.io repeats it in full. Both URLs
/// are written out as the registries serve them.
#[test]
fn archive_urls_are_built_for_npm_and_crates_io() {
    assert_eq!(
        build_tarball_url("npm", "left-pad", "1.3.0").unwrap(),
        "https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz",
    );
    assert_eq!(
        build_tarball_url("npm", "@types/node", "20.1.0").unwrap(),
        "https://registry.npmjs.org/@types/node/-/node-20.1.0.tgz",
    );
    assert_eq!(
        build_tarball_url("crates", "serde", "1.0.200").unwrap(),
        "https://static.crates.io/crates/serde/serde-1.0.200.crate",
    );
}

/// A registry this crate does not serve is an error a caller can render, not a
/// panic and not a URL that 404s later.
#[test]
fn an_unsupported_registry_is_an_error() {
    assert_eq!(
        build_tarball_url("maven", "guava", "33.0.0").unwrap_err(),
        "Unsupported registry: maven",
    );
}

/// PyPI serves no archive at a predictable path: the URL comes out of the
/// metadata endpoint, so the seam is the whole hop — parse the payload, pick
/// the artifact. The JSON here is the shape `pypi.org/pypi/{pkg}/{v}/json`
/// returns, trimmed to the fields that decide the answer.
#[test]
fn a_pypi_payload_parses_and_yields_the_sdist() {
    let payload = r#"{
        "urls": [
            {"packagetype": "bdist_wheel", "url": "https://files/x-1.0-py3-none-any.whl"},
            {"packagetype": "sdist", "url": "https://files/x-1.0.tar.gz"}
        ]
    }"#;

    let metadata: PyPiResponse = serde_json::from_str(payload).unwrap();

    assert_eq!(
        select_pypi_sdist_url(&metadata.urls).unwrap(),
        "https://files/x-1.0.tar.gz",
    );
}

/// The preference order the issue calls real logic: a source distribution over
/// a wheel, but a wheel over nothing. Built by hand rather than parsed, which
/// is also what holds the field names in place.
#[test]
fn a_wheel_is_taken_only_when_there_is_no_sdist() {
    let wheel = PyPiUrl {
        packagetype: "bdist_wheel".to_string(),
        url: "https://files/x-1.0-py3-none-any.whl".to_string(),
    };

    assert_eq!(
        select_pypi_sdist_url(std::slice::from_ref(&wheel)).unwrap(),
        "https://files/x-1.0-py3-none-any.whl",
    );
    assert_eq!(
        select_pypi_sdist_url(&[]).unwrap_err(),
        "No downloadable artifacts found for PyPI package",
    );
}

/// The Go module proxy serves lower-cased paths, each uppercase letter written
/// as `!` plus its lowercase form. Requesting the module's real casing is a
/// 404, which is the whole reason this is not `to_lowercase`.
#[test]
fn a_go_module_path_is_escaped_for_the_proxy() {
    assert_eq!(
        escape_go_module_path("github.com/Masterminds/semver"),
        "github.com/!masterminds/semver",
    );
    assert_eq!(
        escape_go_module_path("github.com/sirupsen/logrus"),
        "github.com/sirupsen/logrus",
    );
}

/// The escaped path, under the proxy, at `@v/{version}.zip`.
#[test]
fn a_go_zip_url_is_built_from_the_escaped_path() {
    assert_eq!(
        build_go_zip_url("github.com/Masterminds/semver", "v3.2.1"),
        "https://proxy.golang.org/github.com/!masterminds/semver/@v/v3.2.1.zip",
    );
}

/// A module zip prefixes every entry with `<module>@<version>/`. That prefix
/// embeds the version, so left on, the two sides of a diff share no path at all
/// and every file reads as removed-then-added. Stripping it is the difference
/// between a diff and a rewrite, which is why the server cannot skip it.
///
/// The intermediate directory is put back: the tree builder needs the
/// directories a path implies, and `src/` is only implied once the prefix is
/// gone.
#[test]
fn the_versioned_module_root_is_stripped_from_every_path() {
    let files = HashMap::from([
        (
            "github.com/x/y@v1.2.3/go.mod".to_string(),
            file("module github.com/x/y\n"),
        ),
        (
            "github.com/x/y@v1.2.3/src/lib.go".to_string(),
            file("package y\n"),
        ),
    ]);

    let stripped = strip_go_module_root(files, "github.com/x/y", "v1.2.3");

    let mut paths: Vec<&str> = stripped.keys().map(String::as_str).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["go.mod", "src", "src/lib.go"]);
    assert_eq!(stripped["src/lib.go"].content, "package y\n");
    assert!(matches!(stripped["src"].file_type, FileType::Directory));
}
