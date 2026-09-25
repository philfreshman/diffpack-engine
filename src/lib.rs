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
/// changed — that `get_diff_for_path` renders for the browser.
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

/// What `get_diff_for_path` hands JS: a [`Patch`] under the camelCase names
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
    let from_key = active.from_key;
    let to_key = active.to_key;

    // Two `Rc` clones let go of the cache's `RefCell` borrow; the file
    // contents themselves are read in place, not copied out first.
    let (from_files, to_files) = EXTRACTION_CACHE.with(|cache| {
        let cache = cache.borrow();
        (cache.get(&from_key).cloned(), cache.get(&to_key).cloned())
    });

    let from_path = old_path.as_deref().unwrap_or(&filename);
    let from_content = from_files
        .as_deref()
        .and_then(|files| file_content(files, from_path));
    let to_content = to_files
        .as_deref()
        .and_then(|files| file_content(files, &filename));

    let result = DiffResult::from(build_patch(
        &filename,
        from_content,
        to_content,
        ignore_whitespace,
    ));
    Ok(serde_wasm_bindgen::to_value(&result)?)
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
}
