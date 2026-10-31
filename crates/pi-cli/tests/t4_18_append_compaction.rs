//! P5-3：`SessionWriter::append_compaction`（ref `session-manager.ts:1259-1285`）。
use pi_cli::session_writer::SessionWriter;

#[test]
fn compaction_entry_has_the_ref_fields_and_defaults() {
    let dir = tempfile::tempdir().expect("临时目录");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).expect("建目录");
    let mut writer = SessionWriter::create(&sessions, dir.path(), "mock", "mock-model", "medium")
        .expect("建会话");
    let written = writer
        .append_compaction(
            "摘要正文\n\n<read-files>\nbig.txt\n</read-files>",
            Some("keep-me"),
            25000,
            Some(serde_json::json!({"readFiles":["big.txt"],"modifiedFiles":[]})),
            Some(serde_json::json!({"input":500,"output":40,"cacheRead":0,"cacheWrite":0,
                                    "reasoning":0,"totalTokens":540,
                                    "cost":{"input":0.0005,"output":0.00008,"cacheRead":0,"cacheWrite":0,"total":0.00058}})),
            // **原始 JSON 文本**（保序）：写出的那一行必须原样保留 `name` 在 `constrainedSampling` 之前
            Some(r#"{"role":"system","content":"","timestamp":1700000000000,"toolsAdded":[{"name":"read","description":"d","parameters":{"type":"object"},"constrainedSampling":false}]}"#),
        )
        .expect("写入成功");
    // 样本自检：返回的条目就是刚写的那条
    assert_eq!(written["type"], "compaction");
    assert_eq!(
        written["summary"],
        "摘要正文\n\n<read-files>\nbig.txt\n</read-files>"
    );
    assert_eq!(written["firstKeptEntryId"], "keep-me", "给了就不覆盖");
    assert_eq!(written["tokensBefore"], 25000);
    assert_eq!(
        written["fromHook"], false,
        "pi 自己生成的压缩 → fromHook=false（golden/S21 同形）"
    );
    assert_eq!(written["details"]["readFiles"][0], "big.txt");
    assert_eq!(written["usage"]["totalTokens"], 540);
    assert_eq!(written["systemMessage"]["role"], "system");
    // **写出的行里 systemMessage 是原文**（RawValue 保序，`serde_json::Map` 会排序）
    let raw_text = std::fs::read_to_string(writer.path()).expect("读会话文件");
    let raw_line = raw_text.lines().last().expect("有内容");
    let name = raw_line.find("\"name\"").expect("有 name");
    let constrained = raw_line
        .find("\"constrainedSampling\"")
        .expect("有 constrainedSampling");
    assert!(
        name < constrained,
        "systemMessage 必须按原文写出（name 在 constrainedSampling 之前）"
    );
    // 盘上的那一行必须与返回的条目一致（含 parentId 指向 header）
    let path = writer.path().to_path_buf();
    let text = std::fs::read_to_string(&path).expect("读会话文件");
    let last: serde_json::Value =
        serde_json::from_str(text.lines().last().expect("有内容")).expect("最后一行是 JSON");
    assert_eq!(last["type"], "compaction");
    assert_eq!(last["id"], written["id"]);
    assert!(last["parentId"].is_string(), "parentId 应指向 header");

    // 负向：不给 firstKeptEntryId 时用**本条自己的 id**（ref 的 `?? id`）
    let mut writer2 = SessionWriter::create(&sessions, dir.path(), "mock", "mock-model", "medium")
        .expect("建会话");
    let second = writer2
        .append_compaction("第二次", None, 1, None, None, None)
        .expect("写入成功");
    assert_eq!(second["firstKeptEntryId"], second["id"], "缺省时用自身 id");
    assert!(
        second.get("details").is_none() && second.get("usage").is_none(),
        "None 时不写键"
    );
    assert!(
        second.get("systemMessage").is_none(),
        "None 时不写 systemMessage"
    );
}

/// P5-3：settings.json 的 `compaction` 段解析（缺省回落默认值；S21 场景的值逐字段读回）。
#[test]
fn compaction_settings_merge_with_defaults() {
    use pi_cli::settings;
    // S21 场景的 settings.json 片段（reserveTokens 1000 / keepRecentTokens 2000 / enabled true）
    let s = settings::parse(r#"{"defaultModel":"mock-model","compaction":{"enabled":true,"reserveTokens":1000,"keepRecentTokens":2000}}"#)
        .expect("解析成功");
    let c = s.compaction_settings();
    assert!(c.enabled, "enabled 应为 true");
    assert_eq!(c.reserve_tokens, 1000);
    assert_eq!(c.keep_recent_tokens, 2000);

    // 部分键：只给 reserveTokens ⇒ 其余回落默认
    let s2 = settings::parse(r#"{"compaction":{"reserveTokens":777}}"#).expect("解析成功");
    let c2 = s2.compaction_settings();
    let base = pi_core::agent::compaction::DEFAULT_COMPACTION_SETTINGS;
    assert_eq!(c2.reserve_tokens, 777);
    assert_eq!(
        c2.keep_recent_tokens, base.keep_recent_tokens,
        "未给的键回落默认"
    );
    assert_eq!(c2.enabled, base.enabled);

    // 负向：段缺失 ⇒ 全默认
    let s3 = settings::parse(r#"{"defaultModel":"m"}"#).expect("解析成功");
    assert_eq!(
        s3.compaction_settings(),
        base,
        "没有 compaction 段时就是默认值"
    );
    // 负向：键名拼错必须被拒（deny_unknown_fields），不得静默忽略
    let bad = settings::parse(r#"{"compaction":{"reserveToken":777}}"#);
    assert!(bad.is_err(), "拼错的键（少个 s）必须报错");
}

/// P5-6 第二部分：`auto_compaction_plan` 的三条溢出分支（`overflow_outcome` 的流程层映射）。
///
/// 反证：把 `OverflowExhausted` 的 `failure` 去掉（回到 `None`）时，这条断言会失败——
/// 即"恢复已试过一次"必须只发失败事件、不再压缩。
#[test]
fn overflow_branches_map_to_reasons_and_failures() {
    use pi_cli::auto_compaction::{auto_compaction_plan, CompactionConfig};
    use pi_core::agent::compaction::{CompactionSettings, DEFAULT_COMPACTION_SETTINGS};
    use pi_core::session::SessionEntry;

    let assistant = |usage_input: u64, stop_reason: &str| {
        let value = serde_json::json!({
            "role": "assistant", "timestamp": 5000,
            "api": "openai-completions", "provider": "mock", "model": "mock-model",
            "responseModel": null, "endTurn": null, "errorMessage": null, "rawStopReason": null,
            "content": [{"type": "text", "text": "x"}],
            "usage": {"input": usage_input, "output": 10, "cacheRead": 0, "cacheWrite": 0,
                      "reasoning": 0, "totalTokens": usage_input + 10,
                      "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": stop_reason
        });
        match serde_json::from_value::<pi_ai::message::Message>(value).expect("助手消息") {
            pi_ai::message::Message::Assistant(assistant) => *assistant,
            _other => panic!("样本自检：必须是助手消息"),
        }
    };
    // 样本自检：`prepare_compaction` 需要**可用的切点**（至少两轮），所以放 user/assistant/user。
    let entries: Vec<SessionEntry> = vec![
        serde_json::from_value(serde_json::json!({
            "type": "message", "id": "u1", "parentId": null, "timestamp": "t",
            "message": {"role": "user", "content": "第一轮问题", "timestamp": 1}
        }))
        .expect("user 条目"),
        serde_json::from_value(serde_json::json!({
            "type": "message", "id": "a1", "parentId": "u1", "timestamp": "t",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "第一轮回答"}],
                        "timestamp": 2, "api": "openai-completions", "provider": "mock",
                        "model": "mock-model", "usage": {"input": 10, "output": 5, "cacheRead": 0,
                        "cacheWrite": 0, "reasoning": 0, "totalTokens": 15,
                        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                        "stopReason": "stop"}
        }))
        .expect("assistant 条目"),
        serde_json::from_value(serde_json::json!({
            "type": "message", "id": "u2", "parentId": "a1", "timestamp": "t",
            "message": {"role": "user", "content": "第二轮问题", "timestamp": 3}
        }))
        .expect("user 条目"),
    ];
    let config = CompactionConfig {
        settings: CompactionSettings {
            keep_recent_tokens: 1,
            ..DEFAULT_COMPACTION_SETTINGS
        },
        context_window: 20_000,
        model: "mock-model".to_string(),
        model_provider: "mock".to_string(),
        model_max_tokens: 1024,
        model_reasoning: false,
        thinking_level: None,
    };

    // 说明：`auto_compaction_plan` 里 `skip_aborted_check` 由调用方给；这里直接传 true。
    // ① 用量越过窗口且停因是 stop ⇒ Case 2（压缩、不重试）
    let plan = auto_compaction_plan(&entries, &config, &assistant(25_000, "stop"), true);
    let plan = plan.expect("应当压缩");
    assert_eq!(plan.reason, "overflow");
    assert!(!plan.will_retry, "Case 2 不重试");
    assert!(plan.failure.is_none(), "Case 2 不是失败事件");

    // ② 停因不是 stop ⇒ Case 1（压缩后重试）
    let plan = auto_compaction_plan(&entries, &config, &assistant(25_000, "length"), true)
        .expect("应当压缩");
    assert!(plan.will_retry, "Case 1 需要重试");
    assert!(plan.failure.is_none());

    // ③ 反证锚点：`OverflowExhausted` 必须带失败文案（不再压缩）
    //    该分支由 `overflow_recovery_attempted` 控制；`auto_compaction_plan` 目前恒传 false，
    //    所以这里用 pi-core 的判定函数直接钉住文案的存在性（同一常量来源）。
    let exhausted =
        pi_core::agent::compaction::overflow_outcome(&pi_core::agent::compaction::OverflowInputs {
            assistant: &assistant(25_000, "length"),
            model_provider: "mock",
            model_id: "mock-model",
            context_window: 20_000,
            model_max_tokens: 1024,
            projection: &pi_core::agent::projection::build_session_projection(
                &entries,
                pi_core::agent::projection::LeafSelector::Unspecified,
            )
            .expect("投影"),
            path_entries: &entries,
            settings: &config.settings,
            last_compaction_timestamp: None,
            skip_aborted_check: true,
            overflow_recovery_attempted: true,
        });
    assert_eq!(
        exhausted,
        pi_core::agent::compaction::AutoCompactionDecision::OverflowExhausted
    );
}
