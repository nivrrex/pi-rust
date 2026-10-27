//! P4-13 步骤 3：thinking 等级 → `budget_tokens` / `display`（限 anthropic 路径）。
//!
//! ref 规则（两处拼起来）：
//! - `api/simple-options.ts:55` `MIN_ANSWER_TOKENS = 1024`
//! - `api/simple-options.ts:57-62` `DEFAULT_THINKING_BUDGETS = { minimal:1024, low:2048, medium:8192, high:16384 }`
//! - `api/simple-options.ts:64-66` `clampReasoning`：`xhigh`/`max` → `high`
//! - `api/simple-options.ts:79-95` `adjustMaxTokensForThinking`
//! - `api/anthropic-messages.ts:889-903` `adjusted = adjustMaxTokensForThinking(base.maxTokens, model.maxTokens, options.reasoning,
//!   options.thinkingBudgets)`，然后 `thinkingBudgetTokens = Math.min(adjusted.thinkingBudget, Math.max(0, maxTokens - 1024))`
//! - `api/anthropic-messages.ts:1173-1177` `params.thinking = { type:"enabled", budget_tokens: options.thinkingBudgetTokens || 1024, display }`
//! - `api/anthropic-messages.ts:1179-1181` `thinkingEnabled === false && thinkingLevelMap?.off !== null` → `{ type: "disabled" }`
//! - `api/anthropic-messages.ts:1164` `display = options.thinkingDisplay ?? "summarized"`
use pi_ai::anthropic::{
    adjust_max_tokens_for_thinking, thinking_budget_for_level, thinking_config_for_level,
    ThinkingConfig,
};

/// golden/S12-anthropic 的请求 #1 记下的 `thinking`（4096 的模型上限、无调用方上限、等级 medium）。
fn golden_thinking() -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .join("golden/S12-anthropic/requests.jsonl");
    let text = std::fs::read_to_string(&path).expect("golden 请求存在");
    let first = text.lines().next().expect("至少一条请求");
    let entry: serde_json::Value = serde_json::from_str(first).expect("请求行是 JSON");
    let body = entry.get("body").cloned().expect("有 body");
    let body: serde_json::Value = match body {
        serde_json::Value::String(text) => serde_json::from_str(&text).expect("body 是 JSON 文本"),
        other => other,
    };
    body.get("thinking")
        .cloned()
        .expect("golden 请求带 thinking")
}

#[test]
fn medium_under_a_4096_ceiling_reproduces_the_recorded_thinking() {
    // 样本自检：golden 里确实有 thinking，且是 enabled + 3072。
    let golden = golden_thinking();
    assert_eq!(golden["type"], "enabled", "{golden}");
    assert_eq!(golden["budget_tokens"], 3072, "{golden}");

    let config =
        thinking_config_for_level(Some("medium"), None, 4096).expect("medium 要发 thinking");
    assert_eq!(
        config.to_json().expect("有 JSON"),
        golden,
        "medium + 4096 上限应与 golden 逐字段相同"
    );
}

#[test]
fn budget_table_follows_the_ref() {
    assert_eq!(thinking_budget_for_level("minimal"), Some(1024));
    assert_eq!(thinking_budget_for_level("low"), Some(2048));
    assert_eq!(thinking_budget_for_level("medium"), Some(8192));
    assert_eq!(thinking_budget_for_level("high"), Some(16384));
    // ref `simple-options.ts:64-66`：xhigh/max 先折成 high。
    assert_eq!(thinking_budget_for_level("xhigh"), Some(16384));
    assert_eq!(thinking_budget_for_level("max"), Some(16384));
    // 负向：不在 ThinkingLevel 联合里的字符串不猜（返回 None）。
    assert_eq!(thinking_budget_for_level("turbo"), None);
}

#[test]
fn minimal_is_capped_by_the_answer_room() {
    // minimal(1024) 在 4096 的顶下不触发夹取 → 仍是 1024；`anthropic-messages.ts:902` 再取 min(1024, 3072)。
    let (max_tokens, budget) = adjust_max_tokens_for_thinking(None, 4096, "minimal").expect("可算");
    assert_eq!((max_tokens, budget), (4096, 1024));
    let config = thinking_config_for_level(Some("minimal"), None, 4096).expect("要发");
    assert_eq!(config.to_json().expect("JSON")["budget_tokens"], 1024);
}

#[test]
fn high_level_is_capped_at_the_ceiling_minus_1024() {
    // high(16384) 超过 4096 的顶 → clamp 到 4096-1024 = 3072（与 medium 同）。
    let (_, budget) = adjust_max_tokens_for_thinking(None, 4096, "high").expect("可算");
    assert_eq!(budget, 3072);
}

#[test]
fn level_off_disables_thinking() {
    // ref `anthropic-messages.ts:1179-1181`
    let config = thinking_config_for_level(Some("off"), None, 4096).expect("off 也要发字段");
    assert_eq!(
        config.to_json().expect("JSON"),
        serde_json::json!({"type":"disabled"})
    );
}

#[test]
fn no_level_emits_no_thinking_field() {
    // 非推理模型：pi-rs 的 `reasoning_effort` 是 None → 整段不发（ref `:1160` 的 `else if (model.reasoning)`）。
    assert!(thinking_config_for_level(None, None, 4096).is_none());
}

#[test]
fn unknown_level_emits_no_field_instead_of_guessing() {
    // 防御：`set_thinking_level` 目前**不校验**等级（`rpc_session.rs:524-528`），
    // 传进未知等级时不猜一个预算，直接不发该字段。
    let config = thinking_config_for_level(Some("turbo"), None, 4096);
    assert!(config.is_none(), "未知等级不应编造 thinking 配置");
    let _unused = ThinkingConfig::Disabled;
}

/// X2：**每个档位到请求体字段**的映射（`thinking.type` / `budget_tokens` / `display`）。
///
/// 与 golden 的单点样本（`medium`）互补：这里把 5 档全过一遍，
/// 保证 `thinking_config_for_level(...).to_json()` 的输出与 ref 的
/// `{type:"enabled", budget_tokens, display:"summarized"}` 一致。
#[test]
fn every_level_maps_to_the_documented_budget_in_the_request_body() {
    // 模型上限取足够大，避免被"答案留白"规则夹住（那条规则有单独用例）。
    let ceiling = 200_000;
    for (level, expected) in [
        ("minimal", 1024u64),
        ("low", 2048),
        ("medium", 8192),
        ("high", 16384),
        // ref `simple-options.ts:64-66`：xhigh/max 折成 high。
        ("xhigh", 16384),
        ("max", 16384),
    ] {
        let config = thinking_config_for_level(Some(level), None, ceiling)
            .unwrap_or_else(|| panic!("{level} 应有 thinking 配置"));
        let body = config.to_json().expect("enabled 档位应产出 thinking 字段");
        assert_eq!(body["type"], "enabled", "{level}");
        assert_eq!(body["budget_tokens"], expected, "{level}");
        assert_eq!(body["display"], "summarized", "{level}");
    }
    // `off` 走另一个分支：显式 `{type:"disabled"}`（不是缺省）。
    let off = thinking_config_for_level(Some("off"), None, ceiling).expect("off 也要有配置");
    assert_eq!(off.to_json().expect("off 有 json")["type"], "disabled");
}
