//! P7-6y/W3：OSC 11 配额状态对照 `tools/difftest/tui-oracle/fixtures-osc11.lines.txt`
//! （侧车由 `gen-osc11-lines.mjs` 生成，生成时已断言与 `fixtures-osc11.json` 18/18 一致）。
#![forbid(unsafe_code)]

use pi_tui::osc11::{Osc11Outcome, Osc11State, Rgb};

fn b64_decode(text: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in text.bytes() {
        if ch == b'=' {
            break;
        }
        let Some(index) = TABLE.iter().position(|&c| c == ch) else {
            continue;
        };
        acc = (acc << 6) | index as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    out
}

#[derive(Debug, Default)]
struct Case {
    id: String,
    queries: u32,
    chunks: Vec<String>,
    /// 每次查询的解析结果（`None` = 超时/未解析）
    parsed: Vec<Option<Rgb>>,
    events: Vec<String>,
}

fn load() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-osc11.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-osc11.lines.txt");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split(' ');
        match parts.next().unwrap_or("") {
            "case" => {
                let id = parts.next().unwrap_or("").to_string();
                let queries = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                cases.push(Case {
                    id,
                    queries,
                    ..Case::default()
                });
            }
            "chunk" => {
                if let Some(case) = cases.last_mut() {
                    case.chunks.push(
                        String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                            .to_string(),
                    );
                }
            }
            "parsed" => {
                if let Some(case) = cases.last_mut() {
                    let present = parts.next() == Some("1");
                    if present {
                        let r = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        let g = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        let b = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        case.parsed.push(Some(Rgb { r, g, b }));
                    } else {
                        case.parsed.push(None);
                    }
                }
            }
            "event" => {
                if let Some(case) = cases.last_mut() {
                    case.events.push(
                        String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                            .to_string(),
                    );
                }
            }
            _ => {}
        }
    }
    cases
}

/// 驱动：先按用例发 `queries` 次查询（配额累加），再逐片喂入；
/// 被消费的应答计入 `consumed`，未消费的原样落到 `events`（= 会漏进组件层）。
fn drive(case: &Case) -> (Vec<Option<Rgb>>, Vec<String>) {
    let mut state = Osc11State::new();
    for _ in 0..case.queries {
        let _ = state.query_background();
    }
    let mut consumed: Vec<Option<Rgb>> = Vec::new();
    let mut events = Vec::new();
    for chunk in &case.chunks {
        match state.on_sequence(chunk) {
            Osc11Outcome::Consumed(rgb) => consumed.push(rgb),
            Osc11Outcome::NotConsumed => events.push(chunk.clone()),
        }
    }
    (consumed, events)
}

#[test]
fn osc11_fixtures_all_pass() {
    let cases = load();
    assert_eq!(cases.len(), 18, "夹具应为 18 例（6 类 × 3）");
    let mut failures = Vec::new();
    for case in &cases {
        let (consumed, events) = drive(case);
        if events != case.events {
            failures.push(format!(
                "{}: 未消费事件不同\n  期望 {:?}\n  实际 {:?}",
                case.id, case.events, events
            ));
            continue;
        }
        // 解析值比对：只有"非超时"的用例才有可比值（超时用例的应答可能仍被消费但不 resolve）
        // 解析结果只在"应答确实到达并被消费"时才有可比性；以下两类跳过（时序语义，非实现差异）：
        // 1) 超时的用例（b1/b2/b3）：Promise 已 resolve undefined；
        // 2) 应答未被消费而漏到组件层的用例（a4/a5/a6 分片、d3 非 OSC 11）：查询同样超时。
        // 只要有事件漏到组件层，说明该应答没被消费，对应查询必然超时（Promise 得 undefined）；
        // 这种用例只比事件序列，不比解析结果。
        let timeout_cases = [
            "b1-timeout",
            "b2-timeout-then-answer",
            "b3-timeout-then-key",
        ];
        if timeout_cases.contains(&case.id.as_str()) || !events.is_empty() {
            continue;
        }
        let expected_rgb = case.parsed.clone();
        if expected_rgb != consumed {
            failures.push(format!(
                "{}: 解析结果不同\n  期望 {:?}\n  实际 {:?}",
                case.id, expected_rgb, consumed
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "OSC 11 夹具未全过：{}/{} 失败\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
    println!("OSC 11 夹具全过：{} 例", cases.len());
}

#[test]
fn osc11_state_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Osc11State>();
}

/// P7-6y/W3.3：夹具里必须存在三类实测事实，且行为符合 ref。
#[test]
fn osc11_measured_facts() {
    // ① 分片应答不被消费（依赖上游 StdinBuffer 拼装）
    let mut state = Osc11State::new();
    let _ = state.query_background();
    assert_eq!(
        state.on_sequence("\u{1b}]11;rgb:1111/"),
        Osc11Outcome::NotConsumed
    );
    assert_eq!(
        state.on_sequence("2222/3333\u{7}"),
        Osc11Outcome::NotConsumed
    );
    assert_eq!(state.pending(), 1, "分片不应消耗配额");
    // ② 配额耗尽后多余应答原样透传
    let mut state = Osc11State::new();
    let _ = state.query_background();
    assert_eq!(
        state.on_sequence("\u{1b}]11;rgb:1111/1111/1111\u{7}"),
        Osc11Outcome::Consumed(Some(Rgb {
            r: 17,
            g: 17,
            b: 17
        }))
    );
    assert_eq!(state.pending(), 0);
    assert_eq!(
        state.on_sequence("\u{1b}]11;rgb:2222/2222/2222\u{7}"),
        Osc11Outcome::NotConsumed,
        "配额耗尽后不消费"
    );
    // ③ `ESC ] 10 ;` 不是 OSC 11，不处理
    let mut state = Osc11State::new();
    let _ = state.query_background();
    assert_eq!(
        state.on_sequence("\u{1b}]10;rgb:1111/2222/3333\u{7}"),
        Osc11Outcome::NotConsumed
    );
}

/// P7-6aa/W2.2：形状匹配但解析失败 ⇒ **消费且减配额**（d1/d2 的语义）。
#[test]
fn shape_match_with_bad_payload_is_consumed() {
    let mut state = Osc11State::new();
    let _ = state.query_background();
    assert_eq!(
        state.on_sequence("\u{1b}]11;not-a-color\u{7}"),
        Osc11Outcome::Consumed(None)
    );
    assert_eq!(state.pending(), 0, "形状匹配即消费，配额应 -1");
    // 短载荷同理
    let mut state = Osc11State::new();
    let _ = state.query_background();
    assert_eq!(
        state.on_sequence("\u{1b}]11;rgb:11\u{7}"),
        Osc11Outcome::Consumed(None)
    );
    assert_eq!(state.pending(), 0);
}

/// 形状匹配且解析成功 ⇒ 返回颜色。
#[test]
fn shape_match_with_good_payload_returns_color() {
    let mut state = Osc11State::new();
    let _ = state.query_background();
    assert_eq!(
        state.on_sequence("\u{1b}]11;rgb:1111/2222/3333\u{7}"),
        Osc11Outcome::Consumed(Some(Rgb {
            r: 17,
            g: 34,
            b: 51
        }))
    );
    assert_eq!(state.pending(), 0);
}

/// 形状不匹配 ⇒ 不消费、不减配额。
#[test]
fn shape_mismatch_is_not_consumed() {
    let mut state = Osc11State::new();
    let _ = state.query_background();
    assert_eq!(
        state.on_sequence("\u{1b}]10;rgb:1111/2222/3333\u{7}"),
        Osc11Outcome::NotConsumed
    );
    assert_eq!(state.on_sequence("a"), Osc11Outcome::NotConsumed);
    assert_eq!(state.pending(), 1, "未消费则配额不变");
}
