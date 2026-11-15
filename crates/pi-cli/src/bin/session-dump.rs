//! F61/2：pi-rs 侧的会话 dump（与 `tools/difftest/session-interop.mjs` 的 TS 侧输出同格式）。
//!
//! 输出两段，供 `scripts/f61-interop.sh` 逐行比对：
//! - `tree`：按**文件顺序**的 `type:id:parentId`；
//! - `context`：`buildContextEntries` 的可见内容（消息文本 / 压缩摘要 / 分支摘要）。
//!
//! 为什么单独做成小二进制而不是测试：比对要跨进程跑两次（pi-rs 与 TS），
//! 一个能打印 JSON 的小程序最直接。

#![forbid(unsafe_code)]

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1).map(PathBuf::from) else {
        eprintln!("用法：session-dump <session.jsonl>");
        std::process::exit(2);
    };
    let file = match pi_core::session_io::SessionFile::read(&path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("读取 {} 失败：{error}", path.display());
            std::process::exit(1);
        }
    };
    let mut tree: Vec<String> = Vec::new();
    for entry in &file.entries {
        let kind = entry.kind();
        let id = entry.id().unwrap_or_default();
        // 统一走 `parent_id()`（它覆盖 Message / Compaction / BranchSummary / Unknown…）。
        let parent = entry.parent_id().unwrap_or("null").to_string();
        tree.push(format!("{kind}:{id}:{parent}"));
    }
    let mut context: Vec<String> = Vec::new();
    for message in file.projected_messages() {
        use pi_ai::message::Message;
        match &message {
            Message::User { content, .. } => {
                context.push(format!("msg:user:{}", text_of(content)));
            }
            Message::Assistant(assistant) => {
                context.push(format!("msg:assistant:{}", assistant_text(assistant)));
            }
            Message::System { .. } => {}
            Message::ToolResult { .. } => {}
            Message::CompactionSummary { summary, .. } => {
                context.push(format!("compaction:{summary}"));
            }
            Message::BranchSummary { summary, .. } => {
                context.push(format!("branch_summary:{summary}"));
            }
            other => {
                let _ = other;
            }
        }
    }
    let value = serde_json::json!({ "tree": tree, "context": context });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_default()
    );
}

fn text_of(content: &pi_ai::UserContent) -> String {
    match content {
        pi_ai::UserContent::Text(text) => text.clone(),
        pi_ai::UserContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                pi_ai::message::Content::Text { text, .. } => Some(text.clone()),
                _other => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

fn assistant_text(assistant: &pi_ai::types::AssistantMessage) -> String {
    assistant
        .content
        .iter()
        .filter_map(|block| match block {
            pi_ai::types::Content::Text { text, .. } => Some(text.clone()),
            _other => None,
        })
        .collect::<Vec<_>>()
        .join("")
}
