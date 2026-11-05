//! P9-1-E3b / S2：`registerFlag`/`getFlag` 的两阶段解析
//! （ref `cli/args.ts:236-250` 收集未知 flag；`agent-session-services.ts:92-125` 扩展加载后校验）。

use std::path::{Path, PathBuf};
use std::process::Command;

fn node_path() -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prepare(agent: &Path) {
    let target = agent.join("extensions/flags");
    std::fs::create_dir_all(&target).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/flag-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

fn run(agent: &Path, node: &Path, log: &Path, extra: &[&str]) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pi-rs"));
    command
        .current_dir(agent)
        .arg("--list-tools")
        .args(extra)
        .env("PI_CODING_AGENT_DIR", agent)
        .env("PI_NODE", node)
        .env("PI_EXT_TEST_LOG", log)
        .env("HOME", agent.join("home"));
    let output = command.output().expect("pi-rs 可运行");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn registered_flags_are_resolved_and_unknown_ones_still_fail() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("cfg");
    std::fs::create_dir_all(&agent).expect("建目录");
    prepare(&agent);
    let log = tmp.path().join("events.log");

    // 1) 不传 flag：`getFlag` 回落到注册默认值。
    let (code, _, stderr) = run(&agent, &node, &log, &[]);
    assert_eq!(code, 0, "{stderr}");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.lines()
            .any(|line| line.starts_with("flags:fallback:false")),
        "默认值应生效：{text}"
    );

    // 2) 字符串 flag 取值、布尔 flag 置真。
    let (code, _, stderr) = run(
        &agent,
        &node,
        &log,
        &["--demo-flag=hello", "--plain-switch"],
    );
    assert_eq!(code, 0, "{stderr}");
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.lines()
            .any(|line| line.starts_with("flags:hello:true")),
        "命令行值应覆盖默认值：{text}"
    );

    // 3) 字符串 flag 只给裸开关（无值）⇒ ref 的报错文案 + 退出码 1。
    let (code, _, stderr) = run(&agent, &node, &log, &["--demo-flag"]);
    assert_eq!(code, 1, "缺值应退出 1：{stderr}");
    assert!(
        stderr.contains("Extension flag \"--demo-flag\" requires a value"),
        "文案应与 ref 一致：{stderr}"
    );

    // 4) 未注册的 flag ⇒ `Unknown option: --nope`，退出码 1（ref 的既有语义）。
    let (code, _, stderr) = run(&agent, &node, &log, &["--nope"]);
    assert_eq!(code, 1, "未注册的 flag 应退出 1：{stderr}");
    assert!(
        stderr.contains("Unknown option: --nope"),
        "文案应与 ref 一致：{stderr}"
    );
    // 多个未注册 flag 用复数 + 逗号列表。
    let (code, _, stderr) = run(&agent, &node, &log, &["--nope1", "--nope2"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stderr.contains("Unknown options: --nope1, --nope2"),
        "复数形式应与 ref 一致：{stderr}"
    );
}

/// 没有扩展时（无 node / 无扩展目录）：未知 flag 仍按 `Unknown option` 报错。
#[test]
fn unknown_flag_without_any_extension_still_fails() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("cfg");
    std::fs::create_dir_all(&agent).expect("建目录");
    let log = tmp.path().join("events.log");
    let (code, _, stderr) = run(&agent, &node, &log, &["--nope"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("Unknown option: --nope"), "{stderr}");
}
