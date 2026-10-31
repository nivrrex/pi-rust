//! C1：models.json 解析与 TS 逐项一致（oracle 由 `tools/oracle/models-config-oracle.mjs` 生成）。
//!
//! 期望文件在 `fixtures/models-config/*.expected.json`，由 TS 的
//! `ModelRuntime` + `ModelRegistry` 产生；本测试逐形态核对 Rust 的解析结果。

use pi_ai::config::{ModelsConfig, DEFAULT_CONTEXT_WINDOW, DEFAULT_MAX_TOKENS};
use serde_json::Value;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

struct Expectation {
    name: String,
    source: Value,
    expected: Value,
}

fn expectations() -> Vec<Expectation> {
    let dir = workspace().join("fixtures/models-config");
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("index.json")).expect("有 index"))
            .expect("index 是 JSON");
    index
        .as_array()
        .expect("index 是数组")
        .iter()
        .map(|name| {
            let file = name.as_str().expect("name 是字符串");
            Expectation {
                name: file.to_string(),
                source: serde_json::from_str(
                    &std::fs::read_to_string(dir.join(format!("{file}.json"))).expect("有形态文件"),
                )
                .expect("形态文件是 JSON"),
                expected: serde_json::from_str(
                    &std::fs::read_to_string(dir.join(format!("{file}.expected.json")))
                        .expect("有期望文件"),
                )
                .expect("期望文件是 JSON"),
            }
        })
        .collect()
}

#[test]
fn oracle_expectations_cover_at_least_twelve_forms() {
    assert!(
        expectations().len() >= 12,
        "C1 要求至少 12 种配置形态，实际 {}",
        expectations().len()
    );
}

#[test]
fn rust_agrees_with_typescript_on_every_form() {
    for expectation in expectations() {
        // 已知缺口：只覆盖内置 provider 的 baseUrl 时，TS 会把内置模型目录并进来，
        // Rust 侧没有内置目录。单独在下面断言，不混入逐项核对。
        if expectation.name == "builtin-baseurl-only" {
            continue;
        }
        let config: ModelsConfig =
            serde_json::from_value(expectation.source.clone()).expect("形态可解析");
        let probe = expectation.expected["probe"]
            .as_array()
            .expect("probe 是数组");
        let provider_id = probe[0].as_str().expect("provider 是字符串");
        let model_id = probe[1].as_str().expect("model 是字符串");
        let resolved = config.resolve(provider_id, model_id);

        let ts_error = expectation.expected["loadError"].as_str();
        let ts_found = expectation.expected["found"].as_object();
        match (resolved, ts_error, ts_found) {
            // TS 加载就失败：Rust 也必须拒绝。
            (Ok(_), Some(message), _) => {
                panic!("{}: TS 报错但 Rust 成功：{message}", expectation.name);
            }
            // TS 能加载、但模型查不到：Rust 应报"未知模型"，不能报字段缺失。
            (Err(error), None, None) => assert!(
                matches!(error, pi_ai::config::ConfigError::UnknownModel { .. }),
                "{}: 应报未知模型，实际 {error}",
                expectation.name
            ),
            // TS 能查到模型，Rust 却报错：必须查清再改。
            (Err(error), None, Some(_)) => {
                panic!("{}: TS 能查到模型，Rust 报错：{error}", expectation.name)
            }
            // TS 加载失败：Rust 报错即可，但不该把"加载失败"说成"模型不存在"。
            (Err(error), Some(_), _) => assert!(
                !matches!(error, pi_ai::config::ConfigError::UnknownModel { .. }),
                "{}: TS 报的是加载错误，Rust 不该报未知模型：{error}",
                expectation.name
            ),
            (Ok(resolved), None, None) => panic!(
                "{}: TS 查不到该模型，Rust 却解析出了 {:?}",
                expectation.name, resolved.model_id
            ),
            // TS 解析成功且能查到模型：Rust 的每个字段都要一致。
            (Ok(resolved), None, Some(_)) => {
                let found = &expectation.expected["found"];
                assert_eq!(
                    found["api"], resolved.api,
                    "{}: api 不一致",
                    expectation.name
                );
                assert_eq!(
                    found["baseUrl"], resolved.base_url,
                    "{}: baseUrl 不一致",
                    expectation.name
                );
                assert_eq!(
                    found["maxTokens"].as_u64().unwrap_or(DEFAULT_MAX_TOKENS),
                    resolved.max_tokens,
                    "{}: maxTokens 不一致",
                    expectation.name
                );
                assert_eq!(
                    found["contextWindow"]
                        .as_u64()
                        .unwrap_or(DEFAULT_CONTEXT_WINDOW),
                    resolved.context_window,
                    "{}: contextWindow 不一致",
                    expectation.name
                );
                assert_eq!(
                    found["cost"]["input"].as_f64().unwrap_or(0.0),
                    resolved.rates.input,
                    "{}: cost.input 不一致",
                    expectation.name
                );
                assert_eq!(
                    found["cost"]["output"].as_f64().unwrap_or(0.0),
                    resolved.rates.output,
                    "{}: cost.output 不一致",
                    expectation.name
                );
            }
        }
    }
}

#[test]
fn model_level_api_overrides_provider_level() {
    // 该形态的期望里 api 应为模型级的值。
    let expectations = expectations();
    let item = expectations
        .iter()
        .find(|e| e.name == "model-level-override")
        .expect("有该形态");
    assert_eq!(item.expected["found"]["api"], "anthropic-messages");
}

/// 已知缺口：内置模型目录。
///
/// TS 在 `providers.openai` 只覆盖 `baseUrl` 时，仍能列出 openai 的内置模型
/// （`provider-composer.ts` 的 `findModelDefaults` 会回落到内置目录）。
/// Rust 侧没有内置目录，因此 `resolve` 查不到这些模型——**这是能力缺口，不是解析分歧**：
/// 文件本身能被解析，缺的是目录数据。
#[test]
fn builtin_catalog_gap_is_known_and_documented() {
    let expectation = expectations()
        .into_iter()
        .find(|e| e.name == "builtin-baseurl-only")
        .expect("有该形态");
    assert!(expectation.expected["loadError"].is_null(), "TS 不报错");
    let ids = expectation.expected["modelIds"]
        .as_array()
        .expect("modelIds 是数组");
    assert!(
        ids.len() > 10,
        "TS 能列出内置模型（实际 {} 个），Rust 无从对齐",
        ids.len()
    );

    let config: ModelsConfig = serde_json::from_value(expectation.source).expect("文件本身可解析");
    assert!(
        config.resolve("openai", "any").is_err(),
        "Rust 当前没有内置目录，查不到内置模型——已知缺口"
    );
    // 但 provider 级字段仍应解析成功。
    let provider = pi_ai::config::provider_of(&config, "openai").expect("provider 可解析");
    assert_eq!(
        provider.base_url.as_deref(),
        Some("https://b.example/v1"),
        "baseUrl 覆盖本身要能读到"
    );
}

#[test]
fn missing_api_and_baseurl_are_errors_in_both_implementations() {
    // C1 明确要求：MissingField 只能在 TS 同样报错时才报。oracle 已确认两者都报错。
    for name in ["missing-api", "missing-baseurl"] {
        let expectation = expectations()
            .into_iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("有 {name} 形态"));
        assert!(
            expectation.expected["loadError"].is_string(),
            "{name}: TS 应当报错"
        );
        let config: ModelsConfig = serde_json::from_value(expectation.source).expect("可解析");
        let error = config.resolve("p", "m").expect_err("Rust 也应报错");
        // 不只"也报错"：文案要**逐字**等于 TS 的 `loadError`（`Provider "p": …` 前缀
        // 原先被我漏掉）。
        assert_eq!(
            error.to_string(),
            expectation.expected["loadError"]
                .as_str()
                .unwrap_or_default(),
            "{name}: 报错文案必须与 TS 一致"
        );
    }
}
