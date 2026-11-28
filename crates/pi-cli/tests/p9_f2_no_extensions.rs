//! F2/S4：`--no-extensions / -ne` 只跳过**发现**，显式 `-e` 仍生效
//! （ref `args.ts:179-180`、`resource-loader.ts:452-454` / `:556-559`）。

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pi-rs")
}

fn node_available() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .map(|out| !String::from_utf8_lossy(&out.stdout).trim().is_empty())
        .unwrap_or(false)
}

fn prepare_project(root: &Path) {
    let dir = root.join(".pi/extensions/echo");
    std::fs::create_dir_all(&dir).expect("建项目级扩展目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/echo-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(dir.join("index.mjs"), body).expect("写扩展");
}

fn list_tools(project: &Path, agent: &Path, extra: &[&str]) -> String {
    let node = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    let mut command = Command::new(bin());
    command
        .arg("--list-tools")
        .args(extra)
        .current_dir(project)
        .env("PI_CODING_AGENT_DIR", agent)
        .env("PI_CODING_AGENT_SESSION_DIR", agent.join("sess"))
        .env("PI_NODE", node)
        .env("PI_JITI", jiti_or_empty());
    let output = command.output().expect("运行 pi-rs");
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn jiti_or_empty() -> String {
    std::env::var("PI_JITI").unwrap_or_default()
}

#[test]
fn no_extensions_skips_discovery_but_keeps_explicit_paths() {
    if !node_available() {
        eprintln!("跳过：本机没有 node");
        return;
    }
    let project = tempfile::tempdir().expect("项目目录");
    let agent = tempfile::tempdir().expect("agent 目录");
    prepare_project(project.path());

    let discovered = list_tools(project.path(), agent.path(), &[]);
    assert!(
        discovered.contains("ext_echo"),
        "不带 -ne 时应发现项目级扩展：{discovered}"
    );

    let skipped = list_tools(project.path(), agent.path(), &["-ne"]);
    assert!(
        !skipped.contains("ext_echo"),
        "-ne 应跳过发现（项目级 .pi/extensions 不再加载）：{skipped}"
    );

    let explicit = list_tools(
        project.path(),
        agent.path(),
        &[
            "--no-extensions",
            "-e",
            project
                .path()
                .join(".pi/extensions/echo")
                .to_str()
                .expect("utf8"),
        ],
    );
    assert!(
        explicit.contains("ext_echo"),
        "-ne 下显式 -e 仍应加载（ref resource-loader.ts:452-454）：{explicit}"
    );
}
