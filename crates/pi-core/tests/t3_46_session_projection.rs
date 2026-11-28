//! P5-2pre：会话投影（ref `core/session-manager.ts:390-573`）与 `context_edit` 的投影规则。
//!
//! 期望值的来源：ref 的 `buildSessionPath:390-416`、`getSessionContextSettings:418-433`、
//! `buildContextEntries:476-512`、`projectContextEntry:519-540`、`buildSessionProjection:543-573`。
//! 每个用例先断言"样本里确实有目标条目"，再断言结果（防空转）。
use pi_core::agent::projection::{
    build_context_entries, build_session_path, build_session_projection,
    get_session_context_settings, LeafSelector,
};
use pi_core::session::SessionEntry;

fn entry(json: serde_json::Value) -> SessionEntry {
    serde_json::from_value(json).expect("条目可反序列化")
}

fn user(id: &str, parent: Option<&str>, text: &str) -> SessionEntry {
    entry(serde_json::json!({
        "type":"message","id":id,"parentId":parent,"timestamp":"t",
        "message":{"role":"user","timestamp":0,"content":text}
    }))
}

fn assistant(id: &str, parent: Option<&str>, text: &str) -> SessionEntry {
    entry(serde_json::json!({
        "type":"message","id":id,"parentId":parent,"timestamp":"t",
        "message":{"role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock",
                   "model":"mock-model","responseModel":null,"endTurn":null,"errorMessage":null,
                   "rawStopReason":null,"content":[{"type":"text","text":text}],
                   "usage":{"input":10,"output":10,"cacheRead":0,"cacheWrite":0,"totalTokens":20,
                            "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
                   "stopReason":"stop"}
    }))
}

fn system_entry(id: &str, parent: Option<&str>, text: &str) -> SessionEntry {
    entry(serde_json::json!({
        "type":"message","id":id,"parentId":parent,"timestamp":"t",
        "message":{"role":"system","timestamp":0,"content":text}
    }))
}

fn compaction_entry(id: &str, parent: Option<&str>, first_kept: &str) -> SessionEntry {
    entry(serde_json::json!({
        "type":"compaction","id":id,"parentId":parent,"timestamp":"t",
        "summary":"旧摘要","firstKeptEntryId":first_kept,"tokensBefore":42
    }))
}

fn edit_entry(
    id: &str,
    parent: Option<&str>,
    target: &str,
    replacement: serde_json::Value,
) -> SessionEntry {
    entry(serde_json::json!({
        "type":"context_edit","id":id,"parentId":parent,"timestamp":"t",
        "targetId":target,"replacement":replacement
    }))
}

#[test]
fn session_path_follows_parent_chain_and_leaf_selector() {
    // 树：a → b → c，a → d（分叉）
    let entries = vec![
        user("a", None, "根"),
        user("b", Some("a"), "b"),
        user("c", Some("b"), "c"),
        user("d", Some("a"), "d"),
    ];
    assert_eq!(entries.len(), 4, "样本自检：四条条目");
    let ids = |path: Vec<SessionEntry>| -> Vec<String> {
        path.iter()
            .filter_map(|entry| entry.id().map(str::to_string))
            .collect()
    };
    assert_eq!(
        ids(build_session_path(&entries, LeafSelector::Id("c"))),
        vec!["a", "b", "c"]
    );
    assert_eq!(
        ids(build_session_path(&entries, LeafSelector::Id("d"))),
        vec!["a", "d"]
    );
    // 没给 → 最后一条（ref `:403` 的 `??=`）
    assert_eq!(
        ids(build_session_path(&entries, LeafSelector::Unspecified)),
        vec!["a", "d"]
    );
    // 显式 null → 空路径（ref `:397-399`）
    assert_eq!(
        ids(build_session_path(&entries, LeafSelector::Null)),
        Vec::<String>::new()
    );
    // 负向：未知 id 必须退化成"最后一条"，不能返回空路径
    assert_eq!(
        ids(build_session_path(&entries, LeafSelector::Id("不存在"))),
        vec!["a", "d"]
    );
}

#[test]
fn context_settings_take_the_last_change() {
    let entries = vec![
        entry(
            serde_json::json!({"type":"thinking_level_change","id":"t1","parentId":null,"timestamp":"t","thinkingLevel":"low"}),
        ),
        entry(
            serde_json::json!({"type":"thinking_level_change","id":"t2","parentId":"t1","timestamp":"t","thinkingLevel":"high"}),
        ),
        entry(
            serde_json::json!({"type":"model_change","id":"m1","parentId":"t2","timestamp":"t","provider":"p1","modelId":"x"}),
        ),
        entry(
            serde_json::json!({"type":"model_change","id":"m2","parentId":"m1","timestamp":"t","provider":"p2","modelId":"y"}),
        ),
        assistant("a1", Some("m2"), "答复"),
    ];
    // 样本自检：确实有两条 thinking_level_change 与两条 model_change
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.kind() == "thinking_level_change")
            .count(),
        2
    );
    let (level, model) = get_session_context_settings(&entries);
    assert_eq!(level, "high", "取最后一条 thinking_level_change");
    assert_eq!(
        model,
        Some(("mock".to_string(), "mock-model".to_string())),
        "助手消息（ref `:427-429`）优先于更早的 model_change"
    );
    // 负向：路径里没有这些条目时，等级是 off、模型是 None
    let only_user = vec![user("u", None, "hi")];
    let (level, model) = get_session_context_settings(&only_user);
    assert_eq!(level, "off");
    assert_eq!(model, None);
}

#[test]
fn context_entries_keep_the_compaction_tail_and_skip_system_messages() {
    // 路径：a(system) → b(user) → c(assistant) → compaction(k0, firstKept=b) → e(user)
    let entries = vec![
        system_entry("a", None, "sys"),
        user("b", Some("a"), "b"),
        assistant("c", Some("b"), "c"),
        compaction_entry("k0", Some("c"), "b"),
        user("e", Some("k0"), "e"),
    ];
    // 样本自检：路径上确实有 compaction，且它的 firstKeptEntryId 指向 b
    assert!(
        entries.iter().any(|entry| entry.kind() == "compaction"),
        "样本要有 compaction 条目"
    );
    let path = build_context_entries(&entries, LeafSelector::Id("e"));
    let ids: Vec<&str> = path.iter().filter_map(|entry| entry.id()).collect();
    assert_eq!(
        ids,
        vec!["k0", "b", "c", "e"],
        "compaction 打头，保留 firstKeptEntryId 起的尾部"
    );
    assert!(
        !ids.contains(&"a"),
        "负向：firstKeptEntryId 之前的条目（含 system）不得保留"
    );
}

#[test]
fn system_messages_inside_the_retained_tail_are_skipped() {
    // ref `:506`：`foundFirstKept && !(… role === "system")` ⇒ 保留区间里的 **system 消息被跳过**（无例外）
    let entries = vec![
        user("b", None, "b"),
        system_entry("s", Some("b"), "sys-after-kept"),
        compaction_entry("k0", Some("s"), "b"),
        user("e", Some("k0"), "e"),
    ];
    let context = build_context_entries(&entries, LeafSelector::Id("e"));
    let ids: Vec<&str> = context.iter().filter_map(|entry| entry.id()).collect();
    assert_eq!(
        ids,
        vec!["k0", "b", "e"],
        "保留区间里的 system 条目按 ref `:506` 跳过"
    );
    assert!(
        !ids.contains(&"s"),
        "负向：system 条目不得出现在最终上下文里"
    );
}

#[test]
fn older_compaction_inside_the_kept_range_contributes_no_messages() {
    // ref `:558-563` 的注释：保留区间里可能出现**更早**的 compaction（它的 id 落在最新保留范围内）
    let entries = vec![
        user("b", None, "b"),
        compaction_entry("k-old", Some("b"), "b"),
        user("e", Some("k-old"), "e"),
        compaction_entry("k-new", Some("e"), "k-old"),
        user("f", Some("k-new"), "f"),
    ];
    let projection = build_session_projection(&entries, LeafSelector::Id("f")).expect("投影成功");
    let kinds: Vec<&str> = projection
        .entries
        .iter()
        .map(|entry| entry.source_entry.kind())
        .collect();
    assert_eq!(
        kinds.first(),
        Some(&"compaction"),
        "最新 compaction 在 index 0"
    );
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "compaction").count(),
        2,
        "样本自检：投影里确实有两条 compaction"
    );
    let older = projection
        .entries
        .iter()
        .skip(1)
        .find(|entry| entry.source_entry.kind() == "compaction")
        .expect("存在更早的 compaction");
    assert!(
        older.messages.is_empty(),
        "index > 0 的 compaction 不贡献消息（ref `:561-563`）"
    );
    // 正负对照：index 0 的那条（k-new）贡献它自己的摘要消息（1 条）
    assert!(
        !projection.entries[0].messages.is_empty(),
        "index 0 的 compaction 必须贡献摘要消息"
    );
}

#[test]
fn context_edit_replacement_and_removal_are_projected() {
    let entries = vec![
        user("u1", None, "原文"),
        assistant("a1", Some("u1"), "原答"),
        edit_entry(
            "x1",
            Some("a1"),
            "u1",
            serde_json::json!({"content": "改后"}),
        ),
        edit_entry("x2", Some("x1"), "a1", serde_json::Value::Null),
    ];
    // 样本自检：确实有一条替换编辑与一条移除编辑
    let edits: Vec<&SessionEntry> = entries
        .iter()
        .filter(|entry| entry.kind() == "context_edit")
        .collect();
    assert_eq!(edits.len(), 2);
    assert!(
        matches!(edits[1], SessionEntry::ContextEdit { replacement, .. } if replacement.is_null())
    );

    let projection = build_session_projection(&entries, LeafSelector::Id("x2")).expect("投影成功");
    let user_message = projection
        .entries
        .iter()
        .find(|entry| entry.source_entry.id() == Some("u1"))
        .expect("u1 在投影里");
    assert_eq!(user_message.messages.len(), 1, "u1 被替换但仍存在");
    let text = match &user_message.messages[0] {
        pi_ai::message::Message::User { content, .. } => match content {
            pi_ai::message::UserContent::Text(text) => text.clone(),
            pi_ai::message::UserContent::Blocks(_not_blocks) => String::new(),
        },
        _not_user => String::new(),
    };
    assert_eq!(text, "改后", "context_edit 的 replacement 生效");
    // 负向：replacement 为 null 的条目必须被移除（贡献 0 条消息）
    let removed = projection
        .entries
        .iter()
        .find(|entry| entry.source_entry.id() == Some("a1"))
        .expect("a1 仍在投影里（条目本身不删，只是不贡献消息）");
    assert!(
        removed.messages.is_empty(),
        "replacement=null ⇒ 该条目不再贡献消息"
    );
}

#[test]
fn projection_messages_are_the_flat_map_of_entries() {
    let entries = vec![user("u1", None, "hi"), assistant("a1", Some("u1"), "yo")];
    let projection = build_session_projection(&entries, LeafSelector::Id("a1")).expect("投影成功");
    assert_eq!(projection.entries.len(), 2);
    let flat: Vec<_> = projection
        .entries
        .iter()
        .flat_map(|entry| entry.messages.iter().cloned())
        .collect();
    assert_eq!(
        projection.messages, flat,
        "messages 必须是各条目消息的 flatMap"
    );
    assert_eq!(projection.messages.len(), 2, "两条消息");
    // 负向：LeafSelector::Null 的投影必须是空的
    let empty = build_session_projection(&entries, LeafSelector::Null).expect("空路径也可投影");
    assert!(empty.messages.is_empty() && empty.entries.is_empty());
}

// ---------------------------------------------------------------------------
// 与 TS oracle 的投影组对拍（P5-2pre）
//
// oracle：`tools/oracle/compaction-oracle.mjs` 用**公开入口**录的 `projectionSamplePlain`
// （`buildSessionProjection` + `estimateProjectedContextTokens` + `prepareCompaction`），3 次逐字节一致。
// **本期只比不依赖 `CompactionSummary` 角色的部分**：`roles` / `entryMessageCounts` / `lastUsageIndex`
// 都因为"compaction 条目的摘要消息"还没实现（等 `pi_ai::Message` 三变体）而暂不比较——见 plan 的待办。
// ---------------------------------------------------------------------------

fn oracle_projection_sample() -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .join("fixtures/oracle/compaction-p5-1.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("读不到 oracle：{error}"));
    serde_json::from_str::<serde_json::Value>(&text).expect("oracle 是 JSON")
        ["projectionSamplePlain"]
        .clone()
}

#[test]
fn projected_cut_point_and_tokens_match_the_oracle() {
    use pi_core::agent::projection::{
        build_session_projection, estimate_projected_context_tokens, find_projected_cut_point,
    };
    let oracle = oracle_projection_sample();
    // 样本自检：oracle 的这条路径确实是 split turn，且第一条投影条目就是 compaction
    assert_eq!(
        oracle["prepared"]["firstKeptEntryId"], "a2",
        "oracle 的切点"
    );
    assert_eq!(oracle["prepared"]["isSplitTurn"], true);
    assert_eq!(oracle["entryIds"][0], "K", "投影首条是 compaction 条目");

    let entries = vec![
        user("u1", None, "第一轮问题"),
        assistant("a1", Some("u1"), "第一轮回答"),
        compaction_entry("K", Some("a1"), "a1"),
        user("u2", Some("K"), "第二轮问题"),
        assistant("a2", Some("u2"), "第二轮回答"),
    ];
    let projection = build_session_projection(&entries, LeafSelector::Id("a2")).expect("投影成功");
    // ① 条目序列与 oracle 一致（这部分不依赖摘要消息）
    let ids: Vec<&str> = projection
        .entries
        .iter()
        .filter_map(|entry| entry.source_entry.id())
        .collect();
    let expected_ids: Vec<&str> = oracle["entryIds"]
        .as_array()
        .expect("entryIds 是数组")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert_eq!(ids, expected_ids, "投影条目序列必须与 oracle 一致");

    // ② 投影 token 估算（走"最后一条 usage 有效"那条路径；`lastUsageIndex` 因摘要消息未实现而暂不比）
    let estimate = estimate_projected_context_tokens(&projection, &entries);
    assert_eq!(
        estimate.tokens,
        oracle["projectedTokens"]["tokens"]
            .as_u64()
            .expect("tokens")
    );
    assert_eq!(
        estimate.usage_tokens,
        oracle["projectedTokens"]["usageTokens"]
            .as_u64()
            .expect("usageTokens")
    );

    // ③ 投影版 cut 点：`firstKeptEntryId`/`isSplitTurn` 与 oracle 的 `prepareCompaction` 结果一致
    let cut = find_projected_cut_point(&projection.entries, 1, projection.entries.len(), 1)
        .expect("cut 计算成功");
    let kept_id = projection.entries[cut.first_kept_entry_index]
        .source_entry
        .id()
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        kept_id,
        oracle["prepared"]["firstKeptEntryId"]
            .as_str()
            .expect("firstKeptEntryId"),
        "切到的条目必须与 oracle 一致"
    );
    assert_eq!(
        cut.is_split_turn,
        oracle["prepared"]["isSplitTurn"]
            .as_bool()
            .expect("isSplitTurn")
    );
    // 负向对照：切点绝不能落在那条 compaction 上（ref `:833` 明确跳过它）
    assert!(
        !matches!(
            projection.entries[cut.first_kept_entry_index].source_entry,
            pi_core::session::SessionEntry::Compaction { .. }
        ),
        "切点不得落在 compaction 条目上"
    );
}

/// 投影组①的**完整**对拍（含 `custom`/`branchSummary`；三变体落地后可以做了）。
#[test]
fn projection_sample_with_agent_roles_matches_the_oracle() {
    use pi_core::agent::projection::{build_session_projection, estimate_projected_context_tokens};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .join("fixtures/oracle/compaction-p5-1.json");
    let oracle: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("读得到 oracle"))
            .expect("oracle 是 JSON");
    let sample = &oracle["projectionSample"];

    // 与 oracle 脚本同样的 9 条条目
    let entries = vec![
        user("u1", None, "第一轮问题"),
        assistant("a1", Some("u1"), "第一轮回答"),
        edit_entry(
            "E1",
            Some("a1"),
            "u1",
            serde_json::json!({"content": "被编辑过的问题"}),
        ),
        compaction_entry("K", Some("E1"), "a1"),
        user("u2", Some("K"), "第二轮问题"),
        entry(serde_json::json!({
            "type":"custom_message","id":"C1","parentId":"u2","timestamp":"t",
            "customType":"note","content":"扩展注入的一行","display":true,"details":{"note":"note"}
        })),
        entry(serde_json::json!({
            "type":"branch_summary","id":"B1","parentId":"C1","timestamp":"t",
            "summary":"分支摘要内容","fromId":"u1"
        })),
        user("u3", Some("B1"), "第三轮问题"),
        assistant("a3", Some("u3"), "第三轮回答"),
    ];
    let projection = build_session_projection(&entries, LeafSelector::Id("a3")).expect("投影成功");

    // 样本自检：oracle 里确实有 custom / branchSummary 两个角色（否则这条用例没覆盖到三变体）
    let oracle_roles: Vec<&str> = sample["roles"]
        .as_array()
        .expect("roles 是数组")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    for role in ["custom", "branchSummary", "compactionSummary"] {
        assert!(oracle_roles.contains(&role), "oracle 样本应含 role={role}");
    }

    let roles: Vec<&str> = projection
        .messages
        .iter()
        .map(pi_ai::message::Message::role)
        .collect();
    assert_eq!(
        roles, oracle_roles,
        "投影消息的 role 序列必须与 oracle 一致"
    );
    let ids: Vec<&str> = projection
        .entries
        .iter()
        .filter_map(|entry| entry.source_entry.id())
        .collect();
    let oracle_ids: Vec<&str> = sample["entryIds"]
        .as_array()
        .expect("entryIds 是数组")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert_eq!(ids, oracle_ids, "投影条目序列必须与 oracle 一致");
    let counts: Vec<u64> = projection
        .entries
        .iter()
        .map(|entry| entry.messages.len() as u64)
        .collect();
    let oracle_counts: Vec<u64> = sample["entryMessageCounts"]
        .as_array()
        .expect("entryMessageCounts 是数组")
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .collect();
    assert_eq!(
        counts, oracle_counts,
        "逐条目的消息数必须与 oracle 一致（含 E1=0 与 K=1）"
    );
    let estimate = estimate_projected_context_tokens(&projection, &entries);
    assert_eq!(
        estimate.tokens,
        sample["projectedTokens"]["tokens"]
            .as_u64()
            .expect("tokens"),
        "投影 token 估算"
    );
    assert_eq!(
        estimate.last_usage_index.map(|index| index as u64),
        sample["projectedTokens"]["lastUsageIndex"].as_u64(),
        "最后一条 usage 的索引（现在含摘要消息，索引会整体后移）"
    );
}

#[test]
fn messages_for_compaction_skip_system_and_compaction_entries() {
    // ref `compaction.ts:97-101`：compaction 条目 → []；其余条目过滤掉 system。
    use pi_core::agent::projection::get_messages_from_projected_entry_for_compaction;
    // 注意：`firstKeptEntryId` **之前**的 system 消息会被 `buildContextEntries` 丢掉（ref `:506`），
    // 所以 system 必须放在 compaction 条目**之后**才能在投影里看到。
    let entries = vec![
        system_entry("s0", None, "sys"),
        user("u1", Some("s0"), "问题"),
        compaction_entry("k0", Some("u1"), "u1"),
        system_entry("s1", Some("k0"), "sys-after"),
    ];
    let projection = build_session_projection(&entries, LeafSelector::Id("s1")).expect("投影成功");
    // 样本自检：投影里确实有一条 system 消息与一条 compaction 条目
    assert!(
        projection
            .messages
            .iter()
            .any(|message| matches!(message, pi_ai::message::Message::System { .. })),
        "样本要含 system 消息"
    );
    let compaction_projected = projection
        .entries
        .iter()
        .find(|entry| entry.source_entry.kind() == "compaction")
        .expect("有 compaction 条目");
    assert!(
        get_messages_from_projected_entry_for_compaction(compaction_projected).is_empty(),
        "compaction 条目不参与待摘要消息"
    );
    let user_projected = projection
        .entries
        .iter()
        .find(|entry| entry.source_entry.id() == Some("u1"))
        .expect("u1 在投影里");
    assert!(
        get_messages_from_projected_entry_for_compaction(user_projected)
            .iter()
            .all(|message| !matches!(message, pi_ai::message::Message::System { .. })),
        "普通条目要过滤掉 system"
    );
    // 负向对照：非 compaction 条目返回的条数与入口消息数一致（此处 u1 只有 1 条）
    assert_eq!(
        get_messages_from_projected_entry_for_compaction(user_projected).len(),
        user_projected.messages.len(),
        "u1 的消息里没有 system ⇒ 过滤前后条数相同"
    );
}

/// P5-2a：`prepare_compaction` 与 oracle 的 `presumably` 字段逐项对拍（两组样本）。
#[test]
fn prepare_compaction_matches_the_oracle() {
    use pi_core::agent::compaction::{prepare_compaction, CompactionSettings};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .join("fixtures/oracle/compaction-p5-1.json");
    let oracle: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("读得到 oracle"))
            .expect("oracle 是 JSON");

    // 与 oracle 脚本一致：keepRecentTokens = 1
    let settings = CompactionSettings {
        keep_recent_tokens: 1,
        ..pi_core::agent::compaction::DEFAULT_COMPACTION_SETTINGS
    };

    let assistant_entry = |id: &str, parent: Option<&str>, text: &str| assistant(id, parent, text);
    let plain = vec![
        user("u1", None, "第一轮问题"),
        assistant_entry("a1", Some("u1"), "第一轮回答"),
        compaction_entry("K", Some("a1"), "a1"),
        user("u2", Some("K"), "第二轮问题"),
        assistant_entry("a2", Some("u2"), "第二轮回答"),
    ];
    let with_roles = vec![
        user("u1", None, "第一轮问题"),
        assistant_entry("a1", Some("u1"), "第一轮回答"),
        edit_entry(
            "E1",
            Some("a1"),
            "u1",
            serde_json::json!({"content": "被编辑过的问题"}),
        ),
        compaction_entry("K", Some("E1"), "a1"),
        user("u2", Some("K"), "第二轮问题"),
        entry(serde_json::json!({
            "type":"custom_message","id":"C1","parentId":"u2","timestamp":"t",
            "customType":"note","content":"扩展注入的一行","display":true,"details":{"note":"note"}
        })),
        entry(serde_json::json!({
            "type":"branch_summary","id":"B1","parentId":"C1","timestamp":"t",
            "summary":"分支摘要内容","fromId":"u1"
        })),
        user("u3", Some("B1"), "第三轮问题"),
        assistant_entry("a3", Some("u3"), "第三轮回答"),
    ];

    for (sample_key, entries) in [
        ("projectionSamplePlain", &plain),
        ("projectionSample", &with_roles),
    ] {
        let expected = &oracle[sample_key]["prepared"];
        // 样本自检：oracle 里确实有这些字段
        assert!(
            expected["firstKeptEntryId"].is_string(),
            "{sample_key} 缺 firstKeptEntryId"
        );
        let prepared = prepare_compaction(entries, &settings)
            .expect("准备成功")
            .unwrap_or_else(|| panic!("{sample_key} 不应返回 None"));
        assert_eq!(
            prepared.first_kept_entry_id,
            expected["firstKeptEntryId"]
                .as_str()
                .expect("firstKeptEntryId"),
            "{sample_key}: firstKeptEntryId"
        );
        assert_eq!(
            prepared.is_split_turn,
            expected["isSplitTurn"].as_bool().expect("isSplitTurn"),
            "{sample_key}: isSplitTurn"
        );
        assert_eq!(
            prepared.tokens_before,
            expected["tokensBefore"].as_u64().expect("tokensBefore"),
            "{sample_key}: tokensBefore"
        );
        assert_eq!(
            prepared.previous_summary.as_deref(),
            expected["previousSummary"].as_str(),
            "{sample_key}: previousSummary"
        );
        let roles: Vec<&str> = prepared
            .messages_to_summarize
            .iter()
            .map(pi_ai::message::Message::role)
            .collect();
        let expected_roles: Vec<&str> = expected["messagesToSummarizeRoles"]
            .as_array()
            .expect("messagesToSummarizeRoles 是数组")
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        assert_eq!(
            roles, expected_roles,
            "{sample_key}: messagesToSummarize 的角色序列"
        );
        let prefix_roles: Vec<&str> = prepared
            .turn_prefix_messages
            .iter()
            .map(pi_ai::message::Message::role)
            .collect();
        let expected_prefix: Vec<&str> = expected["turnPrefixRoles"]
            .as_array()
            .expect("turnPrefixRoles 是数组")
            .iter()
            .filter_map(|value| value.as_str())
            .collect();
        assert_eq!(
            prefix_roles, expected_prefix,
            "{sample_key}: turnPrefixMessages 的角色序列"
        );
        // fileOps：三个集合都要与 oracle 一致（两组样本都为空，但断言不能省）
        let file_ops = &expected["fileOps"];
        assert!(
            !file_ops.is_null(),
            "{sample_key}: oracle 样本必须带 fileOps"
        );
        let as_vec = |key: &str| -> Vec<String> {
            file_ops[key]
                .as_array()
                .expect("fileOps 是数组")
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        };
        assert_eq!(
            prepared
                .file_ops
                .read
                .iter()
                .cloned()
                .collect::<Vec<String>>(),
            as_vec("read"),
            "{sample_key}: fileOps.read"
        );
        assert_eq!(
            prepared
                .file_ops
                .edited
                .iter()
                .cloned()
                .collect::<Vec<String>>(),
            as_vec("edited"),
            "{sample_key}: fileOps.edited"
        );
        assert_eq!(
            prepared
                .file_ops
                .written
                .iter()
                .cloned()
                .collect::<Vec<String>>(),
            as_vec("written"),
            "{sample_key}: fileOps.written"
        );
    }
}

/// P5-2b：从 **golden 会话文件**做投影是**不可行**的——golden 的 id/parentId/firstKeptEntryId 都是
/// 按出现顺序生成的占位符（`<ID16>` 在文件里没有对应条目），树结构在归一化时丢了。
/// 这条用例把"两侧都必须拒绝"钉住（TS oracle 的 `goldenS21Sample.prepared === null` 是同一结论）。
#[test]
fn golden_session_cannot_be_projected_because_ids_are_placeholders() {
    use pi_core::agent::compaction::{prepare_compaction, CompactionSettings};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf();
    let session =
        std::fs::read_to_string(root.join("golden/S21/session.jsonl")).expect("golden 会话");
    let entries: Vec<pi_core::session::SessionEntry> = session
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect();
    // 样本自检：golden 里确实有 compaction 条目，且它的 firstKeptEntryId 指向一个**不存在的** id
    let compaction = entries
        .iter()
        .find(|entry| entry.kind() == "compaction")
        .expect("golden 里有 compaction 条目");
    let pi_core::session::SessionEntry::Compaction {
        first_kept_entry_id,
        ..
    } = compaction
    else {
        panic!("类型不对");
    };
    assert!(
        !entries
            .iter()
            .any(|entry| entry.id() == Some(first_kept_entry_id.as_str())),
        "样本自检：firstKeptEntryId 应当是占位符（匹配不到任何条目）"
    );
    // 与 TS oracle 同样的设置，两侧都必须给 `None`（不是"能压但压错"）
    let prepared = prepare_compaction(
        &entries,
        &CompactionSettings {
            enabled: true,
            reserve_tokens: 1000,
            keep_recent_tokens: 2000,
        },
    )
    .expect("不报错");
    let oracle = oracle_projection_sample();
    let _ = oracle; // 该样本只用于其它用例；这里显式忽略
    assert!(
        prepared.is_none(),
        "golden 的占位符 id 让投影不可行 ⇒ 必须给 None（与 TS oracle 的 prepared=null 一致）"
    );
}

#[test]
fn summarization_prompt_assembly_matches_the_ref_rules() {
    // ref `compaction.ts:734-755`：maxTokens 取 `min(floor(0.8*reserve), model.maxTokens || ∞)`；
    // 文本 = `<conversation>` 包裹 + 可选 `<previous-summary>` + 基础提示词（+ 自定义聚焦）。
    use pi_core::agent::compaction::{build_summarization_prompt, summarization_max_tokens};
    use pi_core::agent::compaction_prompts::{update_summarization_prompt, SUMMARIZATION_PROMPT};
    // ① maxTokens：reserve 16384 ⇒ floor(13107.2)=13107；模型上限 4096 更小 ⇒ 取 4096
    assert_eq!(summarization_max_tokens(16384, 4096), 4096);
    // 模型上限为 0（未知）⇒ 用 reserve 那一侧
    assert_eq!(summarization_max_tokens(16384, 0), 13107);

    let messages = vec![pi_ai::message::Message::User {
        content: pi_ai::message::UserContent::Text("问题".to_string()),
        timestamp: 0,
        extra: serde_json::Map::new(),
    }];
    // ② 初次摘要：`<conversation>` 包裹 + 主提示词
    let plain = build_summarization_prompt(&messages, 16384, 4096, None, None);
    assert!(plain
        .prompt_text
        .starts_with("<conversation>\n[User]: 问题\n</conversation>\n\n"));
    assert!(
        plain.prompt_text.ends_with(SUMMARIZATION_PROMPT),
        "主提示词收尾"
    );
    assert!(
        !plain.prompt_text.contains("<previous-summary>"),
        "没有前摘要时不得出现该段"
    );
    assert_eq!(plain.max_tokens, 4096);
    // ③ 增量摘要：UPDATE 提示词 + `<previous-summary>` 段
    let update = build_summarization_prompt(&messages, 16384, 4096, None, Some("旧摘要"));
    assert!(update
        .prompt_text
        .contains("<previous-summary>\n旧摘要\n</previous-summary>"));
    assert!(
        update.prompt_text.ends_with(&update_summarization_prompt()),
        "UPDATE 提示词收尾"
    );
    // ④ 自定义聚焦：追加 "\n\nAdditional focus: …"，且在提示词之后
    let focused = build_summarization_prompt(&messages, 16384, 4096, Some("只看错误"), None);
    assert!(focused
        .prompt_text
        .ends_with("\n\nAdditional focus: 只看错误"));
    assert!(
        focused.prompt_text.contains(SUMMARIZATION_PROMPT),
        "基础提示词仍在"
    );
    // ⑤ 负向对照：把 agent 角色折成 user 后再序列化（custom 也要出现在 <conversation> 里）
    let with_custom = vec![pi_ai::message::Message::Custom {
        custom_type: "note".to_string(),
        content: pi_ai::message::UserContent::Text("自定义".to_string()),
        display: true,
        details: serde_json::Value::Null,
        timestamp: 0,
        extra: serde_json::Map::new(),
    }];
    let folded = build_summarization_prompt(&with_custom, 1000, 100, None, None);
    assert!(
        !folded.prompt_text.contains("role"),
        "不得把角色字段写进文本"
    );
    assert!(folded.prompt_text.contains("<conversation>"), "仍然有包裹");
}

/// P5-2c 的纯逻辑：`combineUsage` 与 `getSummarizationFailure`（ref `:115-136` 与 `:607-615`）。
#[test]
fn combine_usage_and_failure_match_the_ref_rules() {
    use pi_core::agent::compaction::{combine_usage, get_summarization_failure};
    let usage = |input: u64, output: u64| pi_ai::Usage {
        input,
        output,
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: None,
        reasoning: None,
        total_tokens: input + output,
        cost: pi_ai::Cost::default(),
    };
    let combined = combine_usage(&usage(10, 2), &usage(3, 4));
    assert_eq!(combined.input, 13);
    assert_eq!(combined.output, 6);
    assert_eq!(combined.total_tokens, 19);
    // 可选字段：两侧都没有 ⇒ 结果里也不出现（TS 的对象字面量语义）
    assert_eq!(combined.cache_write_1h, None);
    assert_eq!(combined.reasoning, None);
    // 一侧有 ⇒ 出现且按 0 补另一侧
    let mut with_1h = usage(1, 1);
    with_1h.cache_write_1h = Some(5);
    assert_eq!(
        combine_usage(&with_1h, &usage(1, 1)).cache_write_1h,
        Some(5)
    );

    // 失败判定（ref `:607-615`）
    assert_eq!(
        get_summarization_failure(&pi_ai::StopReason::Error, Some("boom"), "Summarization"),
        Some("Summarization failed: boom".to_string())
    );
    assert_eq!(
        get_summarization_failure(&pi_ai::StopReason::Error, None, "Summarization"),
        Some("Summarization failed: Unknown error".to_string())
    );
    assert!(
        get_summarization_failure(&pi_ai::StopReason::Length, None, "Summarization")
            .unwrap_or_default()
            .contains("token cap"),
        "length 的摘要不完整"
    );
    // 负向对照：正常停止不是失败
    assert_eq!(
        get_summarization_failure(&pi_ai::StopReason::Stop, None, "Summarization"),
        None
    );
}

/// P5-2c：`summarization_options`（ref `compaction.ts:617-632`/`:651-655`）。
#[test]
fn summarization_options_follow_the_ref_rules() {
    use pi_core::agent::compaction::summarization_options;
    // ① 推理模型 + 等级 high ⇒ 带 reasoning
    let options = summarization_options(4096, true, Some("high"));
    assert_eq!(options.reasoning.as_deref(), Some("high"));
    assert!(
        options.cache_retention_none,
        "摘要请求必须关掉 prompt cache（ref :653）"
    );
    assert_eq!(options.max_tokens, 4096);
    // ② 负向：等级 off ⇒ 不带（ref `:630` 的 `!== "off"`）
    assert_eq!(
        summarization_options(4096, true, Some("off")).reasoning,
        None
    );
    // ③ 负向：非推理模型 ⇒ 即使给了等级也不带（ref `:628` 的 `model.reasoning &&`）
    assert_eq!(
        summarization_options(4096, false, Some("high")).reasoning,
        None
    );
    // ④ 负向：没有等级 ⇒ 不带
    assert_eq!(summarization_options(4096, true, None).reasoning, None);
}

/// P5-3a 的触发判定（ref `agent-session.ts:2699-2734`）。
#[test]
fn threshold_outcome_follows_the_ref_rules() {
    use pi_core::agent::compaction::{threshold_outcome, CompactionSettings, ThresholdOutcome};
    let settings = CompactionSettings {
        enabled: true,
        reserve_tokens: 200,
        keep_recent_tokens: 0,
    };
    let entries = vec![
        user("u1", None, "问题"),
        assistant("a1", Some("u1"), "回答"),
    ];
    let projection = build_session_projection(&entries, LeafSelector::Id("a1")).expect("投影成功");
    let usage = pi_ai::Usage {
        input: 900,
        output: 100,
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: None,
        reasoning: None,
        total_tokens: 1000,
        cost: pi_ai::Cost::default(),
    };
    // ① 直接用 usage：1000 > 1000-200 ⇒ 要压缩
    assert_eq!(
        threshold_outcome(
            &projection,
            &entries,
            Some(&usage),
            false,
            1000,
            &settings,
            None,
            None
        ),
        ThresholdOutcome::Compact
    );
    // ② 缩小到阈值以下 ⇒ 不压缩
    let small = pi_ai::Usage {
        input: 1,
        output: 1,
        // 必须显式覆盖：`calculate_context_tokens` 优先用 totalTokens（ref `compaction.ts:162-164`）
        total_tokens: 2,
        cost: pi_ai::Cost::default(),
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: None,
        reasoning: None,
    };
    assert_eq!(
        threshold_outcome(
            &projection,
            &entries,
            Some(&small),
            false,
            1000,
            &settings,
            None,
            None
        ),
        ThresholdOutcome::Skip
    );
    // ④ error 停止 ⇒ 走消息体估算（这里会话很小 ⇒ 不压缩）。
    // 注意样本必须自洽：那条助手消息本身也得是 `stopReason: "error"`，否则它的 usage
    // 仍会被 `get_assistant_usage` 当成有效用量（`compaction.ts:170-183` 只跳过 aborted/error）。
    let error_entries = vec![
        user("u1", None, "问题"),
        entry(serde_json::json!({
            "type":"message","id":"a1","parentId":"u1","timestamp":"t",
            "message":{"role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock",
                       "model":"mock-model","responseModel":null,"endTurn":null,
                       "errorMessage":"boom","rawStopReason":null,
                       "content":[{"type":"text","text":"半截回答"}],
                       "usage":{"input":900,"output":100,"cacheRead":0,"cacheWrite":0,"totalTokens":1000,
                                "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
                       "stopReason":"error"}
        })),
    ];
    let error_projection =
        build_session_projection(&error_entries, LeafSelector::Id("a1")).expect("投影成功");
    assert_eq!(
        threshold_outcome(
            &error_projection,
            &error_entries,
            Some(&usage),
            true,
            1000,
            &settings,
            None,
            None
        ),
        ThresholdOutcome::Skip,
        "error 时改看消息体估算（ref `:2709-2727`）"
    );
    // ⑤ 负向：同一 error 样本，但估算命中的 usage 比最近一次 compaction **旧**（ref `:2719-2725`）
    // ⇒ 必须提前 return false（"陈旧 usage 不得在刚压完后又触发一次"）。
    // 该分支只在 error/零用量路径上生效（直接用 usage 的那条路径不做这个检查）。
    let tokens_with_stale_usage = threshold_outcome(
        &error_projection,
        &error_entries,
        Some(&usage),
        true,
        1000,
        &settings,
        Some(2000),
        Some(1000),
    );
    assert_eq!(
        tokens_with_stale_usage,
        ThresholdOutcome::Skip,
        "陈旧 usage 不得立刻再次触发压缩（ref `:2719-2725`）"
    );
}

/// P5-3a：`compact()` 的编排（两次调用、拼接、用量合并、details、上限系数）。
#[test]
fn compact_orchestrates_split_turn_like_the_ref() {
    use pi_core::agent::compaction::{
        compact, prepare_compaction, turn_prefix_max_tokens, CompactionSettings,
    };
    // 与之前的样本一致：末尾放助手 ⇒ split turn
    let entries = vec![
        user("u1", None, "第一轮问题"),
        assistant("a1", Some("u1"), "第一轮回答"),
        compaction_entry("K", Some("a1"), "a1"),
        user("u2", Some("K"), "第二轮问题"),
        user("u3", Some("u2"), "第三轮问题"),
        assistant("a3", Some("u3"), "第三轮回答"),
    ];
    let settings = CompactionSettings {
        keep_recent_tokens: 1,
        ..pi_core::agent::compaction::DEFAULT_COMPACTION_SETTINGS
    };
    let prepared = prepare_compaction(&entries, &settings)
        .expect("准备成功")
        .expect("应当可压缩");
    assert!(prepared.is_split_turn, "样本自检：必须构造出 split turn");
    assert!(
        !prepared.turn_prefix_messages.is_empty(),
        "样本自检：要有 turn 前缀消息"
    );

    // 假的摘要器：按调用顺序返回可区分的结果，并记录收到的提示词与上限
    let mut calls: Vec<(String, u64)> = Vec::new();
    let mut call_index = 0u32;
    let mut fake = |prompt: &str, max_tokens: u64| -> Result<(String, pi_ai::Usage), String> {
        calls.push((prompt.to_string(), max_tokens));
        call_index += 1;
        let usage = pi_ai::Usage {
            input: 10 * u64::from(call_index),
            output: 1,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: None,
            reasoning: None,
            total_tokens: 10 * u64::from(call_index) + 1,
            cost: pi_ai::Cost::default(),
        };
        Ok((format!("摘要{call_index}"), usage))
    };
    let result = compact(&prepared, 4096, None, &mut fake).expect("压缩成功");

    // ① 两次调用：主/更新提示词 + turn-prefix 提示词（后者用 `# Conversation` 包装）
    assert_eq!(calls.len(), 2, "split turn 调两次摘要");
    let prefix_call = calls
        .iter()
        .find(|(prompt, _max)| prompt.starts_with("# Conversation"))
        .expect("有一次 turn-prefix 摘要");
    assert!(
        prefix_call.0.contains("# Instructions"),
        "turn-prefix 提示词的第二段是指令"
    );
    // ② 上限系数：主摘要 0.8、前缀 0.5（都以 reserveTokens 为基数、再被模型上限夹住）
    let (_, prefix_max) = prefix_call.clone();
    assert_eq!(
        prefix_max,
        turn_prefix_max_tokens(settings.reserve_tokens, 4096)
    );
    assert_eq!(
        prefix_max,
        4096.min((settings.reserve_tokens as f64 * 0.5).floor() as u64)
    );
    // ③ 拼接模板（ref `:1054-1055`）：历史部分先取 `previousSummary ?? "No prior history."`，
    //    但**有待摘要历史时**会被那次调用的文本替换（ref `:1036` 的 `historyText = historyResult.text`）；
    //    再接 `\n\n---\n\n**Turn Context (split turn):**\n\n` + 前缀摘要文本。
    assert_eq!(
        result.summary, "摘要1\n\n---\n\n**Turn Context (split turn):**\n\n摘要2",
        "历史那次是第 1 次调用、前缀是第 2 次"
    );
    // ④ 调用次数：有待摘要历史（1）+ turn 前缀（1）
    assert!(
        !prepared.messages_to_summarize.is_empty(),
        "样本自检：本样本两条消息都要摘要"
    );
    assert_eq!(calls.len(), 2, "split turn 且历史非空 ⇒ 两次调用");
    // ⑤ 用量按 combineUsage 合并：10/1 与 20/1 ⇒ input 30、total 32
    assert_eq!(result.usage.input, 30, "两次调用的 usage 要合并");
    assert_eq!(result.usage.total_tokens, 32);
    // ⑥ details 来自 fileOps（本样本没有文件操作 ⇒ 两个列表都空）
    assert!(result.details.read_files.is_empty() && result.details.modified_files.is_empty());
    assert_eq!(result.tokens_before, prepared.tokens_before);
}

/// P5-3c：`overflow_outcome` 的十步判定（每条守卫一个反例）。
#[test]
fn overflow_outcome_follows_the_ref_guards() {
    use pi_core::agent::compaction::{
        overflow_outcome, AutoCompactionDecision, CompactionSettings, OverflowInputs,
        DEFAULT_COMPACTION_SETTINGS,
    };
    use pi_core::agent::projection::{build_session_projection, LeafSelector};

    // 助手消息（可覆盖字段）：默认 `stop` + 用量 100（远小于窗口）
    fn assistant_message(overrides: serde_json::Value) -> pi_ai::AssistantMessage {
        let mut base = serde_json::json!({
            "role": "assistant", "timestamp": 5000,
            "api": "openai-completions", "provider": "mock", "model": "mock-model",
            "responseModel": null, "endTurn": null, "errorMessage": null, "rawStopReason": null,
            "content": [{"type": "text", "text": "x"}],
            "usage": {"input": 100, "output": 10, "cacheRead": 0, "cacheWrite": 0,
                      "reasoning": 0, "totalTokens": 110,
                      "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "stop"
        });
        if let (Some(base), Some(over)) = (base.as_object_mut(), overrides.as_object()) {
            for (key, value) in over {
                base.insert(key.clone(), value.clone());
            }
        }
        match serde_json::from_value::<pi_ai::message::Message>(base).expect("助手消息可解析")
        {
            pi_ai::message::Message::Assistant(assistant) => *assistant,
            _other => panic!("样本自检：必须是助手消息"),
        }
    }

    let entries = vec![
        user("u1", None, "问题"),
        assistant("a1", Some("u1"), "回答"),
    ];
    let projection =
        build_session_projection(&entries, LeafSelector::Unspecified).expect("投影成功");
    let settings = CompactionSettings {
        keep_recent_tokens: 1,
        ..DEFAULT_COMPACTION_SETTINGS
    };
    // 显式生命周期的函数（闭包无法把返回值的生命周期与参数统一）
    fn base<'a>(
        assistant: &'a pi_ai::AssistantMessage,
        projection: &'a pi_core::agent::projection::SessionProjection,
        entries: &'a [pi_core::session::SessionEntry],
        settings: &'a CompactionSettings,
    ) -> OverflowInputs<'a> {
        OverflowInputs {
            assistant,
            model_provider: "mock",
            model_id: "mock-model",
            context_window: 20_000,
            model_max_tokens: 1024,
            projection,
            path_entries: entries,
            settings,
            last_compaction_timestamp: None,
            skip_aborted_check: true,
            overflow_recovery_attempted: false,
        }
    }

    // ① 正常消息（用量 100 << 窗口 20000，阈值也够不到）⇒ 阈值分支（由 threshold_outcome 再判 Skip）
    let plain = assistant_message(serde_json::json!({}));
    assert_eq!(
        overflow_outcome(&base(&plain, &projection, &entries, &settings)),
        AutoCompactionDecision::Threshold
    );

    // ② 关掉压缩 ⇒ Skip
    let disabled = CompactionSettings {
        enabled: false,
        ..settings
    };
    let mut inputs = base(&plain, &projection, &entries, &settings);
    inputs.settings = &disabled;
    assert_eq!(overflow_outcome(&inputs), AutoCompactionDecision::Skip);

    // ③ aborted：`skip_aborted_check=true` ⇒ Skip；传 `false`（提示词前那次）⇒ 照常往下判
    let aborted = assistant_message(serde_json::json!({"stopReason": "aborted"}));
    assert_eq!(
        overflow_outcome(&base(&aborted, &projection, &entries, &settings)),
        AutoCompactionDecision::Skip
    );
    let mut inputs = base(&aborted, &projection, &entries, &settings);
    inputs.skip_aborted_check = false;
    assert_eq!(overflow_outcome(&inputs), AutoCompactionDecision::Threshold);

    // ④ **换模型**后的溢出不算溢出（provider/model 不一致）⇒ 走阈值分支
    let overflowed = assistant_message(serde_json::json!({
        "usage": {"input": 25000, "output": 10, "cacheRead": 0, "cacheWrite": 0,
                  "reasoning": 0, "totalTokens": 25010,
                  "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}}
    }));
    let mut inputs = base(&overflowed, &projection, &entries, &settings);
    inputs.model_id = "other-model";
    assert_eq!(
        overflow_outcome(&inputs),
        AutoCompactionDecision::Threshold,
        "不同模型的溢出不得触发压缩"
    );

    // ⑤ 同模型 + 用量超窗口 + 停因 `stop` ⇒ Case 2（压缩不重试）
    assert_eq!(
        overflow_outcome(&base(&overflowed, &projection, &entries, &settings)),
        AutoCompactionDecision::OverflowNoRetry
    );

    // ⑥ 停因不是 `stop`（这里用 `length` + 输出远低于期望上限 ⇒ 可恢复截断）⇒ Case 1 重试
    let truncated = assistant_message(serde_json::json!({
        "stopReason": "length",
        "usage": {"input": 25000, "output": 5, "cacheRead": 0, "cacheWrite": 0,
                  "reasoning": 0, "totalTokens": 25005,
                  "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}}
    }));
    assert_eq!(
        overflow_outcome(&base(&truncated, &projection, &entries, &settings)),
        AutoCompactionDecision::OverflowRetry
    );
    // ⑦ 已经试过一次恢复 ⇒ 不再压缩，只报失败
    let mut inputs = base(&truncated, &projection, &entries, &settings);
    inputs.overflow_recovery_attempted = true;
    assert_eq!(
        overflow_outcome(&inputs),
        AutoCompactionDecision::OverflowExhausted
    );

    // ⑧ 助手消息早于压缩边界（时间戳更小）⇒ Skip（压缩后不得被陈旧消息再次触发）
    let mut inputs = base(&overflowed, &projection, &entries, &settings);
    inputs.last_compaction_timestamp = Some("1970-01-01T00:00:05.001Z"); // 5001ms > 5000ms
    assert_eq!(
        overflow_outcome(&inputs),
        AutoCompactionDecision::Skip,
        "压缩边界之前的助手消息不得触发"
    );
    // 反证：把边界放到消息之前，就应当触发（证明上面那条不是因为别的原因）
    let mut inputs = base(&overflowed, &projection, &entries, &settings);
    inputs.last_compaction_timestamp = Some("1970-01-01T00:00:04.999Z"); // 4999ms < 5000ms
    assert_eq!(
        overflow_outcome(&inputs),
        AutoCompactionDecision::OverflowNoRetry
    );
}

/// D1：`getToolStateChanges`（ref `packages/ai/src/utils/transcript.ts:150-167`）。
#[test]
fn tool_state_changes_follow_the_ref_rules() {
    use pi_core::agent::tool_state_changes;
    let tool = |name: &str, description: &str| {
        serde_json::json!({
            "name": name, "description": description,
            "parameters": {"type": "object"}, "constrainedSampling": {"type": "json_schema", "strict": "prefer"}
        })
    };

    // ① 完全相同 ⇒ 既不新增也不移除（ref 的 `getToolStateChanges([a],[a])` 用例）
    let (added, removed) = tool_state_changes(&[tool("a", "d")], &[tool("a", "d")]);
    assert!(added.is_empty() && removed.is_empty(), "无变化时两侧都空");

    // ② 新增：上一份没有 ⇒ 只进 toolsAdded
    let (added, removed) = tool_state_changes(&[tool("a", "d")], &[tool("a", "d"), tool("b", "d")]);
    assert_eq!(added.len(), 1, "新增的进 toolsAdded");
    assert_eq!(added[0]["name"], "b");
    assert!(removed.is_empty());

    // ③ 移除：当前没有 ⇒ 只进 toolsRemoved（且只取 name）
    let (added, removed) = tool_state_changes(&[tool("a", "d"), tool("b", "d")], &[tool("a", "d")]);
    assert!(added.is_empty());
    assert_eq!(removed, vec![serde_json::json!({"name": "b"})], "只取 name");

    // ④ **声明变了 ⇒ 两侧都算**（这正是 golden/S20c 的情形：种子里 read 是精简声明）
    let (added, removed) = tool_state_changes(&[tool("a", "d1")], &[tool("a", "d2")]);
    assert_eq!(added.len(), 1, "声明变化也算 toolsAdded");
    assert_eq!(
        removed,
        vec![serde_json::json!({"name": "a"})],
        "声明变化也算 toolsRemoved"
    );

    // ⑤ **已知限制（写机制）**：ref 的 `declarationsEqual` 用 `JSON.stringify`，**键序参与判等**；
    //    而 pi-rs 的 `serde_json::Value` 是 `BTreeMap`，序列化时键序已排序 ⇒ **仅键序不同**的声明
    //    变化在这里判不出来（语义变化判得出来，见 ④——golden/S20c 属语义变化，不受影响）。
    //    要覆盖它需像 `append_with_raw_system_message` 那样用**原文**比对（随 P5 后续处理）。
    let reordered = serde_json::json!({
        "description": "d", "name": "a", "parameters": {"type": "object"},
        "constrainedSampling": {"strict": "prefer", "type": "json_schema"}
    });
    let (added, removed) = tool_state_changes(&[tool("a", "d")], &[reordered]);
    assert_eq!(
        (added.len(), removed.len()),
        (0, 0),
        "仅键序不同时判为无变化（`Value` 已排序；语义变化仍能判出，见 ④）"
    );
}

/// P5-8/F2.2：S13e 形状——助手被 `context_edit` 省略后，`prepare_compaction` 还必须能给出
/// **split-turn** 方案（`messagesToSummarize` 空、`turnPrefixMessages` 非空）。
///
/// 取证依据：`golden/S13e` 的 compaction 条目 `summary = "No prior history.\n\n---\n\n
/// **Turn Context (split turn):**…"`、`tokensBefore = 1402`，摘要请求的 user 文本是
/// `# Conversation\n[User]: 第一轮：说点长内容`（ref `compaction.ts:1016-1055` 的 split 分支）。
#[test]
fn omitted_assistant_still_yields_a_split_turn_preparation() {
    use pi_core::agent::compaction::{
        prepare_compaction, CompactionSettings, DEFAULT_COMPACTION_SETTINGS,
    };
    // 单回合：user → assistant(error) → context_edit 把该 assistant 换成 null（= 省略）
    let entries = vec![
        user("u1", None, "第一轮：说点长内容"),
        entry(serde_json::json!({
            "type":"message","id":"a1","parentId":"u1","timestamp":"t",
            "message":{"role":"assistant","timestamp":0,"api":"openai-completions","provider":"mock",
                       "model":"mock-model","responseModel":null,"endTurn":null,
                       "errorMessage":"the request exceeds the available context size",
                       "rawStopReason":null,"content":[{"type":"text","text":""}],
                       "usage":{"input":25000,"output":0,"cacheRead":0,"cacheWrite":0,"reasoning":0,
                                "totalTokens":25000,
                                "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
                       "stopReason":"error"}
        })),
        edit_entry("e1", Some("a1"), "a1", serde_json::Value::Null),
    ];
    let settings = CompactionSettings {
        keep_recent_tokens: 1,
        ..DEFAULT_COMPACTION_SETTINGS
    };
    let prepared = prepare_compaction(&entries, &settings).expect("不该报错");
    let prepared = prepared.expect("省略失败助手后仍应能给出方案（golden/S13e 即 split-turn）");
    assert!(prepared.is_split_turn, "应为 split turn");
    assert!(
        prepared.messages_to_summarize.is_empty(),
        "历史应为空（golden 的 summary 以 No prior history. 开头）"
    );
    assert!(
        !prepared.turn_prefix_messages.is_empty(),
        "turn 前缀应非空（golden 摘要请求的对话段就是那条 user 消息）"
    );
}
