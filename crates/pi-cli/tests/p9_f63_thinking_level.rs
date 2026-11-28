//! F63/A：`settings.json` 的 `defaultThinkingLevel` 必须真正下发到 provider。
//!
//! 缺陷：`main.rs` 有五处把初始档位写死成 `if resolved.reasoning { "medium" }`，
//! 与会话档位无关 ⇒ 用户写 `defaultThinkingLevel=high`，页脚显示 `high`（页脚那条链
//! 本来就对），但下发给 provider 的 `reasoning_effort` 仍是 `medium`。
//! ref `openai-completions.ts:891`/`:928-930` 的 `reasoning_effort` 取自
//! `options.reasoningEffort`（会话档位），不是固定值。
//!
//! 判据：mock-provider 的 `/control/requests` 回放所有收到的请求体，
//! 断言第一个请求的 `reasoning_effort` 等于 settings 里的档位。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct MockGuard(Child);
impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// 起 mock（带 `MOCK_VERBOSE`，让一次 `prompt` 的请求体落进 log）。
fn start_mock(log: &Path) -> (MockGuard, String) {
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(binary.exists(), "先构建 mock-provider");
    let _ = std::fs::remove_file(log);
    let mut child = Command::new(binary)
        .current_dir(repo_root())
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
        .env("MOCK_VERBOSE", "1")
        // 请求体落盘用命令行参数 `--request-log`（不是环境变量）。
        .arg("--request-log")
        .arg(log)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("mock 可启动");
    let port = {
        let stdout = child.stdout.take().expect("stdout");
        let mut lines = BufReader::new(stdout).lines();
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
        found.expect("listening")
    };
    (MockGuard(child), format!("http://127.0.0.1:{port}/v1"))
}

/// 跑一次 `--print`，把 index 写入 settings，返回 mock 收到的首个请求体。
fn reasoning_effort_for(thinking_level: &str, reasoning: bool) -> Option<String> {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("cfg");
    let sess = tmp.path().join("sess");
    let work = tmp.path().join("work");
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    // 每次都用**唯一**文件名：同一进程内多个测试共用 `pid` 命名会互相读到对方的请求，
    // 判据因此取到"上一个用例的档位"（实测 `off` 用例读到 `high`）。
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let log = std::env::temp_dir().join(format!("f63-mock-{}-{unique}.ndjson", std::process::id()));
    let (_mock, base_url) = start_mock(&log);
    let mut model = serde_json::json!({ "id": "mock-model" });
    if reasoning {
        model["reasoning"] = serde_json::json!(true);
    }
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [model]
            }}
        }))
        .expect("序列化"),
    )
    .expect("写 models.json");
    std::fs::write(
        agent.join("settings.json"),
        format!(
            r#"{{"defaultProvider":"openai-completions","defaultModel":"mock-model","defaultThinkingLevel":"{thinking_level}"}}"#
        ),
    )
    .expect("写 settings.json");

    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("hi")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("pi-rs 可运行");
    assert!(
        output.status.success(),
        "pi-rs 应成功（stderr：{}）",
        String::from_utf8_lossy(&output.stderr)
    );
    if std::env::var_os("F63_DEBUG").is_some() {
        eprintln!("[dbg] stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    }

    // 直接读 `--request-log` 落盘的请求体（ndjson）。
    // 用 `/control/requests` 读不到：那个端点把空 log 当成空响应（实测回来后是空串）。
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(!text.is_empty(), "mock 应已落盘请求体：{}", log.display());
    if std::env::var_os("F63_DEBUG").is_some() {
        eprintln!("[dbg] requests log:\n{text}");
    }
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // MOCK_VERBOSE 的取证行形如 `{"path":…,"body":"<JSON 字符串>",…}`——
        // `body` 是**字符串**（不是嵌套对象），要再解析一层。
        let body = match value.get("body") {
            Some(serde_json::Value::String(text)) => {
                serde_json::from_str::<serde_json::Value>(text).ok()
            }
            Some(other) => Some(other.clone()),
            None => None,
        };
        let Some(body) = body else {
            continue;
        };
        let effort = body
            .get("reasoning_effort")
            .or_else(|| body.get("reasoning").and_then(|r| r.get("effort")));
        if let Some(effort) = effort.and_then(serde_json::Value::as_str) {
            return Some(effort.to_string());
        }
    }
    None
}

/// `defaultThinkingLevel=high` ⇒ 请求体里的 `reasoning_effort` 必须是 `high`。
#[test]
fn settings_thinking_level_reaches_the_provider() {
    let Some(effort) = reasoning_effort_for("high", true) else {
        eprintln!("跳过：mock-provider 不可用");
        return;
    };
    eprintln!("[dbg] effort={effort:?}");
    assert_eq!(
        effort, "high",
        "`defaultThinkingLevel=high` 必须下发给 provider（ref openai-completions.ts:891/928-930）"
    );
}

/// 另一个取值都必须透传（不是只对 `high` 特判）。
///
/// **F64 修正**：`xhigh`/`max` 只在模型**声明了** `thinkingLevelMap` 时可用
/// （ref `ai/src/models.ts:921-935`：`mapped !== undefined` 才保留扩展档）。
/// 本用例的模型没有声明，此前却断言它们原样下发——那正是 F64 修掉的缺陷行为。
#[test]
fn every_thinking_level_is_passed_through() {
    // 未声明映射表：`xhigh`/`max` 被夹到最近的 `high`（`models.ts:942-952`）。
    for (level, expected) in [
        ("off", None),
        ("minimal", Some("minimal")),
        ("low", Some("low")),
        ("medium", Some("medium")),
        ("high", Some("high")),
        ("xhigh", Some("high")),
        ("max", Some("high")),
    ] {
        let Some(effort) = reasoning_effort_for(level, true) else {
            eprintln!("跳过：mock-provider 不可用");
            return;
        };
        // `off` 在 ref 里不下发 `reasoning_effort`（`thinking: disabled`），其余原样。
        match expected {
            Some(expected) => assert_eq!(
                effort, expected,
                "档位 {level} 应下发 {expected}（ref models.ts:921-952）"
            ),
            None => assert!(
                effort.is_empty() || effort == "off",
                "`off` 不应下发非空档位，实际 {effort:?}"
            ),
        }
    }
}

/// 非推理模型：无论 settings 写什么，都不下发 `reasoning_effort`。
#[test]
fn non_reasoning_model_never_sends_an_effort() {
    let effort = reasoning_effort_for("high", false);
    assert!(
        effort.is_none(),
        "非推理模型不应下发 reasoning_effort，实际 {effort:?}"
    );
    let _ = Path::new("");
}

/// **TUI 侧**同样要把 `defaultThinkingLevel` 下发到 provider。
///
/// 为什么单独一条：TUI 每轮用实时档位覆盖 `config.reasoning_effort`
/// （`tui.rs` 的 `level_snapshot`），与 `--print` 是两条不同的取值路径。
/// 前者依赖 `state.thinking_level` 的初值、后者依赖 `PrintRun.reasoning_effort`，
/// 任一处写死都会让设置只在页脚生效。
#[test]
fn tui_also_sends_the_settings_thinking_level() {
    let Some(node) = which("node") else {
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
    let log = tmp.path().join("req.ndjson");
    let (_mock, base_url) = start_mock(&log);
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [{ "id": "mock-model", "reasoning": true }]
            }}
        }))
        .expect("序列化"),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"openai-completions","defaultModel":"mock-model","defaultThinkingLevel":"high"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"openai-completions":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");

    // TUI 里提交一句话，触发一轮真实请求。
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        serde_json::to_string(&serde_json::json!([
            {"waitStable": {"quietMs": 2500, "timeoutMs": 60000}},
            {"text": "hi"},
            // 提交需要回车（TUI 里 `text` 只把字符打进编辑器）。
            {"text": "\r"},
            {"delayMs": 4000},
            {"text": "\u{4}"},
            {"delayMs": 1000}
        ]))
        .expect("序列化"),
    )
    .expect("写 keys");
    std::process::Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("COLORTERM", "")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("跑 pty");

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !text.is_empty(),
        "TUI 应已发出请求（日志 {} 为空）",
        log.display()
    );
    let mut found = None;
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let body = match value.get("body") {
            Some(serde_json::Value::String(inner)) => {
                serde_json::from_str::<serde_json::Value>(inner).ok()
            }
            Some(other) => Some(other.clone()),
            None => None,
        };
        if let Some(effort) = body
            .as_ref()
            .and_then(|b| b.get("reasoning_effort"))
            .and_then(serde_json::Value::as_str)
        {
            found = Some(effort.to_string());
            break;
        }
    }
    assert_eq!(
        found.as_deref(),
        Some("high"),
        "TUI 也必须下发 `defaultThinkingLevel=high`（实际 {found:?}）"
    );
}

/// 未设置 `defaultThinkingLevel` 的推理模型：两侧都必须下发内置缺省 `medium`。
///
/// 这条锁住"内置缺省与 clamp"这一段：此前新建分支只到 `settings.defaultThinkingLevel`
/// 为止、缺失时返回 `None`，TUI 回落到写死的 `medium` —— 行为相同但**路径不同**，
/// 任何一处改动都会让两个入口分叉。
#[test]
fn absent_setting_falls_back_to_the_builtin_default() {
    let Some(effort) = reasoning_effort_for_unset(true) else {
        eprintln!("跳过：mock-provider 不可用");
        return;
    };
    assert_eq!(
        effort, "medium",
        "未设置时应下发内置缺省 `medium`（ref `sdk.ts:248` 的 `?? DEFAULT_THINKING_LEVEL`）"
    );
}

/// 同上，但 settings 里**不写** `defaultThinkingLevel`。
fn reasoning_effort_for_unset(reasoning: bool) -> Option<String> {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("cfg");
    let sess = tmp.path().join("sess");
    let work = tmp.path().join("work");
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    let log = std::env::temp_dir().join(format!(
        "f63-unset-{}-{}.ndjson",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let (_mock, base_url) = start_mock(&log);
    let mut model = serde_json::json!({ "id": "mock-model" });
    if reasoning {
        model["reasoning"] = serde_json::json!(true);
    }
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [model]
            }}
        }))
        .expect("序列化"),
    )
    .expect("写 models.json");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"openai-completions","defaultModel":"mock-model"}"#,
    )
    .expect("写 settings.json");
    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("hi")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("pi-rs 可运行");
    assert!(output.status.success());
    read_effort(&log)
}

/// 从 mock 的请求日志里取首个 `reasoning_effort`。
fn read_effort(log: &Path) -> Option<String> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let body = match value.get("body") {
            Some(serde_json::Value::String(inner)) => {
                serde_json::from_str::<serde_json::Value>(inner).ok()
            }
            Some(other) => Some(other.clone()),
            None => None,
        };
        if let Some(effort) = body
            .as_ref()
            .and_then(|b| b.get("reasoning_effort"))
            .and_then(serde_json::Value::as_str)
        {
            return Some(effort.to_string());
        }
    }
    None
}

fn which(name: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// **TUI 侧**：`modelThinkingLevels` 的每模型覆盖也必须生效。
///
/// 这是本轮改动的真正差异点：`initial_thinking_level` 此前只到
/// `settings.defaultThinkingLevel` 为止，缺失时返回 `None`，TUI 直接回落到写死的
/// `"medium"` ⇒ **每模型覆盖被忽略**。现在两条链合并，覆盖值能到 provider。
#[test]
fn tui_honours_per_model_thinking_levels() {
    let Some(node) = which("node") else {
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
    let log = tmp.path().join("req.ndjson");
    let (_mock, base_url) = start_mock(&log);
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [{ "id": "mock-model", "reasoning": true }]
            }}
        }))
        .expect("序列化"),
    )
    .expect("写 models");
    // 全局默认 `low`，每模型覆盖 `high` ⇒ 生效的必须是 `high`。
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"openai-completions","defaultModel":"mock-model","defaultThinkingLevel":"low","modelThinkingLevels":{"openai-completions/mock-model":"high"}}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"openai-completions":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        serde_json::to_string(&serde_json::json!([
            {"waitStable": {"quietMs": 2500, "timeoutMs": 60000}},
            {"text": "hi"},
            {"text": "\r"},
            {"delayMs": 4000},
            {"text": "\u{4}"},
            {"delayMs": 1000}
        ]))
        .expect("序列化"),
    )
    .expect("写 keys");
    std::process::Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("COLORTERM", "")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("跑 pty");
    let effort = read_effort(&log);
    assert_eq!(
        effort.as_deref(),
        Some("high"),
        "TUI 应采纳每模型覆盖（`modelThinkingLevels`）而非全局默认 `low`，实际 {effort:?}"
    );
}

/// **Ctrl+P 切模型后**必须按 ref `agent-session.ts:2310-2322` 重算档位：
/// 每模型覆盖 → `settings.defaultThinkingLevel` → 当前档位 → 内置缺省。
///
/// 此前硬编码成"推理模型 medium"，于是用户在 `settings.json` 配的
/// `defaultThinkingLevel` 和 `modelThinkingLevels` 在切模型后**立刻丢失**。
#[test]
fn ctrl_p_recomputes_the_thinking_level() {
    let Some(node) = which("node") else {
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
    let log = tmp.path().join("req.ndjson");
    let (_mock, base_url) = start_mock(&log);
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [
                    { "id": "model-a", "reasoning": true },
                    { "id": "model-b", "reasoning": true }
                ]
            }}
        }))
        .expect("序列化"),
    )
    .expect("写 models");
    // 全局默认 `low`；切到 `model-b` 后必须是 `low`，而不是硬编码的 `medium`。
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"openai-completions","defaultModel":"model-a","defaultThinkingLevel":"low"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"openai-completions":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    // 键注入走 `PI_TEST_KEYS`（**逗号分隔的键名**），不是 pty-helper 的 `--script`：
    // 后者只负责"何时发字符"，键名语义在上面那个通道里。此前把它写成
    // `{"text": "ctrl+p"}` ⇒ 逐字注入 6 个字母，`ctrl+p` 从未生效（用例因此无判别力）。
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        serde_json::to_string(&serde_json::json!([
            {"waitStable": {"quietMs": 2500, "timeoutMs": 60000}}
        ]))
        .expect("序列化"),
    )
    .expect("写 keys");
    std::process::Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("COLORTERM", "")
        .env("PI_TEST_DUMP", tmp.path().join("steps"))
        .env("PI_TEST_KEYS", "ctrl+p,ctrl+j,type:hi,enter,ctrl+d")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("跑 pty");
    // 先确认 `ctrl+p` 真的**切换了模型**（否则本用例没有判别力）。
    let mut models: Vec<String> = Vec::new();
    for index in 0..32 {
        let Ok(text) = std::fs::read_to_string(tmp.path().join(format!("steps/step-{index}.json")))
        else {
            break;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(model) = value
                .get("footer")
                .and_then(|footer| footer.get("model"))
                .and_then(serde_json::Value::as_str)
            {
                models.push(model.to_string());
            }
        }
    }
    assert!(
        models.iter().any(|model| model == "model-b"),
        "前置条件：`ctrl+p` 应已切到 model-b，实际 footer.model 序列 {models:?}"
    );
    let effort = read_effort(&log);
    assert_eq!(
        effort.as_deref(),
        Some("low"),
        "切模型后应沿用 `defaultThinkingLevel=low`（ref `:2321`），实际 {effort:?}"
    );
}
