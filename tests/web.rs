use diffpack_engine::{
    build_diff_tree_for_package, get_diff_for_comparison, get_diff_for_path, prefetch_package,
    set_proxy_origin,
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DiffEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
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

let routes = [];
let requested = [];

export function route_fetch(fragment, status, body, proxyErrorCode) {
    routes.push({ fragment, status, body: new Uint8Array(body), proxyErrorCode });
    replaceFetch(async (url) => {
        const address = String(url);
        requested.push(address);
        const route = routes.find((candidate) => address.includes(candidate.fragment));
        if (!route) {
            throw new TypeError(`No stubbed route for ${address}`);
        }
        const headers = route.proxyErrorCode ? { "Diffpack-Proxy-Error": route.proxyErrorCode } : {};
        return new Response(route.body, { status: route.status, headers });
    });
}

export function requested_urls() {
    return requested.slice();
}

export function listed_url_parameter(address) {
    return new URL(address).searchParams.get("url");
}

export function restore_fetch() {
    globalThis.fetch = realFetch;
    routes = [];
    requested = [];
}
"#)]
extern "C" {
    fn answer_every_fetch_with(status: u16, body: &str);
    fn answer_every_fetch_as_the_proxy(status: u16, code: &str, body: &str);
    fn answer_every_fetch_with_a_body_that_breaks_off();
    fn reject_every_fetch();
    fn route_fetch(fragment: &str, status: u16, body: &[u8], proxy_error_code: Option<String>);
    fn requested_urls() -> Vec<String>;
    fn listed_url_parameter(address: &str) -> Option<String>;
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

const STUB_COMPOSER_LISTING: &str = r#"{
    "minified": "composer/2.0",
    "packages": {"stub/package": [
        {"version": "3.0.0"},
        {"version": "2.0.0", "dist": {"type": "zip", "url": "https://git.drupalcode.org/stub/package/-/archive/2.0.0.zip"}},
        {"version": "1.0.0", "dist": {"type": "zip", "url": "https://api.github.com/repos/stub/package/zipball/1111111111111111111111111111111111111111"}}
    ]}
}"#;

const TEST_PROXY: &str = "https://proxy.test";

fn use_the_test_proxy() {
    set_proxy_origin(TEST_PROXY.to_string()).expect("the test proxy origin is valid");
}

async fn rejection_of_a_composer_version(version: &str) -> JsValue {
    use_the_test_proxy();
    answer_every_fetch_with(200, STUB_COMPOSER_LISTING);
    let result = prefetch_package(
        "composer".to_string(),
        "stub/package".to_string(),
        version.to_string(),
    )
    .await;
    restore_fetch();
    result.expect_err("the stubbed listing cannot yield a download")
}

#[wasm_bindgen_test]
async fn a_composer_version_packagist_does_not_list_rejects_with_no_such_version() {
    let error = rejection_of_a_composer_version("9.9.9").await;

    assert!(error.is_instance_of::<js_sys::Error>(), "{error:?}");
    assert_eq!(
        text_property(&error, "code").as_deref(),
        Some("no_such_version")
    );
    assert_eq!(
        text_property(&error, "message").as_deref(),
        Some("Packagist lists no version 9.9.9 of stub/package")
    );
    assert!(property(&error, "status").is_undefined());
}

#[wasm_bindgen_test]
async fn a_composer_archive_on_an_unsupported_host_rejects_with_unsupported_archive_host() {
    let error = rejection_of_a_composer_version("2.0.0").await;

    assert!(error.is_instance_of::<js_sys::Error>(), "{error:?}");
    assert_eq!(
        text_property(&error, "code").as_deref(),
        Some("unsupported_archive_host")
    );
}

#[wasm_bindgen_test]
async fn a_composer_version_without_an_archive_rejects_with_no_archive() {
    let error = rejection_of_a_composer_version("3.0.0").await;

    assert!(error.is_instance_of::<js_sys::Error>(), "{error:?}");
    assert_eq!(text_property(&error, "code").as_deref(), Some("no_archive"));
    assert_eq!(
        text_property(&error, "message").as_deref(),
        Some("Packagist lists no archive for stub/package 3.0.0")
    );
}

#[wasm_bindgen_test]
fn set_proxy_origin_accepts_an_origin_and_refuses_one_with_a_path() {
    assert!(set_proxy_origin("https://proxy.test/".to_string()).is_ok());

    let refusal = set_proxy_origin("https://proxy.test/v1".to_string())
        .expect_err("an origin with a path is refused");

    assert!(
        refusal
            .as_string()
            .is_some_and(|message| message.starts_with("Invalid proxy origin")),
        "{refusal:?}"
    );
}

const ROUTED_SHA_ONE: &str = "1111111111111111111111111111111111111111";
const ROUTED_SHA_TWO: &str = "2222222222222222222222222222222222222222";

fn routed_listed_url(package: &str, sha: &str) -> String {
    format!("https://api.github.com/repos/{package}/zipball/{sha}")
}

fn routed_listing(package: &str) -> String {
    format!(
        r#"{{"minified": "composer/2.0", "packages": {{"{package}": [
            {{"version": "2.0.0", "dist": {{"type": "zip", "url": "{two}"}}}},
            {{"version": "1.0.0", "dist": {{"type": "zip", "url": "{one}"}}}}
        ]}}}}"#,
        two = routed_listed_url(package, ROUTED_SHA_TWO),
        one = routed_listed_url(package, ROUTED_SHA_ONE),
    )
}

fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (path, content) in entries {
        writer.start_file(*path, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn route_a_package_with_two_versions(package: &str) {
    route_fetch(
        &format!("/v1/composer/p2/{package}.json"),
        200,
        routed_listing(package).as_bytes(),
        None,
    );
    route_fetch(
        ROUTED_SHA_ONE,
        200,
        &zip_of(&[
            ("stub-routed-1111111/composer.json", "{}"),
            ("stub-routed-1111111/src/Thing.php", "<?php echo 1;\n"),
        ]),
        None,
    );
    route_fetch(
        ROUTED_SHA_TWO,
        200,
        &zip_of(&[
            ("stub-routed-2222222/composer.json", "{}"),
            ("stub-routed-2222222/src/Thing.php", "<?php echo 2;\n"),
        ]),
        None,
    );
}

async fn build_both_versions_of(package: &str) -> (Result<JsValue, JsValue>, Vec<String>) {
    let tree = build_diff_tree_for_package(
        "composer".to_string(),
        package.to_string(),
        "1.0.0".to_string(),
        "2.0.0".to_string(),
        0.6,
        false,
    )
    .await;
    let requested = requested_urls();
    restore_fetch();
    (tree, requested)
}

fn modified_files(entry: &DiffEntry, out: &mut Vec<String>) {
    if entry.entry_type == "file" && entry.status == "modified" {
        out.push(entry.path.clone());
    }
    for child in entry.children.iter().flatten() {
        modified_files(child, out);
    }
}

#[wasm_bindgen_test]
async fn a_composer_build_downloads_its_listing_and_archives_through_the_proxy() {
    let package = "stub/through-the-proxy";
    use_the_test_proxy();
    route_a_package_with_two_versions(package);

    let (tree, requested) = build_both_versions_of(package).await;

    let tree: DiffEntry = serde_wasm_bindgen::from_value(tree.expect("the stubbed build works"))
        .expect("diff tree should deserialize");
    let mut modified = Vec::new();
    modified_files(&tree, &mut modified);
    assert_eq!(modified, ["src/Thing.php"]);

    assert!(
        requested.contains(&format!("{TEST_PROXY}/v1/composer/p2/{package}.json")),
        "{requested:?}"
    );
    let mut listed_urls: Vec<String> = requested
        .iter()
        .filter(|url| {
            url.starts_with(&format!(
                "{TEST_PROXY}/v1/composer/archive?package={package}&url="
            ))
        })
        .filter_map(|url| listed_url_parameter(url))
        .collect();
    listed_urls.sort();
    assert_eq!(
        listed_urls,
        [
            routed_listed_url(package, ROUTED_SHA_ONE),
            routed_listed_url(package, ROUTED_SHA_TWO)
        ]
    );
}

#[wasm_bindgen_test]
async fn a_rejected_fetch_is_proxy_unreachable_on_the_proxy_and_network_elsewhere() {
    use_the_test_proxy();
    reject_every_fetch();
    let through_the_proxy = prefetch_package(
        "composer".to_string(),
        "stub/routed".to_string(),
        "1.0.0".to_string(),
    )
    .await
    .expect_err("every fetch is rejected");
    let elsewhere = prefetch_package(
        "crates".to_string(),
        "itoa".to_string(),
        "0.0.0-stub".to_string(),
    )
    .await
    .expect_err("every fetch is rejected");
    restore_fetch();

    assert_eq!(
        text_property(&through_the_proxy, "code").as_deref(),
        Some("proxy_unreachable")
    );
    assert_eq!(
        text_property(&elsewhere, "code").as_deref(),
        Some("network")
    );
}

#[wasm_bindgen_test]
async fn a_proxy_quota_answer_on_a_composer_listing_arrives_with_its_reset_time() {
    use_the_test_proxy();
    route_fetch(
        "/v1/composer/p2/",
        503,
        QUOTA_EXHAUSTED_BODY.as_bytes(),
        Some("quota_exhausted".to_string()),
    );

    let error = prefetch_package(
        "composer".to_string(),
        "stub/routed".to_string(),
        "1.0.0".to_string(),
    )
    .await
    .expect_err("the proxy refused");
    restore_fetch();

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
async fn a_composer_build_of_two_versions_requests_their_listing_once() {
    let package = "stub/listing-once";
    use_the_test_proxy();
    route_a_package_with_two_versions(package);

    let (tree, requested) = build_both_versions_of(package).await;

    tree.expect("the stubbed build works");
    let listing_requests = requested
        .iter()
        .filter(|url| url.contains("/v1/composer/p2/"))
        .count();
    assert_eq!(listing_requests, 1, "{requested:?}");
}
