//! P7-6am/K：**键位 oracle**——读取 `fixtures-editkeys.lines.txt`（由 ref 真实 `Editor` 产出），
//! 把同一按键序列经 `map_key_sequence` 映射后喂给 `pi_tui::apply`，比较编辑后的文本。
#![forbid(unsafe_code)]

use pi_tui::{apply, TuiEvent, TuiState};

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

#[derive(Default)]
struct Case {
    id: String,
    text: String,
    moves: usize,
    seq: String,
    expect_text: String,
}

fn load() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-editkeys.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-editkeys.lines.txt");
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split(' ');
        match parts.next().unwrap_or("") {
            "case" => cases.push(Case {
                id: parts.next().unwrap_or("").to_string(),
                ..Case::default()
            }),
            "text" => {
                if let Some(case) = cases.last_mut() {
                    case.text = String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                        .to_string();
                }
            }
            "moves" => {
                if let Some(case) = cases.last_mut() {
                    case.moves = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                }
            }
            "seq" => {
                if let Some(case) = cases.last_mut() {
                    case.seq = String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                        .to_string();
                }
            }
            "expect_text" => {
                if let Some(case) = cases.last_mut() {
                    case.expect_text =
                        String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                            .to_string();
                }
            }
            _ => {}
        }
    }
    cases
}

#[test]
fn edit_keys_match_ref_oracle() {
    let cases = load();
    assert!(cases.len() >= 24, "夹具应 ≥24 例，实际 {}", cases.len());
    // **跳过应用层已认领的按键**：本夹具由 ref 的**纯 `Editor`** 产出
    // （`fixtures-editkeys.lines.txt`），只覆盖编辑器内部行为。
    // 但真实 TUI 里 `alt+enter` 在**到达编辑器之前**就被动作层接走了：
    //   `custom-editor.ts:138-145` 先遍历 `actionHandlers`，
    //   命中 `app.message.followUp`（默认 `alt+enter`，`core/keybindings.ts:133-136`）
    //   就执行并 `return`，不会落到 `super.handleInput` 的换行判断
    //   （`editor.ts:887-895` 那段只在没有动作认领时才轮到）。
    //
    // 因此这 5 条 `alt+enter-*` 期望值测的是"纯编辑器"层，与本应用的按键语义
    // 不在同一层；把它们当成本项目的期望值会让 `alt+enter` 退化为换行
    // （用户按 `/hotkeys` 里写的键没有效果）。应用层的期望值由
    // `p9_f61_follow_up.rs` 覆盖。
    // 序列形式：`alt+enter` 在多数终端里是 ESC + CR。
    const APP_CLAIMED_SEQUENCES: &[&str] = &["\u{1b}\r"];
    let mut failures = Vec::new();
    for case in &cases {
        if APP_CLAIMED_SEQUENCES.contains(&case.seq.as_str()) {
            continue;
        }
        let mut state = TuiState::new("0.0.0", "work", "0.0%/128k (auto)");
        state.input = case.text.clone();
        state.cursor = case.text.chars().count();
        for _ in 0..case.moves {
            let _ = apply(&mut state, TuiEvent::CursorLeft);
        }
        if let Some(event) = pi_cli::tui::map_key_sequence(&case.seq) {
            let _ = apply(&mut state, event);
        }
        if state.input != case.expect_text {
            failures.push(format!(
                "{}: 按键序列 {:?} 后期望 {:?} 实际 {:?}",
                case.id, case.seq, case.expect_text, state.input
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "键位 oracle 未全过：{}/{} 失败\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
    println!("键位 oracle 全过：{} 例", cases.len());
}
