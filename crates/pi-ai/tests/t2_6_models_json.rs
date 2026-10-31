//! T2.6：从 `models.json` 读 provider 配置，消掉硬编码的 provider 名。
//!
//! 判据：`tools/difftest/run.mjs` 写出的 models.json 形状，以及
//! `spec/f-config.md` 的 `ProviderConfigSchema`（未知字段必须保留）。

use std::collections::BTreeMap;

use pi_ai::config::{ModelsConfig, ProviderConfig};
use serde_json::json;

const SAMPLE: &str = r#"{
  "providers": {
    "mock": {
      "name": "Mock",
      "baseUrl": "http://127.0.0.1:1234/v1",
      "api": "openai-completions",
      "models": [
        {
          "id": "mock-model",
          "name": "Mock Model",
          "reasoning": false,
          "input": ["text", "image"],
          "cost": { "input": 1, "output": 2, "cacheRead": 0, "cacheWrite": 0 },
          "contextWindow": 200000,
          "maxTokens": 4096
        }
      ]
    }
  }
}"#;

#[test]
fn parses_provider_and_model_fields() {
    let config: ModelsConfig = serde_json::from_str(SAMPLE).expect("样例可解析");
    let provider = pi_ai::config::provider_of(&config, "mock").expect("mock provider 存在");

    assert_eq!(provider.api.as_deref(), Some("openai-completions"));
    assert_eq!(
        provider.base_url.as_deref(),
        Some("http://127.0.0.1:1234/v1")
    );
    let model = provider.models.first().expect("至少一个模型");
    assert_eq!(model.id, "mock-model");
    assert_eq!(model.max_tokens, Some(4096));
    assert_eq!(model.context_window, Some(200_000));
    assert_eq!(model.cost.expect("有费率").input, 1.0);
}

#[test]
fn unknown_fields_are_preserved_for_round_trip() {
    let with_extras = json!({
        "providers": {
            "mock": {
                "name": "Mock",
                "baseUrl": "http://127.0.0.1:1/v1",
                "api": "openai-completions",
                "oauth": "radius",
                "authHeader": true,
                "headers": { "X-Trace": "1" },
                "compat": { "supportsStore": false },
                "models": [{ "id": "m", "futureField": 42 }]
            }
        }
    });
    let config: ModelsConfig = serde_json::from_value(with_extras.clone()).expect("可解析");
    let written = serde_json::to_value(&config).expect("可写回");
    assert_eq!(
        written, with_extras,
        "读后写回不得丢字段（语义等价）——与原版 models.json 互通的要求"
    );
}

#[test]
fn resolves_a_model_into_a_client_ready_pair() {
    let config: ModelsConfig = serde_json::from_str(SAMPLE).expect("样例可解析");
    let resolved = config.resolve("mock", "mock-model").expect("能解析出模型");

    assert_eq!(resolved.provider_name, "mock");
    assert_eq!(resolved.api, "openai-completions");
    assert_eq!(resolved.base_url, "http://127.0.0.1:1234/v1");
    assert_eq!(resolved.max_tokens, 4096);
    assert_eq!(resolved.rates.input, 1.0);
    assert_eq!(resolved.rates.output, 2.0);
}

#[test]
fn unknown_provider_or_model_is_reported() {
    let config: ModelsConfig = serde_json::from_str(SAMPLE).expect("样例可解析");
    assert!(
        config.resolve("nope", "mock-model").is_err(),
        "未知 provider 要报错"
    );
    assert!(config.resolve("mock", "nope").is_err(), "未知模型要报错");
}

#[test]
fn providers_without_models_still_parse() {
    let config: ModelsConfig = serde_json::from_str(r#"{"providers":{"x":{}}}"#).expect("可解析");
    assert_eq!(config.providers.len(), 1);
    let provider: ProviderConfig = pi_ai::config::provider_of(&config, "x").expect("x 存在");
    assert!(provider.models.is_empty());
    let empty: BTreeMap<&str, ()> = BTreeMap::new();
    assert!(empty.is_empty());
}

/// 客户端应从配置拿到 provider 名与 base url，而不是硬编码。
#[test]
fn client_takes_provider_and_url_from_config() {
    let config: ModelsConfig = serde_json::from_str(SAMPLE).expect("样例可解析");
    let resolved = config.resolve("mock", "mock-model").expect("能解析");
    let client = pi_ai::openai::ChatCompletionsClient::from_resolved(&resolved);

    assert_eq!(client.provider_name(), "mock", "provider 名来自配置");
    assert_eq!(client.base_url(), "http://127.0.0.1:1234/v1");
    assert_eq!(resolved.max_tokens, 4096, "maxTokens 可直接填进请求");
}
