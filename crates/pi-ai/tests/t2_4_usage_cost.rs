//! T2.4：usage 与 cost 的口径。
//!
//! 判据：`golden/S21/session.jsonl` 里 TS 实际写下的 usage 与 cost，
//! 以及 `calculateContextTokens` 的口径（`spec/d-compaction.md`）。

use pi_ai::decoder::{calculate_context_tokens, calculate_cost, Rates};
use pi_ai::{Cost, Usage};

fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64, total: u64) -> Usage {
    Usage {
        input,
        output,
        cache_read,
        cache_write,
        cache_write_1h: None,
        reasoning: None,
        total_tokens: total,
        cost: Cost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
            total: 0.0,
        },
    }
}

#[test]
fn context_tokens_prefers_total_then_sums_four_fields() {
    assert_eq!(calculate_context_tokens(&usage(400, 60, 0, 0, 460)), 460);
    assert_eq!(
        calculate_context_tokens(&usage(400, 60, 30, 10, 0)),
        500,
        "totalTokens 为 0 时退回 input + output + cacheRead + cacheWrite"
    );
}

/// 把费用的五个 f64 转成位模式，用于逐位比较（Q1：末位浮点不保证正确舍入）。
fn bits(cost: &Cost) -> [u64; 5] {
    [
        cost.input.to_bits(),
        cost.output.to_bits(),
        cost.cache_read.to_bits(),
        cost.cache_write.to_bits(),
        cost.total.to_bits(),
    ]
}

#[test]
fn cost_matches_the_recorded_session_bit_for_bit() {
    // 费率取自 mock 模型配置：input 1.0、output 2.0（每百万 token）。
    let rates = Rates {
        input: 1.0,
        output: 2.0,
        cache_read: 0.0,
        cache_write: 0.0,
    };
    let cost = calculate_cost(rates, &usage(400, 60, 0, 0, 460));
    let recorded = Cost {
        input: 0.00039999999999999996,
        output: 0.00011999999999999999,
        cache_read: 0.0,
        cache_write: 0.0,
        total: 0.00052,
    };
    assert_eq!(
        bits(&cost),
        bits(&recorded),
        "费用必须逐位一致：{:?} vs {:?}",
        bits(&cost),
        bits(&recorded)
    );
}

#[test]
fn cache_write_long_retention_is_charged_at_double_input_rate() {
    let rates = Rates {
        input: 1.0,
        output: 2.0,
        cache_read: 0.5,
        cache_write: 1.25,
    };
    let mut u = usage(0, 0, 0, 300, 0);
    u.cache_write_1h = Some(100);
    let cost = calculate_cost(rates, &u);
    // 短保留 200 × 1.25 + 长保留 100 × (1.0 × 2) = 450，除以 1e6。
    assert_eq!(
        cost.cache_write.to_bits(),
        (450.0_f64 / 1_000_000.0).to_bits()
    );
}

#[test]
fn total_is_summed_in_the_documented_order() {
    let rates = Rates {
        input: 1.0,
        output: 2.0,
        cache_read: 0.5,
        cache_write: 1.25,
    };
    let cost = calculate_cost(rates, &usage(1_000_000, 1_000_000, 1_000_000, 1_000_000, 0));
    let expected_total = cost.input + cost.output + cost.cache_read + cost.cache_write;
    assert_eq!(cost.total.to_bits(), expected_total.to_bits());
}

/// 解码器最终写下的 usage 应当与 TS 会话文件里的形状一致。
#[test]
fn decoder_usage_falls_back_to_input_plus_output_without_total() {
    use pi_ai::decoder::{Rates, StreamDecoder, OPENAI_COMPLETIONS};

    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法");
    // 一个只带 prompt/completion token、不带 total_tokens 的 usage 块。
    let chunk = concat!(
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",",
        "\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"mock-model\",",
        "\"choices\":[],\"usage\":{\"prompt_tokens\":400,\"completion_tokens\":60}}\n\n",
        "data: [DONE]\n\n"
    );
    for piece in chunk.as_bytes().chunks(7) {
        decoder.push(piece).expect("分片喂入不应报错");
    }
    let message = decoder.final_message();
    assert_eq!(message.usage.input, 400);
    assert_eq!(message.usage.output, 60);
    assert_eq!(
        calculate_context_tokens(&message.usage),
        460,
        "没有 total_tokens 时按四项求和"
    );
}

/// TS 的 openai-completions **不使用** provider 给的 `total_tokens`，也把 `prompt_tokens`
/// 扣掉缓存部分才是 input（`ref/pi-0.87.1/packages/ai/src/api/openai-completions.ts:1539-1547`）。
#[test]
fn decoder_subtracts_cache_from_input_and_ignores_provider_total() {
    use pi_ai::decoder::{Rates, StreamDecoder, OPENAI_COMPLETIONS};

    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法");
    let chunk = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hi\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":500,\"completion_tokens\":60,",
        "\"total_tokens\":999,\"prompt_tokens_details\":{\"cached_tokens\":100,\"cache_write_tokens\":40}}}\n\n",
        "data: [DONE]\n\n"
    );
    for piece in chunk.as_bytes().chunks(9) {
        decoder.push(piece).expect("分片喂入不应报错");
    }
    let usage = decoder.final_message().usage;
    assert_eq!(usage.cache_read, 100, "cached_tokens 记为 cacheRead");
    assert_eq!(usage.cache_write, 40, "cache_write_tokens 记为 cacheWrite");
    assert_eq!(
        usage.input, 360,
        "input = prompt_tokens - cacheRead - cacheWrite"
    );
    assert_eq!(usage.output, 60);
    assert_eq!(
        usage.total_tokens, 560,
        "totalTokens 自行求和，不采信 provider 的 999"
    );
}

#[test]
fn decoder_clamps_input_at_zero_when_cache_exceeds_prompt() {
    use pi_ai::decoder::{Rates, StreamDecoder, OPENAI_COMPLETIONS};

    let mut decoder = StreamDecoder::new(OPENAI_COMPLETIONS, "mock", "mock-model", Rates::zero())
        .expect("api 名合法");
    let chunk = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hi\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":1,",
        "\"prompt_tokens_details\":{\"cached_tokens\":80}}}\n\n",
        "data: [DONE]\n\n"
    );
    for piece in chunk.as_bytes().chunks(11) {
        decoder.push(piece).expect("分片喂入不应报错");
    }
    let usage = decoder.final_message().usage;
    assert_eq!(
        usage.input, 0,
        "缓存多于 prompt 时 input 夹到 0，不出现负数"
    );
    assert_eq!(usage.total_tokens, 81);
}
