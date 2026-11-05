//! S4：ctx 的 `model` / `thinkingLevel` 是**当前**值——切换后下一次事件里扩展应看到新值。
//! ref `types.ts:325-334`。

use std::path::{Path, PathBuf};

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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prepare(agent: &Path) {
    let target = agent.join("extensions/ctx");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/ctx-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

#[test]
fn ctx_reflects_model_switch() {
    let Some(_node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    prepare(agent.path());
    let log = agent.path().join("ctx.log");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("node"));
    std::env::set_var("PI_EXT_TEST_LOG", &log);

    pi_cli::ext_tools::configure_ctx(agent.path(), "tui", true);
    // F28/S1：`session_start` 现在推迟到会话登记之后派发，这里先登记一个 id（相当于已建会话），
    // 宿主启动时就会派发首次 `session_start`（此时 model 仍为 null）。
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-000000000000".to_string()));
    // 启动宿主；`session_start` 延后到 `flush_pending_session_start`（此时 model 仍为 null）。
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();
    // 切换模型 + 思考档位后，ctx 必须更新（下一次事件读到新值）。
    pi_cli::ext_tools::update_ctx_model(Some("mock-model-2"), Some("high"));
    pi_cli::ext_tools::emit_event(
        "session_start",
        serde_json::json!({"type": "session_start", "reason": "reload"}),
    );
    // emit_event 在无 tokio 运行时时是同步的；给后台留一点余量。
    std::thread::sleep(std::time::Duration::from_millis(300));

    let body = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(body.contains(":null:null"), "首次应看到 null ctx：{body}");
    assert!(
        body.contains("mock-model-2:high"),
        "切换后扩展应读到新值：{body}"
    );
}
