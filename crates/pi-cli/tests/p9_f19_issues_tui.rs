//! P9-F19：TUI 启动屏的 `[Extension issues]` 要列出**单个扩展**的加载失败，
//! 且同批的好扩展仍然可用（`--list-tools` 里能看到它的工具）。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct MockGuard(Child);
impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn start_mock() -> (MockGuard, String) {
    use std::io::{BufRead, BufReader};
    let binary = repo_root().join("target/debug/mock-provider");
    assert!(binary.exists(), "先构建 mock-provider");
    let mut child = Command::new(binary)
        .current_dir(repo_root())
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
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

fn prepare(agent: &Path, base_url: &str) {
    let broken = agent.join("extensions/syntax");
    std::fs::create_dir_all(&broken).expect("建目录");
    std::fs::write(
        broken.join("index.mjs"),
        "export default function (pi) { this is not valid javascript }\n",
    )
    .expect("写坏扩展");
    let good = agent.join("extensions/good");
    std::fs::create_dir_all(&good).expect("建目录");
    std::fs::copy(
        repo_root().join("tools/ext-host/fixtures/echo-ext.mjs"),
        good.join("index.mjs"),
    )
    .expect("拷好扩展");
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [{ "id": "mock-model" }]
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
}

#[test]
fn tui_shows_extension_issues_for_broken_extension() {
    let Some(node) = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
    else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let (agent, sess, work) = (
        tmp.path().join("cfg"),
        tmp.path().join("sess"),
        tmp.path().join("work"),
    );
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    let (_mock, base_url) = start_mock();
    prepare(&agent, &base_url);

    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, r#"[{"delayMs":3000},{"raw":"\u0004"}]"#).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
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
        .env("HOME", tmp.path().join("home"))
        .status()
        .expect("运行 pty-helper");
    let screen = String::from_utf8_lossy(&std::fs::read(&raw).unwrap_or_default()).to_string();
    assert!(
        screen.contains("[Extension issues]"),
        "TUI 屏幕应出现 [Extension issues] 段"
    );
    assert!(
        screen.contains("syntax"),
        "应列出坏扩展的路径/名字：{}",
        screen.chars().take(600).collect::<String>()
    );

    // 好扩展仍然注册（`--list-tools` 能看到 ext_echo）。
    let listed = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--list-tools")
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_NODE", &node)
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("运行 --list-tools");
    let stdout = String::from_utf8_lossy(&listed.stdout);
    assert!(stdout.contains("ext_echo"), "好扩展应仍可用：{stdout}");
}
