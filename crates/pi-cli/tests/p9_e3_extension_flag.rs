//! P9 E3：`-e/--extension` 显式扩展路径（ref `cli/args.ts:176-179`、`loader.ts:780-795`）。

use std::path::PathBuf;
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

fn run(args: &[&str], agent: &std::path::Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(repo_root())
        .args(args)
        .env("PI_CODING_AGENT_DIR", agent)
        .env("PI_NODE", node_path().expect("需要 node"))
        .env("HOME", agent.join("home"))
        .output()
        .expect("pi-rs 可运行");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn extension_flag_loads_a_single_file_and_a_directory() {
    if node_path().is_none() {
        eprintln!("跳过：本机没有 node");
        return;
    }
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("cfg");
    std::fs::create_dir_all(&agent).expect("建目录");
    let fixture = repo_root().join("tools/ext-host/fixtures/echo-ext.mjs");

    let out = run(
        &["--list-tools", "-e", fixture.to_str().expect("utf8")],
        &agent,
    );
    assert!(out.contains("ext_echo"), "单文件 -e 应加载：{out}");

    // 目录形式：里面是 `index.mjs`。
    let dir = tmp.path().join("extroot/demo");
    std::fs::create_dir_all(&dir).expect("建目录");
    std::fs::copy(&fixture, dir.join("index.mjs")).expect("复制");
    let out = run(
        &[
            "--list-tools",
            "-e",
            tmp.path().join("extroot").to_str().expect("utf8"),
        ],
        &agent,
    );
    assert!(out.contains("ext_echo"), "目录 -e 应发现 index.mjs：{out}");

    // `--extension=` 等号形式（ref 支持）。
    let out = run(
        &[
            "--list-tools",
            &format!("--extension={}", fixture.display()),
        ],
        &agent,
    );
    assert!(out.contains("ext_echo"), "等号形式也应加载：{out}");
}
