//! 切换模型必须派发 `model_select` 给扩展（ref `agent-session.ts:2105-2112`
//! 的 `_emitModelSelect`，`extensions/types.ts:913-919`）。
//!
//! 真机症状：`Ctrl+P` 之后扩展完全收不到 `model_select` —— 页脚里依赖它的
//! 逻辑（虚拟路由模型、订阅后缀）永远停在切换前的状态。
//!
//! 根因：`emit_model_select_json(built, previous_model_json(parts_ref), …)` 里的
//! `previous` 是**在 `model_state` 已改成新模型之后**才取的，于是 `previous == model`，
//! 命中"两个模型相同则不派发"的前置判断 ⇒ 一次都不会发。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Stdio};

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn cycling_models_dispatches_model_select() {
    let tmp = tempfile::tempdir().expect("tmp");
    let agent = tmp.path().join("agent");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(agent.join("extensions/ctxprobe")).expect("agent");
    std::fs::create_dir_all(&work).expect("work");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"acme":{"apiKey":"k","models":[
            {"id":"m-1","provider":"acme","api":"openai-completions","baseUrl":"http://127.0.0.1:1/v1","contextWindow":1000},
            {"id":"m-2","provider":"acme","api":"openai-completions","baseUrl":"http://127.0.0.1:1/v1","contextWindow":1000}
        ]}}}"#,
    )
    .expect("models");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"acme","defaultModel":"m-1"}"#,
    )
    .expect("settings");
    std::fs::write(
        agent.join("extensions/ctxprobe/package.json"),
        r#"{"type":"module","name":"ctxprobe"}"#,
    )
    .expect("package.json");

    let log = tmp.path().join("events.jsonl");
    let ext = format!(
        r#"
import fs from "node:fs";
const OUT = {out:?};
export default function (pi) {{
  pi.on("session_start", () => fs.appendFileSync(OUT, "session_start\n"));
  pi.on("model_select", (event, ctx) => {{
    const model = ctx.model;
    fs.appendFileSync(OUT, JSON.stringify({{
      event: "model_select",
      provider: model?.provider ?? null,
      id: model?.id ?? null,
      source: event?.source ?? null,
    }}) + "\n");
  }});
}}
"#,
        out = log.display().to_string()
    );
    std::fs::write(agent.join("extensions/ctxprobe/index.js"), ext).expect("写扩展");

    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 1500, "timeoutMs": 60000}},
        {"delayMs": 2500}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", tmp.path().join("sess"))
        .env("HOME", tmp.path())
        .env("PI_TEST_KEYS", "ctrl+p,ctrl+d")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 应成功：{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        text.contains("session_start"),
        "扩展没加载（连 session_start 都没有）：{text:?}"
    );
    let rows: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let switched: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|row| row["event"] == "model_select")
        .collect();
    assert!(
        !switched.is_empty(),
        "切换模型后应派发 model_select（扩展事件）：{text}"
    );
    let row = switched.last().expect("至少一条");
    assert_eq!(row["id"], "m-2", "应报告切换后的模型：{row}");
    assert_eq!(row["provider"], "acme", "应带 provider：{row}");
    assert_eq!(row["source"], "cycle", "Ctrl+P 的 source 是 cycle：{row}");
}
