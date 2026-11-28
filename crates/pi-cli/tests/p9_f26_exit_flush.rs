//! P9-F26/S0：退出时**不误报**"会话写入超时"，且扩展在 `session_start` 的 `appendEntry` 能落盘。
//!
//! 四种配置（零扩展 / 全部加载失败 / 一个好扩展 / 十个都调 appendEntry），都不发消息直接 Ctrl+D。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

fn run_case(name: &str, base_url: &str, node: &Path, kind: &str) -> (Duration, String, usize) {
    let tmp = tempfile::tempdir().expect("临时目录");
    let (agent, sess, work) = (
        tmp.path().join("cfg"),
        tmp.path().join("sess"),
        tmp.path().join("work"),
    );
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
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
    let ext_root = agent.join("extensions");
    match kind {
        "zero" => {}
        "allfail" => {
            for name in ["a", "b"] {
                let dir = ext_root.join(name);
                std::fs::create_dir_all(&dir).expect("建目录");
                std::fs::write(dir.join("index.ts"), "export default function (pi) {}\n")
                    .expect("写扩展");
            }
        }
        "one-good" | "ten-append" => {
            let count = if kind == "one-good" { 1 } else { 10 };
            for index in 0..count {
                let dir = ext_root.join(format!("e{index}"));
                std::fs::create_dir_all(&dir).expect("建目录");
                let source = if kind == "one-good" {
                    "echo-ext.mjs"
                } else {
                    "append-ext.mjs"
                };
                std::fs::copy(
                    repo_root().join("tools/ext-host/fixtures").join(source),
                    dir.join("index.mjs"),
                )
                .expect("拷扩展");
            }
        }
        other => panic!("未知用例 {other}"),
    }

    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStrip":{"contains":"ctrl+o more","timeoutMs":30000}},{"raw":"\u0004"}]"#,
    )
    .expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let code = tmp.path().join("code.txt");
    let started = Instant::now();
    let output = Command::new("python3")
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
        .env("PI_NODE", node)
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("运行 pty-helper");
    let elapsed = started.elapsed();
    assert!(output.status.success(), "{name}: pty-helper 失败");
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut custom = 0;
    for file in std::fs::read_dir(&sess)
        .expect("读会话目录")
        .filter_map(Result::ok)
    {
        let text = std::fs::read_to_string(file.path()).unwrap_or_default();
        custom += text
            .lines()
            .filter(|line| line.contains("\"customType\""))
            .count();
    }
    (elapsed, stderr, custom)
}

#[test]
fn exit_never_reports_a_false_session_write_timeout() {
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
    let (_mock, base_url) = start_mock();
    for kind in ["zero", "allfail", "one-good", "ten-append"] {
        let (elapsed, stderr, custom) = run_case(kind, &base_url, Path::new(&node), kind);
        assert!(
            !stderr.contains("退出前等待会话写入超时"),
            "{kind}: 不应误报会话写入超时：{stderr}"
        );
        // 空闲时实测 130–220ms（S0 报告里的 <500ms 依据）；全量并行跑时启动本身就会到秒级，
        // 因此这里只做"没有卡死"的松阈值，退出时序的正确性由"无超时告警"这一条保证。
        assert!(
            elapsed < Duration::from_secs(8),
            "{kind}: 退出耗时 {:?} 过长",
            elapsed
        );
        if kind == "ten-append" {
            assert_eq!(custom, 10, "十个扩展的 appendEntry 都应落盘");
        }
    }
}

/// F30/S4'a：**退出延迟**口径——从 pty 写入 Ctrl+D（mark 时刻）到进程退出 < 500 ms。
///
/// 依赖 `pty-helper.py --marks` 新增的 `t`（单调时钟）；四种配置各跑一次。
#[test]
fn ctrl_d_exit_latency_under_500ms() {
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
    let (_mock, base_url) = start_mock();
    for kind in ["zero", "allfail", "one-good", "ten-append"] {
        let (elapsed_ms, stderr, _custom) = run_case_latency(kind, &base_url, Path::new(&node));
        assert!(
            elapsed_ms < 500.0,
            "{kind}: Ctrl+D 到退出应 <500ms，实测 {elapsed_ms:.0}ms"
        );
        assert!(
            !stderr.contains("退出前等待会话写入超时"),
            "{kind}: 不应出现写入超时：{stderr}"
        );
        eprintln!("{kind}: {elapsed_ms:.0}ms");
    }
}

/// 与 `run_case` 同配置，但用 `--marks` 的 `t` 计算"Ctrl+D → 进程退出"的耗时。
fn run_case_latency(kind: &str, base_url: &str, node: &Path) -> (f64, String, usize) {
    let tmp = tempfile::tempdir().expect("临时目录");
    let (agent, sess, work) = (
        tmp.path().join("cfg"),
        tmp.path().join("sess"),
        tmp.path().join("work"),
    );
    for dir in [&agent, &sess, &work] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": { "openai-completions": {
                "baseUrl": base_url, "api": "openai-completions", "apiKey": "k",
                "models": [{ "id": "mock-model", "contextWindow": 200000 }]
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
    let ext_root = agent.join("extensions");
    match kind {
        "zero" => {}
        "allfail" => {
            for name in ["a", "b"] {
                let dir = ext_root.join(name);
                std::fs::create_dir_all(&dir).expect("建目录");
                std::fs::write(dir.join("index.ts"), "export default function (pi) {}\n")
                    .expect("写扩展");
            }
        }
        "one-good" | "ten-append" => {
            let count = if kind == "one-good" { 1 } else { 10 };
            for index in 0..count {
                let dir = ext_root.join(format!("e{index}"));
                std::fs::create_dir_all(&dir).expect("建目录");
                let source = if kind == "one-good" {
                    "echo-ext.mjs"
                } else {
                    "append-ext.mjs"
                };
                std::fs::copy(
                    repo_root().join("tools/ext-host/fixtures").join(source),
                    dir.join("index.mjs"),
                )
                .expect("拷扩展");
            }
        }
        other => panic!("未知用例 {other}"),
    }
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"waitStrip":{"contains":"ctrl+o more","timeoutMs":30000}},{"mark":"before-ctl-d"},{"raw":"\u0004"}]"#,
    )
    .expect("写 keys");
    let marks = tmp.path().join("marks.json");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--marks")
        .arg(&marks)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", node)
        .env("HOME", tmp.path().join("home"))
        .output()
        .expect("运行 pty-helper");
    let end = std::time::SystemTime::now();
    let _ = end;
    let text = std::fs::read_to_string(&marks).unwrap_or_default();
    let t: f64 = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v["name"] == "before-ctl-d")
        .filter_map(|v| v["t"].as_f64())
        .next()
        .expect("marks 里有 t");
    // 用同一时钟源（CLOCK_MONOTONIC）读当前时刻：`/proc/uptime` 与其同源。
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .expect("读 /proc/uptime")
        .split_whitespace()
        .next()
        .expect("uptime")
        .parse()
        .expect("解析 uptime");
    let elapsed = (uptime - t) * 1000.0;
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (elapsed, stderr, 0)
}
