//! P7-9/P：粘贴折叠夹具（ref 真实 `Editor` 产出，见 `fixtures/paste/README.md`）。
//!
//! 每例对比四项：编辑器文本（含占位符）、显示行、光标（字符下标）、提交展开后的原文。
//! 夹具由 `tools/difftest/gen-paste-cases.mjs` 生成（3 次 sha 一致）。

use pi_tui::{TuiEvent, TuiState};

fn load_cases() -> serde_json::Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/paste/ref-cases.json"
    );
    let text = std::fs::read_to_string(path).expect("读夹具");
    serde_json::from_str(&text).expect("夹具 JSON 合法")
}

fn state() -> TuiState {
    TuiState::new("0.0.0", "work", "0.0%/128k (auto)")
}

/// JSON 里的动作按名字重放：`paste:<文本>` / `type:<文本>` / `key:<序列>`。
fn replay(state: &mut TuiState, actions: &serde_json::Value) {
    for action in actions.as_array().map(Vec::as_slice).unwrap_or_default() {
        let name = action["kind"].as_str().unwrap_or_default();
        let text = action["text"].as_str().unwrap_or_default();
        match name {
            "paste" => {
                let _ = pi_tui::apply(state, TuiEvent::Paste(text.to_string()));
            }
            "type" => {
                for ch in text.chars() {
                    let _ = pi_tui::apply(state, TuiEvent::Char(ch));
                }
            }
            "key" => {
                for ch in text.chars() {
                    let event = match ch {
                        '\u{7f}' => TuiEvent::Backspace,
                        '\r' => TuiEvent::Enter,
                        _other => continue,
                    };
                    let _ = pi_tui::apply(state, event);
                }
                if text.contains("\u{1b}[D") {
                    let count = text.matches("\u{1b}[D").count();
                    for _ in 0..count {
                        let _ = pi_tui::apply(state, TuiEvent::CursorLeft);
                    }
                }
                if text.contains("\u{1b}[C") {
                    let count = text.matches("\u{1b}[C").count();
                    for _ in 0..count {
                        let _ = pi_tui::apply(state, TuiEvent::CursorRight);
                    }
                }
                if text.contains("\u{1b}[H") {
                    let _ = pi_tui::apply(state, TuiEvent::LineStart);
                }
                if text.contains("\u{1b}[F") {
                    let _ = pi_tui::apply(state, TuiEvent::LineEnd);
                }
            }
            _other => {}
        }
    }
}

/// 光标：ref 是 `(line, col)`，我们是一个字符下标。
fn cursor_index(lines: &[String], line: usize, col: usize) -> usize {
    let head: usize = lines
        .iter()
        .take(line)
        .map(|item| item.chars().count() + 1)
        .sum();
    head + col
}

#[test]
fn paste_cases_match_the_reference_editor() {
    let cases = load_cases();
    let mut failures = Vec::new();
    for case in cases.as_array().expect("夹具是数组") {
        let name = case["name"].as_str().unwrap_or("?");
        let mut state = state();
        replay(&mut state, &case["actions"]);
        let expected_text = case["text"].as_str().unwrap_or_default();
        if state.input != expected_text {
            failures.push(format!(
                "{name}: 文本 {:?} != ref {:?}",
                state.input, expected_text
            ));
            continue;
        }
        let expected_lines: Vec<String> = case["lines"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let our_lines: Vec<String> = state.input.split('\n').map(str::to_string).collect();
        if our_lines != expected_lines {
            failures.push(format!(
                "{name}: 显示行 {:?} != ref {:?}",
                our_lines, expected_lines
            ));
        }
        let line = case["cursor"]["line"].as_u64().unwrap_or(0) as usize;
        let col = case["cursor"]["col"].as_u64().unwrap_or(0) as usize;
        let expected_cursor = cursor_index(&expected_lines, line, col);
        if state.cursor != expected_cursor {
            failures.push(format!(
                "{name}: 光标 {} != ref {}",
                state.cursor, expected_cursor
            ));
        }
        let expected_expanded = case["expanded"].as_str().unwrap_or_default();
        if state.expanded_text() != expected_expanded {
            failures.push(format!(
                "{name}: 展开后 {:?} != ref {:?}",
                state.expanded_text(),
                expected_expanded
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} 例不一致：\n{}",
        failures.len(),
        failures.join("\n")
    );
}
