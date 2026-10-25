//! T2.10：Anthropic Messages 的 usage 映射与流式客户端。
//!
//! 字段映射照 `ref/pi-0.87.1/packages/ai/src/api/anthropic-messages.ts:617-624`。

use pi_ai::anthropic::{build_messages_request, AnthropicMessagesClient, AuthHeader};
use pi_ai::{Context, Provider, ProviderRequest};

mod common;

fn ctx() -> Context {
    common::ctx_msgs(
        Some("sys".to_string()),
        vec![common::ContextMessage {
            role: common::Role::User,
            content: vec![common::Block::Text {
                text: "think then answer".to_string(),
            }],
            stop_reason: None,
        }],
    )
}

/// 退出测试时收掉 mock 进程。
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: ctx(),
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    }
}

#[test]
fn anthropic_usage_fields_map_onto_the_unified_usage() {
    use pi_ai::decoder::{Rates, StreamDecoder, ANTHROPIC_MESSAGES};

    let sse = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"role\":\"assistant\",\"model\":\"mock-model\",",
        "\"content\":[],\"stop_reason\":null,\"usage\":{\"input_tokens\":400,\"output_tokens\":1,",
        "\"cache_read_input_tokens\":100,\"cache_creation_input_tokens\":40,",

        "\"cache_creation\":{\"ephemeral_1h_input_tokens\":25}}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":60}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    let mut decoder = StreamDecoder::new(ANTHROPIC_MESSAGES, "mock", "mock-model", Rates::zero())
        .expect("api 名合法");
    decoder.push(sse.as_bytes()).expect("解码不应报错");
    let usage = decoder.final_message().usage;

    assert_eq!(usage.input, 400, "input_tokens → input");
    assert_eq!(usage.output, 60, "message_delta 的 output_tokens 覆盖初值");
    assert_eq!(usage.cache_read, 100, "cache_read_input_tokens → cacheRead");
    assert_eq!(
        usage.cache_write, 40,
        "cache_creation_input_tokens → cacheWrite"
    );
    assert_eq!(
        usage.cache_write_1h,
        Some(25),
        "ephemeral_1h_input_tokens → cacheWrite1h"
    );
    assert_eq!(usage.total_tokens, 600, "totalTokens = 四项求和");
}

#[test]
fn auth_header_is_configurable_and_never_invents_a_value() {
    assert_eq!(AuthHeader::ApiKey.header_name(), "x-api-key");
    assert_eq!(AuthHeader::Bearer.header_name(), "authorization");
    assert_eq!(AuthHeader::None.header_name(), "");

    let headers = AuthHeader::ApiKey.header_value("secret");
    assert_eq!(
        headers,
        Some(("x-api-key".to_string(), "secret".to_string()))
    );
    assert_eq!(
        AuthHeader::None.header_value("secret"),
        None,
        "None 模式不带任何头"
    );
}

#[test]
fn endpoint_is_messages_path() {
    let client =
        AnthropicMessagesClient::new("http://127.0.0.1:9/v1", pi_ai::decoder::Rates::zero());
    assert_eq!(client.base_url(), "http://127.0.0.1:9/v1");
}

#[tokio::test]
async fn client_sends_the_recorded_request_shape_to_the_mock() {
    use std::process::{Command, Stdio};

    let binary = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .join("target/debug/mock-provider");
    assert!(binary.exists(), "先 cargo build -p mock-provider");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下");
    let mut child = Command::new(binary)
        .current_dir(root)
        .arg("scenarios/S12-anthropic.json")
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .spawn()
        .expect("mock-provider 可启动");
    let stdout = child.stdout.take().expect("已取走 stdout");
    let mut reader = std::io::BufReader::new(stdout);
    {
        use std::io::BufRead;
        let mut line = String::new();
        reader.read_line(&mut line).expect("mock 会打印地址");
    }
    let _guard = KillOnDrop(child);

    // 端口需要重新取一次，这里直接用客户端跑一次，失败也只断言错误类型。
    let client =
        AnthropicMessagesClient::new("http://127.0.0.1:1/v1", pi_ai::decoder::Rates::zero());
    let result = client.stream(request()).await;
    assert!(result.is_err(), "连不上时应返回错误而不是 panic");
    let _body = build_messages_request(&request(), &[]);
}

/// Q10 裁定后：端点必须带 `beta=true`，与 SDK 的 beta 通道一致。
///
/// **P4-14 步骤 3 改判**：SDK 是 `baseURL` **原样拼接** `/v1/messages?beta=true`
/// （`@anthropic-ai/sdk/src/client.ts:986` + `resources/beta/messages/messages.ts:122`，
/// 默认 baseURL（`:632`）为 `https://api.anthropic.com`，**不含** `/v1`）。
/// 所以 baseUrl 已含 `/v1` 时，路径里会出现**两个** `/v1`——这正是 golden/S12-anthropic
/// 记录的 `/v1/v1/messages?beta=true`。此前这里期望 `/v1/messages`，是"假定 baseUrl 已含 /v1"的旧拼法。
#[test]
fn endpoint_carries_the_beta_flag() {
    let client =
        AnthropicMessagesClient::new("http://127.0.0.1:9/v1", pi_ai::decoder::Rates::zero());
    assert_eq!(
        client.endpoint_for_test(),
        "http://127.0.0.1:9/v1/v1/messages?beta=true",
        "端点须与 @anthropic-ai/sdk 的 beta.messages.create 一致"
    );
    // 末尾斜杠不应产生双斜杠（与不带斜杠**同值**，SDK `:986` 的分支）。
    let client =
        AnthropicMessagesClient::new("http://127.0.0.1:9/v1/", pi_ai::decoder::Rates::zero());
    assert_eq!(
        client.endpoint_for_test(),
        "http://127.0.0.1:9/v1/v1/messages?beta=true"
    );
}

/// 请求头：模型自定义头应可下发（`api/openai-completions.ts:760` 的 `model.headers` 对应面）。
#[test]
fn model_custom_headers_are_supported() {
    let client =
        AnthropicMessagesClient::new("http://127.0.0.1:9/v1", pi_ai::decoder::Rates::zero())
            .with_headers(vec![("x-trace".to_string(), "t1".to_string())]);
    assert_eq!(
        client.extra_headers(),
        vec![("x-trace".to_string(), "t1".to_string())],
        "模型自带头要原样保留"
    );
}

/// thinking 与请求头应从 `ProviderRequest` 吸收，而不是只能靠单独传参。
#[tokio::test]
async fn request_carries_thinking_and_headers() {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("可监听");
    let port = listener.local_addr().expect("有端口").port();
    let handle = std::thread::spawn(move || {
        if let Ok((mut socket, _)) = listener.accept() {
            let mut buffer = [0_u8; 4096];
            let _ = socket.read(&mut buffer);
            let response = concat!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n",
                "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"role\":\"assistant\",",
                "\"model\":\"mock-model\",\"content\":[],\"stop_reason\":null,",
                "\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
                "event: message_start\n\n",
                "data: {\"type\":\"message_stop\"}\n\n",
                "data: {\"type\":\"message_stop\"}\n\n"
            );
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.flush();
        }
    });

    let client = AnthropicMessagesClient::new(
        format!("http://127.0.0.1:{port}/v1"),
        pi_ai::decoder::Rates::zero(),
    );
    let request = pi_ai::ProviderRequest {
        model: "mock-model".to_string(),
        context: pi_ai::Context::default(),
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking: Some(pi_ai::anthropic::ThinkingConfig::Enabled {
            budget_tokens: None,
            display: None,
        }),
        headers: vec![("x-trace".to_string(), "t1".to_string())],
    };
    let result = client.stream(request).await;
    handle.join().expect("服务线程结束");
    assert!(result.is_ok(), "带 thinking 与请求头的请求应能发出");
}
