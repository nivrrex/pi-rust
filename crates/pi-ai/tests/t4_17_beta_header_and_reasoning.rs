//! P4-15 步骤 5.2：把两处"只有单元级证据"的项做成**端到端**。
//!
//! ① `anthropic-beta` 头（ref `ai/src/api/anthropic-messages.ts:182` 的
//!    `INTERLEAVED_THINKING_BETA = "interleaved-thinking-2025-05-14"`；`:1074` 非空才发；
//!    `:1019-1026` 的触发条件）——此前只用纯函数单测覆盖；现在由 mock 的 `--header-log`
//!    记录**真实请求头**来断言。
//! ② decode 侧 opencode-go 特例（ref `openai-completions.ts:605-623`：按
//!    `reasoning_content → reasoning → reasoning_text` 取**第一个非空**字段，并把**命中的字段名**
//!    当 signature；`provider === "opencode-go"` 且命中 `reasoning` 时记作 `reasoning_content`）
//!    ——此前没有夹具能走到它（仓库里的 SSE 都发 `reasoning_content`）；现在测试**临时**造一个
//!    发 `reasoning` 的 SSE，用真实 client 走一遍。
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pi_ai::anthropic::{AnthropicMessagesClient, AuthHeader};
use pi_ai::config::ModelsConfig;
use pi_ai::openai::ChatCompletionsClient;
use pi_ai::{Context, Message, ProviderRequest};

struct Mock {
    child: Child,
    port: u16,
    dir: std::path::PathBuf,
    header_log: std::path::PathBuf,
}

impl Mock {
    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crate 目录在工作区根下")
            .to_path_buf()
    }

    /// 用**临时**场景启动；`sse` 是临时写出的 SSE 正文。
    fn start(sse: Option<&str>) -> Self {
        let repo = Self::repo_root();
        let binary = repo.join("target/debug/mock-provider");
        assert!(
            binary.exists(),
            "先构建 mock-provider：{}",
            binary.display()
        );
        let dir = std::env::temp_dir().join(format!(
            "pi-rs-t4-17-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let fixture = dir.join("temp.sse");
        let content = sse.map(str::to_string).unwrap_or_else(|| {
            std::fs::read_to_string(repo.join("fixtures/sse/openai/text-basic.sse"))
                .expect("内置夹具可读")
        });
        std::fs::write(&fixture, content).expect("写临时 SSE");
        let scenario = dir.join("scenario.json");
        std::fs::write(
            &scenario,
            format!(
                "{{\"responses\":[{{\"file\":{}}}]}}\n",
                serde_json::to_string(&fixture.to_string_lossy().to_string()).expect("可序列化")
            ),
        )
        .expect("写临时场景");
        let header_log = dir.join("headers.jsonl");
        let mut child = Command::new(&binary)
            .current_dir(&repo)
            .arg(&scenario)
            .arg("--port")
            .arg("0")
            .arg("--header-log")
            .arg(&header_log)
            .stdout(Stdio::piped())
            .spawn()
            .expect("mock 可执行文件存在");
        let stdout = child.stdout.take().expect("已取走 stdout");
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).expect("mock 会打印监听地址");
        let port: u16 = line
            .trim()
            .strip_prefix("listening 127.0.0.1:")
            .expect("地址格式为 listening 127.0.0.1:PORT")
            .parse()
            .expect("端口是数字");
        Self {
            child,
            port,
            dir,
            header_log,
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn request_headers(&self) -> std::collections::BTreeMap<String, String> {
        let text = std::fs::read_to_string(&self.header_log).expect("有请求头日志");
        let line = text
            .lines()
            .find(|line| !line.trim().is_empty())
            .expect("至少一条请求");
        let entry: serde_json::Value = serde_json::from_str(line).expect("日志行是 JSON");
        entry
            .get("headers")
            .and_then(serde_json::Value::as_object)
            .expect("有 headers")
            .iter()
            .map(|(name, value)| {
                (
                    name.to_ascii_lowercase(),
                    value.as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    fn keep(&self) -> &std::path::Path {
        &self.dir
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn resolved(provider_key: &str, api: &str, base_url: &str) -> pi_ai::config::ResolvedModel {
    let text = format!(
        "{{\"providers\":{{\"{provider_key}\":{{\"api\":\"{api}\",\"baseUrl\":\"{base_url}\",\
         \"models\":[{{\"id\":\"mock-model\",\"cost\":{{\"input\":1,\"output\":2,\"cacheRead\":0,\"cacheWrite\":0}}}}]}}}}}}"
    );
    let config: ModelsConfig = serde_json::from_str(&text).expect("夹具可解析");
    config
        .resolve(provider_key, "mock-model")
        .expect("可解析出模型")
}

fn request(thinking: Option<pi_ai::anthropic::ThinkingConfig>) -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: Context::from_messages(
            Some("sys".to_string()),
            &[Message::User {
                content: pi_ai::UserContent::Text("hi".to_string()),
                timestamp: 0,
                extra: serde_json::Map::new(),
            }],
        ),
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking,
        headers: Vec::new(),
    }
}

/// 跑完一条流并给出**最终消息**（`Stream::final_message`，未中毒时）。
async fn drive(
    provider: &dyn pi_ai::Provider,
    request: ProviderRequest,
) -> Option<pi_ai::AssistantMessage> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stream = loop {
        match provider.stream(request.clone()).await {
            Ok(stream) => break stream,
            Err(error) => {
                assert!(Instant::now() < deadline, "等待 mock 就绪超时：{error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    while futures::StreamExt::next(&mut stream).await.is_some() {}
    stream.final_message().expect("槽位未中毒")
}

#[tokio::test]
async fn anthropic_beta_header_is_actually_sent() {
    // 触发条件：推理模型 + thinkingEnabled（ref `:1019-1026`）→ 应发 `anthropic-beta`（`:1074`）。
    let mock = Mock::start(None);
    let resolved = resolved("mock", "anthropic-messages", &mock.base_url());
    let thinking = pi_ai::anthropic::thinking_config_for_level(Some("medium"), None, 4096);
    assert!(
        matches!(
            thinking,
            Some(pi_ai::anthropic::ThinkingConfig::Enabled { .. })
        ),
        "样本自检：等级 medium 给出 Enabled（否则不会触发该 beta）"
    );
    // 直接构造 anthropic 客户端（pi-ai 的测试里没有 pi-cli 依赖）。
    let client =
        AnthropicMessagesClient::from_resolved(&resolved, AuthHeader::default(), "placeholder-key");
    let _final_message = drive(&client, request(thinking)).await;

    let headers = mock.request_headers();
    assert_eq!(
        headers.get("anthropic-beta").map(String::as_str),
        Some(pi_ai::anthropic::INTERLEAVED_THINKING_BETA),
        "真实请求必须带 anthropic-beta：{headers:?}"
    );
}

#[tokio::test]
async fn opencode_go_maps_a_plain_reasoning_delta_to_reasoning_content() {
    // 临时 SSE：delta 里用 **`reasoning`**（仓库夹具都发 `reasoning_content`，走不到该分支）。
    let sse = concat!(
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",\
         \"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",\
         \"choices\":[{\"index\":0,\"delta\":{\"reasoning\":\"先想一步\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",\
         \"choices\":[{\"index\":0,\"delta\":{\"content\":\"答案\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",\
         \"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let mock = Mock::start(Some(sse));
    let _keep = mock.keep();

    // 正向：provider 键 `opencode-go` → signature 记作 `reasoning_content`。
    let go = resolved("opencode-go", "openai-completions", &mock.base_url());
    assert_eq!(
        go.provider_name, "opencode-go",
        "样本自检：provider 名带上了"
    );
    let client = ChatCompletionsClient::from_resolved(&go);
    let message = drive(&client, request(None))
        .await
        .expect("客户端应留下最终消息");
    let signature = first_thinking_signature(&message);
    assert_eq!(
        signature.as_deref(),
        Some("reasoning_content"),
        "opencode-go 的 `reasoning` 增量必须记作 reasoning_content"
    );
}

#[tokio::test]
async fn other_provider_keeps_the_plain_reasoning_signature() {
    let sse = concat!(
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",\
         \"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",\
         \"choices\":[{\"index\":0,\"delta\":{\"reasoning\":\"先想一步\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",\
         \"choices\":[{\"index\":0,\"delta\":{\"content\":\"答案\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",\
         \"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let mock = Mock::start(Some(sse));
    let other = resolved("some-other", "openai-completions", &mock.base_url());
    let client = ChatCompletionsClient::from_resolved(&other);
    let message = drive(&client, request(None))
        .await
        .expect("客户端应留下最终消息");
    assert_eq!(
        first_thinking_signature(&message).as_deref(),
        Some("reasoning"),
        "非 opencode-go 必须保留真实字段名"
    );
}

fn first_thinking_signature(message: &pi_ai::AssistantMessage) -> Option<String> {
    message.content.iter().find_map(|block| match block {
        pi_ai::Content::Thinking {
            thinking_signature, ..
        } => thinking_signature.clone(),
        _not_thinking => None,
    })
}
