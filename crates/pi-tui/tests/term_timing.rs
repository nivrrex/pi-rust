//! P7-6af/A.2：**协商层转发时机**对照 `fixtures-term-timing.lines.txt`。
//!
//! 盲区（A.1）：`term_fixtures.rs` 与 `term_chain.rs` 每步都无条件调 `on_timeout()`，
//! 断言又只比"最终事件序列"，因此"立即转发"与"等 150 ms 后转发"无法区分。
//! 本测试按**步**比对事件，且只在 `short`/`wait` 步驱动计时器：
//! - `short`（30 ms，介于 StdinBuffer 的 10 ms 与协商 150 ms 之间）⇒ 只驱动 `input.flush()`；
//! - `wait`（200 ms）⇒ 驱动 `input.flush()` + `negotiate.on_timeout()`。
#![forbid(unsafe_code)]

use pi_tui::input::{InputEvent, InputState};
use pi_tui::term_negotiate::{Action, NegotiateState};

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

#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Feed,
    Short,
    Wait,
}

#[derive(Debug)]
struct Step {
    kind: Kind,
    chunks: Vec<Vec<u8>>,
}

#[derive(Debug)]
struct Case {
    id: String,
    steps: Vec<Step>,
    expected: Vec<Vec<String>>,
}

fn load() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-term-timing.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-term-timing.lines.txt");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split(' ');
        match parts.next().unwrap_or("") {
            "case" => cases.push(Case {
                id: parts.next().unwrap_or("").to_string(),
                steps: Vec::new(),
                expected: Vec::new(),
            }),
            "step" => {
                let kind = match parts.next().unwrap_or("feed") {
                    "wait" => Kind::Wait,
                    "short" => Kind::Short,
                    other => {
                        let _ = other;
                        Kind::Feed
                    }
                };
                let chunks = parts.next().unwrap_or("-");
                let chunks = if chunks == "-" {
                    Vec::new()
                } else {
                    chunks.split(',').map(b64_decode).collect()
                };
                if let Some(case) = cases.last_mut() {
                    case.steps.push(Step { kind, chunks });
                }
            }
            "events" => {
                let raw = parts.next().unwrap_or("-");
                let events = if raw == "-" {
                    Vec::new()
                } else {
                    raw.split(',')
                        .map(|e| String::from_utf8_lossy(&b64_decode(e)).to_string())
                        .collect()
                };
                if let Some(case) = cases.last_mut() {
                    case.expected.push(events);
                }
            }
            _ => {}
        }
    }
    cases
}

fn drive(case: &Case) -> Vec<Vec<String>> {
    let mut input = InputState::new(false);
    let mut negotiate = NegotiateState::new();
    let mut now = 0u64;
    let mut per_step: Vec<Vec<String>> = Vec::new();
    let mut pending: Vec<String> = Vec::new();

    let handle = |event: InputEvent, pending: &mut Vec<String>, negotiate: &mut NegotiateState| {
        match event {
            InputEvent::Paste(content) => pending.push(format!("\u{1b}[200~{content}\u{1b}[201~")),
            InputEvent::Data(seq) => {
                let (actions, _wake) = negotiate.on_sequence(&seq, 0);
                for action in actions {
                    if let Action::Forward(seq) = action {
                        pending.push(seq);
                    }
                }
            }
        }
    };

    for step in &case.steps {
        now += 3;
        for chunk in &step.chunks {
            let (events, _wake) = input.process(chunk, now);
            for event in events {
                handle(event, &mut pending, &mut negotiate);
            }
        }
        match step.kind {
            Kind::Feed => {}
            Kind::Short => {
                // 只驱动 StdinBuffer 的 escape 超时（10 ms < 30 ms），**不**驱动协商的 150 ms
                for event in input.flush() {
                    handle(event, &mut pending, &mut negotiate);
                }
            }
            Kind::Wait => {
                for event in input.flush() {
                    handle(event, &mut pending, &mut negotiate);
                }
                for action in negotiate.on_timeout() {
                    if let Action::Forward(seq) = action {
                        pending.push(seq);
                    }
                }
            }
        }
        per_step.push(std::mem::take(&mut pending));
    }
    per_step
}

#[test]
fn term_timing_all_pass() {
    let cases = load();
    assert_eq!(cases.len(), 15, "时机夹具应为 15 例");
    let mut failures = Vec::new();
    for case in &cases {
        let actual = drive(case);
        if actual != case.expected {
            failures.push(format!(
                "{}:\n  期望 {:?}\n  实际 {:?}",
                case.id, case.expected, actual
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "转发时机夹具未全过：{}/{} 失败\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
    println!("转发时机夹具全过：{} 例", cases.len());
}
