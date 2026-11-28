//! P4-14 步骤 4：anthropic 的 `usage.cacheWrite1h` 必须**总是**出现。
//!
//! ref `packages/ai/src/api/anthropic-messages.ts:615-625`（`message_start` 分支）：
//! ```
//! :617 output.usage.input       = event.message.usage.input_tokens || 0;
//! :618 output.usage.output      = event.message.usage.output_tokens || 0;
//! :619 output.usage.cacheRead   = event.message.usage.cache_read_input_tokens || 0;
//! :620 output.usage.cacheWrite  = event.message.usage.cache_creation_input_tokens || 0;
//! :621 output.usage.cacheWrite1h = event.message.usage.cache_creation?.ephemeral_1h_input_tokens || 0;
//! :623-624 output.usage.totalTokens = input + output + cacheRead + cacheWrite;
//! ```
//! 关键是 `:621` 的 `|| 0`：**即使服务端没有这个字段，也要写成 0**（`types.ts:402` 的
//! `cacheWrite1h?: number` 于是总是被赋值）。pi-rs 此前是
//! `if let Some(x) = … { self.cache_write_1h = Some(x) }` → 缺失时留 `None`，
//! 而 `Usage.cache_write_1h` 带 `skip_serializing_if = "Option::is_none"`（`types.rs:35`）
//! → **字段整个消失**，与 golden/S12-anthropic 的 `"cacheWrite1h":0` 不符。
use pi_ai::decoder::{Rates, StreamDecoder};

fn sse(lines: &[&str]) -> Vec<u8> {
    let mut text = String::new();
    for line in lines {
        text.push_str("data: ");
        text.push_str(line);
        text.push('\n');
        text.push('\n');
    }
    text.into_bytes()
}

fn decode_usage(lines: &[&str]) -> serde_json::Value {
    let mut decoder = StreamDecoder::new("anthropic-messages", "mock", "mock-model", Rates::zero())
        .expect("decoder 可构造");
    let _events = {
        let mut all = Vec::new();
        all.extend(decoder.push(&sse(lines)).expect("push"));
        all.extend(decoder.finish().expect("finish"));
        all
    };
    let message = serde_json::to_value(decoder.final_message()).expect("消息可序列化");
    message.get("usage").cloned().expect("有 usage")
}

const MESSAGE_START_PLAIN: &str = r#"{"type":"message_start","message":{"id":"m1","role":"assistant","model":"mock-model","content":[],"usage":{"input_tokens":10,"output_tokens":0}}}"#;
const MESSAGE_START_1H: &str = r#"{"type":"message_start","message":{"id":"m1","role":"assistant","model":"mock-model","content":[],"usage":{"input_tokens":10,"output_tokens":0,"cache_creation":{"ephemeral_1h_input_tokens":1024}}}}"#;
const MESSAGE_STOP: &str = r#"{"type":"message_stop"}"#;

#[test]
fn missing_cache_creation_still_reports_zero() {
    // 样本自检：这一行里**确实没有** `cache_creation`。
    assert!(
        !MESSAGE_START_PLAIN.contains("cache_creation"),
        "{MESSAGE_START_PLAIN}"
    );
    let usage = decode_usage(&[MESSAGE_START_PLAIN, MESSAGE_STOP]);
    assert_eq!(
        usage.get("cacheWrite1h"),
        Some(&serde_json::json!(0)),
        "ref `:621` 的 `|| 0`：缺字段也必须写 0，实际 usage：{usage}"
    );
}

#[test]
fn present_cache_creation_is_used_verbatim() {
    let usage = decode_usage(&[MESSAGE_START_1H, MESSAGE_STOP]);
    assert_eq!(
        usage.get("cacheWrite1h"),
        Some(&serde_json::json!(1024)),
        "{usage}"
    );
}

#[test]
fn matches_the_recorded_golden_usage_keys() {
    // 与 golden 的事实钉死：`message_end`（终态消息）的 usage 里**有** `cacheWrite1h`（值 0）。
    //
    // 注意 golden 的 `message_start` 事件里**没有**这个键——那不是同一个来源：
    // `message_start` 带的是**起步时的快照**（usage 还是默认值，`|| 0` 尚未执行），
    // 而 `:621` 赋的值出现在终态消息上（`message_end` / `turn_end`）；
    // 实测 golden 的行 8（message_start）键里没有它，行 15（message_end）里有。
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .join("golden/S12-anthropic/events.jsonl");
    let text = std::fs::read_to_string(&path).expect("golden 事件存在");
    let usages: Vec<(String, serde_json::Value)> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|event| {
            let kind = event.get("type")?.as_str()?.to_string();
            let usage = event.get("message")?.get("usage")?.clone();
            Some((kind, usage))
        })
        .collect();
    let start = usages
        .iter()
        .find(|(kind, _not_usage)| kind == "message_start")
        .expect("golden 里有 message_start");
    let end = usages
        .iter()
        .find(|(kind, _not_usage)| kind == "message_end")
        .expect("golden 里有 message_end");
    assert!(
        start.1.get("cacheWrite1h").is_none(),
        "样本自检：golden 的 message_start 不带该键：{}",
        start.1
    );
    assert_eq!(
        end.1.get("cacheWrite1h"),
        Some(&serde_json::json!(0)),
        "golden 的 message_end usage：{}",
        end.1
    );
}
