//! P7-6y/W2：**链路测试**——字节 → `input.rs`（StdinBuffer 端口）→ `term_negotiate` → 事件。
//!
//! 夹具 `tools/difftest/tui-oracle/fixtures-term-chain.json` 由 ref 的真实 `ProcessTerminal`
//! （内含真实 `StdinBuffer` + 协商逻辑）跑出；每片之后生成器等待 200 ms，**两个定时器都必然到期**
//! （StdinBuffer 的 10 ms ESC 超时、协商层的 150 ms 前缀聚合）。
//!
//! 因此本测试对每片显式驱动**两个**计时器：
//! - `InputState::flush()` —— 代表 `input.rs` 的 10 ms（孤立 ESC）/ 100 ms（序列）超时到期；
//! - `NegotiateState::on_timeout()` —— 代表协商层 150 ms 前缀聚合到期。
#![forbid(unsafe_code)]

use pi_tui::input::{InputEvent, InputState};
use pi_tui::term_negotiate::{Action, NegotiateState};

const SETTLE_MS: u64 = 200;

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
    chunks: Vec<Vec<u8>>,
    events: Vec<String>,
}

fn load() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-term-chain.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-term-chain.lines.txt");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split(' ');
        match parts.next().unwrap_or("") {
            "case" => cases.push(Case {
                id: parts.next().unwrap_or("").to_string(),
                ..Case::default()
            }),
            "chunk" => {
                if let Some(case) = cases.last_mut() {
                    case.chunks.push(b64_decode(parts.next().unwrap_or("")));
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

/// 跑一条链路：每片 ⇒ process + flush（input.rs 定时器）+ on_timeout（协商定时器）。
fn run_chain(chunks: &[Vec<u8>]) -> Vec<String> {
    let mut splitter = InputState::new(false);
    let mut negotiate = NegotiateState::new();
    let _ = negotiate.start_sequences();
    let mut events: Vec<String> = Vec::new();
    let mut now = 0u64;

    let feed = |action: Action, events: &mut Vec<String>| {
        if let Action::Forward(seq) = action {
            events.push(seq);
        }
    };

    for chunk in chunks {
        now += SETTLE_MS;
        let (mut produced, _wake) = splitter.process(chunk, now);
        // 只对"孤立 ESC"驱动 input.rs 的 10 ms 超时（ref `stdin-buffer.ts:24` 的 escapeTimeout）；
        // 其它不完整缓冲在 ref 里由 300 ms 的序列超时兜底，而本夹具每片只等 200 ms ⇒ 不会触发，
        // 因此这里**不能**无条件 flush，否则会把 `\x1b[?` 这类"半条应答"过早拆开。
        if chunk == b"\x1b" {
            produced.extend(splitter.flush());
        }
        for event in produced {
            match event {
                InputEvent::Data(seq) => {
                    let (actions, _wake) = negotiate.on_sequence(&seq, now);
                    for action in actions {
                        feed(action, &mut events);
                    }
                }
                InputEvent::Paste(content) => {
                    // ref `terminal.ts:229-232`：粘贴内容重新包裹后整体转发
                    events.push(format!("\u{1b}[200~{content}\u{1b}[201~"));
                }
            }
        }
        for action in negotiate.on_timeout() {
            feed(action, &mut events);
        }
    }
    events
}

#[test]
fn term_chain_all_pass() {
    let cases = load();
    assert_eq!(cases.len(), 12, "链路夹具应为 12 例（4 类 × 3）");
    let mut failures = Vec::new();
    for case in &cases {
        let actual = run_chain(&case.chunks);
        if actual != case.events {
            failures.push(format!(
                "{}:\n  期望 {:?}\n  实际 {:?}",
                case.id, case.events, actual
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "链路夹具未全过：{}/{} 失败\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
    println!("链路夹具全过：{} 例", cases.len());
}
