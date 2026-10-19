use diffpack_engine::{build_diff_tree_for_package, set_proxy_origin};
use serde::Deserialize;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const LIVE_PROXY_ORIGIN: Option<&str> = option_env!("DIFFPACK_PROXY_ORIGIN");

#[derive(Deserialize)]
struct DiffEntry {
    status: String,
    #[serde(default)]
    children: Option<Vec<DiffEntry>>,
}

fn has_a_change(entry: &DiffEntry) -> bool {
    entry.status != "unchanged" || entry.children.iter().flatten().any(has_a_change)
}

#[wasm_bindgen_test]
#[ignore = "needs a live proxy: set DIFFPACK_PROXY_ORIGIN and run with -- --include-ignored"]
async fn monolog_3_12_0_to_3_12_1_diffs_through_the_live_proxy() {
    let origin =
        LIVE_PROXY_ORIGIN.expect("set DIFFPACK_PROXY_ORIGIN to the proxy to compare through");
    set_proxy_origin(origin.to_string()).expect("DIFFPACK_PROXY_ORIGIN is a valid origin");

    let tree = build_diff_tree_for_package(
        "composer".to_string(),
        "monolog/monolog".to_string(),
        "3.12.0".to_string(),
        "3.12.1".to_string(),
        0.6,
        false,
    )
    .await
    .expect("the live comparison should succeed");

    let tree: DiffEntry =
        serde_wasm_bindgen::from_value(tree).expect("diff tree should deserialize");
    assert!(has_a_change(&tree), "3.12.0 and 3.12.1 differ");
}
