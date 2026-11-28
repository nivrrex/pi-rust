//! P5-1：compaction 纯逻辑与 **TS oracle 逐字段对拍**（RUNBOOK-6 P5.1d）。
//!
//! 期望值来自 `fixtures/oracle/compaction-p5-1.json`，由
//! `node tools/oracle/compaction-oracle.mjs` 从 TS 的 dist 模块录出（连跑 3 次逐字节一致）。
//! 本测试**不写自己的期望值**，只比较；缺样本（pi-rs 未建模的消息种类）显式列出，不静默跳过。
use pi_ai::message::Message;
use pi_core::agent::compaction::{
    calculate_context_tokens, estimate_context_tokens, estimate_tokens, should_compact,
    CompactionSettings, DEFAULT_COMPACTION_SETTINGS,
};

fn oracle() -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .join("fixtures/oracle/compaction-p5-1.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("读不到 oracle（先跑 tools/oracle/compaction-oracle.mjs）：{error}")
    });
    serde_json::from_str(&text).expect("oracle 是 JSON")
}

fn message(json: serde_json::Value) -> Message {
    serde_json::from_value(json).expect("样本消息可反序列化")
}

#[test]
fn defaults_match_the_oracle() {
    let oracle = oracle();
    let defaults = &oracle["defaults"];
    assert_eq!(
        DEFAULT_COMPACTION_SETTINGS,
        CompactionSettings {
            enabled: defaults["enabled"].as_bool().expect("enabled"),
            reserve_tokens: defaults["reserveTokens"].as_u64().expect("reserveTokens"),
            keep_recent_tokens: defaults["keepRecentTokens"]
                .as_u64()
                .expect("keepRecentTokens"),
        },
        "默认设置必须与 ref 一致"
    );
    // 样本自检：oracle 里确实是那三个已知值（不是空对象）
    assert_eq!(defaults["reserveTokens"], 16384);
    assert_eq!(defaults["keepRecentTokens"], 20000);
}

#[test]
fn calculate_context_tokens_matches_the_oracle() {
    let oracle = oracle();
    let native = &oracle["calculatedTokens"]["nativeTotal"];
    let fallback = &oracle["calculatedTokens"]["fallback"];
    let usage = |value: &serde_json::Value| pi_ai::Usage {
        input: value["input"].as_u64().unwrap_or(1),
        output: value["output"].as_u64().unwrap_or(2),
        cache_read: value["cacheRead"].as_u64().unwrap_or(3),
        cache_write: value["cacheWrite"].as_u64().unwrap_or(4),
        cache_write_1h: None,
        reasoning: None,
        total_tokens: value["totalTokens"].as_u64().unwrap_or(0),
        cost: Default::default(),
    };
    assert_eq!(
        calculate_context_tokens(&usage(native)),
        native.as_u64().expect("nativeTotal 是数字"),
        "优先用 totalTokens"
    );
    assert_eq!(
        calculate_context_tokens(&usage(fallback)),
        fallback.as_u64().expect("fallback 是数字"),
        "totalTokens=0 时四项求和"
    );
}

#[test]
fn estimate_tokens_matches_the_oracle_for_modelled_kinds() {
    let oracle = oracle();
    let samples = &oracle["tokenSamples"];
    // pi-rs 的 `pi_ai::Message` 只有 system/user/assistant/toolResult 四种；
    // TS 侧的 bashExecution / branchSummary / compactionSummary 在 pi-rs 里没有对应变体
    // （P4 起 branch_summary/custom_message 会被当成 user 消息）⇒ 这三条**不在此对拍**，
    // 由 `unmodelled_kinds_are_listed_not_silently_skipped` 显式登记。
    let modelled: [(&str, serde_json::Value); 6] = [
        (
            "systemPlain",
            serde_json::json!({"role":"system","timestamp":0,"content":"x".repeat(400)}),
        ),
        (
            "systemWithSectionsAndTools",
            serde_json::json!({
                "role":"system","timestamp":0,
                "content":"y".repeat(40),
                "sections":{"preamble":"a".repeat(40),"tools":"","cwd":null},
                "toolsAdded":[{"name":"read","description":"d"}]
            }),
        ),
        (
            "userString",
            serde_json::json!({"role":"user","timestamp":0,"content":"u".repeat(80)}),
        ),
        (
            "userWithImage",
            serde_json::json!({
                "role":"user","timestamp":0,
                "content":[{"type":"text","text":"z".repeat(8)},{"type":"image","mimeType":"image/png","data":"AAAA"}]
            }),
        ),
        (
            "assistantMixed",
            serde_json::json!({
                "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model","responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,
                "content":[
                    {"type":"thinking","thinking":"t".repeat(12)},
                    {"type":"text","text":"s".repeat(16)},
                    {"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"a.txt"}}
                ],
                "usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
                         "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
                "stopReason":"stop"
            }),
        ),
        (
            "toolResultShort",
            serde_json::json!({
                "role":"toolResult","timestamp":0,"toolCallId":"call_1","toolName":"read",
                "content":[{"type":"text","text":"r".repeat(20)}],"isError":false
            }),
        ),
    ];
    let mut compared = 0;
    for (name, sample) in modelled {
        let expected = samples[name]
            .as_u64()
            .unwrap_or_else(|| panic!("oracle 缺 {name}"));
        assert!(expected > 0, "样本 {name} 的期望值应为正数（否则没覆盖到）");
        assert_eq!(
            estimate_tokens(&message(sample)),
            expected,
            "estimate_tokens({name}) 与 TS oracle 不一致"
        );
        compared += 1;
    }
    assert_eq!(compared, 6, "对拍样本数固定为 6");
    // 图片那一支确实走了 4800：1202 = ceil((8 + 4800)/4)
    assert_eq!(samples["userWithImage"], 1202, "图片估算必须按 4800 计入");
}

#[test]
fn unmodelled_kinds_are_listed_not_silently_skipped() {
    let oracle = oracle();
    let samples = oracle["tokenSamples"]
        .as_object()
        .expect("tokenSamples 是对象");
    let modelled = [
        "systemPlain",
        "systemWithSectionsAndTools",
        "userString",
        "userWithImage",
        "assistantMixed",
        "toolResultShort",
    ];
    let unmodelled: Vec<&String> = samples
        .keys()
        .filter(|key| !modelled.contains(&key.as_str()))
        .collect();
    assert_eq!(
        unmodelled,
        vec!["bashExecution", "branchSummary", "compactionSummary"],
        "未建模的 TS 消息种类必须显式登记（新增/删除都要在这里体现）"
    );
}

#[test]
fn empty_and_filled_text_blocks_differ_like_the_oracle() {
    let oracle = oracle();
    let empty = estimate_tokens(&message(
        serde_json::json!({"role":"user","timestamp":0,"content":[{"type":"text","text":""}]}),
    ));
    let filled = estimate_tokens(&message(
        serde_json::json!({"role":"user","timestamp":0,"content":[{"type":"text","text":"abc"}]}),
    ));
    assert_eq!(
        empty,
        oracle["emptyVsFilled"]["emptyBlock"]
            .as_u64()
            .expect("emptyBlock")
    );
    assert_eq!(
        filled,
        oracle["emptyVsFilled"]["filledBlock"]
            .as_u64()
            .expect("filledBlock")
    );
    assert_ne!(empty, filled, "负向对照：空文本块与有文本块必须不同");
}

#[test]
fn estimate_context_tokens_matches_the_oracle() {
    let oracle = oracle();
    let samples = &oracle["contextEstimateSamples"];

    let with_usage = vec![
        message(serde_json::json!({"role":"user","timestamp":0,"content":"a".repeat(40)})),
        message(serde_json::json!({
            "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model","responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,"content":[{"type":"text","text":"b".repeat(20)}],
            "usage":{"input":100,"output":50,"cacheRead":10,"cacheWrite":5,"totalTokens":165,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"stop"
        })),
        message(serde_json::json!({"role":"user","timestamp":0,"content":"c".repeat(12)})),
    ];
    let estimate = estimate_context_tokens(&with_usage);
    assert_eq!(
        estimate.last_usage_index,
        Some(1),
        "应命中第 2 条（索引 1）"
    );
    assert_eq!(
        estimate.usage_tokens,
        samples["withUsage"]["usageTokens"]
            .as_u64()
            .expect("usageTokens")
    );
    assert_eq!(
        estimate.trailing_tokens,
        samples["withUsage"]["trailingTokens"]
            .as_u64()
            .expect("trailingTokens")
    );
    assert_eq!(
        estimate.tokens,
        samples["withUsage"]["tokens"].as_u64().expect("tokens")
    );

    // 负向：aborted / 全零 usage 都不算有效 usage ⇒ 全部走估算，`lastUsageIndex = null`
    let aborted = vec![
        message(serde_json::json!({
            "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model","responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,"content":[{"type":"text","text":"x".repeat(40)}],
            "usage":{"input":9,"output":9,"cacheRead":0,"cacheWrite":0,"totalTokens":18,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"aborted"
        })),
        message(serde_json::json!({"role":"user","timestamp":0,"content":"d".repeat(8)})),
    ];
    let aborted_estimate = estimate_context_tokens(&aborted);
    assert_eq!(
        aborted_estimate.last_usage_index, None,
        "aborted 不算有效 usage"
    );
    assert_eq!(
        aborted_estimate.tokens,
        samples["abortedOnly"]["tokens"]
            .as_u64()
            .expect("abortedOnly.tokens")
    );

    let zero = vec![message(serde_json::json!({
        "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model","responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,"content":[{"type":"text","text":"x".repeat(40)}],
        "usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
                 "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
        "stopReason":"stop"
    }))];
    assert_eq!(
        estimate_context_tokens(&zero).last_usage_index,
        None,
        "全零 usage 不算有效"
    );

    assert_eq!(
        estimate_context_tokens(&[]).tokens,
        samples["empty"]["tokens"].as_u64().expect("empty.tokens")
    );
}

#[test]
fn should_compact_threshold_boundaries_match_the_oracle() {
    let oracle = oracle();
    let settings = CompactionSettings {
        enabled: true,
        reserve_tokens: 200,
        keep_recent_tokens: 0,
    };
    assert_eq!(
        should_compact(800, 1000, &settings),
        oracle["thresholdSamples"]["exact"]
            .as_bool()
            .expect("exact"),
        "阈值恰好等于时不触发"
    );
    assert_eq!(
        should_compact(801, 1000, &settings),
        oracle["thresholdSamples"]["above"]
            .as_bool()
            .expect("above")
    );
    assert_eq!(
        should_compact(799, 1000, &settings),
        oracle["thresholdSamples"]["below"]
            .as_bool()
            .expect("below")
    );
    let disabled = CompactionSettings {
        enabled: false,
        ..settings
    };
    assert_eq!(
        should_compact(9999, 1000, &disabled),
        oracle["thresholdSamples"]["disabled"]
            .as_bool()
            .expect("disabled"),
        "enabled=false 恒 false"
    );
}

// ---------------------------------------------------------------------------
// P5.1c（文件操作三件套）：与 oracle 的 `fileLists` 对拍
// ---------------------------------------------------------------------------

#[test]
fn file_operations_match_the_oracle() {
    use pi_core::agent::compaction::{
        compute_file_lists, create_file_ops, extract_file_ops_from_message, format_file_operations,
    };
    let oracle = oracle();
    let expected = &oracle["fileLists"];
    assert_eq!(
        expected["read"],
        serde_json::json!(["README.md"]),
        "样本自检：oracle 的 read 列表"
    );
    assert_eq!(
        expected["modified"],
        serde_json::json!(["src/main.rs"]),
        "样本自检：oracle 的 modified 列表"
    );

    // 与 oracle 脚本里同样的三次调用：read(README.md) / edit(src/main.rs) / 无关 user 消息
    let mut ops = create_file_ops();
    extract_file_ops_from_message(
        &message(serde_json::json!({
            "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model",
            "responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,
            "content":[{"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"README.md"}}],
            "usage":{"input":10,"output":2,"cacheRead":0,"cacheWrite":0,"totalTokens":12,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"toolUse"
        })),
        &mut ops,
    );
    extract_file_ops_from_message(
        &message(serde_json::json!({
            "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model",
            "responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,
            "content":[{"type":"toolCall","id":"call_2","name":"edit","arguments":{"path":"src/main.rs"}}],
            "usage":{"input":10,"output":2,"cacheRead":0,"cacheWrite":0,"totalTokens":12,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"toolUse"
        })),
        &mut ops,
    );
    // 非助手消息不得产生任何文件操作（负向对照）
    extract_file_ops_from_message(
        &message(serde_json::json!({"role":"user","timestamp":0,"content":"与文件无关的消息"})),
        &mut ops,
    );

    let (read, modified) = compute_file_lists(&ops);
    assert_eq!(
        read,
        vec!["README.md".to_string()],
        "readFiles 与 oracle 不一致"
    );
    assert_eq!(
        modified,
        vec!["src/main.rs".to_string()],
        "modifiedFiles 与 oracle 不一致"
    );
    assert_eq!(
        format_file_operations(&read, &modified),
        expected["formatted"].as_str().expect("formatted"),
        "格式化结果必须与 oracle 逐字节一致"
    );

    // 负向对照：两者都空时必须是空串（不是"\n\n"）
    assert_eq!(format_file_operations(&[], &[]), "");
    assert_eq!(
        expected["formatted"]
            .as_str()
            .map(|text| text.starts_with("\n\n")),
        Some(true),
        "样本自检：oracle 的非空结果确实是 \\n\\n 前缀"
    );
}

// ---------------------------------------------------------------------------
// P5.1b：cut point / turn 边界与 oracle 对拍
// ---------------------------------------------------------------------------

/// oracle 脚本里的 8 个条目（u1 / a1(toolCall) / toolResult / a2 / u2 / a3 / compaction / u3）。
fn oracle_entries() -> Vec<pi_core::session::SessionEntry> {
    let assistant = |id: &str, content: serde_json::Value| {
        serde_json::json!({
            "type": "message", "id": id, "parentId": null, "timestamp": "2026-01-01T00:00:00.000Z",
            "message": {
                "role": "assistant", "timestamp": 0, "api": "openai-completions", "provider": "mock",
                "model": "mock-model", "responseModel": null, "endTurn": null, "errorMessage": null,
                "rawStopReason": null, "content": content,
                "usage": {"input":10,"output":2,"cacheRead":0,"cacheWrite":0,"totalTokens":12,
                          "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
                "stopReason": "stop"
            }
        })
    };
    let user = |id: &str, text: String| {
        serde_json::json!({
            "type":"message","id":id,"parentId":null,"timestamp":"2026-01-01T00:00:00.000Z",
            "message":{"role":"user","timestamp":0,"content":text}
        })
    };
    let entry = |id: &str, message: serde_json::Value| {
        serde_json::json!({
            "type":"message","id":id,"parentId":null,"timestamp":"2026-01-01T00:00:00.000Z",
            "message": message
        })
    };
    let value = serde_json::json!([
        user("e1", "u".repeat(40)),
        assistant(
            "e2",
            serde_json::json!([{"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"a.txt"}}])
        ),
        entry(
            "e3",
            serde_json::json!({"role":"toolResult","timestamp":0,"toolCallId":"call_1","toolName":"read","content":[{"type":"text","text":"r".repeat(40)}],"isError":false})
        ),
        assistant(
            "e4",
            serde_json::json!([{"type":"text","text":"a".repeat(40)}])
        ),
        user("e5", "v".repeat(40)),
        assistant(
            "e6",
            serde_json::json!([{"type":"text","text":"b".repeat(40)}])
        ),
        serde_json::json!({"type":"compaction","id":"e7","parentId":null,"timestamp":"2026-01-01T00:00:00.000Z",
                           "summary":"旧摘要","firstKeptEntryId":"e7","tokensBefore":1}),
        user("e8", "w".repeat(40)),
    ]);
    serde_json::from_value(value).expect("样本条目可反序列化为 SessionEntry")
}

fn expected_cut(
    oracle: &serde_json::Value,
    name: &str,
) -> pi_core::agent::compaction::CutPointResult {
    let value = &oracle["cutSamples"][name];
    let start = value["turnStartIndex"]
        .as_i64()
        .filter(|number| *number >= 0)
        .map(|number| number as usize);
    pi_core::agent::compaction::CutPointResult {
        first_kept_entry_index: value["firstKeptEntryIndex"]
            .as_u64()
            .expect("firstKeptEntryIndex") as usize,
        turn_start_index: start,
        is_split_turn: value["isSplitTurn"].as_bool().expect("isSplitTurn"),
    }
}

#[test]
fn cut_points_match_the_oracle() {
    use pi_core::agent::compaction::{find_cut_point, CutPointResult};
    use pi_core::session::SessionEntry;

    let oracle = oracle();
    let entries = oracle_entries();
    assert!(
        entries
            .iter()
            .any(|entry| matches!(entry, SessionEntry::Compaction { .. })),
        "样本自检：必须含 compaction 条目（否则跳过它的规则没被覆盖）"
    );
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            SessionEntry::Message {
                message: pi_ai::message::Message::ToolResult { .. },
                ..
            }
        )),
        "样本自检：必须含 toolResult 条目（否则不切在它的规则没被覆盖）"
    );

    assert_eq!(
        find_cut_point(&entries, 0, entries.len(), 1).expect("样本条目都支持"),
        expected_cut(&oracle, "small")
    );
    assert_eq!(
        find_cut_point(&entries, 0, entries.len(), 10_000).expect("样本条目都支持"),
        expected_cut(&oracle, "large")
    );
    assert_eq!(
        find_cut_point(&entries, 0, 4, 1).expect("样本条目都支持"),
        expected_cut(&oracle, "atAssistant")
    );
    assert_eq!(
        find_cut_point(&entries, 0, 2, 10_000).expect("样本条目都支持"),
        expected_cut(&oracle, "atUser")
    );
    assert_eq!(
        find_cut_point(&[], 0, 0, 100).expect("样本条目都支持"),
        expected_cut(&oracle, "emptyRange")
    );

    // 负向对照：切点绝不能落在 toolResult 条目（索引 2）上
    for (start, end, budget) in [(0usize, entries.len(), 1u64), (0, 4, 1), (0, 2, 10_000)] {
        assert_ne!(
            find_cut_point(&entries, start, end, budget)
                .expect("样本条目都支持")
                .first_kept_entry_index,
            2,
            "切点不得落在 toolResult 上"
        );
    }
    // 正/负向：`atAssistant` 必须判为 split turn，`atUser` 必须不是
    assert!(
        find_cut_point(&entries, 0, 4, 1)
            .expect("样本条目都支持")
            .is_split_turn,
        "切在 assistant 上应是 split turn"
    );
    assert!(
        !find_cut_point(&entries, 0, 2, 10_000)
            .expect("样本条目都支持")
            .is_split_turn,
        "切在 user 上不是 split turn"
    );
    let _unused: Option<CutPointResult> = None;
}

#[test]
fn turn_start_indexes_match_the_oracle() {
    use pi_core::agent::compaction::find_turn_start_index;
    let oracle = oracle();
    let entries = oracle_entries();
    let expected = &oracle["turnStartSamples"];
    let index_of = |name: &str| -> Option<usize> {
        expected[name]
            .as_i64()
            .filter(|number| *number >= 0)
            .map(|number| number as usize)
    };
    assert_eq!(
        find_turn_start_index(&entries, 2, 0).expect("样本条目都支持"),
        index_of("fromToolResult"),
        "toolResult 属于 e1"
    );
    assert_eq!(
        find_turn_start_index(&entries, 3, 0).expect("样本条目都支持"),
        index_of("fromAssistant")
    );
    assert_eq!(
        find_turn_start_index(&entries, 4, 0).expect("样本条目都支持"),
        index_of("fromUser"),
        "user 自己是 turn 起点"
    );
    assert_eq!(
        find_turn_start_index(&entries, 0, 2).expect("样本条目都支持"),
        index_of("noneBefore"),
        "起点之后找不到 → None"
    );
}

// ---------------------------------------------------------------------------
// P5.1c（序列化）：与 oracle 的 `serializeSamples` 对拍
// ---------------------------------------------------------------------------

#[test]
fn serialize_conversation_matches_the_oracle() {
    use pi_core::agent::compaction::{serialize_conversation, TOOL_RESULT_MAX_CHARS};
    let oracle = oracle();
    let samples = &oracle["serializeSamples"];

    // ① 单参数工具调用（避开键序差异）、② 只有 thinking、③ 无文本无工具
    let single = vec![message(serde_json::json!({
        "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model",
        "responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,
        "content":[{"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"a.txt"}}],
        "usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
                 "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
        "stopReason":"toolUse"
    }))];
    assert_eq!(
        serialize_conversation(&single),
        samples["singleArgToolCall"]
            .as_str()
            .expect("singleArgToolCall"),
        "单参数工具调用必须逐字节一致"
    );

    let thinking_only = vec![message(serde_json::json!({
        "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model",
        "responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,
        "content":[{"type":"thinking","thinking":"只有思考"}],
        "usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
                 "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
        "stopReason":"stop"
    }))];
    assert_eq!(
        serialize_conversation(&thinking_only),
        samples["onlyThinking"].as_str().expect("onlyThinking")
    );
    assert_eq!(
        serialize_conversation(&[message(
            serde_json::json!({"role":"user","timestamp":0,"content":""})
        )]),
        samples["noTextNoTool"].as_str().expect("noTextNoTool"),
        "空 user 文本不得产生条目"
    );

    // ④ 混合样本：**只比到工具调用之前**（多参数键序是已知限制），并断言截断那一支
    let long = "L".repeat(2500);
    let mixed = vec![
        message(serde_json::json!({"role":"user","timestamp":0,"content":"问题"})),
        message(serde_json::json!({
            "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model",
            "responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,
            "content":[{"type":"thinking","thinking":"想一下"},{"type":"text","text":"回答"},
                       {"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"a.txt","offset":1}}],
            "usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"stop"
        })),
        message(serde_json::json!({
            "role":"toolResult","timestamp":0,"toolCallId":"call_1","toolName":"read",
            "content":[{"type":"text","text": long.clone()}],"isError":false
        })),
    ];
    let produced = serialize_conversation(&mixed);
    let expected = samples["mixed"].as_str().expect("mixed");
    // 到工具调用之前的片段必须一致（含 thinking / text 两行）
    let cut = expected
        .find("[Assistant tool calls]")
        .expect("oracle 里应有工具调用行");
    assert_eq!(
        &produced[..produced
            .find("[Assistant tool calls]")
            .expect("应产生工具调用行")],
        &expected[..cut],
        "工具调用之前的部分必须逐字节一致"
    );
    // 截断分支：2500 字符、上限 2000 ⇒ 标记里的差值必须是 500
    let marker = format!(
        "[... {} more characters truncated]",
        long.chars().count() - TOOL_RESULT_MAX_CHARS
    );
    assert!(produced.contains(&marker), "必须出现截断标记：{marker}");
    assert!(
        expected.contains(&marker),
        "样本自检：oracle 里也要有同一个标记"
    );
    assert_eq!(TOOL_RESULT_MAX_CHARS, 2000);
}

// ---------------------------------------------------------------------------
// P5-1e E2：数值/字符串格式（可修）+ 工具参数键序（不可修，② 已登记）
// ---------------------------------------------------------------------------

#[test]
fn js_stringify_matches_the_oracle_per_value() {
    use pi_core::agent::compaction::js_stringify;
    let oracle = oracle();
    let samples = oracle["jsValueSamples"]
        .as_object()
        .expect("jsValueSamples 是对象");
    // 单值样本：值与期望都由 oracle 给出（不受键序影响）
    let values: Vec<(&str, serde_json::Value)> = vec![
        ("n_1_0", serde_json::json!(1.0)),
        ("n_1e21", serde_json::json!(1e21)),
        ("n_neg0", serde_json::json!(-0.0)),
        ("n_2p53p1", serde_json::json!(9007199254740992u64)),
        ("n_0_1", serde_json::json!(0.1)),
        ("n_1e_7", serde_json::json!(1e-7)),
        ("n_int", serde_json::json!(42)),
        ("n_neg", serde_json::json!(-3.5)),
        ("n_true", serde_json::json!(true)),
        ("n_null", serde_json::Value::Null),
        ("s_u2028", serde_json::json!("\u{2028}")),
        ("s_del", serde_json::json!("\u{7f}")),
        ("s_emoji", serde_json::json!("😀")),
        ("s_ctrl", serde_json::json!("\u{1}")),
        ("s_quote_backslash", serde_json::json!("a\"b\\c")),
        ("s_newline_tab", serde_json::json!("a\nb\tc")),
        (
            "nested_single_key",
            serde_json::json!({"outer": {"inner": 1}}),
        ),
        ("array_mixed", serde_json::json!([1, "two", null, true])),
    ];
    assert_eq!(
        values.len(),
        samples.len(),
        "样本数必须与 oracle 一致（新增/删除都要同步）"
    );
    let mut compared = 0;
    for (name, value) in values {
        let expected = samples[name]
            .as_str()
            .unwrap_or_else(|| panic!("oracle 缺 {name}"));
        // 样本自检：期望值非空，且确实是 JSON 片段
        assert!(!expected.is_empty(), "{name} 的期望为空");
        assert_eq!(
            js_stringify(&value),
            expected,
            "js_stringify({name}) 与 JS 不一致"
        );
        compared += 1;
    }
    assert_eq!(compared, 18, "单值对拍样本数固定为 18");
}

#[test]
fn serialize_single_key_args_match_the_oracle() {
    // 单键参数不受"键序丢失"影响 ⇒ 可以逐字节对拍，也用来做"改回旧写法即转红"的反证
    use pi_core::agent::compaction::serialize_conversation;
    let oracle = oracle();
    let samples = &oracle["serializedArgs"];
    for (name, arguments) in [
        ("singleKeyNumber", serde_json::json!({"n": 1e21})),
        ("singleKeyString", serde_json::json!({"s": "\u{2028}"})),
        ("singleKeyFloat", serde_json::json!({"n": 1.0})),
    ] {
        let message = message(serde_json::json!({
            "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model",
            "responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,
            "content":[{"type":"toolCall","id":"call_1","name":"t","arguments": arguments}],
            "usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"toolUse"
        }));
        let expected = samples[name]
            .as_str()
            .unwrap_or_else(|| panic!("oracle 缺 {name}"));
        assert!(
            expected.contains('='),
            "{name} 的期望样本必须含参数（防空转）"
        );
        assert_eq!(
            serialize_conversation(&[message]),
            expected,
            "{name} 不一致"
        );
    }
}

/// **钉住键序分歧**（P5-1e E2 的 ②）：这些样本的期望来自 TS（插入序 / JS 整数键升序），
/// pi-rs 的 `Map` 是字典序 ⇒ **今天必然不同**。用例默认 `#[ignore]`，解除 ignore 就会暴露分歧。
#[test]
#[ignore = "工具参数键序：pi-rs 的 serde_json::Map 是字典序，TS 用插入序（E2 已登记为已知限制）"]
fn serialize_multi_key_args_diverge_from_the_oracle_by_key_order() {
    use pi_core::agent::compaction::serialize_conversation;
    let oracle = oracle();
    let samples = &oracle["serializedArgs"];
    let fixture_args = oracle["argsJson"].clone();
    for name in [
        "multiArgNonAlpha",
        "integerKeys",
        "duplicateKeys",
        "nestedArgs",
        "numberFormats",
        "stringFormats",
    ] {
        let arguments = fixture_args[name].clone();
        let message = message(serde_json::json!({
            "role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock","model":"mock-model",
            "responseModel":null,"endTurn":null,"errorMessage":null,"rawStopReason":null,
            "content":[{"type":"toolCall","id":"call_1","name":"t","arguments": arguments}],
            "usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"toolUse"
        }));
        assert_eq!(
            serialize_conversation(&[message]),
            samples[name].as_str().unwrap_or_default(),
            "{name}：键序不同（这是已知限制，见 notes/questions.md）"
        );
    }
}

#[test]
fn agent_roles_are_projected_with_their_real_rules() {
    // P5-2pre：`session_entry_to_context_messages` 的 `UnsupportedEntry` 已被**真实投影规则**替换
    // （ref `session-manager.ts:439-466`）⇒ 本用例证明"不再拒绝"，并逐条钉住每一类的结果。
    use pi_core::agent::compaction::session_entry_to_context_messages;
    use pi_core::session::SessionEntry;

    let entry = |json: serde_json::Value| -> SessionEntry {
        serde_json::from_value(json).expect("条目可反序列化")
    };
    let roles = |entry: &SessionEntry| -> Vec<&'static str> {
        session_entry_to_context_messages(entry)
            .expect("已实现真实规则，不应再报错")
            .iter()
            .map(pi_ai::message::Message::role)
            .collect()
    };

    // ① custom_message → 一条 `custom` 消息（`customType`/`content` 保留）
    let custom = entry(serde_json::json!({
        "type":"custom_message","id":"c1","parentId":null,"timestamp":"t",
        "customType":"note","content":"扩展注入的一行","display":true
    }));
    let messages = session_entry_to_context_messages(&custom).expect("支持");
    assert_eq!(messages.len(), 1, "custom_message 产出一条消息");
    match &messages[0] {
        pi_ai::message::Message::Custom {
            custom_type,
            content,
            ..
        } => {
            assert_eq!(custom_type, "note", "customType 要保留");
            assert!(
                matches!(content, pi_ai::message::UserContent::Text(text) if text == "扩展注入的一行"),
                "content 要按 UserContent 解析"
            );
        }
        _not_custom => panic!("custom_message 应映射成 Custom，实际 {messages:?}"),
    }

    // ② branch_summary（有 summary）→ 一条 `branchSummary`；**空 summary → 不产生消息**（ref `:458`）
    let branch = entry(serde_json::json!({
        "type":"branch_summary","id":"b1","parentId":null,"timestamp":"t","summary":"分支摘要","fromId":"u1"
    }));
    assert_eq!(
        roles(&branch),
        vec!["branchSummary"],
        "分支摘要要有自己的角色"
    );
    let branch_empty = entry(serde_json::json!({
        "type":"branch_summary","id":"b2","parentId":null,"timestamp":"t","summary":"","fromId":"u1"
    }));
    assert!(
        session_entry_to_context_messages(&branch_empty)
            .expect("支持")
            .is_empty(),
        "空 summary 不产生消息"
    );

    // ③ compaction → 一条 `compactionSummary`（`tokensBefore` 保留）；
    //    带 `systemMessage` 时（ref `:463`）**两条**：system 在前
    let compaction = entry(serde_json::json!({
        "type":"compaction","id":"k1","parentId":null,"timestamp":"t",
        "summary":"旧摘要","firstKeptEntryId":"k1","tokensBefore":4242
    }));
    let messages = session_entry_to_context_messages(&compaction).expect("支持");
    assert_eq!(messages.len(), 1, "无 systemMessage 时只有摘要消息");
    assert_eq!(messages[0].role(), "compactionSummary");
    match &messages[0] {
        pi_ai::message::Message::CompactionSummary {
            summary,
            tokens_before,
            ..
        } => {
            assert_eq!(summary, "旧摘要");
            assert_eq!(*tokens_before, 4242, "tokensBefore 要保留");
        }
        _not_summary => panic!("compaction 应映射成 CompactionSummary"),
    }
    let compaction_with_system = entry(serde_json::json!({
        "type":"compaction","id":"k2","parentId":null,"timestamp":"t",
        "summary":"旧摘要","firstKeptEntryId":"k2","tokensBefore":7,
        "systemMessage":{"role":"system","timestamp":0,"content":"系统提示"}
    }));
    assert_eq!(
        roles(&compaction_with_system),
        vec!["system", "compactionSummary"],
        "带 systemMessage 时两条，且 system 在前（ref `:463`）"
    );

    // ④ 确定"不产生消息"的三类：context_edit（ref `:461-465`）、thinking_level_change、未知类型
    assert!(session_entry_to_context_messages(&entry(serde_json::json!({
        "type":"context_edit","id":"e1","parentId":null,"timestamp":"t","targetId":"u1","replacement":null
    }))).expect("支持").is_empty());
    assert!(session_entry_to_context_messages(&entry(serde_json::json!({
        "type":"thinking_level_change","id":"t1","parentId":null,"timestamp":"t","thinkingLevel":"high"
    }))).expect("支持").is_empty());
    assert!(
        session_entry_to_context_messages(&entry(serde_json::json!({
            "type":"未来类型","id":"x1","parentId":null,"timestamp":"t"
        })))
        .expect("支持")
        .is_empty(),
        "未知条目按 ref `:465` 给空"
    );

    // ⑤ 正向对照：普通 message 条目仍是它自己
    assert_eq!(
        roles(&entry(serde_json::json!({
            "type":"message","id":"m1","parentId":null,"timestamp":"t",
            "message":{"role":"user","timestamp":0,"content":"hi"}
        }))),
        vec!["user"]
    );
}

#[test]
fn to_llm_messages_folds_agent_roles_into_user() {
    // ref `core/messages.ts:148-188` `convertToLlm`：三个 agent 角色折成 **user**，其余原样。
    use pi_ai::message::{
        Message, UserContent, BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX,
        COMPACTION_SUMMARY_PREFIX, COMPACTION_SUMMARY_SUFFIX,
    };
    let folded = pi_ai::message::to_llm_messages(&[
        Message::Custom {
            custom_type: "note".to_string(),
            content: UserContent::Text("自定义内容".to_string()),
            display: true,
            details: serde_json::Value::Null,
            timestamp: 0,
            extra: serde_json::Map::new(),
        },
        Message::BranchSummary {
            summary: "分支".to_string(),
            from_id: Some("u1".to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        },
        Message::CompactionSummary {
            summary: "压缩".to_string(),
            tokens_before: 1,
            timestamp: 0,
            extra: serde_json::Map::new(),
        },
        Message::User {
            content: UserContent::Text("原样".to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        },
    ]);
    assert_eq!(folded.len(), 4, "每条输入产出一条输出");
    assert!(
        folded.iter().all(|message| message.role() == "user"),
        "四个都应是 user：{folded:?}"
    );
    let text_of = |message: &Message| -> String {
        match message {
            Message::User { content, .. } => match content {
                UserContent::Text(text) => text.clone(),
                UserContent::Blocks(_not_blocks) => String::new(),
            },
            _not_user => String::new(),
        }
    };
    assert_eq!(text_of(&folded[0]), "自定义内容", "custom 的内容原样");
    assert_eq!(
        text_of(&folded[1]),
        format!("{BRANCH_SUMMARY_PREFIX}分支{BRANCH_SUMMARY_SUFFIX}"),
        "分支摘要按 ref 的前后缀包裹"
    );
    assert_eq!(
        text_of(&folded[2]),
        format!("{COMPACTION_SUMMARY_PREFIX}压缩{COMPACTION_SUMMARY_SUFFIX}"),
        "压缩摘要按 ref 的前后缀包裹"
    );
    assert_eq!(text_of(&folded[3]), "原样", "其它角色原样透传");
}
