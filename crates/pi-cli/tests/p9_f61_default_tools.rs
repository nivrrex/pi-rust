//! F61/40：`settings.json` 的 `defaultTools` 决定初始激活的内置工具集
//! （ref `settings-manager.ts:144/1337`、`sdk.ts:259-265`）。
//!
//! 改前：只认命令行 `--tools`，设置里的 `defaultTools` 被完全忽略
//! ⇒ 用户在设置里配了工具集，启动后还是四个默认工具。

use std::path::PathBuf;

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
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    for dir in [&agent, &home, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(agent.join("settings.json"), settings).expect("写 settings");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--list-tools")
        .args(extra_args)
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
