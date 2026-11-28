//! F62 发布准备：产出一份**综合会话**用于双向互通验证。
//!
//! 覆盖任务要求的条目类型：普通消息、`session_info`（`/resume` 重命名）、
//! `branch_summary`（`/tree` 导航，**有摘要**与**无摘要**两种）、`compaction`（`/compact`）。
//!
//! 用写入 API 而不是 pty 驱动 `/tree` 交互：后者要按多次方向键 + 选择菜单，
//! 时序不稳（本机 mock 下 `/compact` 曾让 pty 超时 200s）。写入 API 走的是
//! **同一条生产路径**（`SessionWriter` 的 `append_*`），产出的是真实文件格式。
//!
//! 输出路径由 `PI_F62_OUT` 指定；未设置则跳过（不污染常规 test-gate）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_ai::message::{Message, UserContent};
use pi_ai::types::{AssistantMessage, Content, StopReason, Usage};
use pi_cli::session_writer::SessionWriter;

fn assistant(text: &str) -> Message {
    Message::Assistant(Box::new(AssistantMessage {
        role: "assistant".to_string(),
        content: vec![Content::Text {
            text: text.to_string(),
            text_signature: None,
        }],
        api: "openai-completions".to_string(),
        provider: "p".to_string(),
        model: "m".to_string(),
        response_model: None,
        usage: Usage::default(),
        stop_reason: StopReason::Stop,
        error_message: None,
        raw_stop_reason: None,
        end_turn: None,
        response_id: None,
        timestamp: 1,
    }))
}

fn user(text: &str) -> Message {
    Message::User {
        content: UserContent::Text(text.to_string()),
        timestamp: 1,
        extra: serde_json::Map::new(),
    }
}

#[test]
fn produce_combined_session_for_interop() {
    let Some(out) = std::env::var_os("PI_F62_OUT") else {
        eprintln!("跳过：未设置 PI_F62_OUT");
        return;
    };
    let dir = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&dir).expect("建目录");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&sessions).expect("建 sessions");

    let mut writer = SessionWriter::create(&sessions, &dir, "p", "m", "off").expect("建会话");
    // 会话链：`branch_summary` 的 `fromId` 要从这里取旧叶子（与生产路径一致）。
    let chain = std::sync::Arc::new(std::sync::Mutex::new(None));
    writer.set_chain(std::sync::Arc::clone(&chain));

    // `append_message` 返回 `()`（id 由 writer 内部分配并回写 `chain`），
    // 所以每写一条都从链上取当前叶子。
    let leaf = |chain: &std::sync::Arc<std::sync::Mutex<Option<String>>>| {
        chain
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_default()
    };

    // ① 主分支两轮对话。
    writer.append_message(&user("第一问")).expect("写 user");
    writer
        .append_message(&assistant("第一答"))
        .expect("写 assistant");
    let answer_id = leaf(&chain);

    // ② `session_info`（`/resume` 重命名写的就是这个条目）。
    writer
        .append_session_info("发布测试会话")
        .expect("写 session_info");

    // ③ `/tree` 导航：**无摘要**（`newLeafId` = 第一答，`fromId` = 当前旧叶子）。
    let old_leaf = answer_id.clone();
    writer
        .append_branch_summary(Some(&answer_id), &old_leaf, "", 0, 0)
        .expect("写空摘要 branch_summary");

    // ④ 导航后的新分支。
    writer
        .append_message(&user("分支B的问题"))
        .expect("写 user");
    writer
        .append_message(&assistant("分支B的回答"))
        .expect("写 assistant");
    let leaf_b = leaf(&chain);

    // ⑤ `/tree` 导航：**有摘要**（摘要正文非空）。
    writer
        .append_branch_summary(Some(&answer_id), &leaf_b, "带摘要导航的分支摘要正文", 0, 0)
        .expect("写有摘要 branch_summary");

    // ⑥ `/compact`。
    writer
        .append_compaction("压缩摘要正文", Some(&answer_id), 12000, None, None, None)
        .expect("写 compaction");

    // ⑦ 压缩后的新对话。
    writer
        .append_message(&user("压缩后的问题"))
        .expect("写 user");
    writer
        .append_message(&assistant("压缩后的回答"))
        .expect("写 assistant");
    let _ = writer.ensure_flushed();

    let path = writer.path().to_path_buf();
    assert!(path.exists(), "会话文件应已落盘：{}", path.display());
    eprintln!("PRODUCED {}", path.display());
}
