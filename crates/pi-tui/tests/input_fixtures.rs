//! P7-6q/W3：输入层纯函数对照 `tools/difftest/tui-oracle/fixtures-input.lines.txt`（由 ref 的真实
//! `StdinBuffer` + `keys.ts` 生成）。
//!
//! 失败时输出：用例 id、分片序号、首个差异事件与前后各 2 个事件。
#![forbid(unsafe_code)]

use pi_tui::input::{is_release, parse_key, InputEvent, InputState};

/// 最小 base64 解码（仅测试用，避免新增依赖）。
fn b64_decode(text: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0u32;
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
    kitty: bool,
    timeout: bool,
    chunks: Vec<Vec<u8>>,
    /// (seq, key, release, repeat)
    events: Vec<(String, Option<String>, bool, bool)>,
    pastes: Vec<String>,
}

fn load() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-input.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-input.lines.txt");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split(' ');
        let tag = parts.next().unwrap_or("");
        match tag {
            "case" => {
                cases.push(Case {
                    id: parts.next().unwrap_or("").to_string(),
                    kitty: parts.next() == Some("1"),
                    timeout: parts.next() == Some("1"),
                    ..Case::default()
                });
            }
            "chunk" => {
                if let Some(case) = cases.last_mut() {
                    case.chunks.push(b64_decode(parts.next().unwrap_or("")));
                }
            }
            "event" => {
                let kind = parts.next().unwrap_or("");
                if let Some(case) = cases.last_mut() {
                    if kind == "paste" {
                        case.pastes.push(
                            String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                                .to_string(),
                        );
                    } else {
                        let seq = String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                            .to_string();
                        let key = parts.next().unwrap_or("-");
                        let key = if key == "-" {
                            None
                        } else {
                            Some(key.to_string())
                        };
                        let release = parts.next() == Some("1");
                        let repeat = parts.next() == Some("1");
                        case.events.push((seq, key, release, repeat));
                    }
                }
            }
            _ => {}
        }
    }
    cases
}

#[test]
fn fixtures_input_all_pass() {
    let cases = load();
    assert!(cases.len() >= 220, "夹具用例数不足：{}", cases.len());
    let mut failures = Vec::new();
    for case in &cases {
        let mut state = InputState::new(case.kitty);
        let mut actual: Vec<(String, Option<String>, bool, bool)> = Vec::new();
        let mut pastes: Vec<String> = Vec::new();
        for (chunk_index, chunk) in case.chunks.iter().enumerate() {
            let (events, _wake) = state.process(chunk, 1000 + chunk_index as u64);
            for event in events {
                match event {
                    InputEvent::Data(seq) => {
                        let key = parse_key(&seq, case.kitty);
                        actual.push((seq.clone(), key, is_release(&seq), false));
                    }
                    InputEvent::Paste(content) => pastes.push(content),
                }
            }
        }
        if case.timeout {
            for event in state.flush() {
                match event {
                    InputEvent::Data(seq) => {
                        let key = parse_key(&seq, case.kitty);
                        actual.push((seq.clone(), key, is_release(&seq), false));
                    }
                    InputEvent::Paste(content) => pastes.push(content),
                }
            }
        }
        if actual != case.events || pastes != case.pastes {
            let position = actual
                .iter()
                .zip(&case.events)
                .position(|(a, b)| a != b)
                .unwrap_or(actual.len().min(case.events.len()));
            failures.push(format!(
                "用例 {}（分片数 {}，首个差异事件 #{position}）\n  期望前后: {:?}\n  实际前后: {:?}\n  pastes 期望={:?} 实际={:?}",
                case.id,
                case.chunks.len(),
                &case.events[position.saturating_sub(2)..(position + 3).min(case.events.len())],
                &actual[position.saturating_sub(2)..(position + 3).min(actual.len())],
                case.pastes,
                pastes,
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "输入夹具未全过：{}/{} 失败\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
    println!("输入夹具全过：{} 例", cases.len());
}

/// 公共 API 声明 `Send + Sync`（纯数据，无线程内共享可变状态）。
#[test]
fn input_state_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<InputState>();
}

/// P7-6al/B：**多字节字符跨读取边界**与**粘贴内含 ESC 序列字面**必须不 panic、不丢字。
mod split_and_paste {
    use pi_tui::input::{InputEvent, InputState};

    #[test]
    fn utf8_split_across_chunks_keeps_text() {
        // "中文" = e4 b8 ad e6 96 87，故意切在第二个汉字的中间
        let mut state = InputState::new(false);
        // 第一片已含完整"中"（e4 b8 ad）+ 半截"文"（e6）⇒ 应只交出"中"，半截留到下次
        let (events, _) = state.process(&[0xe4, 0xb8, 0xad, 0xe6], 0);
        let first: Vec<String> = events
            .into_iter()
            .map(|e| match e {
                InputEvent::Data(s) => s,
                InputEvent::Paste(p) => p,
            })
            .collect();
        assert_eq!(
            first,
            vec!["中".to_string()],
            "半截字符不应被交出，也不应丢字"
        );
        let (events, _) = state.process(&[0x96, 0x87], 10);
        let text: Vec<String> = events
            .into_iter()
            .map(|e| match e {
                InputEvent::Data(s) => s,
                InputEvent::Paste(p) => p,
            })
            .collect();
        assert_eq!(
            text,
            vec!["文".to_string()],
            "第二片补齐后应交出剩下的\"文\""
        );
    }

    #[test]
    fn paste_with_csi_literal_and_chinese_does_not_panic() {
        // 复现真机崩溃：粘贴内容含 `\x1b[?1u` 字面 + 中文，且结束标记被切在中间
        let mut state = InputState::new(false);
        let mut content = Vec::from("\x1b[200~粘贴含 ".as_bytes());
        content.extend_from_slice(b"\x1b[?1u");
        content.extend_from_slice(" 结束".as_bytes());
        let (events, _) = state.process(&content, 0);
        assert!(events.is_empty(), "粘贴未闭合时不应产生事件");
        // 结尾标记分两片送达
        let (events, _) = state.process(b"\x1b[201", 10);
        assert!(events.is_empty());
        let (events, _) = state.process(b"~", 20);
        let pasted: Vec<String> = events
            .into_iter()
            .filter_map(|e| match e {
                InputEvent::Paste(p) => Some(p),
                InputEvent::Data(_) => None,
            })
            .collect();
        assert_eq!(pasted.len(), 1);
        assert!(
            pasted[0].contains("\u{1b}[?1u"),
            "粘贴内的字面序列应原样保留"
        );
        assert!(pasted[0].contains("粘贴含"), "中文应完整保留");
    }
}
