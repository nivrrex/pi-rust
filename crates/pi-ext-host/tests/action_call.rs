//! 宿主动作帧：扩展用 `createBashToolDefinition` 注册的 `bash` 工具，执行由宿主完成
//! （ref 的同名构造器也走宿主的 shell 执行器）。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{ActionCall, HostHandle, HostOptions};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// 一个最小的 fixture：注册一个用 `createBashToolDefinition` 构造的工具。
const FIXTURE: &str = r#"
import { createBashToolDefinition } from "@earendil-works/pi-coding-agent";

export default function actionExtension(pi) {
  const tool = createBashToolDefinition({ description: "run a command" });
  pi.registerTool({
    name: "ext_bash",
    description: "通过宿主动作帧执行命令",
    parameters: { type: "object", properties: { command: { type: "string" } }, required: ["command"] },
    execute: (id, args) => tool.execute(id, args),
  });
}
"#;

#[test]
fn bash_tool_executes_through_host_action() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let ext = tmp.path().join("action-ext.mjs");
    std::fs::write(&ext, FIXTURE).expect("写 fixture");
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![ext],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");
    // 假宿主：直接回固定结果，验证"请求确实以宿主动作帧送达"。
    let (tx, rx) = std::sync::mpsc::channel::<ActionCall>();
    host.set_action_sink(Some(tx));
    let worker = std::thread::spawn(move || {
        for call in rx {
            assert_eq!(call.action, "bash");
            let command = call.args["command"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let _ = call.reply.send(serde_json::json!({
                "content": [{ "type": "text", "text": format!("ran:{command}") }],
                "isError": false,
            }));
        }
    });
    let result = host
        .invoke(
            "ext_bash",
            serde_json::json!({"command": "echo hi"}),
            Duration::from_secs(20),
        )
        .expect("调用应成功");
    assert_eq!(result["content"][0]["text"], "ran:echo hi");
    host.shutdown();
    drop(host);
    let _ = worker.join();
}
