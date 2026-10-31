//! P7-6am/M：`BuiltInCatalog` 必须同时接受两种形状，`resolve_with_catalog` 据此补齐 api/baseUrl。
//!
//! 形状② 的来源：ref `core/models-store.ts:46-60` 的 `FileModelsStore`，文件 `<agentDir>/models-store.json`
//! 内容为 `{provider: {models: […], checkedAt, etag}}`。
use pi_ai::config::{BuiltInCatalog, ModelsConfig};

const STORE_SHAPE: &str = r#"{
  "openrouter": {
    "models": [
      { "id": "openrouter/free", "api": "openai-completions", "baseUrl": "https://openrouter.ai/api/v1" }
    ],
    "checkedAt": "2026-10-18T06:00:06Z",
    "etag": "abc"
  }
}"#;

const MODELS_JSON: &str = r#"{
  "providers": {
    "openrouter": { "models": [{ "id": "openrouter/free" }] }
  }
}"#;

/// 与 `MODELS_JSON` 相同，但 provider 显式给了 `api`（这样缺的只剩 `baseUrl`）。
const MODELS_JSON_WITH_API: &str = r#"{
  "providers": {
    "openrouter": { "api": "openai-completions", "models": [{ "id": "openrouter/free" }] }
  }
}"#;

#[test]
fn store_shape_is_accepted_and_fills_base_url() {
    let catalog = BuiltInCatalog::from_json(STORE_SHAPE).expect("models-store 形状必须可解析");
    assert_eq!(catalog.provider_count(), 1);
    let config: ModelsConfig = serde_json::from_str(MODELS_JSON).expect("models.json 可解析");
    let resolved = config
        .resolve_with_catalog("openrouter", "openrouter/free", Some(&catalog))
        .expect("有内置目录时应能补齐 api/baseUrl");
    assert_eq!(resolved.base_url, "https://openrouter.ai/api/v1");
    assert_eq!(resolved.api, "openai-completions");
}

#[test]
fn without_catalog_it_still_errors_on_missing_base_url() {
    let config: ModelsConfig =
        serde_json::from_str(MODELS_JSON_WITH_API).expect("models.json 可解析");
    let error = config
        .resolve_with_catalog("openrouter", "openrouter/free", None)
        .expect_err("没有目录时必须报错（不是静默用空 baseUrl）");
    assert!(
        error.to_string().contains("baseUrl"),
        "错误信息应指出 baseUrl：{error}"
    );
}
