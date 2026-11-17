//! F61/40：`settings.json` 的 `defaultTools` 决定初始激活的内置工具集
//! （ref `settings-manager.ts:144/1337`、`sdk.ts:259-265`）。
//!
//! 改前：只认命令行 `--tools`，设置里的 `defaultTools` 被完全忽略
//! ⇒ 用户在设置里配了工具集，启动后还是四个默认工具。

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// 用给定 settings.json 跑一次 `--list-tools`，返回列出的工具名。
fn listed_tools(settings: &str, extra_args: &[&str]) -> Option<Vec<String>> {
    listed_tools_with_extension(settings, extra_args, None)
}

/// 同上，可选加载一个额外扩展（`-e`），用于"扩展注册同名工具"的场景。
fn listed_tools_with_extension(
    settings: &str,
    extra_args: &[&str],
    extension: Option<&str>,
) -> Option<Vec<String>> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    for dir in [&agent, &home, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(agent.join("settings.json"), settings).expect("写 settings");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_pi-rs"));
    command.arg("--list-tools").args(extra_args);
    if let Some(extension) = extension {
        command
            .arg("-e")
            .arg(repo_root().join("tools/ext-host/fixtures").join(extension));
    }
    let output = command
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .expect("跑 pi-rs");
    assert!(output.status.success(), "--list-tools 应成功");
    let text = String::from_utf8_lossy(&output.stdout);
    let names: Vec<String> = text
        .lines()
        .skip(1) // 表头
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect();
    Some(names)
}

/// 设置里的 `defaultTools` 应决定工具集（含可选工具）。
#[test]
fn default_tools_setting_selects_the_tool_set() {
    let Some(names) = listed_tools(r#"{"defaultTools":["read","grep"]}"#, &[]) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(names.contains(&"read".to_string()), "应含 read：{names:?}");
    assert!(
        names.contains(&"grep".to_string()),
        "应含 grep（可选工具也能被设置选中）：{names:?}"
    );
    assert!(
        !names.contains(&"bash".to_string()),
        "未列出的 bash 不应出现：{names:?}"
    );
}

/// 没有 `defaultTools` 时回到内置默认集。
#[test]
fn absent_setting_keeps_the_builtin_defaults() {
    let Some(names) = listed_tools("{}", &[]) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    for expected in ["read", "bash", "edit", "write"] {
        assert!(
            names.contains(&expected.to_string()),
            "默认应含 {expected}：{names:?}"
        );
    }
}

/// 命令行 `--tools` 优先于设置（ref `sdk.ts:263-265`）。
#[test]
fn cli_tools_flag_wins_over_the_setting() {
    let Some(names) = listed_tools(r#"{"defaultTools":["read","grep"]}"#, &["--tools", "write"])
    else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        names.contains(&"write".to_string()),
        "命令行指定的工具应生效：{names:?}"
    );
    assert!(
        !names.contains(&"grep".to_string()),
        "命令行给了 --tools 时设置不应参与：{names:?}"
    );
}

/// 内置与扩展同名工具（如 `bash`）**不得重复**出现在**运行期工具集**里。
///
/// ref 的工具注册表以名字为键，同名只能有一个。此前 `default_tools` 无条件把
/// 扩展工具追加在内置之后 ⇒ 同名工具同时存在，系统提示的 `<tools>` 段重复列出
/// `bash`（实测）。
///
/// 判据用**系统提示的 `<tools>` 段**而不是 `--list-tools`：后者是 pi-rs 自有的
/// 诊断命令（ref 没有），它本来就该列出全部可用工具（含同名），不代表运行期集合。
#[test]
fn extension_tool_with_the_same_name_is_not_duplicated() {
    let Some(body) = system_prompt_tools_section(
        r#"{"defaultTools":["read","bash"],"defaultProvider":"p","defaultModel":"m1"}"#,
        Some("same-name-bash-ext.mjs"),
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let bash_count = body
        .lines()
        .filter(|line| line.trim_start().starts_with("- bash:"))
        .count();
    assert_eq!(
        bash_count, 1,
        "`bash` 只应出现一次（内置优先，扩展同名工具被跳过）。工具段：\n{body}"
    );
}

/// 跑 `--print --mode json`，从 `message_start` 的 sections 里取 `<tools>` 段。
fn system_prompt_tools_section(settings: &str, extension: Option<&str>) -> Option<String> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(agent.join("settings.json"), settings).expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m1"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_pi-rs"));
    command.arg("--print").arg("--mode").arg("json").arg("hi");
    if let Some(extension) = extension {
        command
            .arg("-e")
            .arg(repo_root().join("tools/ext-host/fixtures").join(extension));
    }
    let output = command
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(section) = value
            .get("message")
            .and_then(|message| message.get("sections"))
            .and_then(|sections| sections.get("tools"))
            .and_then(serde_json::Value::as_str)
        {
            return Some(section.to_string());
        }
    }
    None
}
