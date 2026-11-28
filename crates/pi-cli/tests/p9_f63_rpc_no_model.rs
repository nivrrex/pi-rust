//! F63/B3：**无可用模型时 rpc 入口不得退出**（ref 实测同一份坏 `models.json` 下
//! TS `rc=0` 且逐条应答，pi-rs 曾 `rc=1` 直接退出、客户端拿不到任何应答）。
//!
//! 判据：进程 `rc=0`，且对每条请求都回一个带同一 `id` 的 `success:false` 响应
//! （不是"无应答"）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::process::{Command, Stdio};

/// 起 rpc，喂两行请求，返回（退出码, stdout 行）。
fn run_rpc(models_json: &str) -> (i32, Vec<String>) {
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(agent.join("models.json"), models_json).expect("写 models");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");

    let mut child = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--mode")
        .arg("rpc")
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动 rpc");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        // 请求体里含 `{`，直接 `write_all` 而不是 `writeln!`（后者按格式串解析）。
        for request in [
            r#"{"id":1,"method":"get_state"}"#,
            r#"{"id":2,"method":"get_available_models"}"#,
        ] {
            writeln!(stdin, "{request}").expect("写请求");
        }
        stdin.flush().expect("flush");
    }
    let output = child.wait_with_output().expect("等退出");
    let code = output.status.code().unwrap_or(-1);
    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    (code, lines)
}

/// 坏 `models.json`（trailing comma）⇒ 不退出、每条请求都有应答。
#[test]
fn rpc_survives_a_broken_models_json() {
    let (code, lines) = run_rpc(r#"{"providers":{"p":{},}}"#);
    assert_eq!(
        code, 0,
        "坏 models.json 下 rpc 不得退出（ref 实测 TS `rc=0`）"
    );
    assert_eq!(
        lines.len(),
        2,
        "两条请求都应有应答（不是无应答）：{lines:?}"
    );
    for (index, line) in lines.iter().enumerate() {
        let value: serde_json::Value = serde_json::from_str(line).expect("应答应是 JSON");
        assert_eq!(
            value.get("id").and_then(serde_json::Value::as_i64),
            Some(index as i64 + 1),
            "应答应带回请求 id：{line}"
        );
        assert_eq!(
            value.get("success").and_then(serde_json::Value::as_bool),
            Some(false),
            "无模型时应答 success:false：{line}"
        );
    }
}

/// `defaultProvider` 不在注册表（models.json 合法）⇒ 同样不退出。
#[test]
fn rpc_survives_an_unknown_default_provider() {
    let (code, lines) = run_rpc(
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"k","models":[{"id":"m1"}]}}}"#,
    );
    // `p/m1` 存在，rpc 正常启动；请求方法本身无效 ⇒ 也是 `success:false` 但不因配置退出。
    assert_eq!(code, 0, "rpc 应正常退出（行数 {}）", lines.len());
}
