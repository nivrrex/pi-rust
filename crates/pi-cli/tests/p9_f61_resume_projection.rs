//! F61/12：`/resume` 切换会话后，屏幕必须按**投影口径**重建
//! （ref `handleResumeSession` → `runtimeHost.switchSession` → `renderInitialMessages()`，
//! 后者读 `buildContextEntries`，从最后一条 compaction 起）。
//!
//! 此前用 `settings_selector::replay_messages`（**全文件顺序**的所有 message）⇒
//! resume 一个压缩过的会话时，屏幕上会重新出现**压缩之前**的历史，
//! 与 ref 的行为不一致（也与 F61/4 修的树导航问题同类）。
//!
//! 断言：`-c` 恢复一个"压缩过"的会话时，压缩前的消息**不出现在屏幕上**，
//! 压缩摘要与压缩后的消息出现。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

fn screen_of(raw: &Path) -> String {
    let bytes = std::fs::read(raw).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if next == '\u{7}' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[test]
fn resume_shows_projected_context_not_pre_compaction_history() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&agent, &home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m","name":"m","contextWindow":200000,"maxTokens":1024}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");

    // 会话：压缩前的两轮 + compaction + 压缩后的一轮。
    // `firstKeptEntryId` 指向 `u2`（压缩后保留的起点），所以 `u1/a1` 应被投影排除。
    let session = sessions.join("2026-03-01T00-00-00-000Z_rs.jsonl");
    let header = serde_json::json!({
        "type": "session", "id": "s0", "parentId": serde_json::Value::Null,
        "timestamp": "2026-03-01T00:00:00Z", "cwd": work.display().to_string(), "version": 3,
    });
    let msg = |id: &str, parent: &str, role: &str, text: &str, extra: serde_json::Value| {
        let mut message = serde_json::json!({
            "role": role,
            "content": [{"type": "text", "text": text}],
            "timestamp": 1,
        });
        if let Some(object) = extra.as_object() {
            for (key, value) in object {
                message[key] = value.clone();
            }
        }
        serde_json::json!({
            "type": "message", "id": id, "parentId": parent,
            "timestamp": "2026-03-01T00:00:01Z", "message": message,
        })
    };
    let assistant_extra = serde_json::json!({
        "api": "openai-completions", "provider": "p", "model": "m",
        "usage": {"input": 10, "output": 2, "cacheRead": 0, "cacheWrite": 0,
            "totalTokens": 12, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
        "stopReason": "stop",
    });
    let entries = vec![
        header,
        msg("u1", "s0", "user", "压缩前的提问", serde_json::Value::Null),
        msg(
            "a1",
            "u1",
            "assistant",
            "压缩前的回答",
            assistant_extra.clone(),
        ),
        // `firstKeptEntryId` 必须指向**压缩之前**的条目（ref
        // `session-manager.ts:496-505` 只在 `path[..compactionIdx]` 里找它）。
        // 这里指向 `a1`：即"从 a1 起保留"，于是屏幕会显示 `a1` 而排除 `u1`。
        serde_json::json!({
            "type": "compaction", "id": "c1", "parentId": "a1",
            "timestamp": "2026-03-01T00:00:02Z",
            "summary": "压缩摘要正文", "firstKeptEntryId": "a1", "tokensBefore": 1000,
            "details": {"readFiles": [], "modifiedFiles": []},
        }),
        msg("u2", "c1", "user", "压缩后的提问", serde_json::Value::Null),
        msg("a2", "u2", "assistant", "压缩后的回答", assistant_extra),
    ];
    let mut file = std::fs::File::create(&session).expect("建会话");
    for entry in &entries {
        writeln!(file, "{entry}").expect("写会话");
    }
    drop(file);

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        // **必须走 `/resume` 面板**：`--session` 的启动路径不经过它，
        // 只有面板选中（`SelectorUi::Resume` 的 Enter 分支）才用得上重放逻辑。
        // 两次回车：第一次接受命令补全，第二次真正提交。
        r#"[{"waitStable":{"quietMs":2000,"timeoutMs":90000}},{"mark":"s0"},
            {"text":"/resume"},{"delayMs":800},{"text":"\r"},{"delayMs":2500},{"mark":"panel"},
            {"text":"\r"},{"delayMs":4000},{"mark":"picked"},
            {"delayMs":1500},{"mark":"picked2"},
            {"text":"\u0004"},{"delayMs":1500}]"#,
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("34")
        .arg("--exit-code-file")
        .arg(&code)
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--session")
        .arg(&session)
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_TEST_DUMP", tmp.path().join("steps"))
        .env("PI_TEST_TAIL", "34")
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("COLORTERM", "")
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 应成功（stderr：{}）",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = screen_of(&raw);
    // 用 `PI_TEST_TAIL` 的 `tail.txt`（**真实最后一帧的末尾 N 行**）作为判据：
    // 直接去 raw.bin 里按 ANSI 剥壳取"最后若干行"会被增量重绘的中间帧干扰，
    // 而 `tail.txt` 是渲染层在退出前写的最终屏幕。
    let screen = std::fs::read_to_string(tmp.path().join("steps/tail.txt")).unwrap_or_default();

    // 只检查**面板关闭之后**的屏幕（`Resumed session` 之后的部分）：
    // `/resume` 的会话列表本身会把首条用户消息当**标题**显示
    // （`› 压缩前的提问`），那不是会话内容，不能参与"是否重放旧分支"的判断。
    let after_resume = screen.clone();
    // 压缩之后的内容必须出现；保留起点（`a1`）也必须出现。
    for needle in ["压缩后的提问", "压缩后的回答", "压缩前的回答"] {
        assert!(
            after_resume.contains(needle),
            "resume 之后屏幕应显示 {needle:?}。屏幕尾部：\n{after_resume}"
        );
    }
    // 压缩**之前、且不在保留范围内**的内容不得出现（`firstKeptEntryId = a1`，
    // 所以 `u1` 被排除）。全文件重放会把它带回来。
    assert!(
        !after_resume.contains("压缩前的提问"),
        "投影应排除保留起点之前的消息（全文件重放会把它带回来）。屏幕尾部：\n{after_resume}"
    );
}

/// **单元级对拍**（不依赖 `/resume` 面板的列表/过滤行为）：
/// 同一份"压缩过的会话"，`replay_messages`（旧口径，全文件）会包含压缩前的消息，
/// 而 `build_session_projection`（新口径，ref `buildContextEntries`）不会。
///
/// 这条把"两种口径的差异"直接钉住——`/resume` 面板只是调用这两个函数之一，
/// 面板的按键路径已由 `T36-resume` 等 golden 覆盖。
#[test]
fn replay_and_projection_disagree_on_compacted_sessions() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("compacted.jsonl");
    let header = serde_json::json!({
        "type": "session", "id": "s0", "parentId": serde_json::Value::Null,
        "timestamp": "2026-03-01T00:00:00Z", "cwd": "/tmp", "version": 3,
    });
    let text_msg = |id: &str, parent: &str, role: &str, text: &str| {
        serde_json::json!({
            "type": "message", "id": id, "parentId": parent,
            "timestamp": "2026-03-01T00:00:01Z",
            "message": {"role": role, "content": [{"type": "text", "text": text}], "timestamp": 1},
        })
    };
    let lines = [
        header.to_string(),
        text_msg("u1", "s0", "user", "压缩前的提问").to_string(),
        text_msg("a1", "u1", "assistant", "压缩前的回答").to_string(),
        serde_json::json!({
            "type": "compaction", "id": "c1", "parentId": "a1",
            "timestamp": "2026-03-01T00:00:02Z",
            "summary": "摘要", "firstKeptEntryId": "u2", "tokensBefore": 1000,
        })
        .to_string(),
        text_msg("u2", "c1", "user", "压缩后的提问").to_string(),
        text_msg("a2", "u2", "assistant", "压缩后的回答").to_string(),
    ];
    std::fs::write(&path, lines.join("\n") + "\n").expect("写会话");

    let replayed = pi_cli::settings_selector::replay_messages(&path).expect("可读");
    let replayed_text: Vec<String> = replayed.into_iter().map(|(_a, text)| text).collect();
    assert!(
        replayed_text.iter().any(|t| t.contains("压缩前的提问")),
        "旧口径（全文件重放）**应当**包含压缩前消息——这正是它的问题。实际：{replayed_text:?}"
    );

    let file = pi_core::session_io::SessionFile::read(&path).expect("可读");
    let projected: Vec<String> = file
        .projected_messages()
        .iter()
        .filter_map(|message| match message {
            pi_ai::message::Message::User { content, .. } => Some(match content {
                pi_ai::UserContent::Text(text) => text.clone(),
                pi_ai::UserContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        pi_ai::message::Content::Text { text, .. } => Some(text.clone()),
                        _other => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            }),
            pi_ai::message::Message::Assistant(assistant) => Some(
                assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        pi_ai::types::Content::Text { text, .. } => Some(text.clone()),
                        _other => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _other => None,
        })
        .collect();
    assert!(
        !projected.iter().any(|t| t.contains("压缩前的提问")),
        "投影口径**必须**排除压缩前消息（ref :buildContextEntries）。实际：{projected:?}"
    );
    assert!(
        projected.iter().any(|t| t.contains("压缩后的提问")),
        "投影口径应保留压缩后的消息。实际：{projected:?}"
    );
}
