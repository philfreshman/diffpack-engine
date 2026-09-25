mod core;
mod package;
mod patch;
mod types;

// The crate's native surface: the pieces a Rust dependent links against
// directly, without a browser or a `fetch` in front of them. `examples/bench.rs`
// drives extraction and the tree builder through it, and so does
// `diffpack-server`, which computes diffs with this crate as a Cargo
// dependency. Nothing here reaches JS — only the `#[wasm_bindgen]` functions
// below do. `tests/public_api.rs` links the crate the way a dependent does and
// is what holds this list in place.
pub use crate::core::{build_diff_tree, get_diff_content, whitespace_mode};
pub use crate::package::{
    archive_source, build_go_zip_url, build_tarball_url, choose_archive, escape_go_module_path,
    escape_go_version, extract_archive_bytes, select_pypi_sdist_url, strip_go_module_root,
    unpack_archive, ArchiveSource, PyPiResponse, PyPiUrl,
};
/// The per-file view — neither version, added, removed, byte-identical or
/// changed — that `get_diff_for_comparison` and `get_diff_for_path` render for
/// the browser.
///
/// Supported API: diffpack-server renders its file views through the same
/// function, so the two cannot drift apart on what a view shows.
pub use crate::patch::{build_patch, Patch};
pub use crate::types::{DiffFileEntry, DiffStatus, FileMapEntry, FileType};
use serde::Serialize;
/// `similar`'s own type, which [`whitespace_mode`] returns — so it is part of
/// this crate's surface whether or not it is named here.
///
/// Supported API, re-exported so a dependent takes the type from us rather than
/// from a `similar` of its own, where a version that resolved differently would
/// be a different type — the drift `whitespace_mode` exists to prevent.
pub use similar::WhitespaceMode;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

/// One extracted package, shared between the cache and whoever is reading it.
/// The cache keeps a package for the session, and a diff only reads it, so a
/// hit hands out another `Rc` rather than a copy of every file's content —
/// which was two 80 MB copies per diff on a large crate, in memory wasm
/// never returns (#159).
type PackageFiles = Rc<HashMap<String, FileMapEntry>>;

#[derive(Clone)]
struct ActiveDiff {
    from_key: String,
    to_key: String,
}

thread_local! {
    static EXTRACTION_CACHE: RefCell<HashMap<String, PackageFiles>> =
        RefCell::new(HashMap::new());
    static ACTIVE_DIFF: RefCell<Option<ActiveDiff>> = const { RefCell::new(None) };
}

fn cache_key(registry: &str, pkg: &str, version: &str) -> String {
    format!("{registry}:{pkg}:{version}")
}

async fn get_or_fetch_package(
    registry: &str,
    pkg: &str,
    version: &str,
) -> Result<PackageFiles, JsValue> {
    let key = cache_key(registry, pkg, version);
    if let Some(cached) = EXTRACTION_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Ok(cached);
    }

    let files = Rc::new(package::fetch_and_extract_package(registry, pkg, version).await?);
    EXTRACTION_CACHE.with(|cache| {
        cache.borrow_mut().insert(key, Rc::clone(&files));
    });
    Ok(files)
}

fn file_content<'a>(files: &'a HashMap<String, FileMapEntry>, path: &str) -> Option<&'a str> {
    files.get(path).and_then(|entry| match entry.file_type {
        FileType::File => Some(entry.content.as_str()),
        FileType::Directory => None,
    })
}

/// What the file-view functions hand JS: a [`Patch`] under the camelCase names
/// the app reads (`{ data, isDiff }`). Private, and only a rename — which file
/// view to show is [`build_patch`]'s decision, not this layer's.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiffResult {
    data: String,
    is_diff: bool,
}

impl From<Patch> for DiffResult {
    fn from(patch: Patch) -> Self {
        DiffResult {
            data: patch.data,
            is_diff: patch.is_diff,
        }
    }
}

/// One file's view between two cached versions, found by their cache keys and
/// rendered by [`build_patch`].
///
/// A version missing from the cache is an error rather than a version without
/// the file: nothing was built from it, so there is no answer to give, and
/// "not present in either version" would describe a package nobody looked at.
fn diff_from_cache(
    cache: &HashMap<String, PackageFiles>,
    from_key: &str,
    to_key: &str,
    filename: &str,
    old_path: Option<&str>,
    ignore_whitespace: bool,
) -> Result<Patch, String> {
    let loaded = |key: &str| {
        cache
            .get(key)
            .ok_or_else(|| format!("{key} has not been loaded; build its diff first"))
    };
    let from_files = loaded(from_key)?;
    let to_files = loaded(to_key)?;

    let from_content = file_content(from_files, old_path.unwrap_or(filename));
    let to_content = file_content(to_files, filename);

    Ok(build_patch(
        filename,
        from_content,
        to_content,
        ignore_whitespace,
    ))
}

#[wasm_bindgen]
pub async fn prefetch_package(
    registry: String,
    pkg: String,
    version: String,
) -> Result<(), JsValue> {
    let _ = get_or_fetch_package(&registry, &pkg, &version).await?;
    Ok(())
}

#[wasm_bindgen]
pub async fn build_diff_tree_for_package(
    registry: String,
    pkg: String,
    from: String,
    to: String,
    similarity_threshold: f64,
    ignore_whitespace: bool,
) -> Result<JsValue, JsValue> {
    let (from_files, to_files) = futures::join!(
        get_or_fetch_package(&registry, &pkg, &from),
        get_or_fetch_package(&registry, &pkg, &to)
    );
    let from_files = from_files?;
    let to_files = to_files?;
    let tree = core::build_diff_tree(
        &from_files,
        &to_files,
        similarity_threshold,
        ignore_whitespace,
    );

    let from_key = cache_key(&registry, &pkg, &from);
    let to_key = cache_key(&registry, &pkg, &to);
    ACTIVE_DIFF.with(|state| {
        *state.borrow_mut() = Some(ActiveDiff { from_key, to_key });
    });

    Ok(serde_wasm_bindgen::to_value(&tree)?)
}

#[wasm_bindgen]
pub fn get_diff_for_path(
    filename: String,
    old_path: Option<String>,
    ignore_whitespace: bool,
) -> Result<JsValue, JsValue> {
    let active = ACTIVE_DIFF
        .with(|state| state.borrow().clone())
        .ok_or_else(|| JsValue::from_str("No active diff context"))?;

    // The same read `get_diff_for_comparison` makes, over the pair the active
    // diff names, so the two cannot drift apart. A build sets the active diff
    // only once both versions are cached, and nothing evicts them, so the
    // not-loaded error cannot happen here.
    let result = EXTRACTION_CACHE
        .with(|cache| {
            diff_from_cache(
                &cache.borrow(),
                &active.from_key,
                &active.to_key,
                &filename,
                old_path.as_deref(),
                ignore_whitespace,
            )
        })
        .map_err(|message| JsValue::from_str(&message))?;
    Ok(serde_wasm_bindgen::to_value(&DiffResult::from(result))?)
}

/// One file's diff in the comparison it names, not in whichever comparison was
/// built last.
///
/// [`get_diff_for_path`] reads from the active diff, which a build replaces
/// when it *finishes*. With two builds in flight, the one that finishes last
/// wins, and a read for the other is answered from the wrong pair of versions.
/// Naming the comparison takes the active diff out of the read: both versions
/// are looked up in the extraction cache by their own keys, so builds can run
/// in any order and a read still gets its own files.
///
/// Both versions must have been built (or prefetched) first. One that is not
/// in the cache is an error, never an empty diff.
#[wasm_bindgen]
pub fn get_diff_for_comparison(
    registry: String,
    pkg: String,
    from: String,
    to: String,
    filename: String,
    old_path: Option<String>,
    ignore_whitespace: bool,
) -> Result<JsValue, JsValue> {
    let from_key = cache_key(&registry, &pkg, &from);
    let to_key = cache_key(&registry, &pkg, &to);

    let result = EXTRACTION_CACHE
        .with(|cache| {
            diff_from_cache(
                &cache.borrow(),
                &from_key,
                &to_key,
                &filename,
                old_path.as_deref(),
                ignore_whitespace,
            )
        })
        .map_err(|message| JsValue::from_str(&message))?;
    Ok(serde_wasm_bindgen::to_value(&DiffResult::from(result))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `#[wasm_bindgen]` entry points above are exercised by
    /// `tests/web.rs` under `wasm-pack test`: they need a `fetch` and a
    /// `JsValue`, neither of which exists on the host, where constructing a
    /// `JsValue` aborts the process. What is tested here is everything they
    /// are built out of.
    fn file(content: &str) -> FileMapEntry {
        FileMapEntry {
            file_type: FileType::File,
            content: content.to_string(),
        }
    }

    fn dir() -> FileMapEntry {
        FileMapEntry {
            file_type: FileType::Directory,
            content: String::new(),
        }
    }

    /// The cache is keyed per registry, package and version: two registries
    /// serving a package of the same name must not share an entry.
    #[test]
    fn a_cache_key_separates_registry_package_and_version() {
        assert_eq!(cache_key("npm", "left-pad", "1.3.0"), "npm:left-pad:1.3.0");
        assert_ne!(
            cache_key("npm", "requests", "2.0.0"),
            cache_key("pypi", "requests", "2.0.0")
        );
        assert_ne!(
            cache_key("crates", "serde", "1.0.0"),
            cache_key("crates", "serde", "1.0.1")
        );
    }

    /// Scoped npm names contain a slash, which must not be read as a
    /// separator when the key is compared.
    #[test]
    fn a_scoped_package_name_keys_on_its_whole_name() {
        assert_eq!(
            cache_key("npm", "@types/node", "20.1.0"),
            "npm:@types/node:20.1.0"
        );
    }

    #[test]
    fn only_a_file_has_content() {
        let files = HashMap::from([
            ("a.rs".to_string(), file("body")),
            ("src".to_string(), dir()),
        ]);
        assert_eq!(file_content(&files, "a.rs"), Some("body"));
        assert_eq!(file_content(&files, "src"), None);
        assert_eq!(file_content(&files, "missing.rs"), None);
    }

    /// The rename is the whole of this layer's say in a file view: the JS side
    /// reads `isDiff`, and the content passes through untouched.
    #[test]
    fn a_patch_reaches_js_under_camel_case_names() {
        let result = DiffResult::from(build_patch("a.rs", Some("one\n"), Some("two\n"), false));
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::json!({
                "data": "--- from/a.rs\n+++ to/a.rs\n- one\n+ two",
                "isDiff": true,
            })
        );
    }

    fn package(files: &[(&str, &str)]) -> PackageFiles {
        Rc::new(
            files
                .iter()
                .map(|(path, content)| (path.to_string(), file(content)))
                .collect(),
        )
    }

    /// Two comparisons of one crate in the cache at once, the way the app
    /// leaves them after switching between them.
    fn two_comparisons() -> HashMap<String, PackageFiles> {
        HashMap::from([
            (
                cache_key("crates", "itoa", "1.0.0"),
                package(&[("src/lib.rs", "one\n")]),
            ),
            (
                cache_key("crates", "itoa", "2.0.0"),
                package(&[("src/lib.rs", "two\n")]),
            ),
            (
                cache_key("crates", "itoa", "3.0.0"),
                package(&[("src/lib.rs", "three\n")]),
            ),
        ])
    }

    /// Whichever comparison was built last, a read names its own two versions
    /// and is answered from them.
    #[test]
    fn a_read_is_answered_from_the_versions_it_names() {
        let cache = two_comparisons();

        let first = diff_from_cache(
            &cache,
            &cache_key("crates", "itoa", "1.0.0"),
            &cache_key("crates", "itoa", "2.0.0"),
            "src/lib.rs",
            None,
            false,
        )
        .expect("both versions are loaded");
        let second = diff_from_cache(
            &cache,
            &cache_key("crates", "itoa", "1.0.0"),
            &cache_key("crates", "itoa", "3.0.0"),
            "src/lib.rs",
            None,
            false,
        )
        .expect("both versions are loaded");

        assert_eq!(
            first.data,
            "--- from/src/lib.rs\n+++ to/src/lib.rs\n- one\n+ two"
        );
        assert_eq!(
            second.data,
            "--- from/src/lib.rs\n+++ to/src/lib.rs\n- one\n+ three"
        );
    }

    /// A rename is read from its old path on the old side.
    #[test]
    fn a_renamed_file_is_read_from_its_old_path_in_the_old_version() {
        let cache = HashMap::from([
            (
                cache_key("npm", "left-pad", "1.0.0"),
                package(&[("index.js", "old\n")]),
            ),
            (
                cache_key("npm", "left-pad", "2.0.0"),
                package(&[("lib/index.js", "new\n")]),
            ),
        ]);

        let result = diff_from_cache(
            &cache,
            &cache_key("npm", "left-pad", "1.0.0"),
            &cache_key("npm", "left-pad", "2.0.0"),
            "lib/index.js",
            Some("index.js"),
            false,
        )
        .expect("both versions are loaded");

        assert_eq!(
            result.data,
            "--- from/lib/index.js\n+++ to/lib/index.js\n- old\n+ new"
        );
    }

    /// Both versions loaded, and the file in neither: that is an answer, not
    /// an error — the one a read of a version never built must not give.
    #[test]
    fn a_file_in_neither_loaded_version_is_not_present_not_an_error() {
        let cache = two_comparisons();

        let result = diff_from_cache(
            &cache,
            &cache_key("crates", "itoa", "1.0.0"),
            &cache_key("crates", "itoa", "2.0.0"),
            "gone.rs",
            None,
            false,
        )
        .expect("both versions are loaded");

        assert!(!result.is_diff);
        assert_eq!(result.data, "File not present in either version.");
    }

    /// The whitespace choice reaches the renderer: a reformat reads as
    /// removed and added lines exactly, and as context when ignored.
    #[test]
    fn a_read_passes_the_whitespace_choice_on() {
        let cache = HashMap::from([
            (
                cache_key("crates", "fmt", "1.0.0"),
                package(&[("src/lib.rs", "\tx=1;\n")]),
            ),
            (
                cache_key("crates", "fmt", "2.0.0"),
                package(&[("src/lib.rs", "    x = 1;\n")]),
            ),
        ]);
        let read = |ignore_whitespace| {
            diff_from_cache(
                &cache,
                &cache_key("crates", "fmt", "1.0.0"),
                &cache_key("crates", "fmt", "2.0.0"),
                "src/lib.rs",
                None,
                ignore_whitespace,
            )
            .expect("both versions are loaded")
            .data
        };

        assert_eq!(
            read(false),
            "--- from/src/lib.rs\n+++ to/src/lib.rs\n- \tx=1;\n+     x = 1;"
        );
        let ignoring = read(true);
        assert!(
            !ignoring
                .lines()
                .skip(2)
                .any(|line| line.starts_with('-') || line.starts_with('+')),
            "nothing should read as changed: {ignoring}"
        );
    }

    /// A version that was never built is not a version without the file: it
    /// is a read nothing can answer, and saying "not present" would be a lie.
    #[test]
    fn a_version_not_loaded_is_an_error_not_an_absent_file() {
        let cache = two_comparisons();
        let missing = cache_key("crates", "itoa", "9.9.9");

        let error = diff_from_cache(
            &cache,
            &cache_key("crates", "itoa", "1.0.0"),
            &missing,
            "src/lib.rs",
            None,
            false,
        )
        .expect_err("9.9.9 was never loaded");

        assert!(error.contains(&missing), "error names the version: {error}");
    }
}
