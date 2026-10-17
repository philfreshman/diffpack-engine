use diffpack_engine::{
    archive_source, build_go_zip_url, build_patch, build_tarball_url, choose_archive,
    escape_go_module_path, escape_go_version, get_diff_content, select_pypi_sdist_url,
    strip_go_module_root, unpack_archive, whitespace_mode, ArchiveSource, FileMapEntry, FileType,
    Patch, PyPiResponse, PyPiUrl, WhitespaceMode,
};
use std::collections::HashMap;
use std::io::{Cursor, Write};

fn file(content: &str) -> FileMapEntry {
    FileMapEntry {
        file_type: FileType::File,
        content: content.to_string(),
    }
}

#[test]
fn the_unified_diff_format_is_reachable_and_unchanged() {
    assert_eq!(
        get_diff_content("src/lib.rs", "one\ntwo\nthree\n", "one\n2\nthree\n", false),
        "--- from/src/lib.rs\n+++ to/src/lib.rs\n  one\n- two\n+ 2\n  three"
    );
}

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

#[test]
fn the_whitespace_choice_is_handed_over_as_a_value() {
    assert_eq!(whitespace_mode(true), WhitespaceMode::IgnoreAll);
    assert_eq!(whitespace_mode(false), WhitespaceMode::Exact);
}

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

#[test]
fn a_go_version_is_escaped_into_a_proxy_zip_url_too() {
    let url = "https://proxy.golang.org/github.com/!masterminds/semver/@v/v1.0.0-!r!c1.zip";

    assert_eq!(escape_go_version("v1.0.0-RC1"), "v1.0.0-!r!c1");
    assert_eq!(
        build_go_zip_url("github.com/Masterminds/semver", "v1.0.0-RC1"),
        url
    );
    assert_eq!(
        archive_source("go", "github.com/Masterminds/semver", "v1.0.0-RC1").unwrap(),
        ArchiveSource::Archive {
            url: url.to_string()
        }
    );
}

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

#[test]
fn a_go_module_zip_unpacks_to_paths_without_the_version() {
    let files = unpack_archive("go", "github.com/x/y", "v1.2.3", &go_module_zip()).unwrap();

    let mut paths: Vec<&str> = files.keys().map(String::as_str).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["go.mod", "src", "src/lib.go"]);
    assert_eq!(files["src/lib.go"].content, "package y\n");
    assert!(matches!(files["src"].file_type, FileType::Directory));
}
