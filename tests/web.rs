use diffpack_engine::{build_diff_tree_for_package, get_diff_for_comparison, get_diff_for_path};
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

#[derive(Deserialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct FileDiff {
    data: String,
    is_diff: bool,
}

/// `src/lib.rs` as the active diff has it, whatever that is.
fn active_lib_rs() -> FileDiff {
    let value = get_diff_for_path("src/lib.rs".to_string(), None, false).expect("a diff is active");
    serde_wasm_bindgen::from_value(value).expect("file diff should deserialize")
}

fn lib_rs_in(from: &str, to: &str) -> Result<FileDiff, String> {
    get_diff_for_comparison(
        "crates".to_string(),
        "itoa".to_string(),
        from.to_string(),
        to.to_string(),
        "src/lib.rs".to_string(),
        None,
        false,
    )
    .map(|value| serde_wasm_bindgen::from_value(value).expect("file diff should deserialize"))
    .map_err(|error| error.as_string().unwrap_or_default())
}

/// Built second, 1.0.11..1.0.14 is the active diff; a read that names
/// 1.0.11..1.0.18 is still answered from those two versions, exactly as the
/// active diff answered it while 1.0.18 was the one loaded.
#[wasm_bindgen_test]
async fn a_read_that_names_its_comparison_ignores_the_active_diff() {
    diff_tree("itoa", "1.0.11", "1.0.18").await;
    let expected = active_lib_rs();

    diff_tree("itoa", "1.0.11", "1.0.14").await;
    assert_ne!(active_lib_rs(), expected, "the two comparisons must differ");

    assert_eq!(lib_rs_in("1.0.11", "1.0.18"), Ok(expected));
    // And for the comparison that is active, both reads agree.
    assert_eq!(lib_rs_in("1.0.11", "1.0.14"), Ok(active_lib_rs()));
}

/// A version nothing was built from is an error that names it, not an empty
/// diff.
#[wasm_bindgen_test]
fn a_read_of_a_comparison_never_built_is_an_error() {
    let error = lib_rs_in("0.0.1", "0.0.2").expect_err("nothing was built");
    assert!(error.contains("crates:itoa:0.0.1"), "{error}");
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
