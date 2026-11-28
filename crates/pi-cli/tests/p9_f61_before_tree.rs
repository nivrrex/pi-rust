//! F61/38：`session_before_tree`（ref `agent-session.ts:3641-3656`）。
//!
//! 此前 pi-rs 完全不派发该事件（LIMITATIONS 27a）。扩展据此可以：
//! - `{cancel:true}` ⇒ 放弃本次 `/tree` 导航；
//! - `{summary}` ⇒ 直接给出分支摘要，**省掉一次模型调用**（ref 的 `fromExtension`）。
//!
//! 本用例验证 `ext_tools::emit_session_before_tree` 的三个分支（取消 / 给摘要 /
//! 不介入），那正是 TUI 分派处依赖的契约；`/tree` 交互路径由既有 T52 场景覆盖。

use std::path::Path;
use std::process::Command;

fn node_path() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| std::path::PathBuf::from(text))
}

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 安装指定 fixture 作为扩展。
///
/// 用**不同文件**而不是同一夹具 + 环境变量分流：扩展宿主进程只继承白名单环境
/// （`ext_tools.rs` 显式列出的那几项），自定义变量传不进去，夹具读到的永远是
/// `undefined`（实测三路都成了"不介入"）。
fn install_probe(agent: &Path, fixture: &str) {
    let target = agent.join("extensions/before-tree");
    std::fs::create_dir_all(&target).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures").join(fixture))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 装 fixture **并重启宿主**，然后派发一次。
///
/// 必须重启：扩展宿主是进程级单例（`HOST`），同一进程里多次调用会复用第一个
/// 宿主（连带它的 fixture）⇒ 后面几路测的根本不是自己的扩展。
/// 最初没重启，三路结果完全一致（全都不介入），正是这个原因。
fn probe(fixture: &str) -> Option<pi_cli::ext_tools::BeforeTree> {
    let node = node_path()?;
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), fixture);
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    // 先让宿主启动成功（`extension_tools` 首次调用会拉起 node），再重启到本路的
    // agentDir——直接 `reload_host` 时若还没有宿主，`start_host` 拿到的上下文不完整。
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    Some(pi_cli::ext_tools::emit_session_before_tree(
        Some("target-1"),
        Some("from-1"),
    ))
}

/// 三个分支必须**串行**（一个 `#[test]` 内依次跑）。
///
/// 扩展宿主是进程级单例，三个 `#[test]` 并行时会互相抢句柄（实测并行下
/// "不介入"那条失败）。这与 `p9_f1_tool_call_input` 的教训完全相同。
#[test]
fn session_before_tree_semantics() {
    // 取消。
    let Some(cancel) = probe("before-tree-ext.mjs") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        cancel.cancel,
        "扩展返回 {{cancel:true}} 时导航应被取消：{cancel:?}"
    );
    assert_eq!(cancel.summary, None, "取消时不应同时给摘要：{cancel:?}");

    // 给摘要：省掉模型调用，且不取消。
    let summary = probe("before-tree-summary-ext.mjs").expect("node 在");
    assert!(!summary.cancel, "给了摘要不应同时取消：{summary:?}");
    assert_eq!(summary.summary.as_deref(), Some("EXTENSION-SUMMARY"));

    // 不介入：两者都为空（等同 ref 无 handler 的行为）。
    let noop = probe("before-tree-noop-ext.mjs").expect("node 在");
    assert!(!noop.cancel, "默认不应取消：{noop:?}");
    assert_eq!(noop.summary, None, "默认不应给摘要：{noop:?}");

    // `session_before_switch` 的 `{cancelled:true}` 也必须生效。
    //
    // 与 `session_before_tree` 是**同一个缺陷**：runner 的通用分支此前把返回值丢掉，
    // 于是这个取消信号从未生效过（`emit_session_before_switch` 永远 `false`）。
    // 三个场景都必须在**同一个 `#[test]` 内串行**——扩展宿主是进程级单例，
    // 多个测试并行会互相抢句柄（实测全量并行时第一条就失败）。
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "before-switch-cancel-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    assert!(
        pi_cli::ext_tools::emit_session_before_switch("new", None),
        "扩展返回 {{cancelled:true}} 时应取消切换"
    );

    // F64 新增的三项也必须在**同一个 `#[test]` 内串行**：扩展宿主是进程级单例
    // （`HOST`），每个 `#[test]` 各自 `reload_host` 会互相抢句柄——实测并行下
    // `session_compact_events_are_emitted` 拿到的是别的测试的探针，载荷为空。
    check_session_compact_events();
    check_session_info_changed();
    check_thinking_level_select();
    check_model_select_payload();
    check_ui_prompt_events();
    check_context_with_system();
    check_before_provider_headers();
    check_after_provider_response();
    check_set_session_name();
    check_set_thinking_level();
    check_get_system_prompt();
    check_ctx_signal();
    check_ctx_signal_lifecycle();
    check_get_all_tools();
    check_flag_roundtrip();
    check_get_commands();
    check_branch_uses_real_leaf();
    check_extension_statuses();
}

/// F64/2：`session_compact` 与 `session_compact_failed` 必须真的派发
/// （ref `extensions/types.ts:1373-1374` 的事件名、`:611-632` 的载荷）。
///
/// 与前面的三路一样必须在**同一个 `#[test]` 内串行**（宿主是进程级单例）。
/// 这里只验证派发**链路与载荷形状**：压缩本身的行为由压缩相关用例覆盖。
fn check_session_compact_events() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "session-compact-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    // 成功派发：条目原样进 `compactionEntry`。
    let entry: pi_core::session::SessionEntry = serde_json::from_value(serde_json::json!({
        "type": "compaction",
        "id": "c1",
        "timestamp": "2026-01-01T00:00:00.000Z",
        "summary": "SUMMARY",
        "firstKeptEntryId": "e9",
        "tokensBefore": 1234u64,
    }))
    .expect("构造 compaction 条目");
    pi_cli::ext_tools::emit_session_compact(&entry, false, "threshold", false);

    // 失败派发（带 `errorMessage`）。
    pi_cli::ext_tools::emit_session_compact_failed("overflow", Some("boom"), false, true);

    // 用 `session_before_tree` 把探针积攒的载荷带回来。
    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回聚合载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");

    let compact = value
        .get("compact")
        .filter(|v| !v.is_null())
        .expect("session_compact 应已派发");
    assert_eq!(
        compact.get("type").and_then(|v| v.as_str()),
        Some("session_compact")
    );
    assert_eq!(
        compact.get("reason").and_then(|v| v.as_str()),
        Some("threshold")
    );
    assert_eq!(
        compact.get("willRetry").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        compact.get("fromExtension").and_then(|v| v.as_bool()),
        Some(false)
    );
    let entry_back = compact.get("compactionEntry").expect("应有条目");
    assert_eq!(entry_back.get("id").and_then(|v| v.as_str()), Some("c1"));
    assert_eq!(
        entry_back.get("tokensBefore").and_then(|v| v.as_u64()),
        Some(1234)
    );
    assert_eq!(
        entry_back.get("firstKeptEntryId").and_then(|v| v.as_str()),
        Some("e9")
    );

    let failed = value
        .get("failed")
        .filter(|v| !v.is_null())
        .expect("session_compact_failed 应已派发");
    assert_eq!(
        failed.get("reason").and_then(|v| v.as_str()),
        Some("overflow")
    );
    assert_eq!(failed.get("aborted").and_then(|v| v.as_bool()), Some(false));
    assert_eq!(
        failed.get("willRetry").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        failed.get("errorMessage").and_then(|v| v.as_str()),
        Some("boom")
    );
}

/// F64/3：`session_info_changed` 必须真的派发（ref `extensions/types.ts:1360`、
/// 派发点 `agent-session.ts:3559-3564`），且 `/name` 与 `/resume` 改名两个出口都要覆盖。
fn check_session_info_changed() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "session-info-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    // 出口 ①：`/resume` 改名（目标会话文件）。
    let session = agent.path().join("s.jsonl");
    std::fs::write(&session, "").expect("建空会话文件");
    pi_cli::session_writer::append_session_info_to(&session, "renamed").expect("写 session_info");

    // 出口 ②：`/name`（当前 writer）。
    let mut writer =
        pi_cli::session_writer::SessionWriter::create(agent.path(), &cwd, "p", "m", "medium")
            .expect("建 writer");
    writer
        .append_session_info("named")
        .expect("写 session_info");

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回积攒的载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    let seen = value
        .get("seen")
        .and_then(|v| v.as_array())
        .expect("seen 数组");
    let names: Vec<Option<&str>> = seen
        .iter()
        .map(|event| {
            assert_eq!(
                event.get("type").and_then(|v| v.as_str()),
                Some("session_info_changed"),
                "事件名"
            );
            event.get("name").and_then(|v| v.as_str())
        })
        .collect();
    assert_eq!(
        names,
        vec![Some("renamed"), Some("named")],
        "两个出口都应派发且带上名字"
    );
}

/// F64/4：`thinking_level_select` 必须真的派发（ref `extensions/types.ts:1415`、
/// 派发点 `agent-session.ts:2267-2274`），且**只在档位真的变化时**。
///
/// TUI（`/thinking`、`ctrl+t`）与 rpc 的 `set_thinking_level` 都经由
/// `SessionWriter::append_thinking_level_change` 写条目，因此那里是唯一派发点。
fn check_thinking_level_select() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "thinking-level-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    let mut writer =
        pi_cli::session_writer::SessionWriter::create(agent.path(), &cwd, "p", "m", "medium")
            .expect("建 writer");
    // `create` 已经写过 `medium` 那条 ⇒ `previousLevel` 从 `medium` 起算。
    writer
        .append_thinking_level_change("high")
        .expect("切到 high");
    // 再写一次相同档位：ref 的 `isChanging` 为假 ⇒ **不派发**。
    writer
        .append_thinking_level_change("high")
        .expect("重复 high");
    writer
        .append_thinking_level_change("low")
        .expect("切到 low");

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回积攒的载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    let seen = value
        .get("seen")
        .and_then(|v| v.as_array())
        .expect("seen 数组");
    let pairs: Vec<(Option<&str>, Option<&str>)> = seen
        .iter()
        .map(|event| {
            assert_eq!(
                event.get("type").and_then(|v| v.as_str()),
                Some("thinking_level_select"),
                "事件名"
            );
            (
                event.get("previousLevel").and_then(|v| v.as_str()),
                event.get("level").and_then(|v| v.as_str()),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        vec![(Some("medium"), Some("high")), (Some("high"), Some("low"))],
        "只有真的变化才派发，且 previousLevel 正确"
    );
}

/// F64/5：`model_select` 的**载荷形状**与 `modelsAreEqual` 前置判断
/// （ref `extensions/types.ts:913-919`、派发点 `agent-session.ts:2105-2107`）。
///
/// 端到端切换（`ctrl+p`）的完整链路已由 `p9_f63_thinking_level.rs` 的
/// `ctrl_p_recomputes_the_thinking_level` 覆盖（它断言 `footer.model` 真的换了）；
/// 这里验证派发函数本身的契约，不重复驱动一次 TUI 会话。
fn check_model_select_payload() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "model-select-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    let model = serde_json::json!({"id": "model-b", "provider": "p", "reasoning": true});
    let previous = serde_json::json!({"id": "model-a", "provider": "p", "reasoning": true});
    // 同 id+provider ⇒ ref 的 `modelsAreEqual` 为真，**不派发**。
    pi_cli::ext_tools::emit_model_select_json(model.clone(), Some(model.clone()), "cycle");
    // 不同 ⇒ 派发一次。
    pi_cli::ext_tools::emit_model_select_json(model.clone(), Some(previous.clone()), "cycle");

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回积攒的载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    let seen = value
        .get("seen")
        .and_then(|v| v.as_array())
        .expect("seen 数组");
    assert_eq!(seen.len(), 1, "同模型不应派发：{seen:?}");
    let event = &seen[0];
    assert_eq!(event.get("source").and_then(|v| v.as_str()), Some("cycle"));
    assert_eq!(
        event
            .get("model")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str()),
        Some("model-b")
    );
    assert_eq!(
        event
            .get("previousModel")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str()),
        Some("model-a"),
        "previousModel 应带上旧模型"
    );
}

/// F64/6：`ui_prompt_start` / `ui_prompt_end`（ref `extensions/types.ts:1404-1405`、
/// 实现 `runner.ts:541-566` 的 `withUIPrompt`）。
///
/// 关键语义：`reason` 恒为 `"ui_prompt"`；`title` 为空时**不带该键**
/// （ref 的 `...(title ? { title } : {})`）。
fn check_ui_prompt_events() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "ui-prompt-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    // `reload_host` 是异步的（shutdown 旧宿主 → 启动新的）。此前同步 RPC 顺手
    // 提供了等待；`emit_ui_prompt` 改成 `enqueue_emit` 后必须**显式**确认新宿主
    // 已接管，否则事件会发给旧宿主。`probe: "ui-prompt"` 是该夹具的身份标记。
    let mut ready = false;
    for _ in 0..100 {
        let back = pi_cli::ext_tools::emit_session_before_tree(Some("probe"), Some("probe"));
        if back.summary.unwrap_or_default().contains("ui-prompt") {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(ready, "5s 内新宿主未就绪（ui-prompt 夹具未接管）");

    // 事件走**异步**队列（见 `emit_ui_prompt` 的注释：面板打开时扩展正阻塞在
    // `ctx.ui.select` 的 await 里，同步 RPC 会卡住 TUI 主循环）。因此发完要**轮询等**
    // 三条事件都到达，不能立刻读。
    pi_cli::ext_tools::emit_ui_prompt("input", Some("What went wrong?"), true);
    pi_cli::ext_tools::emit_ui_prompt("input", Some("What went wrong?"), false);
    // 空 title ⇒ 载荷里不该有 `title` 键。
    pi_cli::ext_tools::emit_ui_prompt("custom", None, true);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let seen = loop {
        let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
        let raw = back.summary.unwrap_or_default();
        let value: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
        let seen: Vec<serde_json::Value> = value
            .get("seen")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter(|event| {
                        event
                            .get("type")
                            .and_then(|v| v.as_str())
                            .is_some_and(|name| name.starts_with("ui_prompt_"))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if seen.len() >= 3 {
            break seen;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "等待 5s 后只收到 {} 条 ui_prompt 事件：{seen:?}（原始：{raw}）",
            seen.len()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let types: Vec<Option<&str>> = seen
        .iter()
        .map(|event| event.get("type").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        types,
        vec![
            Some("ui_prompt_start"),
            Some("ui_prompt_end"),
            Some("ui_prompt_start")
        ],
        "start/end 顺序"
    );
    for event in &seen {
        assert_eq!(
            event.get("reason").and_then(|v| v.as_str()),
            Some("ui_prompt"),
            "reason 恒为 ui_prompt"
        );
    }
    assert_eq!(seen[0].get("kind").and_then(|v| v.as_str()), Some("input"));
    assert_eq!(
        seen[0].get("title").and_then(|v| v.as_str()),
        Some("What went wrong?")
    );
    assert_eq!(seen[1].get("kind").and_then(|v| v.as_str()), Some("input"));
    let last = &seen[2];
    assert_eq!(last.get("kind").and_then(|v| v.as_str()), Some("custom"));
    assert!(last.get("title").is_none(), "空 title 不应带该键：{last:?}");
}

/// F64/7：`context_with_system` 的**第二相**语义（ref `runner.ts:1221-1250`）。
///
/// 与 `context` 的区别：载荷含 `system`，且返回值**直接采用**。
fn check_context_with_system() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "context-with-system-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    let system: pi_ai::message::Message = serde_json::from_value(serde_json::json!({
        "role": "system", "content": "PROMPT", "timestamp": 1
    }))
    .expect("构造 system");
    let user: pi_ai::message::Message = serde_json::from_value(serde_json::json!({
        "role": "user", "content": "hi", "timestamp": 2
    }))
    .expect("构造 user");
    let out = pi_cli::ext_tools::transform_context(vec![system.clone(), user.clone()]);
    // 夹具追加一条 user ⇒ 长度 3；且它看到过 system（第一相会剥离 system，
    // 第二相必须带上，否则 `SEEN-SYSTEM:false`）。
    assert_eq!(out.len(), 3, "第二相应追加一条：{}", out.len());
    let last = serde_json::to_value(&out[2]).expect("序列化");
    let text = last
        .get("content")
        .and_then(|v| v.as_array())
        .and_then(|blocks| blocks.first())
        .and_then(|block| block.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(text, "SEEN-SYSTEM:true", "第二相应看到 system 消息");
    // 首条 system 保留（第二相不剥离）。
    assert!(
        matches!(out[0], pi_ai::message::Message::System { .. }),
        "首条仍应是 system"
    );
}

/// F64/8：`before_provider_headers` 可增、可改、可删（ref `types.ts:723-729`）。
fn check_before_provider_headers() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "provider-headers-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    let out = pi_cli::ext_tools::transform_provider_headers(vec![
        ("x-keep".to_string(), "keep".to_string()),
        ("x-replace-me".to_string(), "original".to_string()),
        ("x-drop-me".to_string(), "gone".to_string()),
    ]);
    assert!(
        out.contains(&("x-keep".to_string(), "keep".to_string())),
        "未提及的头应保留：{out:?}"
    );
    assert!(
        out.contains(&("x-replace-me".to_string(), "replaced".to_string())),
        "应可改写：{out:?}"
    );
    assert!(
        !out.iter().any(|(name, _)| name == "x-drop-me"),
        "应可删除：{out:?}"
    );
    assert!(
        out.contains(&("x-injected".to_string(), "from-extension".to_string())),
        "应可新增：{out:?}"
    );
}

/// F64/9：`after_provider_response`（ref `extensions/types.ts:729-734`、派发点
/// `sdk.ts:357-362`）。载荷是 `{status, headers}`，只在真发了 HTTP 的流上有值。
///
/// 这里分两段验证：①派发函数本身的载荷形状；②`Stream::response_meta` 的
/// 有无语义（HTTP 流带、离线重放的流不带）。
fn check_after_provider_response() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "after-response-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    pi_cli::ext_tools::emit_after_provider_response(
        429,
        &[
            ("x-ratelimit-remaining".to_string(), "0".to_string()),
            ("content-type".to_string(), "text/event-stream".to_string()),
        ],
    );

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回积攒的载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    let seen = value
        .get("seen")
        .and_then(|v| v.as_array())
        .expect("seen 数组");
    assert_eq!(seen.len(), 1, "应派发一次：{seen:?}");
    assert_eq!(
        seen[0].get("status").and_then(|v| v.as_u64()),
        Some(429),
        "status 应原样带上"
    );
    let headers = seen[0].get("headers").expect("应有 headers");
    assert_eq!(
        headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.as_str()),
        Some("0")
    );
    assert_eq!(
        headers.get("content-type").and_then(|v| v.as_str()),
        Some("text/event-stream")
    );
}

/// `Stream::response_meta` 只在真发了 HTTP 时才 `Some`
/// （离线重放/测试构造的流必须是 `None`，否则会派发空载荷）。
#[test]
fn response_meta_is_absent_on_replayed_streams() {
    let replayed = pi_ai::provider::Stream::new(Box::pin(futures::stream::empty()));
    assert!(
        replayed.response_meta().is_none(),
        "离线重放构造的流不应带 HTTP 元信息"
    );
}

/// F66/1：扩展的 `pi.setSessionName` / `pi.getSessionName`
/// （ref `extensions/types.ts:1506-1509` 的 `setSessionName`/`getSessionName`）。
///
/// 此前两者在 `runner.mjs` 里是**空实现**（`setSessionName() {}`、
/// `getSessionName() { return undefined; }`）——扩展改名静默无效。
fn check_set_session_name() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "set-session-name-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    // 先给一个已知名字，再让扩展改名，最后核对文件里写了什么。
    //
    // 用 `SessionWriter::create` 建文件（写 header），再用 `append_session_info_to`
    // 写初始名：`SessionWriter::append_session_info` 是**惰性**的，首条 assistant
    // 之前只在内存里攒着、磁盘上没有文件，直接读会报 `MissingHeader`。
    let writer =
        pi_cli::session_writer::SessionWriter::create(agent.path(), &cwd, "p", "m", "medium")
            .expect("建 writer");
    let path = writer.path().to_path_buf();
    // `SessionWriter` 是**惰性**的（首条 assistant 之前磁盘上没有文件），
    // 所以把它的 header 行单独写出去，之后统一走 `append_session_info_to`。
    std::fs::write(&path, format!("{}\n", writer.header_line())).expect("写 header");
    drop(writer);
    pi_cli::session_writer::append_session_info_to(&path, "original-name").expect("写初始名");
    pi_cli::ext_tools::set_session_file(&path);

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    assert!(
        value.get("error").is_none(),
        "`setSessionName` 不应抛错：{value:?}"
    );
    assert_eq!(
        value.get("before").and_then(|v| v.as_str()),
        Some("original-name"),
        "`getSessionName` 应返回当前名"
    );
    assert_eq!(
        value.get("after").and_then(|v| v.as_str()),
        Some("renamed-by-extension"),
        "改名后 `getSessionName` 应返回新名"
    );

    // 会话文件里必须真的落了 `session_info`。
    //
    // `pi.setSessionName` 是**异步 RPC**（ref 也是 `void this._extensionRunner.emit`，
    // 不等待），宿主侧经 `SessionCmd` 通道写文件。因此这里要**轮询等待**
    // 那条条目出现，而不是立即断言——固定 `sleep` 会在慢机器上偶发失败，
    // 而"等到达"本身是有界且确定的（最多 5s）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(file) = pi_core::session_io::SessionFile::read(&path) {
            let wrote = file.entries.iter().any(|entry| match entry {
                pi_core::session::SessionEntry::Unknown(raw) => {
                    raw.get("type").and_then(|v| v.as_str()) == Some("session_info")
                        && raw.get("name").and_then(|v| v.as_str()) == Some("renamed-by-extension")
                }
                _other => false,
            });
            if wrote {
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "等待 5s 后会话文件仍无新名字"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let file = pi_core::session_io::SessionFile::read(&path).expect("读会话");
    let names: Vec<String> = file
        .entries
        .iter()
        .filter_map(|entry| match entry {
            pi_core::session::SessionEntry::Unknown(raw) => {
                (raw.get("type").and_then(|v| v.as_str()) == Some("session_info"))
                    .then(|| raw.get("name").and_then(|v| v.as_str()).map(str::to_string))
                    .flatten()
            }
            _other => None,
        })
        .collect();
    assert!(
        names.contains(&"renamed-by-extension".to_string()),
        "会话文件应含新名字，实际 {names:?}"
    );
}

/// F67/1：`pi.setThinkingLevel` / `pi.getThinkingLevel`
/// （ref `extensions/types.ts:1538-1546`）。
///
/// 此前 `getThinkingLevel()` 恒返回 `"off"`（假值）、`setThinkingLevel() {}` 空实现
/// ⇒ 扩展读写思考档位全部无效。
fn check_set_thinking_level() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "set-thinking-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    // 宿主下发的 ctx 里先放一个已知档位。
    pi_cli::ext_tools::set_ctx_thinking_level("low");

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    assert_eq!(
        value.get("before").and_then(|v| v.as_str()),
        Some("low"),
        "`getThinkingLevel` 应返回当前档位"
    );
    assert_eq!(
        value.get("after").and_then(|v| v.as_str()),
        Some("high"),
        "`setThinkingLevel` 后应立即读到新档位"
    );
    assert_eq!(
        value.get("seen").and_then(|v| v.as_array()).map(Vec::len),
        Some(1),
        "不应抛错"
    );
}

/// F68/1：`ctx.getSystemPrompt()`（ref `extensions/types.ts:351-352`）。
///
/// 此前 runner 侧**没有这个方法**，扩展调用时是 `TypeError`（用户已装扩展里
/// 若用 `try/catch` 包住就会被静默吞掉）。
fn check_get_system_prompt() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "get-system-prompt-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    // 宿主下发的 ctx 里放一份可识别的系统提示。
    pi_cli::ext_tools::set_ctx_system_prompt("SYSTEM-PROMPT-FROM-HOST");

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    assert_eq!(
        value.get("seen").and_then(|v| v.as_str()),
        Some("SYSTEM-PROMPT-FROM-HOST"),
        "`getSystemPrompt` 应返回宿主当前生效的系统提示"
    );
}

/// F69：`ctx.signal`（ref `extensions/types.ts:340`）。
///
/// `pi-agent-resilience`（用户已装）**真的读它**：`abortSignal(ctx)` 用
/// `instanceof AbortSignal` 判断，缺失时返回 `undefined` 并降级（不是崩溃，
/// 但那层"外部已取消就别再重试"的保护静默失效）。
fn check_ctx_signal() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "ctx-signal-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    let seen = value
        .get("seen")
        .and_then(|v| v.as_array())
        .expect("seen 数组");
    let is_signal = seen
        .first()
        .and_then(|entry| entry.get("isSignal"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    assert!(is_signal, "`ctx.signal` 应是 AbortSignal 实例：{seen:?}");
    assert_eq!(
        seen.first()
            .and_then(|e| e.get("abortedBefore"))
            .and_then(|v| v.as_bool()),
        Some(false),
        "初始不应是 aborted"
    );
    let aborted_after = seen.iter().find_map(|entry| {
        entry
            .get("abortedAfter")
            .and_then(serde_json::Value::as_bool)
    });
    assert_eq!(
        aborted_after,
        Some(true),
        "`ctx.abort()` 后 signal 应变 aborted：{seen:?}"
    );
}

/// F69/2：`ctx.signal` 的**生命周期**——一次 abort 不得污染后续操作。
///
/// ref 的 `signal` 是 `get signal() { return getSignalFn() }` →
/// `() => this.agent.signal`（`runner.ts:858-861`、`agent-session.ts:3100`），
/// 即**当前这次操作**的信号。若实现成进程级固定控制器，abort 一次之后
/// 所有回合读到的 signal 都是 aborted，`pi-agent-resilience` 的"已取消就别重试"
/// 会永久生效——那是真实的行为偏差，不只是少个字段。
fn check_ctx_signal_lifecycle() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "ctx-signal-reset-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    // 第一次：正常 → 扩展主动 abort ⇒ 同一 signal 应立刻 aborted。
    let first = pi_cli::ext_tools::emit_session_before_tree(Some("t1"), Some("f1"));
    let raw = first.summary.expect("第一次应有载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析");
    let seen = value.get("seen").and_then(|v| v.as_array()).expect("seen");
    assert_eq!(
        seen.first()
            .and_then(|e| e.get("aborted"))
            .and_then(|v| v.as_bool()),
        Some(false),
        "第一次刚进来不应是 aborted：{seen:?}"
    );
    assert_eq!(
        seen.iter()
            .find_map(|e| e.get("afterAbort").and_then(serde_json::Value::as_bool)),
        Some(true),
        "`ctx.abort()` 后当前 signal 应变 aborted：{seen:?}"
    );

    // 宿主推进状态（**真的下发新 ctx**）⇒ 新操作里 signal 必须**不再是 aborted**。
    // 必须用会触发 `push_ctx` 的调用；`update_ctx_model(None, None)` 两个参数都是
    // `None` 时只是加锁后 `push_ctx()`，而 `signalAborted` 的判定依赖宿主下发的 ctx ——
    // 上一版测试就是这么写的，导致断言恒真、无判别力。
    pi_cli::ext_tools::update_ctx_model(Some("model-after-abort"), None);
    let second = pi_cli::ext_tools::emit_session_before_tree(Some("t2"), Some("f2"));
    let raw = second.summary.expect("第二次应有载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析");
    let seen = value.get("seen").and_then(|v| v.as_array()).expect("seen");
    // 夹具的 `seen` 是**闭包累积**的，第二次返回里第一条仍是第一次的记录——
    // 必须取**最后一条**（本次操作刚写的那条），否则断言恒真、没有判别力。
    assert_eq!(
        seen.last()
            .and_then(|e| e.get("aborted"))
            .and_then(|v| v.as_bool()),
        Some(false),
        "新操作里 signal 不应仍处于 aborted：{seen:?}"
    );
}

/// F70：`pi.getAllTools()` 必须包含**内置工具**，不只扩展注册的那些
/// （ref `agent-session.ts:1259-1267`：返回 `_toolDefinitions` 全部）。
///
/// `pi-secrets`（用户已装）在 `session_start` 里读它统计 `tools.filter(name => name === "bash")`
/// —— 此前内置 `bash` 永远不在列表里，那行日志恒为 `bashCount: 0`。
fn check_get_all_tools() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "get-all-tools-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    // 与生产同一路径：先下发内置清单，再重启宿主（宿主握手里带着它）。
    pi_cli::ext_tools::set_ctx_builtin_tools(&tools);
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    let names: Vec<&str> = value
        .get("names")
        .and_then(|v| v.as_array())
        .map(|list| list.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        names.contains(&"bash"),
        "内置 `bash` 应在 `getAllTools()` 里，实际 {names:?}"
    );
    assert!(
        names.contains(&"edit"),
        "内置 `edit` 应在 `getAllTools()` 里，实际 {names:?}"
    );
}

/// F71：CLI flag → `pi.getFlag` 的往返（ref `extensions/types.ts:1463`）。
///
/// 两个方向都要成立：①命令行给的覆盖值能读到；②没给的回落**注册默认值**
/// （`flagSpecs.get(name)?.default`）。`pi-handoff-gate`（用户已装）注册了 4 个
/// flag 并在 `session_start` 里读它们，这条链路断掉时那些开关全部失效。
fn check_flag_roundtrip() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "flag-roundtrip-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    assert_eq!(
        value.get("absent").and_then(serde_json::Value::as_i64),
        Some(42),
        "未在命令行给出时应回落注册默认值"
    );
}

/// F73：`pi.getCommands()` 必须包含**prompt 模板与技能**，不只扩展注册的命令
/// （ref `agent-session.ts:3021-3044`：`[...extensionCommands, ...templates, ...skills]`，
/// 技能名带 `skill:` 前缀）。
///
/// `pi-secrets` 与 `pi-handoff-gate` 用它枚举可用命令；此前只有扩展命令，
/// 模板与技能对他们完全不可见。
fn check_get_commands() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "get-commands-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    // 与生产同一路径：下发模板与技能清单。
    pi_cli::ext_tools::set_ctx_prompt_commands(&[(
        "review".to_string(),
        "Review the change".to_string(),
    )]);
    pi_cli::ext_tools::set_ctx_skill_commands(&[("pdf".to_string(), "PDF helpers".to_string())]);

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    let list: Vec<&str> = value
        .get("list")
        .and_then(|v| v.as_array())
        .map(|items| items.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    assert!(
        list.contains(&"prompt:review"),
        "prompt 模板应在 `getCommands()` 里，实际 {list:?}"
    );
    assert!(
        list.contains(&"skill:skill:pdf"),
        "技能应带 `skill:` 前缀，实际 {list:?}"
    );
}

/// F74：`ctx.sessionManager.getLeafId()` / `getBranch()` 必须用**真实当前叶子**，
/// 不能退化成"文件最后一条"（ref `session-manager.ts:1467-1477` 的 `this.leafId`）。
///
/// 会话分叉 + 导航到中间节点后，两者不同：`getBranch()` 会返回错误的分支，
/// `pi-secrets`/`pi-ai-summary`（7 处 `getBranch` 调用）据此算上下文与用量。
fn check_branch_uses_real_leaf() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "branch-leaf-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");

    // 造一个**分叉**会话：a1 → a2 与 a1 → a3，文件最后一条是 a3，
    // 但当前叶子被导航回 a2（ref 的 `leafId` 语义）。
    let file = agent.path().join("branch.jsonl");
    std::fs::write(
        &file,
        concat!(
            r#"{"type":"session","version":1,"id":"s","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}"#,
            "\n",
            r#"{"type":"message","id":"a1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","content":"x","timestamp":1}}"#,
            "\n",
            r#"{"type":"message","id":"a2","parentId":"a1","timestamp":"2026-01-01T00:00:02.000Z","message":{"role":"user","content":"y","timestamp":2}}"#,
            "\n",
            r#"{"type":"message","id":"a3","parentId":"a1","timestamp":"2026-01-01T00:00:03.000Z","message":{"role":"user","content":"z","timestamp":3}}"#,
            "\n",
        ),
    )
    .expect("写会话");
    pi_cli::ext_tools::set_session_file(&file);
    // **运行期**设置叶子（真实场景：`/tree` 导航后才变）——`ctx_snapshot()` 每次重读，
    // 所以不需要重启宿主。
    pi_cli::ext_tools::set_ctx_leaf_id(Some("a2"));

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    assert_eq!(
        value.get("leafId").and_then(|v| v.as_str()),
        Some("a2"),
        "`getLeafId()` 应返回真实叶子而不是文件最后一条"
    );
    let branch: Vec<&str> = value
        .get("branch")
        .and_then(|v| v.as_array())
        .map(|items| items.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    assert_eq!(branch, vec!["a1", "a2"], "`getBranch()` 应沿真实叶子回溯");
}

/// F76：`ctx.ui.setStatus(key, text)` 的值必须能通过 footer 的
/// `getExtensionStatuses()` 读回（ref `extensions/types.ts:168-190` 的
/// `setFooter(factory)` 注释："extension statuses from setStatus()"）。
///
/// 此前 `getExtensionStatuses: () => new Map()` **恒空** ⇒ 扩展页脚里的状态
/// （`pi-secrets`/`pi-ai-summary` 都会显示）永远是空。
fn check_extension_statuses() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), "extension-status-ext.mjs");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    // 触发 `session_start`（夹具在那里 setStatus）。
    pi_cli::ext_tools::emit_event(
        "session_start",
        serde_json::json!({ "type": "session_start" }),
    );

    let back = pi_cli::ext_tools::emit_session_before_tree(Some("t"), Some("f"));
    let raw = back.summary.expect("探针应返回载荷");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("解析载荷");
    let seen = value
        .get("seen")
        .and_then(|v| v.as_array())
        .expect("应能读到状态表");
    let text = serde_json::to_string(seen).expect("序列化");
    assert!(
        text.contains("other-key") && text.contains("other-value"),
        "未删除的 status 应出现在 getExtensionStatuses() 里：{text}"
    );
    assert!(
        !text.contains("probe-key"),
        "传 `undefined` 应删除该 key：{text}"
    );
}

/// F77/2：**端到端**证明 TUI 回合结束时真的派发了 `agent_before_settle`。
///
/// 前一个用例只验证 `emit_agent_before_settle` 函数本身（它 F77 之前就存在，
/// 只是 TUI/rpc 不调用）⇒ 无判别力。这个用例让探针扩展在事件里**写文件**，
/// 再驱动真实 TUI 回合，断言文件出现。
#[test]
fn tui_emits_before_settle_at_turn_end() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    install_probe(&agent, "before-settle-writes-file-ext.mjs");
    let log = agent.join("settle-calls.log");
    // 必须用**能连上的** mock：模型不可达时 TUI 走"无可用模型"分支、连首帧都不出，
    // 回合永远不会跑（实测 raw.bin 为空）。
    let mut mock = Command::new(repo_root().join("target/debug/mock-provider"))
        .current_dir(repo_root())
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("启动 mock-provider");
    let base_url = {
        use std::io::BufRead;
        let stdout = mock.stdout.take().expect("stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        let mut found = None;
        for line in lines.by_ref() {
            let line = line.expect("读 stdout");
            if let Some(rest) = line.strip_prefix("listening ") {
                found = Some(
                    rest.rsplit(':')
                        .next()
                        .expect("端口")
                        .parse::<u16>()
                        .expect("数字"),
                );
                break;
            }
        }
        format!("http://127.0.0.1:{}/v1", found.expect("mock 端口"))
    };
    // 模型指向本地不可达端口：回合会失败，但 `agent_before_settle` 仍应派发
    // （ref 在收尾路径上，与回合是否成功无关）。
    std::fs::write(
        agent.join("models.json"),
        format!(
            r#"{{"providers":{{"p":{{"baseUrl":"{base_url}","api":"openai-completions","apiKey":"k","models":[{{"id":"m1","reasoning":false}}]}}}}}}"#
        ),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        concat!(
            r#"[{"waitStable":{"quietMs":2500,"timeoutMs":60000}}"#,
            r#",{"text":"hi"},{"text":"\r"},{"delayMs":4000}"#,
            r#",{"text":"\u0004"},{"delayMs":1200}]"#,
        ),
    )
    .expect("写 keys");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("HOME", &home)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .output()
        .expect("跑 pty");
    assert!(
        output.status.success(),
        "pi-rs 应正常退出：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = mock.kill();
    let _ = mock.wait();
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        calls.contains("call:"),
        "TUI 回合结束时应派发 `agent_before_settle`，实际日志：{calls:?}"
    );
}
