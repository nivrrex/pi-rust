//! P4-13 步骤 2.3：provider 来源接线——经**生产构造函数** `pi_cli::runtime::build_provider`
//! 走真实 client 构造请求，断言 opencode-go 特例真的生效。
//!
//! 修复前的病（"字段存在、值写死"）：`crates/pi-ai/src/openai.rs:191`（`provider: "mock"`）、
//! `crates/pi-ai/src/anthropic.rs:237`（同）；而生产构造点 `runtime.rs` 用的是 `new(base_url, rates)`，
//! **不传 provider 名** → 发送侧特例（`openai.rs:278` → `context.rs:125`）与解码侧特例
//! （`openai.rs:325` → `decoder.rs:456`）在**生产路径上永远不触发**。
//!
//! ref `ai/src/api/openai-completions.ts:1335-1337`：`model.provider === "opencode-go" && signature === "reasoning"`
//! → 字段名记作 `reasoning_content`；否则按 signature 原名。
//!
//! 判据里"样本必须真的含目标 thinking 块"：下面 `request()` 的上下文里确实放了
//! `Content::Thinking { thinking_signature: Some("reasoning") }`，并先断言它存在。
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pi_ai::config::ModelsConfig;
use pi_ai::{Content, Context, Message, ProviderRequest};

/// 测试用的 mock-provider 进程（照 `crates/pi-ai/tests/t2_3c_http.rs` 的写法）。
struct MockProvider {
    child: Child,
    port: u16,
    log: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl MockProvider {
    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crate 目录在工作区根下")
            .to_path_buf()
    }

    /// 用**临时场景**启动：场景与请求日志都落在临时目录，夹具按绝对路径引用仓库里的 SSE。
    fn start(sse_fixture: &str) -> Self {
        let repo = Self::repo_root();
        let binary = repo.join("target/debug/mock-provider");
        assert!(
            binary.exists(),
            "先构建 mock-provider：cargo build -p mock-provider（缺失：{}）",
            binary.display()
        );
        let dir = tempfile::tempdir().expect("建临时目录");
        let scenario = dir.path().join("scenario.json");
        let fixture = repo.join("fixtures/sse/openai").join(sse_fixture);
        assert!(fixture.exists(), "夹具存在：{}", fixture.display());
        std::fs::write(
            &scenario,
            format!(
                "{{\"responses\":[{{\"file\":{}}}]}}\n",
                serde_json::to_string(&fixture.to_string_lossy().to_string()).expect("可序列化")
            ),
        )
        .expect("写临时场景");
        let log = dir.path().join("requests.jsonl");

        let mut child = Command::new(&binary)
            .current_dir(&repo)
            .arg(&scenario)
            .arg("--port")
            .arg("0")
            .arg("--request-log")
            .arg(&log)
            .stdout(Stdio::piped())
            .spawn()
            .expect("mock-provider 可执行文件存在");
        let stdout = child.stdout.take().expect("已取走 stdout");
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).expect("mock 会打印监听地址");
        let port: u16 = line
            .trim()
            .strip_prefix("listening 127.0.0.1:")
            .unwrap_or_else(|| panic!("地址格式为 listening 127.0.0.1:PORT，实际：{line:?}"))
            .parse()
            .expect("端口是数字");
        Self {
            child,
            port,
            log,
            _dir: dir,
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 读回 mock 记下的请求体（第一条 `POST /v1/chat/completions`）。
    fn first_request_body(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(&self.log).expect("mock 应已写下请求日志");
        let line = text
            .lines()
            .find(|line| !line.trim().is_empty())
            .expect("至少有一条请求记录");
        let entry: serde_json::Value = serde_json::from_str(line).expect("日志行是 JSON");
        let body = entry.get("body").cloned().expect("日志有 body");
        match body {
            serde_json::Value::String(text) => {
                serde_json::from_str(&text).expect("body 是 JSON 文本")
            }
            other => other,
        }
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 带一个 thinking 块的助手消息（签名 `reasoning` —— opencode-go 的特例只在这种情况下改写字段名）。
fn assistant_with_thinking() -> Message {
    Message::Assistant(Box::new(pi_ai::AssistantMessage {
        role: "assistant".to_string(),
        content: vec![
            Content::Thinking {
                thinking: "先读文件再回答".to_string(),
                thinking_signature: Some("reasoning".to_string()),
                redacted: None,
            },
            Content::Text {
                text: "答案".to_string(),
                text_signature: None,
            },
        ],
        api: "openai-completions".to_string(),
        provider: "opencode-go".to_string(),
        model: "mock-model".to_string(),
        response_model: None,
        usage: Default::default(),
        response_id: None,
        timestamp: 0,
        end_turn: None,
        stop_reason: pi_ai::StopReason::Stop,
        error_message: None,
        raw_stop_reason: None,
    }))
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: Context::from_messages(
            Some("sys".to_string()),
            &[
                assistant_with_thinking(),
                Message::User {
                    content: pi_ai::UserContent::Text("继续".to_string()),
                    timestamp: 0,
                    extra: serde_json::Map::new(),
                },
            ],
        ),
        max_completion_tokens: 64,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    }
}

/// 用 models.json 夹具解析出 `ResolvedModel`（provider 键由调用方给）。
fn resolved_for(provider_key: &str, base_url: &str) -> pi_ai::config::ResolvedModel {
    let models_json = format!(
        "{{\"providers\":{{\"{provider_key}\":{{\"name\":\"夹具\",\"baseUrl\":\"{base_url}\",\
         \"api\":\"openai-completions\",\"models\":[{{\"id\":\"mock-model\",\
         \"cost\":{{\"input\":1,\"output\":2,\"cacheRead\":0,\"cacheWrite\":0}}}}]}}}}}}"
    );
    let config: ModelsConfig = serde_json::from_str(&models_json).expect("夹具 models.json 可解析");
    config
        .resolve(provider_key, "mock-model")
        .expect("夹具里能解析出 mock-model")
}

/// 样本自检：请求上下文里**确实**有目标 thinking 块（防止断言一个空集合）。
fn assert_sample_has_thinking(request: &ProviderRequest) {
    let has = request.context.messages.iter().any(|message| match message {
        Message::Assistant(assistant) => assistant.content.iter().any(|block| {
            matches!(block, Content::Thinking { thinking_signature: Some(signature), .. } if signature == "reasoning")
        }),
        _not_assistant => false,
    });
    assert!(
        has,
        "样本必须含 thinking_signature == \"reasoning\" 的 thinking 块"
    );
}

#[tokio::test]
async fn opencode_go_provider_sends_reasoning_content() {
    let mock = MockProvider::start("text-basic.sse");
    let resolved = resolved_for("opencode-go", &mock.base_url());
    let request = request();
    assert_sample_has_thinking(&request);

    let provider = pi_cli::runtime::build_provider(&resolved, Some("sk-test".to_string()))
        .expect("provider 构造成功");
    let mut stream = stream_with_retry(provider.as_ref(), request).await;
    while futures::StreamExt::next(&mut stream).await.is_some() {}

    let body = mock.first_request_body();
    let assistant = body["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("messages 是数组，实际 body：{body}"))
        .iter()
        .find(|message| message.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .unwrap_or_else(|| panic!("请求体里有助手消息，实际 body：{body}"));
    assert_eq!(
        assistant.get("reasoning_content").and_then(|v| v.as_str()),
        Some("先读文件再回答"),
        "opencode-go 必须落到 reasoning_content：{assistant}"
    );
    assert!(
        assistant.get("reasoning").is_none(),
        "opencode-go 不应再发原字段名 reasoning：{assistant}"
    );
}

#[tokio::test]
async fn other_provider_keeps_the_actual_field_name() {
    let mock = MockProvider::start("text-basic.sse");
    let resolved = resolved_for("some-other-provider", &mock.base_url());
    let request = request();
    assert_sample_has_thinking(&request);

    let provider = pi_cli::runtime::build_provider(&resolved, Some("sk-test".to_string()))
        .expect("provider 构造成功");
    let mut stream = stream_with_retry(provider.as_ref(), request).await;
    while futures::StreamExt::next(&mut stream).await.is_some() {}

    let body = mock.first_request_body();
    let assistant = body["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("messages 是数组，实际 body：{body}"))
        .iter()
        .find(|message| message.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .unwrap_or_else(|| panic!("请求体里有助手消息，实际 body：{body}"));
    assert_eq!(
        assistant.get("reasoning").and_then(|v| v.as_str()),
        Some("先读文件再回答"),
        "非 opencode-go 保持原字段名：{assistant}"
    );
    assert!(
        assistant.get("reasoning_content").is_none(),
        "非 opencode-go 不应改名：{assistant}"
    );
}

#[tokio::test]
async fn client_reports_the_provider_it_was_built_with() {
    let mock = MockProvider::start("text-basic.sse");
    // anthropic 侧的 provider 名**不进请求体**（只进事件里的 message.provider），
    // 因此这里断言构造器把名字带进去了；端到端的事件侧由场景 S12-anthropic 覆盖。
    let resolved = resolved_for("opencode-go", &mock.base_url());
    let client = pi_ai::openai::ChatCompletionsClient::from_resolved(&resolved);
    assert_eq!(client.provider_name(), "opencode-go");

    let anthropic = pi_ai::anthropic::AnthropicMessagesClient::from_resolved(
        &resolved,
        pi_ai::anthropic::AuthHeader::default(),
        "k",
    );
    assert_eq!(
        anthropic.provider_name(),
        "opencode-go",
        "anthropic client 也必须带上 provider 名"
    );

    // 生产构造点对两条 api 都能成功构造（等价于老的 `new(base_url, rates)` 路径）。
    for api in ["openai-completions", "anthropic-messages"] {
        let mut resolved = resolved_for("opencode-go", &mock.base_url());
        resolved.api = api.to_string();
        let _provider = pi_cli::runtime::build_provider(&resolved, None)
            .unwrap_or_else(|error| panic!("{api} 应能构造：{error}"));
    }
}

#[tokio::test]
async fn unknown_api_is_rejected() {
    // 负向对照：`build_provider` 仍要拒绝不支持的 api（原有行为不得被接线改动破坏）。
    let mut resolved = resolved_for("opencode-go", "http://127.0.0.1:1");
    resolved.api = "not-an-api".to_string();
    let error = pi_cli::runtime::build_provider(&resolved, None)
        .err()
        .expect("不支持的 api 必须报错");
    assert!(error.contains("not-an-api"), "{error}");
}

/// mock 进程刚起来时端口可能还没 accept，照 `t2_3c_http.rs` 的做法等一下。
async fn stream_with_retry(
    provider: &dyn pi_ai::Provider,
    request: ProviderRequest,
) -> pi_ai::provider::Stream {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match provider.stream(request.clone()).await {
            Ok(stream) => return stream,
            Err(error) => {
                assert!(Instant::now() < deadline, "等待 mock 就绪超时：{error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}
