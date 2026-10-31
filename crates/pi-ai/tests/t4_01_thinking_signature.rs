//! P4-6②：`reasoning_content` 上的 thinking 块**必须带 `thinkingSignature`**。
//!
//! 差异来源（S12-openai 的 A 层第 6 行 / B 层第 10 行）：golden 的 thinking 块是
//! `{"type":"thinking","thinking":"…","thinkingSignature":"reasoning_content"}`，而我没有 signature。
//! ref：`ai/src/api/openai-completions.ts:605-623`——`reasoningFields = ["reasoning_content","reasoning","reasoning_text"]`
//! 取**第一个非空**字段，并把**该字段名**记为块的 signature（`provider === "opencode-go"` 且字段是
//! `reasoning` 时才记成 `"reasoning_content"`）；字段本身在 `ai/src/types.ts:373`。
use pi_ai::decoder::{Rates, StreamDecoder};

#[test]
fn reasoning_content_sets_thinking_signature() {
    let chunk = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先读文件再回答\"}}]}\n\n";
    // 对照必须真的触发：这条 chunk 里确实带 reasoning_content。
    assert!(chunk.contains("reasoning_content"));

    let mut decoder = StreamDecoder::new("openai-completions", "mock", "mock-model", Rates::zero())
        .expect("decoder 可构造");
    decoder.push(chunk.as_bytes()).expect("push");
    let _events = decoder.finish().expect("finish");
    // signature 落在**最终消息**上（事件流的 `thinking_end` 里没有这个字段，golden 也没有）。
    let message = decoder.final_message();
    let json = serde_json::to_string(&message).expect("最终消息可序列化");

    assert!(
        json.contains("\"thinking\":\"先读文件再回答\""),
        "必须产生 thinking 块：{json}"
    );
    assert!(
        json.contains("\"thinkingSignature\":\"reasoning_content\""),
        "thinking 块必须带 signature（当前缺）：{json}"
    );
}

/// 负向用例（**decode 侧**）：非 opencode-go 的 provider **不走特例**——
/// ref `openai-completions.ts:619-623`：命中 `reasoning` 字段时 signature 记作 `"reasoning"`，
/// 只有 `model.provider === "opencode-go"` 才记成 `"reasoning_content"`（`:620-621`）。
#[test]
fn non_opencode_go_keeps_the_actual_field_name() {
    let chunk = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"先想一下\"}}]}\n\n";
    // 对照必须真的触发：样本里确实用的是 `reasoning` 字段（不是 reasoning_content）。
    assert!(chunk.contains("\"reasoning\"") && !chunk.contains("reasoning_content"));

    let mut decoder = StreamDecoder::new("openai-completions", "mock", "mock-model", Rates::zero())
        .expect("decoder 可构造");
    decoder.push(chunk.as_bytes()).expect("push");
    let _ = decoder.finish().expect("finish");
    let json = serde_json::to_string(&decoder.final_message()).expect("可序列化");
    assert!(
        json.contains("\"thinkingSignature\":\"reasoning\""),
        "非 opencode-go 必须原样记录字段名 reasoning：{json}"
    );
    assert!(
        !json.contains("\"thinkingSignature\":\"reasoning_content\""),
        "非 opencode-go 不得改写成 reasoning_content：{json}"
    );
}
