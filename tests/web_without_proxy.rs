use diffpack_engine::prefetch_package;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn a_composer_download_before_any_proxy_origin_is_set_rejects_with_proxy_not_configured() {
    let error = prefetch_package(
        "composer".to_string(),
        "monolog/monolog".to_string(),
        "3.12.1".to_string(),
    )
    .await
    .expect_err("Composer needs the proxy");

    assert!(error.is_instance_of::<js_sys::Error>(), "{error:?}");
    let code = js_sys::Reflect::get(&error, &"code".into()).unwrap();
    assert_eq!(code.as_string().as_deref(), Some("proxy_not_configured"));
}
