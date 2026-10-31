//! P3-4：内置模型目录。
//!
//! TS 在 models.json 未定义模型时回落到内置 catalog
//! （`core/model-runtime.ts:183-190`、`api/provider-composer.ts` 的 `findModelDefaults`）。
//! 目录内容由 oracle 导出到 `fixtures/models-config/builtin-catalog.json`。

use pi_ai::config::{BuiltInCatalog, ModelsConfig};

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn catalog() -> BuiltInCatalog {
    let path = workspace().join("fixtures/models-config/builtin-catalog.json");
    let text = std::fs::read_to_string(&path).expect("内置目录夹具存在");
    BuiltInCatalog::from_json(&text).expect("内置目录可解析")
}

#[test]
fn catalog_matches_the_typescript_export() {
    let catalog = catalog();
    assert!(
        catalog.provider_count() >= 40,
        "TS 侧导出了 41 个 provider，实际 {}",
        catalog.provider_count()
    );
    assert!(
        catalog.total_models() >= 1000,
        "内置模型总数：{}",
        catalog.total_models()
    );
}

#[test]
fn a_builtin_model_resolves_when_models_json_omits_it() {
    let catalog = catalog();
    let model = catalog
        .find("openai", "gpt-4")
        .expect("内置目录里应有 gpt-4");
    assert_eq!(model.id, "gpt-4");
    assert_eq!(
        model.api, "openai-responses",
        "内置目录里 gpt-4 走的是 responses 形态"
    );
    assert_eq!(
        Some(model.max_tokens.unwrap_or(123)),
        Some(catalog.max_tokens("openai", "gpt-4"))
    );
}

#[test]
fn builtin_catalog_fills_in_missing_api_for_builtin_providers() {
    // 内置 provider 只覆盖 baseUrl 时，api 与费率从内置目录回落——
    // 这正是 TS 的行为（C1 oracle 的 `builtin-baseurl-only` 形态）。
    let catalog = catalog();
    let config: ModelsConfig = serde_json::from_str(
        r#"{"providers":{"openai":{"baseUrl":"https://b.example/v1","models":[]}}}"#,
    )
    .expect("配置可解析");
    let resolved = config
        .resolve_with_catalog("openai", "gpt-4", Some(&catalog))
        .expect("内置 provider 应能从目录回落");
    assert_eq!(
        resolved.api, "openai-responses",
        "api 从内置目录回落（openai 的 gpt-4 在 TS 里是 responses 形态）"
    );
    assert_eq!(
        resolved.base_url, "https://b.example/v1",
        "baseUrl 覆盖优先于内置"
    );
}

#[test]
fn non_builtin_provider_still_errors_without_api() {
    let catalog = catalog();
    let config: ModelsConfig = serde_json::from_str(
        r#"{"providers":{"p":{"baseUrl":"http://x/v1","models":[{"id":"m"}]}}}"#,
    )
    .expect("配置可解析");
    assert!(
        config
            .resolve_with_catalog("p", "m", Some(&catalog))
            .is_err(),
        "非内置 provider 不该从目录回落，仍应报缺 api"
    );
}

#[test]
fn models_json_takes_precedence_over_the_builtin_catalog() {
    // models.json 显式定义 gpt-4 时，应以配置为准而不是内置目录。
    let config: ModelsConfig = serde_json::from_str(
        r#"{"providers":{"openai":{"baseUrl":"https://my-proxy/v1","models":[
             {"id":"gpt-4","maxTokens":123,"contextWindow":456,"cost":{"input":7,"output":8,"cacheRead":0,"cacheWrite":0}}]}}}"#,
    )
    .expect("配置可解析");
    let catalog = catalog();
    let resolved = config
        .resolve_with_catalog("openai", "gpt-4", Some(&catalog))
        .expect("models.json 里的模型可解析");
    assert_eq!(resolved.max_tokens, 123, "以 models.json 为准");
    assert_eq!(resolved.context_window, 456);
    assert_eq!(resolved.rates.input, 7.0);
    assert_eq!(resolved.base_url, "https://my-proxy/v1");
}

#[test]
fn unknown_provider_and_model_still_error_out() {
    let catalog = catalog();
    assert!(catalog.find("nope", "gpt-4").is_none());
    assert!(catalog.find("openai", "nope").is_none());
}
