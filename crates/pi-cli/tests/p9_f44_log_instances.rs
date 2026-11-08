//! F44/S0：并发两个 pi-rs 实例，退出时**各自只输出自己的**扩展日志。
//!
//! F43 用"启动时文件偏移"实现增量输出；两个实例同时运行时，A 的偏移区间里可能落进
//! B 写入的行 ⇒ A 退出会把 B 的日志一并打出来（用户常同时开多个 pi-rs）。

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

fn prepare(agent: &Path, home: &Path) {
    std::fs::create_dir_all(agent.join("extensions/marker")).expect("建扩展目录");
    std::fs::create_dir_all(home).expect("建 HOME");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("extensions/marker/package.json"),
        r#"{"type":"module","name":"marker"}"#,
    )
    .expect("写扩展 package.json");
    // 每个实例（靠 `PI_F44_INSTANCE` 区分）在 session_start 与延迟后再各打一条标记。
    std::fs::write(
        agent.join("extensions/marker/index.js"),
        concat!(
            "export default function marker(pi) {\n",
            "  const tag = process.env.PI_F44_INSTANCE ?? \"?\";\n",
            "  pi.on(\"session_start\", () => {\n",
            "    process.stderr.write(`[marker] ${tag} start\\n`);\n",
            "    setTimeout(() => process.stderr.write(`[marker] ${tag} later\\n`), 400);\n",
            "  });\n",
            "}\n",
        ),
    )
    .expect("写扩展入口");
}

fn markers(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            line.find("[marker] ")
                .map(|index| line[index..].trim().to_string())
        })
        .collect()
}

#[test]
fn concurrent_instances_print_only_their_own_logs() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    prepare(&agent, &home);
    let out = Command::new("python3")
        .arg(repo_root().join("tools/difftest/tui-two-instances.py"))
        .arg(&agent)
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .output()
        .expect("运行并发夹具");
    assert!(out.status.success(), "夹具应成功");
    let json: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).expect("解析 JSON");
    let a_exit = markers(json["a_exit"].as_str().unwrap_or_default());
    let b_exit = markers(json["b_exit"].as_str().unwrap_or_default());
    let log = markers(json["log_file"].as_str().unwrap_or_default());

    assert!(
        a_exit.iter().all(|line| line.contains(" A ")),
        "A 退出时不得输出 B 的日志：{a_exit:?}"
    );
    assert!(
        b_exit.iter().all(|line| line.contains(" B ")),
        "B 退出时不得输出 A 的日志：{b_exit:?}"
    );
    assert!(!a_exit.is_empty(), "A 应输出自己的日志（至少一条）");
    assert!(!b_exit.is_empty(), "B 应输出自己的日志（至少一条）");
    // 文件仍应累积两个实例的行。
    assert!(
        log.iter().any(|line| line.contains(" A ")) && log.iter().any(|line| line.contains(" B ")),
        "日志文件应同时包含 A 与 B：{log:?}"
    );
}
