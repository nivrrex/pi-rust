//! P3.4：用**场景脚本**驱动 agent loop（覆盖 S01–S13 中可直接重放的部分）。
//!
//! 与 `t3_25_agent_loop` 的分工：那条用桩 provider 钉"事件顺序"这类机制；这条用真实
//! `tools/mock-provider` 二进制 + 真实四个工具跑 golden 场景，比对**轮数、工具结果、
//! 最终助手文本**——即"loop 的行为是否与 TS 录下来的一致"。
//!
//! 未纳入的场景与原因见 `notes/decisions.md`：需要 runner 原语（`steps`、闸门、断连）或
//! 需要 CLI 层（slash 命令、会话切换）的，属 P3.5/P3.6 范围。

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use pi_ai::message::{Message, ToolResultContent};
use pi_ai::openai::ChatCompletionsClient;
use pi_ai::Provider;
use pi_core::agent::{AgentEvent, AgentLoop, CollectSink, LoopConfig};
use pi_core::tools::bash::BashTool;
use pi_core::tools::edit::EditTool;
use pi_core::tools::read::ReadTool;
use pi_core::tools::tool::Tool;
use pi_core::tools::write::WriteTool;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("工作区根")
        .to_path_buf()
}

struct MockGuard(Child);

impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// 启动 mock，返回 (守卫, 端口)。
fn start_mock(scenario: &str) -> (MockGuard, u16) {
    let binary = workspace_root().join("target/debug/mock-provider");
    assert!(
        binary.exists(),
        "先构建 mock-provider：cargo build -p mock-provider"
    );
    let mut child = Command::new(binary)
        .current_dir(workspace_root())
        .arg(format!("scenarios/{scenario}.json"))
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .spawn()
        .expect("mock-provider 可启动");
    let port = {
        use std::io::BufRead;
        let stdout = child.stdout.take().expect("已取走 stdout");
        let mut line = String::new();
        std::io::BufReader::new(stdout)
            .read_line(&mut line)
            .expect("mock 会打印监听地址");
        line.trim()
            .strip_prefix("listening 127.0.0.1:")
            .expect("地址格式为 listening 127.0.0.1:PORT")
            .parse::<u16>()
            .expect("端口可解析")
    };
    (MockGuard(child), port)
}

/// golden 会话里的消息条目。
fn golden_messages(scenario: &str) -> Vec<Value> {
    let path = workspace_root().join(format!("golden/{scenario}/session.jsonl"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()));
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["type"] == "message")
        .map(|entry| entry["message"].clone())
        .collect()
}

fn text_of(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => String::new(),
    }
}

fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadTool),
        Arc::new(WriteTool),
        Arc::new(EditTool),
        Arc::new(BashTool),
    ]
}

/// 重放一个场景，返回 `(turn 数, 工具结果 (文本, isError), 最后一次助手文本)`。
async fn replay(scenario: &str) -> (usize, Vec<(String, bool)>, String) {
    let scenario_json: Value = serde_json::from_str(
        &std::fs::read_to_string(workspace_root().join(format!("scenarios/{scenario}.json")))
            .expect("读场景"),
    )
    .expect("场景可解析");
    let (mock, port) = start_mock(scenario);
    let _keep_alive = &mock;

    let dir = tempfile::tempdir().expect("临时目录");
    if let Some(files) = scenario_json["files"].as_object() {
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content.as_str().unwrap_or_default())
                .expect("写场景文件");
        }
    }
    // 场景里写死的文件名（如 `input.txt`）都在临时目录里现造。
    let prompt = scenario_json["prompt"].as_str().unwrap_or("hi").to_string();

    let provider: Arc<dyn Provider> = Arc::new(ChatCompletionsClient::new(
        format!("http://127.0.0.1:{port}/v1"),
        pi_ai::decoder::Rates::zero(),
    ));
    let sink = Arc::new(CollectSink::default());
    let mut agent_loop = AgentLoop::new(
        provider,
        tools(),
        sink.clone(),
        LoopConfig {
            model: "mock-model".to_string(),
            max_completion_tokens: 4096,
            ..LoopConfig::default()
        },
    );
    agent_loop.cwd = dir.path().to_path_buf();

    let transcript = agent_loop
        .run(
            vec![Message::User {
                content: pi_ai::UserContent::Text(prompt),
                timestamp: 0,
                extra: serde_json::Map::new(),
            }],
            &|_| Vec::new(),
            &|_| Vec::new(),
            &CancellationToken::new(),
        )
        .await
        .expect("循环应成功");

    let turns = sink
        .snapshot()
        .iter()
        .filter(|event| matches!(event, AgentEvent::TurnStart))
        .count();
    // 路径归一化：golden 里把工作目录写成 `<TMP>/work`，比对前把真实临时目录替换掉。
    let work = dir.path().to_string_lossy().to_string();
    let normalize = |text: String| text.replace(&work, "<TMP>/work");
    let tool_results: Vec<(String, bool)> = transcript
        .iter()
        .filter_map(|message| match message {
            Message::ToolResult {
                content, is_error, ..
            } => Some((
                normalize(match content.first() {
                    Some(ToolResultContent::Text { text, .. }) => text.clone(),
                    Some(ToolResultContent::Image { .. }) => String::from("[image]"),
                    None => String::new(),
                }),
                *is_error,
            )),
            _ => None,
        })
        .collect();
    let final_text = transcript
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::Assistant(assistant) => Some(
                assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        pi_ai::Content::Text { text, .. } => Some(text.clone()),
                        pi_ai::Content::Thinking { .. }
                        | pi_ai::Content::ToolCall { .. }
                        | pi_ai::Content::Image { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            ),
            _ => None,
        })
        .unwrap_or_default();
    (turns, tool_results, final_text)
}

/// 可直接重放的场景：`openai-completions` + 有 prompt + 不需要 runner 原语。
const SCENARIOS: &[&str] = &[
    "S02",
    "S03",
    "S04",
    "S04b",
    "S04c",
    "S05",
    "S06",
    "S13b-text",
    "S13b-toolcall",
    "S26",
    "S26b",
    "S26c",
    "S26d",
    "S26e",
    "S26f",
];

#[tokio::test]
async fn loop_replay_matches_golden_for_every_listed_scenario() {
    let mut problems: Vec<String> = Vec::new();
    for scenario in SCENARIOS {
        let golden = golden_messages(scenario);
        if golden.is_empty() {
            problems.push(format!("{scenario}: golden 没有消息，无法比对"));
            continue;
        }
        let golden_assistants = golden.iter().filter(|m| m["role"] == "assistant").count();
        let golden_tools: Vec<(String, bool)> = golden
            .iter()
            .filter(|m| m["role"] == "toolResult")
            .map(|m| (text_of(m), m["isError"].as_bool().unwrap_or(false)))
            .collect();
        let golden_final = golden
            .iter()
            .rev()
            .find(|m| m["role"] == "assistant")
            .map(text_of)
            .unwrap_or_default();

        let (turns, tools, final_text) = replay(scenario).await;

        if turns != golden_assistants {
            problems.push(format!(
                "{scenario}: turn 数 {turns} != golden 的 assistant 条数 {golden_assistants}"
            ));
        }
        if tools.len() != golden_tools.len() {
            problems.push(format!(
                "{scenario}: 工具结果 {} 条 != golden {} 条（我的：{:?}）",
                tools.len(),
                golden_tools.len(),
                tools
            ));
            continue;
        }
        for (index, (mine, expected)) in tools.iter().zip(golden_tools.iter()).enumerate() {
            if mine.1 != expected.1 {
                problems.push(format!(
                    "{scenario}: 第 {index} 个工具结果 isError {} != {}",
                    mine.1, expected.1
                ));
            }
            if mine.0 != expected.0 {
                problems.push(format!(
                    "{scenario}: 第 {index} 个工具结果文本不同\n  我: {:?}\n  TS: {:?}",
                    mine.0, expected.0
                ));
            }
        }
        if final_text != golden_final {
            problems.push(format!(
                "{scenario}: 最终助手文本不同\n  我: {final_text:?}\n  TS: {golden_final:?}"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "与 golden 不一致的场景 {} 处：\n{}",
        problems.len(),
        problems.join("\n")
    );
}
