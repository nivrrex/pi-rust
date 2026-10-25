//! P7-8a/R：`models.json` 里 provider 只写 `models`（不写 `baseUrl`/`api`）时必须能解析。
//!
//! 真机现场（`<SMOKE_DIR>/run-real.sh` 生成的 cfg-real）：
//! ```text
//! providers.openrouter = { models: [...3 条...] }   // 没有 baseUrl / api
//! settings.defaultProvider = "openrouter"
//! settings.defaultModel = "openrouter/free"
//! ```
//! 报错：`Provider "openrouter": Provider openrouter: "baseUrl" is required when defining custom models.`
//!
//! ref 的规则（`core/provider-composer.ts:198-205` 的 `findModelDefaults` + `:171-172`）：
//! `baseUrl = definition.baseUrl ?? providerConfig.baseUrl ?? defaults?.baseUrl`，
//! 其中 `defaults` 依次取：同 id 的内置模型 → 同 api 的 → `openai-completions` 的 → **该 provider 的第一个模型**。
//! 所以"自定义 models 不写 baseUrl"是**合法**的，只要 provider 在内置目录里。

use pi_ai::config::{BuiltInCatalog, ModelsConfig};

fn catalog() -> BuiltInCatalog {
    // 内置目录里 openrouter 只有别的 model id（用户写的 `openrouter/free` 不在目录里）
    BuiltInCatalog::from_json(
        r#"{"openrouter":{"models":[
            {"id":"openrouter/auto","api":"openai-completions","baseUrl":"https://openrouter.ai/api/v1","maxTokens":4096,"contextWindow":128000},
            {"id":"openrouter/claude","api":"anthropic-messages","baseUrl":"https://openrouter.ai/api/v1","maxTokens":8192,"contextWindow":200000}
        ]}}"#,
    )
    .expect("目录 JSON 合法")
}

fn user_config() -> ModelsConfig {
    // 与 cfg-real 同形态（脱敏）：provider 只有 models，没有 baseUrl/api
    serde_json::from_str::<ModelsConfig>(
        r#"{"providers":{"openrouter":{"models":[
            {"id":"openrouter/free"},
            {"id":"openrouter/auto"}
        ]}}}"#,
    )
    .expect("models.json 合法")
}

/// 红用例：只写 `models` 的 provider 必须能从内置目录继承 `baseUrl`/`api`（不再报 baseUrl 错误）。
#[test]
fn custom_models_inherit_provider_defaults_from_catalog() {
    let resolved = user_config()
        .resolve_with_catalog("openrouter", "openrouter/free", Some(&catalog()))
        .expect("应当能从内置目录继承 baseUrl/api");
    assert_eq!(resolved.base_url, "https://openrouter.ai/api/v1");
    assert_eq!(resolved.api, "openai-completions");
}

/// 已有精确内置模型时仍取该模型自己的字段（不能被"第一个模型"覆盖）。
#[test]
fn exact_builtin_still_wins() {
    let resolved = user_config()
        .resolve_with_catalog("openrouter", "openrouter/auto", Some(&catalog()))
        .expect("解析成功");
    assert_eq!(resolved.base_url, "https://openrouter.ai/api/v1");
    assert_eq!(resolved.api, "openai-completions");
}

/// 没有目录时仍按原样报 `baseUrl` 缺失（不静默猜一个地址）。
#[test]
fn without_catalog_missing_baseurl_still_errors() {
    let error = user_config()
        .resolve_with_catalog("openrouter", "openrouter/free", None)
        .expect_err("无目录时缺 baseUrl 必须报错");
    let text = format!("{error}");
    // ref 先校验 `api`（`provider-composer.ts:166-170`）再校验 `baseUrl`（`:171-172`）；
    // 这里只要求"报出配置错误"，不硬编码先报哪个字段。
    assert!(
        text.contains("api") || text.contains("baseUrl"),
        "应报配置错误：{text}"
    );
}

/// ref `provider-composer.ts:171`：`baseUrl = definition.baseUrl ?? providerConfig.baseUrl ?? defaults?.baseUrl`
/// —— **模型级最优先**。此前 `ModelDefinition` 根本没有 `baseUrl` 字段，
/// openrouter 这类"baseUrl 写在模型条目里"的配置会被静默丢掉。
#[test]
fn model_level_baseurl_wins() {
    let config: ModelsConfig = serde_json::from_str(
        r#"{"providers":{"openrouter":{
            "baseUrl":"http://provider-level/v1",
            "models":[{"id":"openrouter/free","api":"openai-completions",
                        "baseUrl":"http://model-level/v1"}]}}}"#,
    )
    .expect("models.json 合法");
    let resolved = config
        .resolve_with_catalog("openrouter", "openrouter/free", None)
        .expect("解析成功");
    assert_eq!(resolved.base_url, "http://model-level/v1");
}

/// 只写模型级 `baseUrl`（provider 级没有）也要能解析。
#[test]
fn model_level_baseurl_alone_is_enough() {
    let config: ModelsConfig = serde_json::from_str(
        r#"{"providers":{"openrouter":{"models":[
            {"id":"openrouter/free","api":"openai-completions","baseUrl":"http://only-model/v1"}]}}}"#,
    )
    .expect("models.json 合法");
    let resolved = config
        .resolve_with_catalog("openrouter", "openrouter/free", None)
        .expect("解析成功");
    assert_eq!(resolved.base_url, "http://only-model/v1");
}
