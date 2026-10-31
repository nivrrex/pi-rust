//! P4-5 第 1 条要求的补齐单测：`context_edit` 的应用、`newest_session_file`、`SessionWriter::open`。
//!
//! 这三处此前只有**差分测试**覆盖（跑整个场景），没有单元测试；本文件补上，
//! 并**覆盖失败路径**（不存在的目录、空文件、编辑目标不存在等）。

use std::path::{Path, PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pi-rs-t34-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

fn write_session(dir: &Path, name: &str, lines: &[String]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, lines.join("\n") + "\n").expect("写会话文件");
    path
}

fn header(id: &str) -> String {
    format!(
        r#"{{"type":"session","version":3,"id":"{id}","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}}"#
    )
}

fn user(id: &str, parent: Option<&str>, text: &str) -> String {
    let parent = match parent {
        Some(value) => format!("\"{value}\""),
        None => "null".to_string(),
    };
    format!(
        r#"{{"type":"message","id":"{id}","parentId":{parent},"timestamp":"2026-01-01T00:00:01.000Z","message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}],"timestamp":1}}}}"#
    )
}

/// 用**真实的 `Message`** 序列化生成条目：手写 JSON 容易缺字段，
/// 缺字段会让反序列化失败、消息被静默丢掉（我第一版就踩了这个：少 `usage`）。
fn assistant(id: &str, parent: &str, text: &str) -> String {
    let message = pi_ai::message::Message::Assistant(Box::new(pi_ai::types::AssistantMessage {
        role: "assistant".to_string(),
        content: vec![pi_ai::Content::Text {
            text: text.to_string(),
            text_signature: None,
        }],
        api: "openai-completions".to_string(),
        provider: "mock".to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: pi_ai::Usage::default(),
        stop_reason: pi_ai::StopReason::Stop,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 2,
    }));
    let body = serde_json::to_string(&message).expect("序列化消息");
    format!(
        r#"{{"type":"message","id":"{id}","parentId":"{parent}","timestamp":"2026-01-01T00:00:02.000Z","message":{body}}}"#
    )
}

fn context_edit(id: &str, parent: &str, target: &str, replacement: &str) -> String {
    format!(
        r#"{{"type":"context_edit","id":"{id}","parentId":"{parent}","timestamp":"2026-01-01T00:00:03.000Z","targetId":"{target}","replacement":{replacement}}}"#
    )
}

/// `context_edit` 的 `replacement` 是**部分字段补丁**：只改了 `content`，其余字段保留。
#[test]
fn context_edit_merges_the_patch_into_the_target_message() {
    let dir = temp_dir("ctx-edit");
    let path = write_session(
        &dir,
        "s.jsonl",
        &[
            header("h1"),
            user("m1", None, "第一个问题"),
            assistant("m2", "m1", "原始回答"),
            context_edit(
                "ce1",
                "m2",
                "m2",
                r#"{"content":[{"type":"text","text":"被替换的回答"}]}"#,
            ),
        ],
    );
    let file = pi_core::session_io::SessionFile::read(&path).expect("读得到");
    let transcript = file.transcript().expect("条目都读得回");
    assert_eq!(transcript.len(), 2, "两条消息（user + assistant）");
    let text = serde_json::to_string(&transcript[1]).expect("序列化");
    assert!(text.contains("被替换的回答"), "补丁生效，实际：{text}");
    assert!(!text.contains("原始回答"), "原文被替换掉");
    assert!(
        text.contains("\"openai-completions\""),
        "补丁只覆盖 content，其余字段必须保留"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `replacement: null` 表示**移除**该条（S07 的语义）。
#[test]
fn context_edit_with_null_replacement_removes_the_entry() {
    let dir = temp_dir("ctx-remove");
    let path = write_session(
        &dir,
        "s.jsonl",
        &[
            header("h1"),
            user("m1", None, "保留我"),
            assistant("m2", "m1", "移除我"),
            context_edit("ce1", "m2", "m2", "null"),
        ],
    );
    let file = pi_core::session_io::SessionFile::read(&path).expect("读得到");
    let transcript = file.transcript().expect("条目都读得回");
    assert_eq!(transcript.len(), 1, "被移除的那条不应出现");
    let text = serde_json::to_string(&transcript[0]).expect("序列化");
    assert!(text.contains("保留我"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 负向对照：`context_edit` 指向**不存在的 id** 时不应崩、也不应改动别的条目。
#[test]
fn context_edit_targeting_an_unknown_id_is_ignored() {
    let dir = temp_dir("ctx-unknown");
    let path = write_session(
        &dir,
        "s.jsonl",
        &[
            header("h1"),
            user("m1", None, "只有我"),
            context_edit("ce1", "m1", "nope", r#"{"content":[]}"#),
        ],
    );
    let file = pi_core::session_io::SessionFile::read(&path).expect("读得到");
    let transcript = file.transcript().expect("条目都读得回");
    assert_eq!(transcript.len(), 1, "未知目标不应影响现有条目");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 失败路径：空文件必须报错（不能静默当成空会话）。
#[test]
fn reading_an_empty_session_file_fails() {
    let dir = temp_dir("empty");
    let path = write_session(&dir, "empty.jsonl", &[]);
    assert!(
        pi_core::session_io::SessionFile::read(&path).is_err(),
        "空文件应报错"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `newest_session_file`：按 **mtime** 取最近（ref `session-manager.ts:749-767`），
/// 并可要求 header 里的 `cwd` 匹配（`sessionCwdMatches`）。此前按文件名字典序取最大，
/// 与"最近"不是一回事。
#[test]
fn newest_session_file_uses_mtime_and_matches_cwd() {
    use std::time::{Duration, SystemTime};

    let dir = temp_dir("newest");
    let older = dir.join("2026-01-01T00-00-00.000Z_aaaa.jsonl");
    let newer = dir.join("2026-03-01T00-00-00.000Z_bbbb.jsonl");
    for (path, cwd) in [(&older, "/work/a"), (&newer, "/work/b")] {
        std::fs::write(
            path,
            format!("{{\"type\":\"session\",\"version\":3,\"id\":\"x\",\"cwd\":\"{cwd}\"}}\n"),
        )
        .expect("写文件");
    }
    // 文件名顺序与 mtime 顺序相反：判定必须跟着 mtime 走。
    let base = SystemTime::now();
    for (path, offset) in [(&older, 20u64), (&newer, 10u64)] {
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("打开");
        let times = std::fs::FileTimes::new().set_modified(base - Duration::from_secs(offset));
        file.set_times(times).expect("设置 mtime");
    }
    // 非 jsonl 的不参与。
    std::fs::write(dir.join("zzz.txt"), "x").expect("写文件");

    let newest = pi_cli::session_writer::newest_session_file(&dir, None).expect("有最近会话");
    assert_eq!(newest, newer, "无 cwd 过滤时按 mtime 取最近");

    let for_a =
        pi_cli::session_writer::newest_session_file(&dir, Some(std::path::Path::new("/work/a")))
            .expect("cwd 匹配");
    assert_eq!(for_a, older, "cwd 过滤后只能拿到本项目那条");
    assert!(
        pi_cli::session_writer::newest_session_file(&dir, Some(std::path::Path::new("/work/zzz")))
            .is_none(),
        "没有匹配 cwd 时应为 None"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 失败路径：目录不存在时返回 `None`，不 panic。
#[test]
fn newest_session_file_returns_none_for_a_missing_dir() {
    let missing = std::env::temp_dir().join("pi-rs-t34-does-not-exist");
    let _ = std::fs::remove_dir_all(&missing);
    assert!(pi_cli::session_writer::newest_session_file(&missing, None).is_none());
}

/// `SessionWriter::open`：**不写** header / `model_change` / `thinking_level_change`，
/// 并以最后一条 entry 的 id 作为后续条目的 `parentId`。
#[test]
fn open_appends_without_rewriting_the_header() {
    let dir = temp_dir("open");
    let path = write_session(
        &dir,
        "seed.jsonl",
        &[header("h1"), user("m1", None, "第一个问题")],
    );
    let before = std::fs::read_to_string(&path).expect("读");
    let mut writer = pi_cli::session_writer::SessionWriter::open(&path).expect("打得开");
    assert_eq!(
        writer.last_entry_id(),
        Some("m1"),
        "最后一条 id 必须是 m1（不新起一条）"
    );
    writer
        .append_message(&pi_ai::message::Message::User {
            content: pi_ai::message::UserContent::Text("第二个问题".to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        })
        .expect("追加");
    let after = std::fs::read_to_string(&path).expect("读");
    assert!(after.starts_with(&before), "原有内容一字不改");
    assert_eq!(after.lines().count(), 3, "只多了一条：header + m1 + 新消息");
    let added: serde_json::Value =
        serde_json::from_str(after.lines().next_back().expect("有末行")).expect("是 JSON");
    assert_eq!(
        added["parentId"], "m1",
        "新条目的 parentId 接在最后一条之后"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 失败路径：打开不存在的文件必须报错。
#[test]
fn open_a_missing_file_fails() {
    let dir = temp_dir("open-missing");
    let missing = dir.join("nope.jsonl");
    assert!(pi_cli::session_writer::SessionWriter::open(&missing).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 失败路径：打开**空文件**必须报错（比继续写更有用）。
#[test]
fn open_an_empty_file_fails() {
    let dir = temp_dir("open-empty");
    let path = write_session(&dir, "empty.jsonl", &[]);
    assert!(pi_cli::session_writer::SessionWriter::open(&path).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// P4-5b 第2件：`custom_message` 与 `branch_summary` **都参与上下文**，转成 **user** 消息。
///
/// 引证：`convertToLlm`〔packages/coding-agent/src/core/messages.ts:163-176〕
/// ——`custom` → user（字符串包成单个 text 块）；`branchSummary` → user（前缀+summary+后缀）。
#[test]
fn custom_and_branch_summary_entries_become_user_messages() {
    let dir = temp_dir("extra-entry-types");
    let custom = r#"{"type":"custom_message","id":"c1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","customType":"note","content":"自定义内容","display":true}"#;
    let branch = r#"{"type":"branch_summary","id":"b1","parentId":"c1","timestamp":"2026-01-01T00:00:02.000Z","fromId":"m9","summary":"分支摘要正文"}"#;
    let path = write_session(
        &dir,
        "s.jsonl",
        &[header("h1"), custom.to_string(), branch.to_string()],
    );
    // 对照必须真的触发：样本里确实有这两类条目。
    let raw = std::fs::read_to_string(&path).expect("读");
    assert!(raw.contains("custom_message"), "样本必须含 custom_message");
    assert!(raw.contains("branch_summary"), "样本必须含 branch_summary");

    let file = pi_core::session_io::SessionFile::read(&path).expect("读得到");
    let transcript = file.transcript().expect("条目都读得回");
    assert_eq!(transcript.len(), 2, "两类条目都要产出消息：{transcript:#?}");
    let text = serde_json::to_string(&transcript).expect("序列化");
    assert!(
        text.contains("自定义内容"),
        "custom 的内容要进上下文：{text}"
    );
    assert!(
        text.contains(
            "The following is a summary of a branch that this conversation came back from"
        ),
        "branch_summary 要带前前缀：{text}"
    );
    assert!(text.contains("分支摘要正文"), "摘要正文要在：{text}");
    assert!(text.contains("</summary>"), "后缀要在：{text}");
    assert!(
        text.contains(r#""role":"user""#),
        "两者都是 user 消息：{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// P4-5b：带 assistant 的种子经 `transcript()` 往返后**不能丢** assistant
/// （S27 的请求体里少了它）。
#[test]
fn transcript_keeps_the_assistant_message_from_a_seed() {
    let dir = temp_dir("keep-assistant");
    let a = assistant("m2", "m1", "第一个回答");
    let path = write_session(
        &dir,
        "s.jsonl",
        &[header("h1"), user("m1", None, "第一个问题"), a],
    );
    let file = pi_core::session_io::SessionFile::read(&path).expect("读得到");
    let transcript = file.transcript().expect("条目都读得回");
    assert_eq!(transcript.len(), 2, "user + assistant：{transcript:#?}");
    let text = serde_json::to_string(&transcript).expect("序列化");
    assert!(text.contains("assistant"), "assistant 不能丢：{text}");
    assert!(text.contains("第一个回答"), "正文要在：{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// P4-5c 失败路径：条目**读不回**时必须**报错**（带条目 id），而不是静默丢消息。
///
/// 这正是 S27 那个真缺陷的护栏：手写的 assistant 少了 `usage.cost`，
/// 以前的 `from_value(...).ok()` 会让整条消息无声消失。
#[test]
fn transcript_reports_entries_that_cannot_be_read_back() {
    let dir = temp_dir("unreadable-entry");
    // **真正不可表示**的畸形条目：`content` 是字符串（类型要求是内容块数组）。
    // 注意：**缺 `usage.cost` 不算**——TS 明确"parsed without validation"
    // （`packages/coding-agent/src/core/session-manager.ts:442-443`），它容忍缺字段，
    // 我们也给 `Usage` 加了 `serde(default)` 来对齐。
    let broken = r#"{"type":"message","id":"m2","parentId":"m1","timestamp":"2026-01-01T00:00:02.000Z","message":{"role":"assistant","content":"这是个字符串而不是数组","timestamp":2,"api":"openai-completions","provider":"mock","model":"mock-model","stopReason":"stop","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}}}}"#;
    let path = write_session(
        &dir,
        "s.jsonl",
        &[header("h1"), user("m1", None, "问题"), broken.to_string()],
    );
    // 对照必须真的触发：样本里确实有一条 assistant 且它缺 cost。
    let raw = std::fs::read_to_string(&path).expect("读");
    assert!(raw.contains("assistant"), "样本必须含 assistant");
    assert!(
        raw.contains("\"cost\""),
        "样本必须带 cost（缺 cost 已按 TS 的行为被容忍）"
    );

    let file = pi_core::session_io::SessionFile::read(&path).expect("读得到");
    let error = file
        .transcript()
        .expect_err("读不回必须报错，不能静默丢消息");
    let text = error.to_string();
    assert!(text.contains("m2"), "错误里要带**条目 id**，实际：{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// P4-5d 第 3 件：**TS 容忍缺字段**（`session-manager.ts:442-443` "parsed without validation"），
/// 实测 TS 对"assistant 缺 `usage.cost` 的手写会话"照常把该 assistant 放进请求体
/// （roles 为 system,user,assistant,...）。我们靠 `Usage` 的 `#[serde(default)]` 对齐。
#[test]
fn message_missing_usage_cost_is_tolerated_like_ts() {
    let dir = temp_dir("tolerate-missing-cost");
    // 手写 seed：**故意不给 usage.cost**。
    let assistant_missing_cost = r#"{"type":"message","id":"m2","parentId":"m1","timestamp":"2026-01-01T00:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"第一个回答"}],"timestamp":2,"api":"openai-completions","provider":"mock","model":"mock-model","stopReason":"stop","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2}}}"#;
    let raw = std::fs::read_to_string(write_session(
        &dir,
        "s.jsonl",
        &[
            header("h1"),
            user("m1", None, "第一个问题"),
            assistant_missing_cost.to_string(),
            user("m3", Some("m2"), "第二个问题"),
        ],
    ))
    .expect("读");
    // 对照必须真的触发：样本里确实有一条 assistant 且它缺 cost。
    assert!(raw.contains("assistant"), "样本必须含 assistant");
    assert!(!raw.contains("\"cost\""), "样本必须**缺** cost");

    let path = dir.join("s.jsonl");
    let file = pi_core::session_io::SessionFile::read(&path).expect("读得到");
    let transcript = file.transcript().expect("缺 cost 必须容忍，不能报错");
    assert_eq!(transcript.len(), 3, "三条消息都要在：{transcript:#?}");
    assert!(
        transcript
            .iter()
            .any(|m| matches!(m, pi_ai::message::Message::Assistant { .. })),
        "assistant 必须保留：{transcript:#?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `-c` 的 cwd 过滤规则（ref `session-manager.ts:1791-1796`）：
/// **默认分区目录不过滤**（目录改名后旧会话还能续写），自定义目录才按 header `cwd` 过滤。
#[test]
fn continue_filters_by_cwd_only_for_custom_session_dirs() {
    let agent = tempfile::tempdir().expect("agent dir");
    let cwd = std::path::Path::new("/root/Desktop/colibri");
    let default_dir = pi_core::session_path::default_session_dir(
        &cwd.to_string_lossy(),
        &agent.path().to_string_lossy(),
    );
    assert!(
        !pi_cli::session_writer::should_filter_by_cwd(&default_dir, cwd, agent.path()),
        "默认分区目录不应按 header cwd 过滤"
    );
    let custom = agent.path().join("sessions").join("shared");
    assert!(
        pi_cli::session_writer::should_filter_by_cwd(&custom, cwd, agent.path()),
        "自定义会话目录应按 header cwd 过滤"
    );
}

/// 真机场景（OI-2）：目录改名后，分区目录里唯一一份会话的 header `cwd` 是旧路径，
/// `-c` 仍必须选中它（调用方按上面的规则传 `None`）。
#[test]
fn continue_picks_newest_in_default_dir_even_if_header_cwd_is_stale() {
    let agent = tempfile::tempdir().expect("agent dir");
    let cwd = "/root/Desktop/colibri";
    let dir = pi_core::session_path::default_session_dir(cwd, &agent.path().to_string_lossy());
    std::fs::create_dir_all(&dir).expect("建分区目录");
    let path = dir.join("2026-09-26T08-51-54-437Z_01a0dce9-f1c5-71d6-82fa-73a0662d8e78.jsonl");
    std::fs::write(
        &path,
        concat!(
            r#"{"type":"session","version":3,"id":"s","timestamp":"2026-09-26T08:51:54.437Z","cwd":"/root/Desktop/Colibrì"}"#,
            "\n",
            r#"{"type":"message","timestamp":"2026-09-26T08:52:00.000Z","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#,
            "\n"
        ),
    )
    .expect("写会话");

    // 默认目录：不过滤 ⇒ 选中它。
    assert_eq!(
        pi_cli::session_writer::newest_session_file(&dir, None).as_deref(),
        Some(path.as_path())
    );
    // 若（错误地）按旧 cwd 过滤，就一份都找不到 —— 这正是修复前的行为。
    assert!(
        pi_cli::session_writer::newest_session_file(&dir, Some(std::path::Path::new(cwd)))
            .is_none()
    );
}
