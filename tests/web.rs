use diffpack_engine::{build_diff_tree_for_package, get_diff_for_path};
use serde::Deserialize;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DiffEntry {
    status: String,
    #[serde(default)]
    children: Option<Vec<DiffEntry>>,
}

fn all_statuses(entry: &DiffEntry, out: &mut Vec<String>) {
    out.push(entry.status.clone());
    if let Some(children) = &entry.children {
        for child in children {
            all_statuses(child, out);
        }
    }
}

async fn diff_tree(pkg: &str, from: &str, to: &str) -> DiffEntry {
    let value = build_diff_tree_for_package(
        "crates".to_string(),
        pkg.to_string(),
        from.to_string(),
        to.to_string(),
        0.6,
        false,
    )
    .await
    .expect("diff should succeed");
    serde_wasm_bindgen::from_value(value).expect("diff tree should deserialize")
}

#[wasm_bindgen_test]
async fn diffing_two_distinct_versions_finds_real_changes() {
    let tree = diff_tree("itoa", "1.0.11", "1.0.18").await;

    let mut statuses = Vec::new();
    all_statuses(&tree, &mut statuses);

    assert!(
        statuses.iter().any(|status| status != "unchanged"),
        "expected at least one changed file between 1.0.11 and 1.0.18, got {statuses:?}"
    );
}

/// Both sides of `build_diff_tree_for_package` request the same cache key when a
/// version is diffed against itself. Concurrently joining the two fetches must not
/// panic on the `EXTRACTION_CACHE` `RefCell`, and the result must show no changes.
#[wasm_bindgen_test]
async fn diffing_a_version_against_itself_reports_no_changes() {
    let tree = diff_tree("itoa", "1.0.18", "1.0.18").await;

    let mut statuses = Vec::new();
    all_statuses(&tree, &mut statuses);

    assert!(
        statuses.iter().all(|status| status == "unchanged"),
        "expected every entry unchanged when diffing a version against itself, got {statuses:?}"
    );
}

/// `get_diff_for_path` renders through `build_patch` but still hands JS the
/// camelCase `{ data, isDiff }` the app reads, not `Patch`'s own `is_diff`.
#[wasm_bindgen_test]
async fn a_file_view_reaches_js_as_data_and_is_diff() {
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FileView {
        data: String,
        is_diff: bool,
    }

    diff_tree("itoa", "1.0.18", "1.0.18").await;

    let value = get_diff_for_path("Cargo.toml".to_string(), None, false)
        .expect("the active diff should have a view for Cargo.toml");
    let view: FileView =
        serde_wasm_bindgen::from_value(value).expect("the view should be { data, isDiff }");

    assert!(!view.is_diff, "a version against itself is byte-identical");
    assert!(view.data.contains("name = \"itoa\""), "got {view:?}");
}
