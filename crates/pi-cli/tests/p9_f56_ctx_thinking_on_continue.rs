//! F56/G2：`-c` 恢复分支必须把 **当前思考档位** 也推进扩展 ctx。
//!
//! ref：`ctx.thinkingLevel` 是**当前值**（`types.ts:325-334`），扩展页脚直接读它
//! （`pi-ai-summary/src/footer-cny.ts:357-359`：`(ctx as any).thinkingLevel || "off"`）。
//! 所以 `-c` 之后若只推 `ctx.model` 而漏推 `ctx.thinkingLevel`，扩展页脚就会把
//! 真实档位（例如 `high`）显示成 `thinking off` —— 这正是真机上看到的现象。
//!
//! 本用例用**合成会话**（不含任何真实内容），跑真实 `pi-rs -c`，断言屏幕页脚。
//! 关掉扩展（`--no-extensions`）时用**内置**页脚做对照：内置页脚走 `state.thinking_level`，
//! 两条路径都要对。

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
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m","defaultThinkingLevel":"low"}"#,
    )
    .expect("写 settings");
    // `reasoning: true` 才会显示档位（ref `footer.ts:206-211`）。
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m","reasoning":true,"contextWindow":200000}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");
}

/// 合成会话：`thinking_level_change(high)` + 一条 assistant。
///
/// 会话里是 `high`、settings 默认是 `low` ⇒ 页脚出现 `high` 就证明取的是**会话值**
/// 且**推到了扩展 ctx**。
fn write_session(path: &Path, cwd: &Path) {
    let header = serde_json::json!({
        "type": "session", "id": "e000", "parentId": serde_json::Value::Null,
        "timestamp": "2026-01-01T00:00:00Z", "cwd": cwd.display().to_string(), "version": 3,
    });
    let thinking = serde_json::json!({
        "type": "thinking_level_change", "id": "e001", "parentId": "e000",
        "timestamp": "2026-01-01T00:00:00Z", "thinkingLevel": "high",
    });
    let assistant = serde_json::json!({
        "type": "message", "id": "e002", "parentId": "e001",
        "timestamp": "2026-01-01T00:00:00Z",
        "message": {
            "role": "assistant",
            "content": [{ "type": "text", "text": "ok" }],
            "api": "openai-completions", "provider": "p", "model": "m",
            "usage": { "input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                       "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 } },
            "stopReason": "stop", "timestamp": 0
        }
    });
    let body = [header, thinking, assistant]
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, body + "\n").expect("写会话");
}

/// 跑 `pi-rs -c [--no-extensions]`，返回整屏（`PI_TEST_TAIL` = 40）。
fn screen(no_extensions: bool) -> Option<String> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    std::fs::create_dir_all(&home).expect("建 HOME");
    std::fs::create_dir_all(&work).expect("建 work");
    std::fs::create_dir_all(&sess).expect("建 sessions");
    write_agent(&agent);
    write_session(&sess.join("2026-01-01T00-00-00-000Z_g2.jsonl"), &work);
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"m0"},{"text":"\u0004"},{"delayMs":600}]"#,
    )
    .expect("写 keys");
    let dump = tmp.path().join("steps");
    let mut cmd = Command::new("python3");
    cmd.arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("40")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("-c");
    if no_extensions {
        cmd.arg("--no-extensions");
    }
    let status = cmd
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
    std::fs::read_to_string(dump.join("tail.txt")).ok()
}

/// 内置页脚（`--no-extensions`）必须显示会话里的 `high`。
#[test]
fn builtin_footer_shows_session_thinking_level_on_continue() {
    let Some(text) = screen(true) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        text.contains("• high"),
        "内置页脚应显示会话档位 high，实际屏幕：\n{text}"
    );
}

/// **扩展在 `session_start` 读到的 `ctx.thinkingLevel` 必须是当前档位**。
///
/// 用 `tools/ext-host/fixtures/ctx-ext.mjs`（它把 `ctx.thinkingLevel` 追加进
/// `PI_EXT_TEST_LOG`）作为判据——这是扩展侧**可观测**的值，比"页脚文字里有没有 off"
/// 更直接（页脚文字还受主题/宽度影响）。
///
/// 改前：`-c` 分支只推 `ctx.model`，扩展读到 `null`（夹具记为 `…:null`）
/// ⇒ `pi-ai-summary` 的 `(ctx as any).thinkingLevel || "off"` 生效
/// ⇒ 页脚 ` • thinking off`（真机 G2 现象）。
#[test]
fn extension_receives_thinking_level_on_continue() {
    let node = match node_path() {
        Some(node) => node,
        None => {
            eprintln!("跳过：本机没有 node");
            return;
        }
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    for dir in [&home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    write_agent(&agent);
    write_session(&sess.join("2026-01-01T00-00-00-000Z_g2ext.jsonl"), &work);
    // 装夹具扩展
    let ext = agent.join("extensions").join("ctx-ext");
    std::fs::create_dir_all(&ext).expect("建扩展目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/ctx-ext.mjs"))
        .expect("读 ctx-ext 夹具");
    std::fs::write(ext.join("index.mjs"), body).expect("写扩展");
    let log = tmp.path().join("ext.log");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"m0"},{"text":"\u0004"},{"delayMs":600}]"#,
    )
    .expect("写 keys");
    let dump = tmp.path().join("steps");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("40")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("-c")
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_EXT_TEST_LOG", &log)
        .env("PI_TEST_DUMP", &dump)
        .env("PI_TEST_TAIL", "40")
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!text.is_empty(), "扩展应写出 ctx 日志，实际为空");
    // 夹具格式：`ctx:<cwd>:<mode>:<hasUI>:<model>:<thinkingLevel>`
    let line = text
        .lines()
        .find(|line| line.starts_with("ctx:"))
        .expect("应有一行 ctx 记录");
    let level = line.rsplit(':').next().unwrap_or("");
    assert_ne!(
        level, "null",
        "扩展读到的 ctx.thinkingLevel 不应是 null（-c 分支必须推它）：{line}"
    );
    assert_eq!(level, "high", "扩展应读到会话档位 high：{line}");
}
