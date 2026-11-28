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

/// `models-store.json` 是**叠加**在编译期内置目录上的（ref `remote-catalog-provider.ts:12-21`）：
/// 同 id 覆盖、新 id 追加。只放 `auth.json` 的机器读到别处拷来的 store 时应生效。
#[test]
fn store_overlays_builtin_catalog_by_model_id() {
    let builtin_json = r#"{
      "openrouter": [
        { "id": "a", "api": "openai-completions", "contextWindow": 1000 },
        { "id": "b", "api": "openai-completions", "contextWindow": 2000 }
      ]
    }"#;
    let store_json = r#"{
      "openrouter": { "models": [
        { "id": "b", "api": "openai-completions", "contextWindow": 9999 },
        { "id": "c", "api": "openai-completions", "contextWindow": 3000 }
      ] }
    }"#;
    let mut base = BuiltInCatalog::from_json(builtin_json).expect("内置目录可解析");
    let overlay = BuiltInCatalog::from_json(store_json).expect("store 可解析");
    base.overlay(&overlay);

    assert_eq!(
        base.find("openrouter", "a").map(|m| m.context_window),
        Some(Some(1000))
    );
    assert_eq!(
        base.find("openrouter", "b").map(|m| m.context_window),
        Some(Some(9999)),
        "同 id 应被 store 覆盖"
    );
    assert_eq!(
        base.find("openrouter", "c").map(|m| m.context_window),
        Some(Some(3000)),
        "store 独有的模型应追加"
    );
    let ids: Vec<&str> = base
        .entries()
        .filter(|(provider, _)| *provider == "openrouter")
        .map(|(_, model)| model.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "b", "c"], "覆盖后不应重复");
}

/// 只有内置目录（没有 `models.json`，例如只放 `auth.json` 的机器）：解析出的 `contextWindow`
/// 必须是目录里的值，不能退回 128000 的缺省（真机 footer 的 `x%/Nk` 直接受影响）。
#[test]
fn catalog_only_resolution_keeps_context_window() {
    let builtin_json = r#"{
      "openrouter": [
        { "id": "big", "api": "openai-completions",
          "baseUrl": "https://openrouter.ai/api/v1",
          "contextWindow": 262144, "maxTokens": 235929 }
      ]
    }"#;
    let catalog = BuiltInCatalog::from_json(builtin_json).expect("目录可解析");
    let resolved = ModelsConfig::default()
        .resolve_with_catalog("openrouter", "big", Some(&catalog))
        .expect("空 models.json 也能解析目录模型");
    assert_eq!(resolved.context_window, 262144);
    assert_eq!(resolved.max_tokens, 235929);
    assert_eq!(resolved.base_url, "https://openrouter.ai/api/v1");
}
