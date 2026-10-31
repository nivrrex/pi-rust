//! G0 验证测试：事件序与最终消息必须与 `fixtures/expected/*.expected.json` 一致。
//!
//! 期望结果由 `tools/oracle/run.sh` 用 npm 包里的 TS 版 pi-ai 生成，本文件不含手写期望值。

use std::path::{Path, PathBuf};

use pi_ai::{
    calculate_cost, AssistantEvent, AssistantMessage, Content, Rates, StopReason, StreamDecoder,
    Usage,
};
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|e| panic!("定位仓库根目录失败: {e}"))
}

fn read_chunks(name: &str) -> Vec<Vec<u8>> {
    let path = repo_root().join("fixtures/sse").join(name);
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("读取夹具失败 {}: {e}", path.display()));
    let meta_path = path.with_extension("sse.chunks.json");
    let Ok(meta) = std::fs::read_to_string(&meta_path) else {
        return vec![bytes];
    };
    let meta: Value =
        serde_json::from_str(&meta).unwrap_or_else(|e| panic!("解析 {meta_path:?} 失败: {e}"));
    let sizes: Vec<usize> = meta["chunks"]
        .as_array()
        .unwrap_or_else(|| panic!("{meta_path:?} 缺少 chunks 数组"))
        .iter()
        .map(|v| v.as_u64().unwrap_or(0) as usize)
        .collect();
    let mut parts = Vec::new();
    let mut offset = 0;
    for size in sizes {
        parts.push(bytes[offset..offset + size].to_vec());
        offset += size;
    }
    parts.push(bytes[offset..].to_vec());
    parts
}

fn rates() -> Rates {
    Rates {
        input: 1.0,
        output: 2.0,
        cache_read: 0.0,
        cache_write: 0.0,
    }
}

fn content_json(content: &Content) -> Value {
    match content {
        Content::Text { text, .. } => json!({"type": "text", "text": text}),
        Content::Thinking { thinking, .. } => json!({"type": "thinking", "thinking": thinking}),
        Content::Image {
            data, mime_type, ..
        } => json!({"type": "image", "data": data, "mimeType": mime_type}),
        Content::ToolCall {
            id,
            name,
            arguments,
            ..
        } => {
            json!({"type": "toolCall", "id": id, "name": name, "arguments": Value::Object(arguments.clone())})
        }
    }
}

fn message_json(message: &AssistantMessage) -> Value {
    let mut map = Map::new();
    map.insert("role".into(), json!(message.role));
    map.insert("api".into(), json!(message.api));
    map.insert("provider".into(), json!(message.provider));
    map.insert("model".into(), json!(message.model));
    map.insert("stopReason".into(), json!(message.stop_reason));
    if let Some(raw) = &message.raw_stop_reason {
        map.insert("rawStopReason".into(), json!(raw));
    }
    if let Some(err) = &message.error_message {
        map.insert("errorMessage".into(), json!(err));
    }
    if let Some(end_turn) = message.end_turn {
        map.insert("endTurn".into(), json!(end_turn));
    }
    map.insert(
        "content".into(),
        Value::Array(message.content.iter().map(content_json).collect()),
    );
    let mut usage = Map::new();
    usage.insert("input".into(), json!(message.usage.input));
    usage.insert("output".into(), json!(message.usage.output));
    usage.insert("cacheRead".into(), json!(message.usage.cache_read));
    usage.insert("cacheWrite".into(), json!(message.usage.cache_write));
    usage.insert("totalTokens".into(), json!(message.usage.total_tokens));
    if let Some(reasoning) = message.usage.reasoning {
        usage.insert("reasoning".into(), json!(reasoning));
    }
    usage.insert("cost".into(), json!("<COST_BITS>"));
    let _unused_cost_fields = json!({
            "input": message.usage.cost.input,
            "output": message.usage.cost.output,
            "cacheRead": message.usage.cost.cache_read,
            "cacheWrite": message.usage.cost.cache_write,
            "total": message.usage.cost.total,
    });
    map.insert("usage".into(), Value::Object(usage));
    Value::Object(map)
}

fn event_json(event: &AssistantEvent, decoder: &StreamDecoder) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), json!(event.type_name()));
    match event {
        AssistantEvent::Start => {}
        AssistantEvent::TextStart { content_index }
        | AssistantEvent::ThinkingStart { content_index }
        | AssistantEvent::ToolCallStart { content_index } => {
            map.insert("contentIndex".into(), json!(content_index));
        }
        AssistantEvent::TextEnd {
            content_index,
            content,
        }
        | AssistantEvent::ThinkingEnd {
            content_index,
            content,
        } => {
            map.insert("contentIndex".into(), json!(content_index));
            map.insert("content".into(), json!(content));
        }
        AssistantEvent::TextDelta {
            content_index,
            delta,
        }
        | AssistantEvent::ThinkingDelta {
            content_index,
            delta,
        }
        | AssistantEvent::ToolCallDelta {
            content_index,
            delta,
        } => {
            map.insert("contentIndex".into(), json!(content_index));
            map.insert("delta".into(), json!(delta));
        }
        AssistantEvent::ToolCallEnd { content_index } => {
            map.insert("contentIndex".into(), json!(content_index));
            let (id, name, arguments) = decoder
                .tool_call_at(*content_index)
                .unwrap_or_else(|| panic!("toolcall_end 指向不存在的块 {content_index}"));
            map.insert(
                "toolCall".into(),
                json!({"type": "toolCall", "id": id, "name": name, "arguments": Value::Object(arguments)}),
            );
        }
        AssistantEvent::Done { reason } => {
            map.insert("reason".into(), json!(reason));
            map.insert("message".into(), message_json(&decoder.final_message()));
        }
        AssistantEvent::Error { reason, .. } => {
            map.insert("reason".into(), json!(reason));
            map.insert("error".into(), message_json(&decoder.final_message()));
        }
    }
    Value::Object(map)
}

/// 用夹具驱动解码器，返回归一化后的事件序列（含末尾的 `__result`）。
fn decode_fixture(api: &str, fixture: &str) -> (Vec<Value>, AssistantMessage) {
    let mut decoder = StreamDecoder::new(api, "mock", "mock-model", rates())
        .unwrap_or_else(|e| panic!("创建解码器失败: {e}"));
    let mut events = Vec::new();
    for chunk in read_chunks(fixture) {
        for event in decoder
            .push(&chunk)
            .unwrap_or_else(|e| panic!("解码失败: {e}"))
        {
            events.push(event_json(&event, &decoder));
        }
    }
    for event in decoder.finish().unwrap_or_else(|e| panic!("收尾失败: {e}")) {
        events.push(event_json(&event, &decoder));
    }
    let final_message = decoder.final_message();
    events.push(json!({"type": "__result", "message": message_json(&final_message)}));
    (events, final_message)
}

fn expected(case: &str) -> Value {
    let path = repo_root()
        .join("fixtures/expected")
        .join(format!("{case}.expected.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取期望结果失败 {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("解析期望结果失败 {}: {e}", path.display()))
}

fn assert_case(case: &str, api: &str, fixture: &str) {
    let want = expected(case);
    let (got_list, final_message) = decode_fixture(api, fixture);
    let got_events = Value::Array(got_list);
    let want_events = want["events"].clone();
    // cost 精确值走 costBits：serde_json 的十进制→f64 与正确舍入结果差 1 ULP。
    if let Some(bits) = want.get("costBits") {
        for key in ["input", "output", "cacheRead", "cacheWrite", "total"] {
            let want_bits = u64::from_str_radix(
                bits[key]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches("0x"),
                16,
            )
            .unwrap_or_else(|e| panic!("用例 {case} 的 costBits.{key} 不是合法十六进制: {e}"));
            let got_bits = match key {
                "input" => final_message.usage.cost.input,
                "output" => final_message.usage.cost.output,
                "cacheRead" => final_message.usage.cost.cache_read,
                "cacheWrite" => final_message.usage.cost.cache_write,
                _ => final_message.usage.cost.total,
            }
            .to_bits();
            assert_eq!(
                got_bits, want_bits,
                "用例 {case} 的 cost.{key} 位模式不一致"
            );
        }
    }
    if want_events != got_events {
        let empty = vec![];
        let wa = want_events.as_array().unwrap_or(&empty);
        let ga = got_events.as_array().unwrap_or(&empty);
        for (index, (w, g)) in wa.iter().zip(ga.iter()).enumerate() {
            assert_eq!(
                w, g,
                "用例 {case} 第 {index} 个事件不匹配\n期望: {w}\n实际: {g}"
            );
        }
        panic!(
            "用例 {case} 事件个数不匹配：期望 {} 个，实际 {} 个\n期望尾部: {}\n实际尾部: {}",
            wa.len(),
            ga.len(),
            wa.last().unwrap_or(&Value::Null),
            ga.last().unwrap_or(&Value::Null)
        );
    }
}

#[test]
fn openai_text_basic() {
    assert_case(
        "openai-text-basic",
        "openai-completions",
        "openai/text-basic.sse",
    );
}

#[test]
fn openai_reasoning_block() {
    assert_case(
        "openai-reasoning",
        "openai-completions",
        "openai/reasoning.sse",
    );
}

#[test]
fn openai_parallel_tool_calls() {
    assert_case(
        "openai-parallel-calls",
        "openai-completions",
        "openai/parallel-toolcalls.sse",
    );
}

#[test]
fn openai_crlf_line_endings() {
    assert_case("openai-crlf", "openai-completions", "openai/crlf.sse");
}

#[test]
fn openai_multiline_data() {
    assert_case(
        "openai-multiline-data",
        "openai-completions",
        "openai/multiline-data.sse",
    );
}

#[test]
fn openai_multibyte_split_across_chunks() {
    assert_case(
        "openai-multibyte-split",
        "openai-completions",
        "openai/multibyte-split.sse",
    );
}

#[test]
fn openai_error_chunk() {
    assert_case(
        "openai-error-chunk",
        "openai-completions",
        "openai/error-chunk.sse",
    );
}

#[test]
fn anthropic_basic() {
    assert_case(
        "anthropic-basic",
        "anthropic-messages",
        "anthropic/basic.sse",
    );
}

#[test]
fn anthropic_tool_use_delta() {
    assert_case(
        "anthropic-tool-use-delta",
        "anthropic-messages",
        "anthropic/tool-use-delta.sse",
    );
}

#[test]
fn anthropic_ping_only() {
    assert_case(
        "anthropic-ping-only",
        "anthropic-messages",
        "anthropic/ping-only.sse",
    );
}

// ---------------- V3：取消 ----------------

/// 按第 n 个事件边界（连续两个换行）把夹具切成两半。
fn split_after_events(bytes: &[u8], n: usize) -> (Vec<u8>, Vec<u8>) {
    let mut offset = 0;
    for _ in 0..n {
        let idx = bytes[offset..]
            .windows(2)
            .position(|w| w == b"\n\n")
            .unwrap_or_else(|| panic!("夹具里找不到第 {n} 个事件边界"));
        offset += idx + 2;
    }
    (bytes[..offset].to_vec(), bytes[offset..].to_vec())
}

fn openai_text_bytes() -> Vec<u8> {
    std::fs::read(repo_root().join("fixtures/sse/openai/text-basic.sse"))
        .unwrap_or_else(|e| panic!("读取夹具失败: {e}"))
}

#[test]
fn cancel_between_events_stops_production() {
    let bytes = openai_text_bytes();
    // 前 4 个事件：role、Hello、" world"、finish_reason+usage。
    let (head, tail) = split_after_events(&bytes, 4);
    let token = CancellationToken::new();
    let mut decoder = StreamDecoder::new("openai-completions", "mock", "mock-model", rates())
        .unwrap_or_else(|e| panic!("创建解码器失败: {e}"))
        .with_cancel(token.clone());

    let produced: Vec<AssistantEvent> = decoder
        .push(&head)
        .unwrap_or_else(|e| panic!("解码失败: {e}"));
    assert_eq!(
        produced.len(),
        6,
        "前 4 个 SSE 事件（role、两个文本增量、finish_reason）应产出 6 个助手事件，实际 {produced:?}"
    );
    assert!(!decoder.is_cancelled());

    token.cancel();
    assert!(decoder.is_cancelled());
    // T2.5 起：取消会显式报错，而不是静默产出空事件——上层需要区分
    // "被取消" 与 "provider 正常地没再发事件"。
    let after = decoder.push(&tail);
    assert!(after.is_err(), "取消后 push 应报错");

    let end = decoder.finish().unwrap_or_else(|e| panic!("收尾失败: {e}"));
    assert!(end.is_empty(), "取消后 finish 也不得补发事件，实际 {end:?}");
    // finish_reason 在取消前已经到达，停止原因保持为 stop。
    assert_eq!(decoder.final_message().stop_reason, StopReason::Stop);
}

#[test]
fn cancel_mid_text_stream_keeps_partial_content() {
    let bytes = openai_text_bytes();
    // 前 2 个事件：role 与第一个文本增量。
    let (head, tail) = split_after_events(&bytes, 2);
    let token = CancellationToken::new();
    let mut decoder = StreamDecoder::new("openai-completions", "mock", "mock-model", rates())
        .unwrap_or_else(|e| panic!("创建解码器失败: {e}"))
        .with_cancel(token.clone());

    let _ = decoder
        .push(&head)
        .unwrap_or_else(|e| panic!("解码失败: {e}"));
    token.cancel();
    assert!(decoder.push(&tail).is_err(), "取消后 push 应报错");

    let message = decoder.final_message();
    assert_eq!(
        message.stop_reason,
        StopReason::Aborted,
        "被取消的流停止原因是 aborted"
    );
    let Some(Content::Text { text, .. }) = message.content.first() else {
        panic!("期望首个内容块是文本，实际 {:?}", message.content);
    };
    assert_eq!(text, "Hello", "取消时已收到的文本必须保留");
}

#[test]
fn cancel_before_any_byte_yields_nothing() {
    let bytes = openai_text_bytes();
    let token = CancellationToken::new();
    token.cancel();
    let mut decoder = StreamDecoder::new("openai-completions", "mock", "mock-model", rates())
        .unwrap_or_else(|e| panic!("创建解码器失败: {e}"))
        .with_cancel(token);
    assert!(decoder.push(&bytes).is_err(), "已取消的解码器应直接报错");
    assert_eq!(
        decoder.final_message().stop_reason,
        StopReason::Aborted,
        "被取消的流停止原因是 aborted，不是 pending"
    );
}

#[test]
fn unsupported_api_is_rejected() {
    let result = StreamDecoder::new("google-generative-ai", "mock", "mock-model", rates());
    assert!(result.is_err());
}

/// R2：`calculateCost` 的运算顺序逐字复刻，用 pi-ai 生成的 oracle 做**位模式**比较。
///
/// 期望值来自 `tools/oracle/cost-oracle.mjs`（跑的是 npm 包里的 TS `calculateCost`）。
#[test]
fn cost_matches_typescript_bitwise() {
    let path = repo_root().join("fixtures/expected/cost-oracle.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取 cost oracle 失败 {}: {e}", path.display()));
    let cases: Vec<Value> =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("解析 cost oracle 失败: {e}"));
    assert!(
        cases.len() >= 20,
        "cost oracle 至少要 20 组，实际 {} 组",
        cases.len()
    );

    for case in &cases {
        let label = case["label"].as_str().unwrap_or("?");
        let rate = case["cost"]
            .get("cacheRead")
            .and_then(Value::as_f64)
            .map(|cache_read| Rates {
                input: case["cost"]["input"].as_f64().unwrap_or_default(),
                output: case["cost"]["output"].as_f64().unwrap_or_default(),
                cache_read,
                cache_write: case["cost"]["cacheWrite"].as_f64().unwrap_or_default(),
            });
        let Some(rates) = rate else { continue };

        // 按 TS 的选档规则挑费率：inputTokens = input + cacheRead + cacheWrite。
        let input_tokens = case["usage"]["input"].as_f64().unwrap_or_default()
            + case["usage"]["cacheRead"].as_f64().unwrap_or_default()
            + case["usage"]["cacheWrite"].as_f64().unwrap_or_default();
        let mut effective = rates;
        let mut matched = -1.0f64;
        if let Some(tiers) = case["cost"].get("tiers").and_then(Value::as_array) {
            for tier in tiers {
                let threshold = tier["inputTokensAbove"].as_f64().unwrap_or_default();
                if input_tokens > threshold && threshold > matched {
                    effective = Rates {
                        input: tier["input"].as_f64().unwrap_or_default(),
                        output: tier["output"].as_f64().unwrap_or_default(),
                        cache_read: tier["cacheRead"].as_f64().unwrap_or_default(),
                        cache_write: tier["cacheWrite"].as_f64().unwrap_or_default(),
                    };
                    matched = threshold;
                }
            }
        }

        let mut usage = Usage {
            input: case["usage"]["input"].as_u64().unwrap_or_default(),
            output: case["usage"]["output"].as_u64().unwrap_or_default(),
            cache_read: case["usage"]["cacheRead"].as_u64().unwrap_or_default(),
            cache_write: case["usage"]["cacheWrite"].as_u64().unwrap_or_default(),
            cache_write_1h: None,
            reasoning: None,
            total_tokens: 0,
            cost: pi_ai::Cost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
                total: 0.0,
            },
        };
        usage.cache_write_1h = case["usage"]["cacheWrite1h"].as_u64();
        let cost = calculate_cost(effective, &usage);

        for key in ["input", "output", "cacheRead", "cacheWrite", "total"] {
            let want = f64::from_bits(
                u64::from_str_radix(
                    case["bits"][key]
                        .as_str()
                        .unwrap_or_default()
                        .trim_start_matches("0x"),
                    16,
                )
                .unwrap_or_else(|e| panic!("用例 {label} 的 bits.{key} 非法: {e}")),
            );
            let got = match key {
                "input" => cost.input,
                "output" => cost.output,
                "cacheRead" => cost.cache_read,
                "cacheWrite" => cost.cache_write,
                _ => cost.total,
            };
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "用例 {label} 的 cost.{key} 位模式不一致：TS={want:?} Rust={got:?}"
            );
        }
    }
}

/// 回归测试：serde_json 的十进制→f64 解析必须与 `str::parse` 的正确舍入结果位模式一致。
///
/// 起因见 `notes/decisions.md`：serde_json 默认的快速路径在长尾小数上不保证正确舍入，
/// 例如 `0.0000049999999999999996` 会被解析成相邻的一个 ULP。生产路径同样受影响
/// （models.json 的费率就是 JSON 解析来的），所以修在库依赖的 `float_roundtrip` 特性上，
/// 不是在测试里绕开。
#[test]
fn serde_json_float_parsing_is_correctly_rounded() {
    for literal in [
        "0.0000049999999999999996",
        "0.000039999999999999996",
        "1.3999999999999998e-05",
        "9.999999999999999e-06",
        "4.4999999999999996e-05",
        "0.1",
        "1e-7",
    ] {
        let want: f64 = literal
            .parse()
            .unwrap_or_else(|e| panic!("std 无法解析 {literal}: {e}"));
        let got: f64 = serde_json::from_str(literal)
            .unwrap_or_else(|e| panic!("serde_json 无法解析 {literal}: {e}"));
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "字面量 {literal} 的解析结果不一致：serde_json={got:?} std={want:?}"
        );
    }
}
