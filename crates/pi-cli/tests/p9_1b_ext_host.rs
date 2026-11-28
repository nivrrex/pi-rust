//! P9-1-B：扩展宿主接入的三条路径（`--list-tools` 列扩展工具、无 node 降级、握手失败降级）。
//!
//! 测试都通过**独立进程**调用 `pi-rs --list-tools`（与用户路径一致），并只用临时目录，
//! 不碰 `~/.pi`。

use std::path::{Path, PathBuf};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_pi-rs")
}

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
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

/// 造一个带扩展的临时 agent 目录：`extensions/<name>/index.mjs`。
fn agent_dir_with_extension(root: &Path, source: &Path) {
    let dir = root.join("extensions/demo");
    std::fs::create_dir_all(&dir).expect("建扩展目录");
    std::fs::copy(source, dir.join("index.mjs")).expect("拷扩展");
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/ext-host/fixtures/echo-ext.mjs")
}

fn run_list_tools(agent_dir: &Path, node: &str) -> (String, String, bool) {
    let output = std::process::Command::new(binary())
        .arg("--list-tools")
        .env("PI_CODING_AGENT_DIR", agent_dir)
        .env("PI_NODE", node)
        .env("HOME", agent_dir)
        .output()
        .expect("运行 --list-tools");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

/// 有 node + 有扩展 ⇒ 扩展工具出现在列表里，且排在内置之后。
#[test]
fn list_tools_includes_extension_tools() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    agent_dir_with_extension(dir.path(), &fixture());
    let (stdout, stderr, ok) = run_list_tools(dir.path(), &node.to_string_lossy());
    assert!(ok, "--list-tools 应以 0 退出；stderr={stderr}");
    let names: Vec<&str> = stdout
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert!(names.contains(&"read"), "{names:?}");
    let echo = stdout
        .lines()
        .find(|line| line.starts_with("ext_echo"))
        .expect("应有 ext_echo");
    assert!(echo.contains("extension"), "{echo:?}");
    let ext_index = names
        .iter()
        .position(|name| *name == "ext_echo")
        .expect("有");
    let builtin_index = names.iter().position(|name| *name == "read").expect("有");
    assert!(builtin_index < ext_index, "内置应排在扩展之前：{names:?}");
}

/// 没有 node（`PI_NODE` 指向不存在的文件）⇒ 只有内置工具 + 一条明确的 stderr 提示。
#[test]
fn list_tools_without_node_degrades() {
    let dir = tempfile::tempdir().expect("tempdir");
    agent_dir_with_extension(dir.path(), &fixture());
    let (stdout, stderr, ok) = run_list_tools(dir.path(), "/nonexistent/node-p9-1b");
    assert!(
        ok,
        "无 node 也应正常退出（不加载扩展而已）；stderr={stderr}"
    );
    assert!(stdout.contains("read"), "内置工具应仍在：{stdout}");
    assert!(!stdout.contains("ext_echo"), "无 node 不应列出扩展工具");
    assert!(
        stderr.contains("找不到 node"),
        "应给出明确提示，实际 stderr={stderr:?}"
    );
}

/// 协议版本不符（子进程声明 999）⇒ 降级为"只有内置工具"，不阻塞其它功能。
#[test]
fn list_tools_with_protocol_mismatch_degrades() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    agent_dir_with_extension(dir.path(), &fixture());
    let output = std::process::Command::new(binary())
        .arg("--list-tools")
        .env("PI_CODING_AGENT_DIR", dir.path())
        .env("PI_NODE", &node)
        .env("HOME", dir.path())
        .env("PI_EXT_PROTOCOL", "999")
        .output()
        .expect("运行 --list-tools");
    assert!(output.status.success(), "版本不符不应让进程失败");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("read"), "{stdout}");
    assert!(!stdout.contains("ext_echo"), "版本不符时扩展工具不应出现");
    assert!(
        stderr.contains("protocol mismatch") || stderr.contains("扩展未加载"),
        "应提示握手失败，实际 {stderr:?}"
    );
}
