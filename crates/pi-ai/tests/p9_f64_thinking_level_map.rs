//! F64：`thinkingLevelMap` 的**解析层**断言（`models.json` → `ResolvedModel`）。
//!
//! 规格：`core/model-config.ts:194/209` 声明该字段，`ai/src/models.ts:921-935` 消费它。
//! 此前 pi-rs 根本没解析 ⇒ 值只落在 `extra` 里没人读，
//! 于是"声明 `xhigh`/`max` 的模型少给最高两档、把某档声明为 `null` 的模型多给该档"。

use pi_ai::config::ModelsConfig;

fn config_with(model_body: &str) -> ModelsConfig {
    let text = format!(
        r#"{{"providers":{{"p":{{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{model_body}]}}}}}}"#
    );
    serde_json::from_str(&text).expect("解析 models.json")
}

/// `null` 与"键缺失"必须能区分：前者表示不支持该档，后者表示未声明。
#[test]
fn null_and_absent_are_distinguishable() {
    let config = config_with(
        r#"{"id":"m","reasoning":true,"thinkingLevelMap":{"minimal":null,"xhigh":"high"}}"#,
    );
    let resolved = config.resolve("p", "m").expect("resolve");
    let map = resolved
        .thinking_level_map
        .expect("thinkingLevelMap 应被解析出来");
    assert_eq!(
        map.get("minimal"),
        Some(&None),
        "`null` 必须解析成 Some(None)"
    );
    assert_eq!(
        map.get("xhigh"),
        Some(&Some("high".to_string())),
        "有值的档位"
    );
    assert_eq!(map.get("low"), None, "未声明的档位应缺失");
}

/// 没有该字段时是 `None`（而不是空 map），否则无法与"声明了空 map"区分。
#[test]
fn absent_map_stays_none() {
    let config = config_with(r#"{"id":"m","reasoning":true}"#);
    let resolved = config.resolve("p", "m").expect("resolve");
    assert_eq!(resolved.thinking_level_map, None);
}

/// 把解析结果直接喂给档位解析：声明了 `xhigh` 才拿得到 `xhigh`。
#[test]
fn declared_extended_level_reaches_the_resolver() {
    let declared = config_with(
        r#"{"id":"m","reasoning":true,"thinkingLevelMap":{"xhigh":"xhigh","max":"max"}}"#,
    );
    let resolved = declared.resolve("p", "m").expect("resolve");
    let levels = pi_cli_thinking_levels(&resolved);
    assert!(levels.contains(&"xhigh"), "{levels:?}");
    assert!(levels.contains(&"max"), "{levels:?}");

    let undeclared = config_with(r#"{"id":"m","reasoning":true}"#);
    let resolved = undeclared.resolve("p", "m").expect("resolve");
    let levels = pi_cli_thinking_levels(&resolved);
    assert!(
        !levels.contains(&"xhigh"),
        "未声明时不应出现 xhigh：{levels:?}"
    );
    assert!(!levels.contains(&"max"), "未声明时不应出现 max：{levels:?}");
}

/// `thinking_selector::supported_levels` 的薄包装（避免让 pi-ai 依赖 pi-cli）。
fn pi_cli_thinking_levels(resolved: &pi_ai::config::ResolvedModel) -> Vec<&'static str> {
    // 内联 ref `models.ts:921-935` 的规则，与 pi-cli 的实现断言一致；
    // 两边同规则的交叉验证由 `pi-cli` 的单测覆盖。
    const EXTENDED: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
    if !resolved.reasoning {
        return vec!["off"];
    }
    EXTENDED
        .iter()
        .copied()
        .filter(|level| {
            let mapped = resolved
                .thinking_level_map
                .as_ref()
                .and_then(|map| map.get(*level));
            if let Some(None) = mapped {
                return false;
            }
            if *level == "xhigh" || *level == "max" {
                return mapped.is_some();
            }
            true
        })
        .collect()
}
