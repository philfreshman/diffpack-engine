use diffpack_engine::{
    build_diff_tree_for_package, get_diff_for_comparison, get_diff_for_path, prefetch_package,
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;
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

#[wasm_bindgen_test]
async fn a_read_that_names_its_comparison_ignores_the_active_diff() {
    diff_tree("itoa", "1.0.11", "1.0.18").await;
    let expected = active_lib_rs();

    diff_tree("itoa", "1.0.11", "1.0.14").await;
    assert_ne!(active_lib_rs(), expected, "the two comparisons must differ");

    assert_eq!(lib_rs_in("1.0.11", "1.0.18"), Ok(expected));
    assert_eq!(lib_rs_in("1.0.11", "1.0.14"), Ok(active_lib_rs()));
}

#[wasm_bindgen_test]
fn a_read_of_a_comparison_never_built_is_an_error() {
    let error = lib_rs_in("0.0.1", "0.0.2").expect_err("nothing was built");
    assert!(error.contains("crates:itoa:0.0.1"), "{error}");
}

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

fn downloads_of(archive_url: &str) -> u32 {
    let entries = js_sys::eval(&format!(
        "performance.getEntriesByName({archive_url:?}).length"
    ))
    .expect("resource timing is available");
    entries.as_f64().expect("a count") as u32
}

#[wasm_bindgen_test]
async fn concurrent_builds_and_a_prefetch_of_a_shared_version_download_it_once() {
    let build = |ignore_whitespace| {
        build_diff_tree_for_package(
            "crates".to_string(),
            "itoa".to_string(),
            "1.0.9".to_string(),
            "1.0.10".to_string(),
            0.6,
            ignore_whitespace,
        )
    };

    let (exact, ignoring_whitespace, prefetch) = futures::join!(
        build(false),
        build(true),
        prefetch_package(
            "crates".to_string(),
            "itoa".to_string(),
            "1.0.10".to_string()
        ),
    );

    exact.expect("the exact build should succeed");
    ignoring_whitespace.expect("the whitespace-ignoring build should succeed");
    prefetch.expect("the prefetch should succeed");
    assert!(lib_rs_in("1.0.9", "1.0.10").is_ok());
    assert_eq!(
        downloads_of("https://static.crates.io/crates/itoa/itoa-1.0.10.crate"),
        1
    );
}

#[wasm_bindgen(inline_js = r#"
let realFetch;

function replaceFetch(stub) {
    realFetch ??= globalThis.fetch;
    globalThis.fetch = stub;
}

export function answer_every_fetch_with(status, body) {
    replaceFetch(async () => new Response(body, { status }));
}

export function answer_every_fetch_as_the_proxy(status, code, body) {
    replaceFetch(async () => new Response(body, {
        status,
        headers: { "Diffpack-Proxy-Error": code, "Content-Type": "application/json" },
    }));
}

export function answer_every_fetch_with_a_body_that_breaks_off() {
    replaceFetch(async () => {
        const brokenBody = new ReadableStream({
            start(controller) {
                controller.error(new TypeError("network error"));
            },
        });
        return new Response(brokenBody, { status: 200 });
    });
}

export function reject_every_fetch() {
    replaceFetch(async () => {
        throw new TypeError("Failed to fetch");
    });
}

export function restore_fetch() {
    globalThis.fetch = realFetch;
}
"#)]
extern "C" {
    fn answer_every_fetch_with(status: u16, body: &str);
    fn answer_every_fetch_as_the_proxy(status: u16, code: &str, body: &str);
    fn answer_every_fetch_with_a_body_that_breaks_off();
    fn reject_every_fetch();
    fn restore_fetch();
}

const UNPUBLISHED_ITOA_DOWNLOAD: &str =
    "https://static.crates.io/crates/itoa/itoa-0.0.0-stub.crate";

const QUOTA_EXHAUSTED_BODY: &str = r#"{"error":"quota_exhausted","message":"diffpack's Composer proxy has used today's free allowance. It resets at 00:00 UTC.","resetsAt":"2026-10-20T00:00:00Z"}"#;

async fn rejection_of_a_stubbed_download() -> JsValue {
    let result = prefetch_package(
        "crates".to_string(),
        "itoa".to_string(),
        "0.0.0-stub".to_string(),
    )
    .await;
    restore_fetch();
    result.expect_err("a stubbed failure must reject")
}

fn property(error: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(error, &JsValue::from_str(name)).expect("an Error has readable properties")
}

fn text_property(error: &JsValue, name: &str) -> Option<String> {
    property(error, name).as_string()
}

fn number_property(error: &JsValue, name: &str) -> Option<f64> {
    property(error, name).as_f64()
}

#[wasm_bindgen_test]
async fn a_plain_404_rejects_with_not_found_even_when_its_body_looks_like_a_proxy_error() {
    answer_every_fetch_with(404, QUOTA_EXHAUSTED_BODY);

    let error = rejection_of_a_stubbed_download().await;

    assert!(error.is_instance_of::<js_sys::Error>(), "{error:?}");
    assert_eq!(text_property(&error, "code").as_deref(), Some("not_found"));
    assert_eq!(number_property(&error, "status"), Some(404.0));
    assert_eq!(
        text_property(&error, "message"),
        Some(format!(
            "Couldn't download {UNPUBLISHED_ITOA_DOWNLOAD} (HTTP 404)"
        ))
    );
}

#[wasm_bindgen_test]
async fn a_download_that_never_got_an_answer_rejects_as_a_network_failure() {
    reject_every_fetch();

    let error = rejection_of_a_stubbed_download().await;

    assert!(error.is_instance_of::<js_sys::Error>(), "{error:?}");
    assert_eq!(text_property(&error, "code").as_deref(), Some("network"));
    assert!(property(&error, "status").is_undefined());
}

#[wasm_bindgen_test]
async fn a_proxy_quota_answer_rejects_with_its_code_and_reset_time() {
    answer_every_fetch_as_the_proxy(503, "quota_exhausted", QUOTA_EXHAUSTED_BODY);

    let error = rejection_of_a_stubbed_download().await;

    assert!(error.is_instance_of::<js_sys::Error>(), "{error:?}");
    assert_eq!(
        text_property(&error, "code").as_deref(),
        Some("quota_exhausted")
    );
    assert_eq!(number_property(&error, "status"), Some(503.0));
    assert_eq!(
        text_property(&error, "resetsAt").as_deref(),
        Some("2026-10-20T00:00:00Z")
    );
}

#[wasm_bindgen_test]
async fn a_download_whose_body_breaks_off_rejects_as_a_network_failure_with_its_status() {
    answer_every_fetch_with_a_body_that_breaks_off();

    let error = rejection_of_a_stubbed_download().await;

    assert!(error.is_instance_of::<js_sys::Error>(), "{error:?}");
    assert_eq!(text_property(&error, "code").as_deref(), Some("network"));
    assert_eq!(number_property(&error, "status"), Some(200.0));
}
