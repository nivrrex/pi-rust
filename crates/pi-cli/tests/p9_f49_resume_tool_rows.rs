//! F49/Y2：`-c` 恢复时，工具行必须带上**参数与结果**（折叠态 `✓ 名 {args} …`、
//! 展开态 `ctrl+o` 能看到 diff / bash 输出 / report_to_user 的 kind）。
//!
//! 根因（`crates/pi-cli/src/tui.rs` 的 `transcript_from_messages`）：
//! 1. `Content::ToolCall` 分支把 `output`/`is_error`/`result_payload` 全部写死 `None`；
//! 2. `Message::ToolResult` 落到"忽略"分支（`tui.rs:273-277`）。
//! 于是恢复出来的工具行既没有结果正文、也没有 `isError`，折叠行拿不到 `result`，
//! 展开态只剩一行路径（真机：edit 无 diff、bash 无输出、report_to_user 只有 `done`）。
//!
//! ref `interactive-mode.ts:3982-4010`：`renderSessionItems` 先建 `ToolExecutionComponent`，
//! 再在遇到 `message.role === "toolResult"` 时用 `renderedPendingTools.get(toolCallId)`
//! 把它 `updateResult(message)` —— **按 toolCallId 配对**。

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

/// 合成会话：assistant 发 5 个工具调用，随后 5 条 toolResult（其中一条 isError）。
fn session_jsonl(cwd: &str) -> String {
    let usage = r#""usage":{"input":10,"output":5,"cacheRead":0,"cacheWrite":0,"totalTokens":15,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}}"#;
    let calls = [
        (
            "call_edit",
            "edit",
            r#"{"path":"alpha.txt","old":"A","new":"B"}"#,
        ),
        ("call_bash", "bash", r#"{"command":"echo synth-line-1"}"#),
        ("call_read", "read", r#"{"path":"alpha.txt"}"#),
        (
            "call_report",
            "report_to_user",
            r#"{"kind":"done","summary":"合成汇报"}"#,
        ),
        ("call_bad", "zeta_broken", r#"{"x":1}"#),
    ];
    let mut lines = vec![format!(
        r#"{{"type":"session","version":3,"id":"s-f49","timestamp":"2026-01-01T00:00:00.000Z","cwd":{cwd:?}}}"#
    )];
    lines.push(
        r#"{"type":"model_change","id":"e1","parentId":null,"timestamp":"2026-01-01T00:00:00.000Z","provider":"p","modelId":"m"}"#.to_string(),
    );
    lines.push(format!(
        r#"{{"type":"message","id":"u1","parentId":"e1","timestamp":"2026-01-01T00:00:00.000Z","message":{{"role":"user","content":[{{"type":"text","text":"跑五个合成工具"}}],"timestamp":1}}}}"#
    ));
    let mut content = String::from(r#"[{"type":"text","text":"依次执行："}"#);
    for (id, name, args) in calls {
        content.push_str(&format!(
            r#",{{"type":"toolCall","id":"{id}","name":"{name}","arguments":{args}}}"#
        ));
    }
    content.push(']');
    lines.push(format!(
        r#"{{"type":"message","id":"a1","parentId":"u1","timestamp":"2026-01-01T00:00:01.000Z","message":{{"role":"assistant","api":"openai-completions","provider":"p","model":"m","content":{content},"stopReason":"toolUse",{usage},"timestamp":2}}}}"#
    ));
    // edit 的结果带 details.diff；bash 多行输出；出错的工具 isError=true。
    let results = [
        (
            "call_edit",
            "edit",
            "已替换 1 处",
            false,
            r#""details":{"diff":"--- a/alpha.txt\n+++ b/alpha.txt\n-A\n+B"}"#,
        ),
        (
            "call_bash",
            "bash",
            "synth-line-1\nsynth-line-2\nsynth-line-3",
            false,
            r#""details":{"exitCode":0}"#,
        ),
        ("call_read", "read", "A\n", false, r#""details":{}"#),
        (
            "call_report",
            "report_to_user",
            "已汇报",
            false,
            r#""details":{}"#,
        ),
        (
            "call_bad",
            "zeta_broken",
            "Tool zeta_broken not found",
            true,
            r#""details":{}"#,
        ),
    ];
    let mut parent = "a1".to_string();
    for (i, (call_id, name, text, is_error, details)) in results.into_iter().enumerate() {
        let id = format!("r{i}");
        lines.push(format!(
            r#"{{"type":"message","id":"{id}","parentId":"{parent}","timestamp":"2026-01-01T00:00:02.000Z","message":{{"role":"toolResult","toolCallId":"{call_id}","toolName":"{name}","content":[{{"type":"text","text":{text:?}}}],"isError":{is_error},{details},"timestamp":{}}}}}"#,
            3 + i
        ));
        parent = id;
    }
    lines.join("\n") + "\n"
}

fn run_tui(keys: &str) -> Option<Vec<u8>> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(work.join("alpha.txt"), "A\n").expect("写夹具文件");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m","contextWindow":197000}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        sess.join("2026-01-01T00-00-00-000Z_s-f49.jsonl"),
        session_jsonl(&work.to_string_lossy()),
    )
    .expect("写会话");
    let keys_path = tmp.path().join("keys.json");
    std::fs::write(&keys_path, keys).expect("写 keys");
    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
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
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    Some(std::fs::read(&raw_path).expect("读 raw"))
}

/// 折叠态：工具行必须带**参数摘要**（`{"command":"echo synth-line-1"}` 这类 JSON）。
#[test]
fn resumed_tool_rows_carry_args() {
    let Some(raw) = run_tui(r#"[{"delayMs":3500},{"raw":"\u0004"},{"delayMs":600}]"#) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    let plain: String = text
        .chars()
        .filter(|ch| *ch == '\n' || *ch == '\r' || !ch.is_control())
        .collect();
    assert!(
        plain.contains("synth-line-1") || plain.contains("echo synth-line-1"),
        "恢复出的 bash 工具行应带命令行参数，实际屏上找不到：{:?}",
        plain
            .lines()
            .filter(|line| line.contains("bash"))
            .take(3)
            .collect::<Vec<_>>()
    );
}

/// 展开态（ctrl+o）：必须能看到结果正文（bash 输出、edit 的 diff）。
#[test]
fn expanded_resumed_tool_rows_show_results() {
    let Some(raw) = run_tui(
        r#"[{"delayMs":3500},{"raw":"\u000f"},{"delayMs":2500},{"raw":"\u0004"},{"delayMs":600}]"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    let plain: String = text
        .chars()
        .filter(|ch| *ch == '\n' || *ch == '\r' || !ch.is_control())
        .collect();
    assert!(
        plain.contains("synth-line-2"),
        "展开后应看到 bash 的输出正文，实际：{:?}",
        plain
            .lines()
            .filter(|line| line.contains("synth-line"))
            .take(4)
            .collect::<Vec<_>>()
    );
    // edit 的结果正文（`已替换 1 处`）必须出现；`details.diff` 由扩展侧渲染
    // （原生 pi-tui 不消费 details，见 ref `renderers/edit.ts` 属于工具定义侧渲染）。
    assert!(
        plain.contains("已替换 1 处"),
        "展开后应看到 edit 的结果正文，实际：{:?}",
        plain
            .lines()
            .filter(|line| line.contains("已替换") || line.contains("edit"))
            .take(6)
            .collect::<Vec<_>>()
    );
}
