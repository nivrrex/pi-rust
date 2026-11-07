//! P9-1-E1c / S3：9 个真实扩展的逐个调用矩阵。
//!
//! 由 `PI_MATRIX_EXTENSIONS_DIR` 指定**副本**的扩展根目录（测试自己不改原件），
//! `PI_MATRIX_OUT` 指定原始 JSON 输出路径。策略：
//! - 只调用**无副作用**的工具（`bash` 跑 `true`、`cg_recall` 查询、`todo` 列清单）；
//! - 触网 / 写盘 / 要凭据 / 需 UI 的工具**不调用**，只记录跳过理由与缺口。
//!
//! 未设置 `PI_MATRIX_EXTENSIONS_DIR` 时跳过（普通 `cargo test` 不会碰真实扩展）。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{discover_extensions_in_order, HostHandle, HostOptions};
use serde_json::json;

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
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

/// 工具名 → (是否调用, 参数, 跳过理由/缺口)。
fn policy(name: &str) -> (bool, serde_json::Value, &'static str) {
    match name {
        // `bash` 走宿主动作帧（`host.action.call`），矩阵里跑 `true`（无副作用）。
        "bash" => (true, json!({"command": "true"}), ""),
        // `cg_recall` 需要 context-gateway 真实生成的 `cg_` 归档 id（环境相关）。
        "cg_recall" => (
            false,
            json!({}),
            "需要有效的 cg_ 归档 id（由 context-gateway 运行时生成）",
        ),
        "todo" => (true, json!({"action": "list"}), ""),
        "report_to_user" => (false, json!({}), "需要 UI 面板（agent→用户汇报通道）"),
        "ask_user" => (false, json!({}), "需要 select/input 面板（ctx.ui 交互）"),
        "request_secret" => (false, json!({}), "需要用户凭据与掩码输入框"),
        "web_read" => (false, json!({}), "会触网"),
        "web_shot" => (false, json!({}), "会触网 + 需要浏览器引擎"),
        "web_eval" => (false, json!({}), "会触网 + 需要浏览器引擎"),
        "web_search" => (false, json!({}), "会触网"),
        _other => (false, json!({}), "未列入策略表（未调用）"),
    }
}

#[test]
fn matrix_over_real_extensions() {
    let Ok(root) = std::env::var("PI_MATRIX_EXTENSIONS_DIR") else {
        eprintln!("跳过：未设置 PI_MATRIX_EXTENSIONS_DIR");
        return;
    };
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent_dir = PathBuf::from(&root).join("agent");
    let cwd = PathBuf::from(&root).join("work");
    std::fs::create_dir_all(&cwd).expect("建工作目录");
    let extensions = discover_extensions_in_order(&cwd, &agent_dir);
    let mut records: Vec<serde_json::Value> = Vec::new();
    let host = HostHandle::start(HostOptions {
        node,
        runner: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/ext-host/runner.mjs"),
        extensions: extensions.clone(),
        cwd: cwd.clone(),
        env: vec![(
            "PI_CODING_AGENT_DIR".to_string(),
            agent_dir.display().to_string(),
        )],
    })
    .expect("宿主应能启动（9 个真实扩展）");

    // 宿主动作帧：用 `true` 兜底（矩阵只验证"能调通"，不验证命令输出）。
    let (tx, rx) = std::sync::mpsc::channel::<pi_ext_host::ActionCall>();
    host.set_action_sink(Some(tx));
    let worker = std::thread::spawn(move || {
        for call in rx {
            let _ = call.reply.send(json!({
                "content": [{ "type": "text", "text": "ok" }],
                "isError": false,
            }));
        }
    });
    let mut tools: Vec<(String, serde_json::Value)> = host
        .tools()
        .iter()
        .map(|spec| (spec.name.clone(), spec.parameters.clone()))
        .collect();
    tools.sort_by(|a, b| a.0.cmp(&b.0));
    let tools_all = tools.clone();
    for (name, parameters) in tools {
        let (invoke, args, reason) = policy(&name);
        if !invoke {
            records.push(json!({
                "tool": name,
                "status": "skipped",
                "reason": reason,
            }));
            continue;
        }
        match host.invoke(&name, args, Duration::from_secs(20)) {
            Ok(value) => {
                let text = value
                    .get("content")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .chars()
                    .take(120)
                    .collect::<String>();
                let is_error = value
                    .get("isError")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                records.push(json!({
                    "tool": name,
                    "status": if is_error { "error" } else { "ok" },
                    "snippet": text,
                    "parameters": parameters,
                }));
            }
            Err(error) => records.push(json!({
                "tool": name,
                "status": "failed",
                "reason": error.to_string(),
            })),
        }
    }
    // 渲染钩子覆盖（S5）：逐个工具跑一次 `renderCall`，记录帧类型。
    let mut renders: Vec<serde_json::Value> = Vec::new();
    for (name, _parameters) in &tools_all {
        let frame = host
            .render(name, "call", json!({"args": {}}), 80)
            .ok()
            .flatten();
        let kind = match &frame {
            None => "none",
            Some(value) => value.get("kind").and_then(|k| k.as_str()).unwrap_or("?"),
        };
        renders.push(json!({ "tool": name, "frame": kind }));
    }
    let loaded: Vec<String> = extensions.iter().map(|p| p.display().to_string()).collect();
    let report = json!({
        "extensions_loaded": loaded,
        "tools": records,
        "renders": renders,
        "commands": host
            .commands()
            .iter()
            .map(|command| command.name.clone())
            .collect::<Vec<_>>(),
        "events": host.events(),
    });
    let text = serde_json::to_string_pretty(&report).expect("序列化");
    if let Ok(out) = std::env::var("PI_MATRIX_OUT") {
        std::fs::write(out, &text).expect("写输出");
    }
    eprintln!("{text}");
    host.shutdown();
    let _ = worker.join();
    assert!(
        !report["tools"].as_array().expect("数组").is_empty(),
        "至少应发现若干工具"
    );
}
