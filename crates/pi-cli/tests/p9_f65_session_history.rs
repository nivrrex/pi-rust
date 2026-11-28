//! F65：`--session` / `-c` 打开已有会话时**历史必须显示**。
//!
//! 这条以前被误记为"不显示历史内容"。实际根因不在 pi-rs：写夹具时 assistant 消息
//! 少了 `api`/`provider`/`model`/`usage` 这些必填字段，整条 `Message` 反序列化失败，
//! `#[serde(flatten)] extra` 把外层 `parentId` 一起吞掉 ⇒ 条目链断开 ⇒
//! `build_session_projection` 只走到第一条。补全字段后两侧都正常。
//!
//! 用例因此固定**夹具必须完整**这一前提：先断言会话文件能被正确投影成 2 条消息，
//! 再断言屏幕上两条都在。若将来有人再写缺字段的夹具，第一条断言会先报错，
//! 而不是让"历史不显示"这个假象复现。

use std::path::Path;
use std::process::Command;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// 一条**字段完整**的 assistant 消息（缺任何一个都会让整条 entry 解析失败）。
fn assistant_entry(id: &str, parent: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "message",
        "id": id,
        "parentId": parent,
        "timestamp": "2026-01-01T00:00:02.000Z",
        "message": {
            "role": "assistant",
            "content": [{ "type": "text", "text": text }],
            "api": "openai-completions",
            "provider": "p",
            "model": "m1",
            "timestamp": 2000,
            "stopReason": "stop",
            "usage": {
                "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0,
                "cost": { "input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0 }
            }
        }
    })
}

fn write_session(dir: &Path, cwd: &Path) -> std::path::PathBuf {
    let path = dir.join("s1.jsonl");
    let header = serde_json::json!({
        "type": "session", "version": 1, "id": "sess-1",
        "timestamp": "2026-01-01T00:00:00.000Z",
        "cwd": cwd.display().to_string()
    });
    let user = serde_json::json!({
        "type": "message",
        "id": "a1",
        "parentId": null,
        "timestamp": "2026-01-01T00:00:01.000Z",
        "message": {
            "role": "user",
            "content": [{ "type": "text", "text": "FIRST-USER-LINE" }],
            "timestamp": 1000
        }
    });
    let body = format!(
        "{}\n{}\n{}\n",
        serde_json::to_string(&header).expect("header"),
        serde_json::to_string(&user).expect("user"),
        serde_json::to_string(&assistant_entry("a2", "a1", "FIRST-ASSISTANT-LINE"))
            .expect("assistant"),
    );
    std::fs::write(&path, body).expect("写会话文件");
    path
}

/// **前提**：完整夹具必须投影出 2 条消息（1 user + 1 assistant）。
///
/// 这条同时是"夹具别写残"的守卫：缺字段时投影只剩 1 条，这里先失败，
/// 不会让人误以为 pi-rs 不显示历史。
#[test]
fn complete_fixture_projects_both_messages() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).expect("建目录");
    let path = write_session(tmp.path(), &work);
    let file = pi_core::session_io::SessionFile::read(&path).expect("读会话");
    assert_eq!(file.entries.len(), 2, "两条 entry 都应解析成功");
    let messages = file.projected_messages();
    assert_eq!(
        messages.len(),
        2,
        "投影应有 user + assistant 两条，实际 {messages:?}"
    );
    let has_assistant = messages
        .iter()
        .any(|message| matches!(message, pi_ai::message::Message::Assistant(_)));
    assert!(has_assistant, "投影里必须有 assistant");
}

/// 端到端：`--session` 打开后屏幕上**同时**有用户行与助手行。
#[test]
fn session_flag_renders_the_history() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    let steps = tmp.path().join("steps");
    for dir in [&agent, &home, &work, &sess, &steps] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    // 有模型才走"正常恢复"分支（否则首屏只有无模型 warning）。
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1","reasoning":false}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    let session = write_session(&sess, &work);
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":2500,"timeoutMs":60000}},{"text":"\u0004"},{"delayMs":1200}]"#,
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
        .arg("--session")
        .arg(&session)
        .current_dir(&work)
        .env("HOME", &home)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_TEST_DUMP", &steps)
        .output()
        .expect("跑 pty");
    assert!(
        output.status.success(),
        "pi-rs 应正常退出：{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let snapshot = std::fs::read_to_string(steps.join("step-0.json")).expect("应有首帧快照");
    let value: serde_json::Value = serde_json::from_str(&snapshot).expect("解析快照");
    let items: Vec<&str> = value
        .get("items")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|item| item.get("kind").and_then(|v| v.as_str()))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        items,
        vec!["user", "assistant"],
        "首帧应按顺序含用户行与助手行，实际 {items:?}"
    );
}

/// F63/C 遗留：`Could not restore model …` 必须进**转录区**，不能只写 stderr。
///
/// TUI 是 inline 渲染，stderr 每行都会把首屏挤下一行（ref 用 `showWarning` 放会话区，
/// `interactive-mode.ts:1129-1137`）。此前这条用 `eprintln!`，屏幕上根本看不到。
#[test]
fn restore_failure_warning_lands_in_the_transcript() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    let steps = tmp.path().join("steps");
    for dir in [&agent, &home, &work, &sess, &steps] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1","reasoning":false}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    // 会话里记着一个**已不存在**的 provider/model ⇒ 恢复失败。
    let header = serde_json::json!({
        "type": "session", "version": 1, "id": "s",
        "timestamp": "2026-01-01T00:00:00.000Z",
        "cwd": work.display().to_string()
    });
    let model_change = serde_json::json!({
        "type": "model_change", "id": "m1", "parentId": null,
        "timestamp": "2026-01-01T00:00:01.000Z",
        "provider": "gone", "modelId": "nope"
    });
    let session = sess.join("s1.jsonl");
    std::fs::write(
        &session,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&header).expect("header"),
            serde_json::to_string(&model_change).expect("model_change")
        ),
    )
    .expect("写会话");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":2500,"timeoutMs":60000}},{"text":"\u0004"},{"delayMs":1200}]"#,
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
        .arg("--session")
        .arg(&session)
        .current_dir(&work)
        .env("HOME", &home)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_TEST_DUMP", &steps)
        .output()
        .expect("跑 pty");
    assert!(output.status.success(), "不应致命退出");
    let snapshot = std::fs::read_to_string(steps.join("step-0.json")).expect("应有首帧快照");
    let value: serde_json::Value = serde_json::from_str(&snapshot).expect("解析快照");
    let items = value
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let kinds: Vec<&str> = items
        .iter()
        .filter_map(|item| item.get("kind").and_then(|v| v.as_str()))
        .collect();
    assert!(
        kinds.contains(&"warning"),
        "应有 warning 条目，实际 {kinds:?}"
    );
    let text = items
        .iter()
        .filter_map(|item| item.get("first").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("Could not restore model"),
        "转录区应含恢复失败提示，实际：{text}"
    );
}
