//! F1/S5：`tool_call` 的**就地修改**语义（ref `agent-loop.ts:727`/`:770-775`、
//! `agent-session.ts:538-543`、`runner.ts:1134-1152`）。
//!
//! - 扩展对 `event.input` 的**就地修改**必须回传到宿主并作用于真实工具调用；
//! - 扩展**返回** `{ input }` 必须无效（ref 只使用 `block/reason/terminate`）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pi_core::tools::tool::{Tool, ToolContext};

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn prepare(agent: &Path) {
    let target = agent.join("extensions").join("mutate");
    std::fs::create_dir_all(&target).expect("建目录");
    let body =
        std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/input-mutate-ext.mjs"))
            .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

async fn call(arguments: serde_json::Value) -> serde_json::Value {
    let agent = tempfile::tempdir().expect("agent dir");
    prepare(agent.path());
    let work = tempfile::tempdir().expect("work dir");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("需要 node"));
    pi_cli::ext_tools::configure_ctx(agent.path(), "print", false);
    let tools: Vec<Arc<dyn Tool>> =
        pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    let tool = tools
        .iter()
        .find(|tool| tool.name() == "ext_echo_args")
        .cloned()
        .expect("应有扩展工具");
    let context = ToolContext {
        cwd: work.path().to_path_buf(),
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let result = tool.execute(arguments, &context, &cancel).await;
    assert!(!result.is_error, "不应报错：{}", result.text_content());
    serde_json::from_str(&result.text_content()).expect("工具应回显 JSON 参数")
}

/// 两个场景**必须串行**：`ext_tools` 的扩展宿主是**进程级单例**（`HOST`），
/// 而 `call()` 每次建自己的临时 agentDir。两个 `#[tokio::test]` 并行时，先启动
/// 宿主的那个测试独占句柄，另一个既拿不到自己的扩展、其临时目录还可能已被 drop
/// ⇒ `Cannot find module .../extensions/mutate/index.mjs`（真机 precommit 实测）。
///
/// 因此合成一个测试、顺序跑两个场景：既保住覆盖，又消除对进程级单例的并行争用。
#[tokio::test]
async fn tool_call_input_semantics() {
    if node_path().is_none() {
        eprintln!("跳过：本机没有 node");
        return;
    }
    // 场景 1：扩展**就地修改** `event.input` 必须回传到真实工具调用。
    let seen = call(serde_json::json!({"mutateInPlace": true})).await;
    assert_eq!(
        seen.get("injected").and_then(serde_json::Value::as_str),
        Some("in-place"),
        "就地修改应生效，实际参数：{seen}"
    );
    assert!(
        seen.get("mutateInPlace").is_none(),
        "扩展删掉的字段也应消失，实际参数：{seen}"
    );

    // 场景 2：扩展**返回** `{ input }` 必须无效（ref 只使用 block/reason/terminate）。
    let seen = call(serde_json::json!({"returnInput": true})).await;
    assert_eq!(
        seen.get("injected").and_then(serde_json::Value::as_str),
        None,
        "返回 {{ input }} 在 ref 里无效，不应改参数，实际参数：{seen}"
    );
    assert_eq!(
        seen.get("returnInput").and_then(serde_json::Value::as_bool),
        Some(true),
        "原参数应保持不变，实际参数：{seen}"
    );
}
