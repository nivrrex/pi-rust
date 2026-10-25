//! P7-5 / W0(b)3：`responseId` 的捕获与序列化。
//!
//! 判据来自 ref：`packages/ai/src/api/openai-completions.ts:558` `output.responseId ||= chunk.id;`
//! 与 `packages/ai/src/types.ts:522`（可选字段 `responseId`）。
//! 正向：含顶层 `id` 的 chunk ⇒ `final_message().response_id` 有值，且序列化里出现 `responseId`。
//! 负向对照：没有 `id` 的流 ⇒ `response_id` 为 `None`，且序列化**不出现**该键（既有会话不受影响）。

use pi_ai::decoder::{Rates, StreamDecoder, OPENAI_COMPLETIONS};

fn decode(bytes: &[u8]) -> pi_ai::AssistantMessage {
    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法");
    let _ = decoder.push(bytes).expect("解码不应报错");
    let _ = decoder.finish().expect("收尾不应报错");
    decoder.final_message()
}

fn sse(id_line: &str) -> Vec<u8> {
    let body = format!(
        "data: {{\"id\":\"{id_line}\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"hi\"}},\"finish_reason\":null}}]}}\n\n\
         data: {{\"id\":\"{id_line}\",\"object\":\"chat.completion.chunk\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
         data: [DONE]\n\n"
    );
    body.into_bytes()
}

#[test]
fn response_id_is_captured_from_the_first_chunk_id() {
    let message = decode(&sse("chatcmpl-test"));
    assert_eq!(
        message.response_id.as_deref(),
        Some("chatcmpl-test"),
        "首个 chunk 的 id 必须写进 responseId（ref openai-completions.ts:558）"
    );
    let json = serde_json::to_string(&message).expect("可序列化");
    assert!(
        json.contains("\"responseId\":\"chatcmpl-test\""),
        "序列化里应出现 responseId"
    );
}

#[test]
fn response_id_is_absent_when_the_stream_has_no_id() {
    // 负向对照：把 id 置空（等价于"流里没有可用的响应标识"）。
    let message = decode(&sse(""));
    assert!(
        message.response_id.is_none(),
        "没有 id 时 responseId 必须是 None"
    );
    let json = serde_json::to_string(&message).expect("可序列化");
    assert!(
        !json.contains("responseId"),
        "None 时不得序列化出该键（既有会话/判据不受影响）"
    );
}
