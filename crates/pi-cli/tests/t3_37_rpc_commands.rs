//! P4-5a 第③件：`set_model` / `set_thinking_level` / `queue_update` 的单测。
//!
//! 这三个行为此前**只有差分测试**（S16/S10/S11）覆盖。按"最小抽取、不为测试大改结构"的要求，
//! 我把其中**纯的部分**抽成了 `rpc_session` 里的自由函数（`queue_update_json`、
//! `thinking_level_changed_json`、`validate_set_model`）——生产路径调用它们，
//! 所以这些测试覆盖的是**真实代码**，不是副本。

/// `queue_update` 的线上形状：`{"type":"queue_update","steering":[…],"followUp":[…]}`。
#[test]
fn queue_update_json_has_both_lists() {
    let text = pi_cli::rpc_session::queue_update_json(
        &["改用中文回答".to_string()],
        &["再说一句".to_string()],
    );
    let value: serde_json::Value = serde_json::from_str(&text).expect("是 JSON");
    assert_eq!(value["type"], "queue_update");
    assert_eq!(value["steering"][0], "改用中文回答");
    // **键名是 `followUp`（驼峰）**，拼错会被 TS 静默忽略。
    assert_eq!(value["followUp"][0], "再说一句");
    assert!(value.get("follow_up").is_none(), "不得出现下划线形态");
}

/// 两个列表都为空时也要发（入队/消费都会经过它）。
#[test]
fn queue_update_json_keeps_empty_lists() {
    let value: serde_json::Value =
        serde_json::from_str(&pi_cli::rpc_session::queue_update_json(&[], &[])).expect("是 JSON");
    assert_eq!(value["steering"], serde_json::json!([]));
    assert_eq!(value["followUp"], serde_json::json!([]));
}

/// `thinking_level_changed` 的线上形状（ref `agent-session.ts:2269`）。
#[test]
fn thinking_level_changed_json_carries_the_level() {
    let value: serde_json::Value =
        serde_json::from_str(&pi_cli::rpc_session::thinking_level_changed_json("high"))
            .expect("是 JSON");
    assert_eq!(value["type"], "thinking_level_changed");
    assert_eq!(value["level"], "high");
}

/// `set_model`：当前模型可以通过（**即使没变也要能过**——TS 会无条件写条目）。
#[test]
fn validate_set_model_accepts_the_current_model() {
    assert!(pi_cli::rpc_session::validate_set_model(
        "mock",
        "mock-model",
        "mock",
        "mock-model",
        "mock-model"
    )
    .is_ok());
}

/// 失败路径：provider 或 model 不匹配时的文案必须与 TS 一致
/// （`rpc-mode.ts:475-476` 的 `Model not found: provider/id`）。
#[test]
fn validate_set_model_rejects_unknown_model_with_ts_message() {
    let error = pi_cli::rpc_session::validate_set_model(
        "other",
        "mock-model",
        "mock",
        "mock-model",
        "mock-model",
    )
    .expect_err("provider 不同必须报错");
    assert_eq!(error, "Model not found: other/mock-model");

    let error =
        pi_cli::rpc_session::validate_set_model("mock", "nope", "mock", "mock-model", "mock-model")
            .expect_err("model 不同必须报错");
    assert_eq!(error, "Model not found: mock/nope");
}
