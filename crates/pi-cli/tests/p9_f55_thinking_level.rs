//! F55/G2：thinking 档位的**来源链**必须与 ref 一致。
//!
//! ref 的取值链（`core/sdk.ts:230-254`）：
//! 1. `options.thinkingLevel`（CLI 参数）；
//! 2. 会话**叶子分支路径**上最后一条 `thinking_level_change`（`session-manager.ts:418-433`
//!    的 `getSessionContextSettings`，路径由 `buildSessionPath` 沿 `parentId` 上溯得到，
//!    **不是全文件扫描**）；
//! 3. 每模型覆盖 `modelThinkingLevels["provider/model"]`；
//! 4. `settings.defaultThinkingLevel`；
//! 5. `DEFAULT_THINKING_LEVEL`。
//!
//! 其中第 2 步的"路径"是关键：分支切换后（旧分支上还有一条 `thinking_level_change`），
//! 全文件扫描会取到**另一条分支**的值。三个用例都跑真实的 `pi-rs` 二进制（pty + PI_TEST_DUMP），
//! 断言的是**屏幕页脚**（用户看到的那一行），不是内部字段。

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

/// 一个最小 agent 配置：单 provider/单模型，`apiKey` 给 DUMMY（不联网）。
fn write_agent(agent: &Path, reasoning: bool, default_thinking: Option<&str>) {
    std::fs::create_dir_all(agent).expect("建 agent 目录");
    let mut settings = serde_json::json!({
        "theme": "dark",
        "defaultProvider": "p",
        "defaultModel": "m",
    });
    if let Some(level) = default_thinking {
        settings["defaultThinkingLevel"] = serde_json::Value::String(level.to_string());
    }
    std::fs::write(
        agent.join("settings.json"),
        serde_json::to_string(&settings).expect("序列化 settings"),
    )
    .expect("写 settings");
    let models = serde_json::json!({
        "providers": {
            "p": {
                "baseUrl": "http://127.0.0.1:1/v1",
                "api": "openai-completions",
                "apiKey": "DUMMY",
                "models": [{ "id": "m", "reasoning": reasoning, "contextWindow": 200000 }]
            }
        }
    });
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string(&models).expect("序列化 models"),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");
}

/// 会话条目：(type, 附加字段)。`parent` 用**下标**表示（`None` = 根）。
enum Entry {
    Session,
    Thinking(&'static str),
    Assistant,
    Compaction,
}

fn write_session(path: &Path, cwd: &Path, entries: &[(Entry, Option<usize>)]) {
    let mut out = String::new();
    let mut ids: Vec<String> = Vec::new();
    for (index, (entry, parent)) in entries.iter().enumerate() {
        let id = format!("e{index:03}");
        let parent_id = parent.map(|p| ids[p].clone());
        let value = match entry {
            Entry::Session => serde_json::json!({
                "type": "session",
                "id": id,
                "parentId": serde_json::Value::Null,
                "timestamp": "2026-01-01T00:00:00Z",
                "cwd": cwd.display().to_string(),
                "version": 3,
            }),
            Entry::Thinking(level) => serde_json::json!({
                "type": "thinking_level_change",
                "id": id,
                "parentId": parent_id,
                "timestamp": "2026-01-01T00:00:00Z",
                "thinkingLevel": level,
            }),
            Entry::Assistant => serde_json::json!({
                "type": "message",
                "id": id,
                "parentId": parent_id,
                "timestamp": "2026-01-01T00:00:00Z",
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "ok" }],
                    "api": "openai-completions",
                    "provider": "p",
                    "model": "m",
                    "usage": { "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                               "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 } },
                    "stopReason": "stop",
                    "timestamp": 0
                }
            }),
            Entry::Compaction => serde_json::json!({
                "type": "compaction",
                "id": id,
                "parentId": parent_id,
                "timestamp": "2026-01-01T00:00:00Z",
                "summary": "compacted",
                "firstKeptEntryId": ids.first().cloned().unwrap_or_default(),
                "tokensBefore": 1,
            }),
        };
        ids.push(id);
        out.push_str(&serde_json::to_string(&value).expect("序列化条目"));
        out.push('\n');
    }
    std::fs::write(path, out).expect("写会话");
}

/// 在 pty 下跑 `pi-rs -c`，返回屏幕最后 3 行（去尾空白）。
fn footer_of_continue(agent: &Path, home: &Path, work: &Path, sess: &Path) -> Option<String> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let keys_path = tmp.path().join("keys.json");
    // 首帧静止后打 mark，再 ctrl+d 退出；`PI_TEST_DUMP` 让 pi-rs 把屏幕落盘。
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
        .current_dir(work)
        .env("PI_CODING_AGENT_DIR", agent)
        .env("PI_CODING_AGENT_SESSION_DIR", sess)
        .env("PI_NODE", &node)
        .env("HOME", home)
        .env("PI_TEST_DUMP", &dump)
        .env("PI_TEST_TAIL", "3")
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    let tail = std::fs::read_to_string(dump.join("tail.txt")).ok()?;
    Some(
        tail.lines()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// (a) `thinking_level_change(high)` 在**最早**，之后 ≥2 次压缩，页脚仍须 `high`。
///
/// ref 靠 `getSessionContextSettings` 走**叶子分支路径**：压缩条目不切断 `parentId` 链，
/// 所以第 3 条条目仍在这条路径上。若实现改成"只从 compaction 之后的子范围扫描"，
/// 这条用例会看不到任何 `thinking_level_change` ⇒ 回落到默认值。
#[test]
fn thinking_level_survives_multiple_compactions() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    std::fs::create_dir_all(&home).expect("建 HOME");
    std::fs::create_dir_all(&work).expect("建 work");
    std::fs::create_dir_all(&sess).expect("建 sessions");
    // 默认档位设成 low：如果实现回落到默认值，页脚会是 `low`，与期望的 `high` 不同。
    write_agent(&agent, true, Some("low"));
    let session = sess.join("2026-01-01T00-00-00-000Z_test-high.jsonl");
    write_session(
        &session,
        &work,
        &[
            (Entry::Session, None),
            (Entry::Thinking("high"), Some(0)),
            (Entry::Assistant, Some(1)),
            (Entry::Compaction, Some(2)),
            (Entry::Assistant, Some(3)),
            (Entry::Compaction, Some(4)),
            (Entry::Assistant, Some(5)),
        ],
    );
    let Some(footer) = footer_of_continue(&agent, &home, &work, &sess) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        footer.contains("• high"),
        "两次压缩后思考档位仍应为 high，实际页脚：\n{footer}"
    );
}

/// (a2) 分支切换：**别的分支**上还有一条 `thinking_level_change(off)`，而叶子所在路径上是
/// `high` ⇒ ref 沿 `parentId` 上溯只看到 `high`，**全文件倒扫**会看到 `off`。
///
/// 这条是真正的判据：pi-rs 当前实现是"读整个文件、取最后一条 `thinking_level_change`"，
/// 与 ref 的 `getSessionContextSettings(buildSessionPath(...))` 不等价。
#[test]
fn thinking_level_uses_leaf_branch_not_whole_file() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    std::fs::create_dir_all(&home).expect("建 HOME");
    std::fs::create_dir_all(&work).expect("建 work");
    std::fs::create_dir_all(&sess).expect("建 sessions");
    write_agent(&agent, true, Some("low"));
    let session = sess.join("2026-01-01T00-00-00-000Z_test-branch.jsonl");
    // e3 是**死分支**上的 off；e4 才是叶子（父节点与 e3 相同）⇒ 路径 = e0,e1,e2,e4。
    write_session(
        &session,
        &work,
        &[
            (Entry::Session, None),
            (Entry::Thinking("high"), Some(0)),
            (Entry::Assistant, Some(1)),
            (Entry::Thinking("off"), Some(2)),
            (Entry::Assistant, Some(2)),
        ],
    );
    let Some(footer) = footer_of_continue(&agent, &home, &work, &sess) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        footer.contains("• high"),
        "档位必须取自叶子分支（high），不能取全文件最后一条（off）。实际页脚：\n{footer}"
    );
}

/// (b) 自定义 provider 的模型 `reasoning=true` + `settings.defaultThinkingLevel=high`、
/// 会话里**没有** `thinking_level_change` ⇒ 页脚 `high`（不是 `off`/`medium`）。
#[test]
fn thinking_level_falls_back_to_settings_default() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    std::fs::create_dir_all(&home).expect("建 HOME");
    std::fs::create_dir_all(&work).expect("建 work");
    std::fs::create_dir_all(&sess).expect("建 sessions");
    write_agent(&agent, true, Some("high"));
    let session = sess.join("2026-01-01T00-00-00-000Z_test-default.jsonl");
    write_session(
        &session,
        &work,
        &[(Entry::Session, None), (Entry::Assistant, Some(0))],
    );
    let Some(footer) = footer_of_continue(&agent, &home, &work, &sess) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        footer.contains("• high"),
        "会话无 thinking 条目时应取 settings 默认 high，实际页脚：\n{footer}"
    );
}

/// (c) `reasoning=false` ⇒ ref 不在页脚显示档位（`footer.ts:206-211` 只在 `state.model.reasoning`
/// 为真时才拼 `• <level>`）。
#[test]
fn non_reasoning_model_shows_no_thinking_level() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    std::fs::create_dir_all(&home).expect("建 HOME");
    std::fs::create_dir_all(&work).expect("建 work");
    std::fs::create_dir_all(&sess).expect("建 sessions");
    write_agent(&agent, false, Some("high"));
    let session = sess.join("2026-01-01T00-00-00-000Z_test-noreason.jsonl");
    write_session(
        &session,
        &work,
        &[
            (Entry::Session, None),
            (Entry::Thinking("high"), Some(0)),
            (Entry::Assistant, Some(1)),
        ],
    );
    let Some(footer) = footer_of_continue(&agent, &home, &work, &sess) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        !footer.contains("• high") && !footer.contains("• off"),
        "非推理模型不应显示思考档位，实际页脚：\n{footer}"
    );
}
