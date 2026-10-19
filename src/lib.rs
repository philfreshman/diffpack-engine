mod core;
mod fetch_failure;
mod package;
mod patch;
mod store;
mod types;

pub use crate::core::{build_diff_tree, get_diff_content, whitespace_mode};
pub use crate::package::{
    archive_source, build_go_zip_url, build_tarball_url, choose_archive, escape_go_module_path,
    escape_go_version, extract_archive_bytes, select_pypi_sdist_url, strip_go_module_root,
    unpack_archive, ArchiveSource, PyPiResponse, PyPiUrl,
};
pub use crate::patch::{build_patch, Patch};
use crate::store::{PackageFiles, PackageStore};
pub use crate::types::{DiffFileEntry, DiffStatus, FileMapEntry, FileType};
use serde::Serialize;
pub use similar::WhitespaceMode;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[derive(Clone)]
struct ActiveDiff {
    from_key: String,
    to_key: String,
}

thread_local! {
    static PACKAGES: Rc<PackageStore<JsValue>> = PackageStore::new();
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
    let (registry, pkg, version) = (registry.to_owned(), pkg.to_owned(), version.to_owned());
    let fetch = async move { package::fetch_and_extract_package(&registry, &pkg, &version).await };
    let packages = PACKAGES.with(Rc::clone);
    packages.get_or_fetch(key, fetch).await
}

fn file_content<'a>(files: &'a HashMap<String, FileMapEntry>, path: &str) -> Option<&'a str> {
    files.get(path).and_then(|entry| match entry.file_type {
        FileType::File => Some(entry.content.as_str()),
        FileType::Directory => None,
    })
}

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

    let result = PACKAGES
        .with(|packages| {
            diff_from_cache(
                &packages.extracted(),
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

    let result = PACKAGES
        .with(|packages| {
            diff_from_cache(
                &packages.extracted(),
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
