//! F63/D：`settings.json` 关键设置的**加载期**校验——只警告、不阻断、不改文件。
//!
//! ref 只对 JSON 语法/读写错误报警（`settings-manager.ts:591-593` 的 `recordError`
//! → `settings-diagnostics.ts:4-9` 的 `Invalid settings file <path>: <msg>`）；
//! **没有**枚举与名称存在性校验（逐个 grep 确认）。因此本文件的第 2 组
//! （枚举类）与第 3 组（注册表类）属"用户要求的偏离"，登记在 LIMITATIONS，
//! 且只产生 warning。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_cli::settings::{collect_registry_warnings, collect_warnings, Settings};

fn base() -> Settings {
    Settings::default()
}

/// ① JSON 语法错误：不阻断，文案照 ref，且**不改文件**。
#[test]
fn broken_json_warns_but_does_not_abort() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let path = tmp.path().join("settings.json");
    let body = r#"{"defaultProvider":"p",}"#;
    std::fs::write(&path, body).expect("写");

    let error = pi_cli::settings::parse(body).expect_err("坏 JSON 应解析失败");
    assert!(!error.is_empty());
    // 文件未被改写（加载期校验不得改文件）。
    assert_eq!(std::fs::read_to_string(&path).expect("读"), body);
}

/// ② `defaultThinkingLevel` 枚举。
#[test]
fn invalid_thinking_level_warns() {
    let mut settings = base();
    settings.default_thinking_level = Some("turbo".to_string());
    let warnings = collect_warnings(&settings);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].key, "defaultThinkingLevel");
    assert!(warnings[0].message.contains("\"turbo\""), "{warnings:?}");
}

/// ② 合法枚举不告警。
#[test]
fn valid_thinking_level_is_silent() {
    let mut settings = base();
    settings.default_thinking_level = Some("high".to_string());
    assert!(collect_warnings(&settings).is_empty());
}

/// ② `defaultProjectTrust` 枚举。
#[test]
fn invalid_project_trust_warns() {
    let mut settings = base();
    settings.default_project_trust = Some("maybe".to_string());
    let warnings = collect_warnings(&settings);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].key, "defaultProjectTrust");
}

/// ③ `defaultProvider`/`defaultModel` 不在注册表。
#[test]
fn unknown_default_model_warns() {
    let mut settings = base();
    settings.default_provider = Some("gone".to_string());
    settings.default_model = Some("gone".to_string());
    let known = [("p", "m1")];
    let warnings = collect_registry_warnings(&settings, &known, &[]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].key, "defaultProvider");
    assert!(warnings[0].message.contains("gone/gone"), "{warnings:?}");
}

/// ③ 存在的组合不告警。
#[test]
fn known_default_model_is_silent() {
    let mut settings = base();
    settings.default_provider = Some("p".to_string());
    settings.default_model = Some("m1".to_string());
    assert!(collect_registry_warnings(&settings, &[("p", "m1")], &[]).is_empty());
}

/// ③ `enabledModels` 里不存在的条目**不**产生额外警告（F80）。
///
/// ref 对这类条目的处理是 `No models match pattern "<pattern>"`
/// （`model-resolver.ts:325/349`），pi-rs 已由另一条路径产出同样提示。
/// 再报一条 `enabledModels entry … is not in the model registry` 只会重复刷屏，
/// 并把 `T80-model-scope-invalid` 的屏幕多顶一行。
#[test]
fn unknown_enabled_models_entry_is_silent() {
    let mut settings = base();
    settings.enabled_models = Some(vec!["p/m1".to_string(), "gone/x".to_string()]);
    let warnings = collect_registry_warnings(&settings, &[("p", "m1")], &[]);
    assert!(
        warnings.is_empty(),
        "`enabledModels` 的无效条目不在这里告警（ref 用 No models match pattern）：{warnings:?}"
    );
}

/// ③ `defaultTools` 名称不存在。
#[test]
fn unknown_default_tools_entry_warns() {
    let mut settings = base();
    settings.default_tools = Some(vec!["read".to_string(), "nope".to_string()]);
    let warnings = collect_registry_warnings(&settings, &[], &["read", "bash"]);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].key, "defaultTools");
    assert!(warnings[0].message.contains("\"nope\""), "{warnings:?}");
}

/// pty：坏 `settings.json` 下进程**不退出**，且屏幕出现 ref 文案。
#[test]
fn broken_settings_json_does_not_abort_the_tui() {
    let node = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|t| !t.is_empty());
    let Some(node) = node else {
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
    std::fs::write(agent.join("settings.json"), r#"{"defaultProvider":"p",}"#)
        .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        serde_json::to_string(&serde_json::json!([
            {"waitStable": {"quietMs": 2500, "timeoutMs": 60000}},
            {"text": "\u{4}"},
            {"delayMs": 1000}
        ]))
        .expect("序列化"),
    )
    .expect("写 keys");
    let code = tmp.path().join("code.txt");
    let raw = tmp.path().join("raw.bin");
    std::process::Command::new("python3")
        .arg(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/difftest/pty-helper.py"),
        )
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
        .arg("--exit-code-file")
        .arg(&code)
        .arg("--out")
        .arg(&raw)
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
    let exit = std::fs::read_to_string(&code)
        .ok()
        .and_then(|t| t.trim().parse::<i32>().ok())
        .unwrap_or(-1);
    assert_eq!(
        exit, 0,
        "坏 settings.json 不得致命退出（ref `settings-diagnostics.ts:4-9` 只警告）"
    );
    let screen = std::fs::read_to_string(&raw).unwrap_or_default();
    assert!(
        screen.contains("Invalid settings file"),
        "应显示 ref 文案 `Invalid settings file <path>: …`"
    );
}

/// 注册表类警告必须**真的出现在启动输出里**，而不只是单元函数返回。
///
/// 这条钉的是接线：`collect_registry_warnings` 曾在只有单元用例、**未接进启动路径**
/// 的状态下存在——函数对、真机完全静默。三处都必须在 stderr 出现。
#[test]
fn registry_warnings_reach_the_startup_output() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"gone-model","defaultThinkingLevel":"turbo","defaultProjectTrust":"maybe","defaultTools":["read","nope"],"enabledModels":["p/missing"]}"#,
    )
    .expect("写 settings");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("hi")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("HOME", &home)
        .output()
        .expect("跑 pi-rs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    for needle in [
        "defaultThinkingLevel \"turbo\"",
        "defaultProjectTrust \"maybe\"",
        "defaultProvider/defaultModel \"p/gone-model\"",
        "defaultTools entry \"nope\"",
    ] {
        assert!(
            stderr.contains(needle),
            "启动输出应含 {needle}，实际 stderr：\n{stderr}"
        );
    }
}

/// `hideThinkingBlock` 类型错（非布尔）⇒ ref 原样的 `Invalid settings file` 文案，不阻断。
///
/// 这一项**不需要**额外检查：serde 反序列化直接报错，走与其它类型错相同的路径
/// （`settings-diagnostics.ts:7` 的文案）。
#[test]
fn wrong_hide_thinking_block_type_warns_like_ref() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1","hideThinkingBlock":"yes"}"#,
    )
    .expect("写 settings");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--print")
        .arg("hi")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("HOME", &home)
        .output()
        .expect("跑 pi-rs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid settings file"),
        "应报 ref 的 `Invalid settings file`：\n{stderr}"
    );
    assert!(
        stderr.contains("expected a boolean"),
        "应指出类型不符（serde 原样信息）：\n{stderr}"
    );
    // 不阻断：进程仍走到后续步骤（无可用模型的报错，而不是设置解析失败）。
    assert!(
        !stderr.contains("pi-rs: ") || !stderr.contains("settings.json 解析失败"),
        "不应把类型错当成致命错误：\n{stderr}"
    );
}

/// 同一条警告**只出现一次**（`load_settings` 被多个装配路径重复调用）。
///
/// 实测缺陷：TUI 启动时 `load_settings` 被读 3 次（theme、defaultModel、
/// `build_tui_agent_parts`），同一条 `defaultThinkingLevel "turbo"` 在屏幕上
/// 出现 3 遍——用户会以为是 3 个不同问题。
#[test]
fn each_warning_is_printed_only_once() {
    let node = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|t| !t.is_empty());
    let Some(node) = node else {
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
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1","defaultThinkingLevel":"turbo"}"#,
    )
    .expect("写 settings");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        serde_json::to_string(&serde_json::json!([
            {"waitStable": {"quietMs": 2500, "timeoutMs": 60000}},
            {"text": "\u{4}"},
            {"delayMs": 1000}
        ]))
        .expect("序列化"),
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    std::process::Command::new("python3")
        .arg(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/difftest/pty-helper.py"),
        )
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(&raw)
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
    let text = std::fs::read_to_string(&raw).unwrap_or_default();
    let needle = "defaultThinkingLevel \"turbo\"";
    assert_eq!(
        text.matches(needle).count(),
        1,
        "同一条警告应只出现一次（实际 {} 次）",
        text.matches(needle).count()
    );
}

/// TUI 模式下 settings 诊断必须在**会话区**，不能占用首行（F63/K）。
///
/// 为什么：inline 模式下写 stderr 的字符直接落在首行，会把启动屏挤下去一行
/// （实测警告占据 TUI 第 0 行）。ref 的 `startupDiagnostics` 走
/// `showWarning`（`interactive-mode.ts:1129-1137`），位置在会话区。
#[test]
fn tui_shows_settings_diagnostics_in_the_transcript() {
    let node = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|t| !t.is_empty());
    let Some(node) = node else {
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
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1","defaultThinkingLevel":"turbo"}"#,
    )
    .expect("写 settings");
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        serde_json::to_string(&serde_json::json!([
            {"waitStable": {"quietMs": 2500, "timeoutMs": 60000}},
            {"text": "\u{4}"},
            {"delayMs": 1000}
        ]))
        .expect("序列化"),
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    std::process::Command::new("python3")
        .arg(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/difftest/pty-helper.py"),
        )
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("28")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(&raw)
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
    let bytes = std::fs::read(&raw).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    // 去 ANSI 后按行找警告位置。
    let plain = pi_tui::strip_ansi(text.as_ref());
    let mut position = None;
    for (index, line) in plain.lines().enumerate() {
        if line.contains("defaultThinkingLevel \"turbo\"") {
            position = Some((index, line.to_string()));
            break;
        }
    }
    let Some((line_index, line)) = position else {
        panic!("TUI 应显示该诊断：\n{plain}");
    };
    assert!(
        line_index > 3,
        "诊断应在会话区（启动屏之后），实际第 {line_index} 行：{line}"
    );
}
