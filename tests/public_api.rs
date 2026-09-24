//! The crate's surface as a dependent sees it.
//!
//! An integration test is its own crate: it links `diffpack-engine` the way
//! `diffpack-server` does and can reach nothing that `lib.rs` has not made
//! public. So the `use` below is half the assertion — an item that stops being
//! exported fails the build here rather than at the far end of a version bump.
//! The calls are the other half, and say what `use` alone cannot: that a
//! signature is usable from outside. `PyPiUrl` and `FileMapEntry` both have to
//! be constructible, or their functions are reachable in name only.
//!
//! `src/`'s own unit tests already cover what these functions decide, so what
//! is asserted here is that reachability and the bytes a second implementation
//! would have to match — not every branch a second time.

use diffpack_engine::{
    archive_source, build_go_zip_url, build_patch, build_tarball_url, choose_archive,
    escape_go_module_path, get_diff_content, select_pypi_sdist_url, strip_go_module_root,
    unpack_archive, whitespace_mode, ArchiveSource, FileMapEntry, FileType, Patch, PyPiResponse,
    PyPiUrl, WhitespaceMode,
};
use std::collections::HashMap;
use std::io::{Cursor, Write};

fn file(content: &str) -> FileMapEntry {
    FileMapEntry {
        file_type: FileType::File,
        content: content.to_string(),
    }
}

// ---- the unified diff format ------------------------------------------

/// The one item whose *output* is the contract rather than just its signature:
/// diffpack-server renders file views from these bytes while the tree's counts
/// come from the same lines, so a byte that moves here makes the two disagree
/// about the same file. Written out literally rather than assembled, so it
/// disagrees with the renderer if either side moves.
#[test]
fn the_unified_diff_format_is_reachable_and_unchanged() {
    assert_eq!(
        get_diff_content("src/lib.rs", "one\ntwo\nthree\n", "one\n2\nthree\n", false),
        "--- from/src/lib.rs\n+++ to/src/lib.rs\n  one\n- two\n+ 2\n  three"
    );
}

/// The whitespace choice as its consequence rather than as a mapping: the same
/// reformat read both ways.
#[test]
fn ignoring_whitespace_turns_a_reformat_into_context() {
    let from = "fn main() {\n\tlet x=1;\n}\n";
    let to = "fn main() {\n    let x = 1;\n}\n";

    assert_eq!(
        get_diff_content("a.rs", from, to, true),
        "--- from/a.rs\n+++ to/a.rs\n  fn main() {\n      let x = 1;\n  }"
    );
    assert_eq!(
        get_diff_content("a.rs", from, to, false),
        "--- from/a.rs\n+++ to/a.rs\n  fn main() {\n- \tlet x=1;\n+     let x = 1;\n  }"
    );
}

// ---- one file's view ---------------------------------------------------

/// The four cases `build_patch` decides for itself, written out literally. The
/// fifth, a changed file, is `get_diff_content`'s output and is pinned above.
///
/// The added and removed files end in `\n` on purpose: each line is split on
/// `\n`, so both get an empty last `+`/`-` line that the tree's counts do not
/// see. That is today's output, and it is pinned as such — changing it is a
/// change to make once, here, with every renderer calling this one function.
#[test]
fn a_file_view_is_reachable_and_unchanged_in_every_case() {
    assert_eq!(
        build_patch("gone.rs", None, None, false),
        Patch {
            data: "File not present in either version.".to_string(),
            is_diff: false,
        }
    );
    assert_eq!(
        build_patch("src/new.rs", None, Some("one\ntwo\n"), false),
        Patch {
            data: "--- /dev/null\n+++ to/src/new.rs\n+ one\n+ two\n+ ".to_string(),
            is_diff: true,
        }
    );
    assert_eq!(
        build_patch("src/old.rs", Some("one\ntwo\n"), None, false),
        Patch {
            data: "--- from/src/old.rs\n+++ /dev/null\n- one\n- two\n- ".to_string(),
            is_diff: true,
        }
    );
    assert_eq!(
        build_patch("src/lib.rs", Some("same\n"), Some("same\n"), false),
        Patch {
            data: "same\n".to_string(),
            is_diff: false,
        }
    );
}

/// The shape diffpack-server stores in `patches.json`, snake_case and all. It
/// re-exports this type in place of its own, so a renamed field would be a
/// stored file it can no longer read.
#[test]
fn a_patch_serialises_under_its_rust_field_names() {
    let patch = Patch {
        data: "--- /dev/null\n+++ to/a.rs\n+ only".to_string(),
        is_diff: true,
    };
    let json = serde_json::json!({
        "data": "--- /dev/null\n+++ to/a.rs\n+ only",
        "is_diff": true,
    });

    assert_eq!(serde_json::to_value(&patch).unwrap(), json);
    assert_eq!(serde_json::from_value::<Patch>(json).unwrap(), patch);
}

// ---- the whitespace choice --------------------------------------------

/// The same choice handed over as a value, for a caller configuring its own
/// `TextDiff` rather than going through `get_diff_content`.
#[test]
fn the_whitespace_choice_is_handed_over_as_a_value() {
    assert_eq!(whitespace_mode(true), WhitespaceMode::IgnoreAll);
    assert_eq!(whitespace_mode(false), WhitespaceMode::Exact);
}

// ---- URL construction --------------------------------------------------

#[test]
fn archive_urls_are_built_for_npm_and_crates_io() {
    assert_eq!(
        build_tarball_url("npm", "left-pad", "1.3.0").unwrap(),
        "https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz"
    );
    assert_eq!(
        build_tarball_url("npm", "@types/node", "20.1.0").unwrap(),
        "https://registry.npmjs.org/@types/node/-/node-20.1.0.tgz"
    );
    assert_eq!(
        build_tarball_url("crates", "serde", "1.0.200").unwrap(),
        "https://static.crates.io/crates/serde/serde-1.0.200.crate"
    );
    assert_eq!(
        build_tarball_url("maven", "guava", "33.0.0").unwrap_err(),
        "Unsupported registry: maven"
    );
}

// ---- PyPI artifact selection -------------------------------------------

/// PyPI serves no archive at a predictable path, so the reachable seam is the
/// whole hop: parse the metadata payload, then pick the artifact. The JSON is
/// the shape `pypi.org/pypi/{pkg}/{version}/json` returns, trimmed to the
/// fields that decide the answer.
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
        "https://files/x-1.0.tar.gz"
    );
}

/// Built by hand rather than parsed, which is what holds the field names in
/// place: a dependent modelling its own metadata still has to be able to
/// construct the argument.
#[test]
fn a_wheel_is_taken_only_when_there_is_no_sdist() {
    let wheel = PyPiUrl {
        packagetype: "bdist_wheel".to_string(),
        url: "https://files/x-1.0-py3-none-any.whl".to_string(),
    };

    assert_eq!(
        select_pypi_sdist_url(std::slice::from_ref(&wheel)).unwrap(),
        "https://files/x-1.0-py3-none-any.whl"
    );
    assert_eq!(
        select_pypi_sdist_url(&[]).unwrap_err(),
        "No downloadable artifacts found for PyPI package"
    );
}

// ---- the Go module proxy -----------------------------------------------

#[test]
fn a_go_module_path_is_escaped_into_a_proxy_zip_url() {
    assert_eq!(
        escape_go_module_path("github.com/Masterminds/semver"),
        "github.com/!masterminds/semver"
    );
    assert_eq!(
        build_go_zip_url("github.com/Masterminds/semver", "v3.2.1"),
        "https://proxy.golang.org/github.com/!masterminds/semver/@v/v3.2.1.zip"
    );
}

/// The intermediate directory is put back: the tree builder needs the
/// directories a path implies, and `src/` is only implied once the versioned
/// prefix is gone.
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

// ---- the archive lookup --------------------------------------------------

/// Matched on rather than compared, which is what holds the variants and their
/// field in place: a dependent re-exporting the enum has to be able to take
/// it apart.
#[test]
fn an_archive_source_says_whether_to_fetch_the_archive_or_a_listing() {
    match archive_source("crates", "serde", "1.0.200").unwrap() {
        ArchiveSource::Archive { url } => assert_eq!(
            url,
            "https://static.crates.io/crates/serde/serde-1.0.200.crate"
        ),
        other => panic!("expected an archive, got {other:?}"),
    }
    match archive_source("pypi", "requests", "2.32.3").unwrap() {
        ArchiveSource::Listing { url } => {
            assert_eq!(url, "https://pypi.org/pypi/requests/2.32.3/json")
        }
        other => panic!("expected a listing, got {other:?}"),
    }
    assert_eq!(
        archive_source("maven", "guava", "33.0.0").unwrap_err(),
        "Unsupported registry: maven"
    );
}

#[test]
fn an_archive_url_is_chosen_out_of_a_pypi_listing() {
    let listing = r#"{
        "urls": [
            {"packagetype": "bdist_wheel", "url": "https://files/x-1.0-py3-none-any.whl"},
            {"packagetype": "sdist", "url": "https://files/x-1.0.tar.gz"}
        ]
    }"#;

    assert_eq!(
        choose_archive("pypi", listing).unwrap(),
        "https://files/x-1.0.tar.gz"
    );
    assert!(choose_archive("crates", listing).is_err());
}

/// A module zip exactly as the proxy lays it out: every entry under
/// `<module>@<version>/`, with no directory entries of its own.
fn go_module_zip() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, content) in [
        ("github.com/x/y@v1.2.3/go.mod", "module github.com/x/y\n"),
        ("github.com/x/y@v1.2.3/src/lib.go", "package y\n"),
    ] {
        writer.start_file(path, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// The whole Go path from outside the crate, from the zip's bytes. Composing
/// the public helpers by hand leaves `y@v1.2.3/` on every path, which a diff
/// reads as every file removed and added again.
#[test]
fn a_go_module_zip_unpacks_to_paths_without_the_version() {
    let files = unpack_archive("go", "github.com/x/y", "v1.2.3", &go_module_zip()).unwrap();

    let mut paths: Vec<&str> = files.keys().map(String::as_str).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["go.mod", "src", "src/lib.go"]);
    assert_eq!(files["src/lib.go"].content, "package y\n");
    assert!(matches!(files["src"].file_type, FileType::Directory));
}
