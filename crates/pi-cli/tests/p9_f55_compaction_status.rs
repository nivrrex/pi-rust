//! F55/B：`-c` 恢复视图末尾的 `Session compacted N times`。
//!
//! ref `interactive-mode.ts:4064-4070`（`renderInitialMessages()` 之后）：
//! ```ts
//! const compactionCount = allEntries.filter((e) => e.type === "compaction").length;
//! if (compactionCount > 0) {
//!   const times = compactionCount === 1 ? "1 time" : `${compactionCount} times`;
//!   this.showStatus(`Session compacted ${times}`);
//! }
//! ```
//! `showStatus` = `Spacer(1)` + `Text(msg, 1, 0)` + dim（`interactive-mode.ts:3695-3711`），
//! 所以它是**状态行**（转录区最后一行），不是转录块。
//!
//! 改前 pi-rs 完全没有这一行；改后用真实 `pi-rs` 二进制跑 pty，断言屏幕里出现该行。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn write_agent(agent: &Path) {
    std::fs::create_dir_all(agent).expect("建 agent 目录");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(agent.join("auth.json"), r#"{"p":{"type":"api_key","key":"DUMMY"}}"#)
        .expect("写 auth");
}

/// 合成会话：1 个 user、1 个 assistant、`compactions` 次压缩，每两次压缩间一问一答。
fn write_session(path: &Path, cwd: &Path, compactions: usize) {
    let mut lines: Vec<String> = Vec::new();
    let mut next = 0usize;
    let mut id = |next: &mut usize| {
        *next += 1;
        format!("e{:03}", *next)
    };
    let header = id(&mut next);
    lines.push(
        serde_json::json!({
            "type": "session",
            "id": header,
            "parentId": serde_json::Value::Null,
            "timestamp": "2026-01-01T00:00:00Z",
            "cwd": cwd.display().to_string(),
            "version": 3,
        })
        .to_string(),
    );
    let mut parent = header;
    fn push_message(
        lines: &mut Vec<String>,
        parent: &mut String,
        role: &str,
        text: &str,
        next: &mut usize,
    ) {
        let mut message = serde_json::json!({
            "role": role,
            "content": [{ "type": "text", "text": text }],
        });
        if role == "assistant" {
            message["api"] = serde_json::json!("openai-completions");
            message["provider"] = serde_json::json!("p");
            message["model"] = serde_json::json!("m");
            message["usage"] = serde_json::json!({
                "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 }
            });
            message["stopReason"] = serde_json::json!("stop");
            message["timestamp"] = serde_json::json!(0);
        }
        *next += 1;
        let entry = format!("e{:03}", *next);
        lines.push(
            serde_json::json!({
                "type": "message",
                "id": entry,
                "parentId": parent.clone(),
                "timestamp": "2026-01-01T00:00:00Z",
                "message": message,
            })
            .to_string(),
        );
        *parent = entry;
    }
    push_message(&mut lines, &mut parent, "user", "第一问", &mut next);
    push_message(&mut lines, &mut parent, "assistant", "第一答", &mut next);
    let first_kept = parent.clone();
    for index in 0..compactions {
        next += 1;
        let entry = format!("e{:03}", next);
        lines.push(
            serde_json::json!({
                "type": "compaction",
                "id": entry,
                "parentId": parent.clone(),
                "timestamp": "2026-01-01T00:00:00Z",
                "summary": "compacted",
                "firstKeptEntryId": first_kept.clone(),
                "tokensBefore": 1,
            })
            .to_string(),
        );
        parent = entry;
        push_message(&mut lines, &mut parent, "user", &format!("第{}问", index + 2), &mut next);
        push_message(&mut lines, &mut parent, "assistant", &format!("第{}答", index + 2), &mut next);
    }
    std::fs::write(path, lines.join("\n") + "\n").expect("写会话");
}

/// 跑 `pi-rs -c`，返回整个可见屏幕（`PI_TEST_TAIL` = 40 行即整屏）。
fn screen_of_continue(compactions: usize) -> Option<String> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    std::fs::create_dir_all(&home).expect("建 HOME");
    std::fs::create_dir_all(&work).expect("建 work");
    std::fs::create_dir_all(&sess).expect("建 sessions");
    std::fs::create_dir_all(work.join(".git")).expect("占位 .git"); // 页脚不显示分支，简化
    write_agent(&agent);
    write_session(
        &sess.join("2026-01-01T00-00-00-000Z_comp.jsonl"),
        &work,
        compactions,
    );
    let keys_path = tmp.path().join("keys.json");
    std::fs::write(
        &keys_path,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"m0"},{"text":"\u0004"},{"delayMs":600}]"#,
    )
    .expect("写 keys");
    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let dump = tmp.path().join("steps");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("40")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys_path)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("-c")
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_TEST_DUMP", &dump)
        .env("PI_TEST_TAIL", "40")
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    Some(std::fs::read_to_string(dump.join("tail.txt")).ok()?)
}

/// 两次压缩 ⇒ 屏幕出现 `Session compacted 2 times`。
///
/// 改前（`AgentParts` 没有 `compaction_count`、`tui.rs` 也没有这段）屏幕上**没有**该行，
/// 断言失败文本为：`期望出现 "Session compacted 2 times"`。
#[test]
fn compaction_count_status_row_is_rendered() {
    let Some(screen) = screen_of_continue(2) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("Session compacted 2 times"),
        "期望出现 \"Session compacted 2 times\"，实际屏幕：\n{screen}"
    );
}

/// 只有 1 次压缩时用单数 `1 time`（ref 的 `compactionCount === 1 ? "1 time" : …`）。
#[test]
fn compaction_count_uses_singular_for_one() {
    let Some(screen) = screen_of_continue(1) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("Session compacted 1 time"),
        "期望出现 \"Session compacted 1 time\"，实际屏幕：\n{screen}"
    );
    assert!(
        !screen.contains("1 times"),
        "单数不应写成 \"1 times\"，实际屏幕：\n{screen}"
    );
}

/// 没有压缩条目时**不出现**该行（ref 的 `if (compactionCount > 0)`）。
#[test]
fn compaction_status_absent_without_compactions() {
    let Some(screen) = screen_of_continue(0) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !screen.contains("Session compacted"),
        "没有压缩条目时不应出现该行，实际屏幕：\n{screen}"
    );
}
