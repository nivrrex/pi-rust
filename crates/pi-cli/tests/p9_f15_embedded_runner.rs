//! P9-F15 / S4：单文件发行包必须能用**内嵌** runner 加载 `.ts` 扩展。
//!
//! 做法：把被测二进制拷到空目录（没有 `tools/ext-host/`，也不在源码树里），
//! 用一个只含真实 `pi-handoff-gate` 副本的 agent 目录 + `PI_JITI` 跑 `--list-tools`，
//! 断言列出了 `todo` / `report_to_user`。
//!
//! 注意：开发树里 `CARGO_MANIFEST_DIR` 回退总是存在，所以必须**换目录**才能覆盖内嵌路径。

use std::path::{Path, PathBuf};
use std::process::Command;

fn node_path() -> Option<PathBuf> {
    Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
        .map(PathBuf::from)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[test]
fn standalone_binary_uses_the_embedded_runner() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let real_ext = std::env::var("HOME")
        .map(|home| PathBuf::from(home).join(".pi/agent/extensions/pi-handoff-gate"))
        .ok()
        .filter(|path| path.is_dir());
    let Some(real_ext) = real_ext else {
        eprintln!("跳过：本机没有 pi-handoff-gate");
        return;
    };
    // jiti 位置不硬编码：先看 PI_JITI，再问 `npm root -g`（`<root>/jiti`）。
    let jiti = std::env::var("PI_JITI").ok().or_else(|| {
        let root = Command::new("sh")
            .arg("-c")
            .arg("npm root -g 2>/dev/null")
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|text| !text.is_empty())?;
        let candidate = PathBuf::from(root).join("jiti");
        candidate
            .join("lib/jiti.cjs")
            .exists()
            .then(|| candidate.display().to_string())
    });
    let Some(jiti) = jiti else {
        eprintln!("跳过：本机没有 jiti");
        return;
    };

    let tmp = tempfile::tempdir().expect("临时目录");
    // 1) 二进制单独放在一个空目录（没有 tools/ext-host）。
    let solo = tmp.path().join("solo");
    std::fs::create_dir_all(&solo).expect("建目录");
    let binary = solo.join("pi-rs");
    std::fs::copy(env!("CARGO_BIN_EXE_pi-rs"), &binary).expect("拷二进制");
    // 2) agent 目录里放真实扩展。
    let agent = tmp.path().join("agent");
    copy_dir(&real_ext, &agent.join("extensions/pi-handoff-gate")).expect("拷扩展");
    std::fs::write(agent.join("models.json"), "{\"providers\":{}}").expect("写 models.json");

    let output = Command::new(&binary)
        .current_dir(&solo)
        .arg("--list-tools")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_NODE", &node)
        .env("PI_JITI", &jiti)
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("运行 pi-rs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("runner.mjs"),
        "内嵌 runner 不应报找不到：{stderr}"
    );
    assert!(
        stdout.contains("todo") && stdout.contains("report_to_user"),
        "单文件二进制应能加载 .ts 扩展：stdout={stdout} stderr={stderr}"
    );
}
