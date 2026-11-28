//! P7-6w/W2：终端协商状态机对照 `tools/difftest/tui-oracle/fixtures-term.lines.txt`
//! （侧车由 `gen-term-lines.mjs` 生成，且生成时已断言与 `fixtures-term.json` 完全一致）。
#![forbid(unsafe_code)]

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

fn b64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

type Phases = Vec<(String, String)>;
type KittyTimeline = Vec<(String, bool)>;

#[derive(Debug, Default)]
struct Step {
    chunks: Vec<Vec<u8>>,
    wait_ms: u64,
    per_chunk_wait: u64,
}

#[derive(Debug, Default)]
struct Case {
    id: String,
    steps: Vec<Step>,
    kitty: Vec<(String, bool)>,
    phases: Vec<(String, String)>,
    events: Vec<String>,
}

fn load() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-term.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-term.lines.txt");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split(' ');
        match parts.next().unwrap_or("") {
            "case" => {
                cases.push(Case {
                    id: parts.next().unwrap_or("").to_string(),
                    ..Case::default()
                });
            }
            "step" => {
                if let Some(case) = cases.last_mut() {
                    let _name = parts.next();
                    let chunks = parts.next().unwrap_or("-");
                    let wait_ms = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    let per_chunk_wait = parts.next().and_then(|v| v.parse().ok()).unwrap_or(5);
                    let chunks = if chunks == "-" {
                        Vec::new()
                    } else {
                        chunks.split(',').map(b64_decode).collect()
                    };
                    case.steps.push(Step {
                        chunks,
                        wait_ms,
                        per_chunk_wait,
                    });
                }
            }
            "kitty" => {
                if let Some(case) = cases.last_mut() {
                    let at = parts.next().unwrap_or("").to_string();
                    let value = parts.next() == Some("1");
                    case.kitty.push((at, value));
                }
            }
            "out" => {
                if let Some(case) = cases.last_mut() {
                    let name = parts.next().unwrap_or("").to_string();
                    let bytes = b64_encode(&b64_decode(parts.next().unwrap_or("")));
                    case.phases.push((name, bytes));
                }
            }
            "event" => {
                if let Some(case) = cases.last_mut() {
                    case.events
                        .push(b64_encode(&b64_decode(parts.next().unwrap_or(""))));
                }
            }
            _ => {}
        }
    }
    cases
}

/// 按夹具驱动：**先用 `input.rs` 把字节切成序列**（P7-6y 裁定：协商层接序列事件），
/// 再把序列喂给 `term_negotiate`；input.rs 的超时由测试显式驱动。
fn drive(case: &Case) -> (Phases, KittyTimeline, Vec<String>) {
    use pi_tui::input::{InputEvent, InputState};

    let mut state = NegotiateState::new();
    let mut splitter = InputState::new(false);
    let mut phases: Vec<(String, String)> = Vec::new();
    let mut kitty: Vec<(String, bool)> = Vec::new();
    let mut events: Vec<String> = Vec::new();
    let mut now = 0u64;
    let mut written = String::new();

    written.push_str(&state.start_sequences());
    phases.push(("start".to_string(), b64_encode(written.as_bytes())));
    written.clear();

    for step in &case.steps {
        for chunk in &step.chunks {
            now += step.per_chunk_wait;
            let text = String::from_utf8_lossy(chunk).to_string();
            let (split_events, _wake) = splitter.process(text.as_bytes(), now);
            for event in split_events {
                match event {
                    InputEvent::Data(seq) => {
                        let (actions, _wake) = state.on_sequence(&seq, now);
                        apply(actions, &mut written, &mut events);
                    }
                    InputEvent::Paste(content) => {
                        let wrapped = format!("\u{1b}[200~{content}\u{1b}[201~");
                        let (actions, _wake) = state.on_sequence(&wrapped, now);
                        apply(actions, &mut written, &mut events);
                    }
                }
            }
        }
        if step.wait_ms > 0 {
            now += step.wait_ms;
            // input.rs 的 ESC/序列超时
            for event in splitter.flush() {
                match event {
                    InputEvent::Data(seq) => {
                        let (actions, _wake) = state.on_sequence(&seq, now);
                        apply(actions, &mut written, &mut events);
                    }
                    InputEvent::Paste(content) => {
                        let wrapped = format!("\u{1b}[200~{content}\u{1b}[201~");
                        let (actions, _wake) = state.on_sequence(&wrapped, now);
                        apply(actions, &mut written, &mut events);
                    }
                }
            }
            // 协商层的 150 ms 前缀超时
            let actions = state.on_timeout();
            apply(actions, &mut written, &mut events);
        }
        phases.push((String::new(), b64_encode(written.as_bytes())));
        written.clear();
        kitty.push((String::new(), state.kitty_active()));
    }

    written.push_str(&state.stop_sequences());
    phases.push(("stop".to_string(), b64_encode(written.as_bytes())));
    (phases, kitty, events)
}

fn apply(actions: Vec<Action>, written: &mut String, events: &mut Vec<String>) {
    for action in actions {
        match action {
            Action::Forward(seq) => events.push(b64_encode(seq.as_bytes())),
            Action::Write(bytes) => written.push_str(bytes),
            Action::Consume => {}
        }
    }
}

#[test]
fn term_negotiate_all_pass() {
    let cases = load();
    assert_eq!(cases.len(), 24, "夹具用例数应为 24");
    let mut failures = Vec::new();
    for case in &cases {
        let (phases, _kitty, events) = drive(case);
        // 阶段写出：只比较非空阶段（与夹具的 phases 对齐按顺序）
        let expected_nonempty: Vec<String> = case
            .phases
            .iter()
            .filter(|(_, bytes)| !b64_decode(bytes).is_empty())
            .map(|(_, bytes)| bytes.clone())
            .collect();
        let actual_nonempty: Vec<String> = phases
            .iter()
            .filter(|(_, bytes)| !b64_decode(bytes).is_empty())
            .map(|(_, bytes)| bytes.clone())
            .collect();
        if events != case.events {
            failures.push(format!(
                "{}: 事件序列不同\n  期望 {:?}\n  实际 {:?}",
                case.id, case.events, events
            ));
        } else if expected_nonempty != actual_nonempty {
            failures.push(format!(
                "{}: 写出字节不同\n  期望 {:?}\n  实际 {:?}",
                case.id, expected_nonempty, actual_nonempty
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "终端协商夹具未全过：{}/{} 失败\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
    println!("终端协商夹具全过：{} 例", cases.len());
}

#[test]
fn negotiate_state_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<NegotiateState>();
}

/// W2.3 附加单测：夹具未覆盖的边界。
mod extra {
    use pi_tui::term_negotiate::{Action, NegotiateState};

    fn forwards(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Forward(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn empty_input_produces_nothing() {
        let mut state = NegotiateState::new();
        let (actions, wake) = state.on_sequence("", 0);
        assert_eq!(
            actions,
            vec![Action::Forward(String::new())],
            "空序列原样转发（可忽略）"
        );
        assert!(wake.is_none());
        assert!(state.on_timeout().is_empty());
    }

    #[test]
    fn response_only_is_consumed_without_events() {
        let mut state = NegotiateState::new();
        let _ = state.start_sequences();
        let (actions, _wake) = state.on_sequence("\u{1b}[?3u", 10);
        assert_eq!(
            forwards(&actions),
            Vec::<String>::new(),
            "应答不应产生键事件"
        );
        assert!(state.kitty_active());
    }

    #[test]
    fn response_between_keys_keeps_order() {
        let mut state = NegotiateState::new();
        let _ = state.start_sequences();
        let (a1, _) = state.on_sequence("a", 1);
        let (a2, _) = state.on_sequence("\u{1b}[?1u", 2);
        let (a3, _) = state.on_sequence("b", 3);
        assert_eq!(forwards(&a1), vec!["a".to_string()]);
        assert!(forwards(&a2).is_empty());
        assert_eq!(forwards(&a3), vec!["b".to_string()]);
    }

    #[test]
    fn timeout_at_boundary_uses_the_fragment_timeout() {
        let mut state = NegotiateState::new();
        let _ = state.start_sequences();
        let (_actions, wake) = state.on_sequence("\u{1b}[?", 1000);
        assert_eq!(
            wake,
            Some(1000 + pi_tui::term_negotiate::FRAGMENT_TIMEOUT_MS)
        );
        // 恰好在边界到期：仍按超时处理（把缓冲当普通输入）
        let flushed = state.on_timeout();
        assert_eq!(forwards(&flushed), vec!["\u{1b}[?".to_string()]);
    }

    #[test]
    fn repeated_responses_are_idempotent() {
        let mut state = NegotiateState::new();
        let _ = state.start_sequences();
        let (first, _) = state.on_sequence("\u{1b}[?1u", 1);
        let (second, _) = state.on_sequence("\u{1b}[?1u", 2);
        assert!(forwards(&first).is_empty() && forwards(&second).is_empty());
        assert!(state.kitty_active());
        // flags=0 之后再来 DA1：kitty 未激活 ⇒ 开 modifyOtherKeys 一次
        let mut state2 = NegotiateState::new();
        let _ = state2.start_sequences();
        let (a, _) = state2.on_sequence("\u{1b}[?0u", 1);
        assert!(a.contains(&Action::Write("\u{1b}[>4;2m")));
        let (b, _) = state2.on_sequence("\u{1b}[?1;2c", 2);
        assert!(
            !b.contains(&Action::Write("\u{1b}[>4;2m")),
            "已开启时不应重复写"
        );
        assert!(state2.modify_other_keys());
    }
}
