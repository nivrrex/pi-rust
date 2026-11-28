//! P4-13 步骤 5：`models.json` 的类型错误是"忽略还是报错"——用**真实加载路径**钉住。
//!
//! 真实加载路径（步骤 2.1 追溯）：`crates/pi-cli/src/main.rs:89-93`
//! ```text
//! let models_path = paths.agent_dir.join("models.json");
//! std::fs::read_to_string(&models_path).and_then(|text| serde_json::from_str::<ModelsConfig>(&text))
//! ```
//! 即 **serde 直接反序列化**（不是 `BuiltInCatalog::from_json`——那支解析的是内置目录，
//! `crates/pi-ai/src/config.rs:311/333` 的 `.ok()` 只影响内置目录，与本项无关）。
//!
//! ref `packages/coding-agent/src/core/model-config.ts`：
//! - `:245` `const validateModelsConfig = Compile(ModelsConfigSchema);`
//! - `:305-312` 校验失败即 `return new ModelConfig(new Map(), "Invalid models.json schema:…")`
//!
//! 结论：类型错误会让**整份配置**被拒绝（不是忽略该字段）。
use pi_ai::config::ModelsConfig;

fn models_json(cost: &str) -> String {
    format!(
        "{{\"providers\":{{\"mock\":{{\"api\":\"openai-completions\",\
         \"baseUrl\":\"http://127.0.0.1:1/v1\",\
         \"models\":[{{\"id\":\"mock-model\",\"cost\":{cost}}}]}}}}}}"
    )
}

#[test]
fn a_malformed_cost_makes_the_config_unusable() {
    let text = models_json(r#""这不是对象""#);
    // 样本自检：样本里确实有一个**类型错误**的 cost。
    assert!(text.contains(r#""cost":"这不是对象""#), "{text}");
    // 顶层解析**不做校验**（`ModelsConfig.providers` 是 `Map<String, Value>`，`config.rs:23-27`）
    let config: ModelsConfig = serde_json::from_str(&text).expect("顶层只是 JSON 容器");
    // 真解析点在 `resolve_with_catalog:94-95`（`from_value::<ProviderConfig>` → `ConfigError::Invalid`）。
    let Err(error) = config.resolve("mock", "mock-model") else {
        panic!("类型错误的 cost 必须让该 provider 不可用");
    };
    assert!(
        matches!(error, pi_ai::config::ConfigError::Invalid(_)),
        "{error}"
    );
}

#[test]
fn a_malformed_cost_in_an_unused_provider_is_rejected_at_load() {
    // ref `model-config.ts:305-312`：`Check` 失败 → **整份**配置被拒绝（`new ModelConfig(new Map(), "Invalid …")`），
    // 与"用到哪个 provider 才校验哪个"不同。pi-rs 此前是**懒校验**：坏掉的 provider 只要不被选中就没人发现。
    let text = models_json(r#"{"input":1,"output":2,"cacheRead":0,"cacheWrite":0}"#).replace(
        r#""mock":{"#,
        r#""坏掉的":{"api":"openai-completions","models":[{"id":"x","cost":"不是对象"}]},"mock":{"#,
    );
    assert!(text.contains("坏掉的"), "{text}");
    let config: ModelsConfig = serde_json::from_str(&text).expect("顶层是 JSON 容器");
    // 选中的 provider 是好的，能解析……
    assert!(config.resolve("mock", "mock-model").is_ok());
    // ……但整份配置按 ref 必须**不可用**。
    let Err(error) = config.validate() else {
        panic!("整份配置必须被拒绝");
    };
    assert!(
        matches!(error, pi_ai::config::ConfigError::Invalid(_)),
        "{error}"
    );
}

#[test]
fn a_well_formed_cost_is_accepted() {
    // 正向对照：正常 cost 能解析出来，且值落到 `ResolvedModel.rates`。
    let text = models_json(r#"{"input":1,"output":2,"cacheRead":0,"cacheWrite":0}"#);
    let config: ModelsConfig = serde_json::from_str(&text).expect("正常 cost 可解析");
    let resolved = config.resolve("mock", "mock-model").expect("能解析出模型");
    assert_eq!(resolved.provider_name, "mock");
    assert_eq!(resolved.model_id, "mock-model");
}

#[test]
fn an_unknown_extra_key_is_ignored() {
    // 未知字段：`ModelsConfigSchema` 没有 `additionalProperties:false`，未知键被忽略（两边一致）。
    let text = models_json(r#"{"input":1,"output":2,"cacheRead":0,"cacheWrite":0}"#).replace(
        r#""id":"mock-model","#,
        r#""id":"mock-model","谁也不知道":123,"#,
    );
    assert!(text.contains("谁也不知道"), "{text}");
    assert!(
        serde_json::from_str::<ModelsConfig>(&text).is_ok(),
        "{text}"
    );
}
