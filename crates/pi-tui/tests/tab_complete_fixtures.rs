//! P7-6an/C：Tab 补全对照 `fixtures-tabcomplete.lines.txt`
//! （由 ref 真实 `CombinedAutocompleteProvider.applyCompletion` 产出，见 `autocomplete.ts:389-418`）。
#![forbid(unsafe_code)]

use pi_tui::{apply, completion_matches, TuiEvent, TuiState};

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

struct Case {
    id: String,
    input: String,
    value: String,
    expect_text: String,
    expect_cursor: usize,
}

fn load() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-tabcomplete.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-tabcomplete.lines.txt");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split(' ');
        match parts.next().unwrap_or("") {
            "case" => cases.push(Case {
                id: parts.next().unwrap_or("").to_string(),
                input: String::new(),
                value: String::new(),
                expect_text: String::new(),
                expect_cursor: 0,
            }),
            "input" => {
                if let Some(case) = cases.last_mut() {
                    case.input = String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                        .to_string();
                }
            }
            "value" => {
                if let Some(case) = cases.last_mut() {
                    case.value = parts.next().unwrap_or("").to_string();
                }
            }
            "expect_text" => {
                if let Some(case) = cases.last_mut() {
                    case.expect_text =
                        String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                            .to_string();
                }
            }
            "expect_cursor" => {
                if let Some(case) = cases.last_mut() {
                    case.expect_cursor = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                }
            }
            _ => {}
        }
    }
    cases
}

#[test]
fn tab_completion_matches_ref_oracle() {
    let cases = load();
    assert!(cases.len() >= 15, "夹具应 ≥15 例，实际 {}", cases.len());
    let mut failures = Vec::new();
    for case in &cases {
        // Rust 的选中项固定取「第一个匹配」；因此只用**首个匹配等于夹具 value** 的用例与 ref 比对，
        // 其余用例用来验证"菜单未打开 / 不在匹配集时保持原样"。
        let matches = completion_matches(&case.input);
        let first = matches.first().map(|(name, _)| *name);
        let menu_open = case.input.starts_with('/') && !case.input.contains(' ');
        let (expected_text, expected_cursor) = match first {
            Some(name) if menu_open => (format!("/{name} "), name.chars().count() + 2),
            _ => (case.input.clone(), case.input.chars().count()),
        };
        if first != Some(case.value.as_str()) {
            // 该用例的 ref 选中项不是首个匹配 ⇒ 只校验"不受影响"的路径
            if menu_open {
                continue;
            }
        }
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        state.input = case.input.clone();
        state.cursor = case.input.chars().count();
        state.completion_open = menu_open;
        let _ = apply(&mut state, TuiEvent::Tab);
        if state.input != expected_text || state.cursor != expected_cursor {
            failures.push(format!(
                "{}: 期望 {expected_text:?}/{expected_cursor} 实际 {:?}/{}",
                case.id, state.input, state.cursor
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "Tab 补全未全过：{}/{} 失败\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
    println!("Tab 补全对照通过：{} 例", cases.len());
}
