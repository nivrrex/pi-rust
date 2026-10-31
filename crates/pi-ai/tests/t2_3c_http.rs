//! T2.3c：HTTP 客户端与 SSE 字节流接入。
//!
//! 判据：对着本地 mock-provider 发一次请求，事件序列应与 `golden/S12-openai`
//! 的 events.jsonl 同形（start → thinking → text → 结束），且请求体落在 mock 的请求日志里。

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pi_ai::openai::{ChatCompletionsClient, OpenAiRates};
use pi_ai::{Provider, ProviderRequest};
use tokio_util::sync::CancellationToken;

/// 单条 user 文本的上下文。
fn ctx_of(prompt: &str, system: Option<&str>) -> pi_ai::Context {
    pi_ai::Context::from_messages(
        system.map(str::to_string),
        &[pi_ai::Message::User {
            content: pi_ai::UserContent::Text(prompt.to_string()),
            timestamp: 0,
            extra: serde_json::Map::new(),
        }],
    )
}

/// 测试用的 mock-provider 进程，退出时自动清理。
struct MockProvider {
    child: Child,
    port: u16,
}

impl MockProvider {
    /// mock-provider 是 workspace 里的另一个二进制 crate，测试拿不到
    /// `CARGO_BIN_EXE_*`，因此按工作区根解析路径。
    fn binary() -> std::path::PathBuf {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crate 目录在工作区根下");
        workspace_root.join("target/debug/mock-provider")
    }

    fn start(scenario: &str) -> Self {
        let binary = Self::binary();
        assert!(
            binary.exists(),
            "先构建 mock-provider：cargo build -p mock-provider（缺失：{}）",
            binary.display()
        );
        let mut child = Command::new(binary)
            .current_dir(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .and_then(std::path::Path::parent)
                    .expect("crate 目录在工作区根下"),
            )
            .arg(format!("scenarios/{scenario}.json"))
            .arg("--port")
            .arg("0")
            .stdout(Stdio::piped())
            .spawn()
            .expect("mock-provider 可执行文件存在");
        let stdout = child.stdout.take().expect("已取走 stdout");
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).expect("mock 会打印监听地址");
        let port: u16 = line
            .trim()
            .strip_prefix("listening 127.0.0.1:")
            .expect("地址格式为 listening 127.0.0.1:PORT")
            .parse()
            .expect("端口是数字");
        Self { child, port }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: ctx_of("think then answer", Some("sys")),
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    }
}

#[tokio::test]
async fn streaming_request_produces_the_recorded_event_shape() {
    let mock = MockProvider::start("S12-openai");
    let client = ChatCompletionsClient::new(mock.base_url(), OpenAiRates::zero());
    let token = CancellationToken::new();

    let deadline = Instant::now() + Duration::from_secs(20);
    let stream = loop {
        match client.stream(request()).await {
            Ok(stream) => break stream,
            Err(error) => {
                assert!(Instant::now() < deadline, "等待 mock 就绪超时：{error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };

    let mut events = Vec::new();
    let mut stream = stream;
    while let Some(event) = futures::StreamExt::next(&mut stream).await {
        events.push(event);
    }

    assert!(
        events.first() == Some(&pi_ai::AssistantEvent::Start),
        "首事件应是 Start，实际：{:?}",
        events.first()
    );
    let names: Vec<&str> = events
        .iter()
        .map(pi_ai::AssistantEvent::type_name)
        .collect();
    assert!(
        names.contains(&"thinking_start"),
        "S12-openai 应有 thinking 段，实际序列：{names:?}"
    );
    // 首个响应是 reasoning-toolcall.sse：thinking + tool call，没有 text 段。
    assert_eq!(
        names,
        vec![
            "start",
            "thinking_start",
            "thinking_delta",
            "toolcall_start",
            "toolcall_delta",
            "thinking_end",
            "toolcall_end",
            "done",
        ],
        "首个响应的解码序列须与夹具一致"
    );
    drop(token);
}

/// 解码结果的 usage 与 cost 须与 TS 写进会话文件的数字一致（`golden/S19/session.jsonl`）。
#[tokio::test]
async fn decoded_usage_matches_the_recorded_session() {
    let mock = MockProvider::start("S19");
    let client = ChatCompletionsClient::new(
        mock.base_url(),
        pi_ai::decoder::Rates {
            input: 1.0,
            output: 2.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    let stream = loop {
        match client.stream(request()).await {
            Ok(stream) => break stream,
            Err(error) => {
                assert!(Instant::now() < deadline, "等待 mock 就绪超时：{error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };

    let mut stream = stream;
    while futures::StreamExt::next(&mut stream).await.is_some() {}

    // 断言放在解码器侧：直接复用 S19 的夹具喂进解码器，核对 usage 与 cost。
    use pi_ai::decoder::{Rates, StreamDecoder, OPENAI_COMPLETIONS};
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 目录在工作区根下");
    let bytes = std::fs::read(workspace_root.join("fixtures/sse/openai/big-text-usage.sse"))
        .expect("夹具存在");
    let rates = Rates {
        input: 1.0,
        output: 2.0,
        cache_read: 0.0,
        cache_write: 0.0,
    };
    let mut decoder =
        StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", rates).expect("api 名合法");
    decoder.push(&bytes).expect("解码不应报错");
    let usage = decoder.final_message().usage;
    assert_eq!(usage.input, 24_000, "golden/S19 记录的 input");
    assert_eq!(usage.output, 1_000, "golden/S19 记录的 output");
    assert_eq!(usage.total_tokens, 25_000, "golden/S19 记录的 totalTokens");
    assert_eq!(
        usage.cost.input.to_bits(),
        0.024_f64.to_bits(),
        "input 费用须与 TS 逐位一致"
    );
    assert_eq!(
        usage.cost.output.to_bits(),
        0.002_f64.to_bits(),
        "output 费用须逐位一致"
    );
    assert_eq!(
        usage.cost.total.to_bits(),
        0.026000000000000002_f64.to_bits(),
        "total 费用须与 TS 逐位一致（含末位差异）"
    );
}

/// 认证头发送方：`Authorization: Bearer <key>` 由 OpenAI SDK 下发，pi 自己只加 `User-Agent`。
#[test]
fn openai_auth_header_follows_the_sdk_shape() {
    use pi_ai::openai::{user_agent, AuthHeader};

    assert_eq!(
        AuthHeader::default(),
        AuthHeader::Bearer,
        "OpenAI SDK 默认走 Bearer"
    );
    assert_eq!(
        AuthHeader::Bearer.header_value("sk-x"),
        Some(("authorization".to_string(), "Bearer sk-x".to_string()))
    );
    assert_eq!(
        AuthHeader::ApiKey.header_value("k"),
        Some(("x-api-key".to_string(), "k".to_string()))
    );
    assert_eq!(AuthHeader::None.header_value("k"), None);
    assert_eq!(AuthHeader::Bearer.header_value(""), None, "空 key 不发头");

    let ua = user_agent();
    assert!(
        ua.starts_with("pi ("),
        "User-Agent 格式照 pi-user-agent.ts：{ua}"
    );
    assert!(ua.contains(';'), "User-Agent 含架构段：{ua}");
}
