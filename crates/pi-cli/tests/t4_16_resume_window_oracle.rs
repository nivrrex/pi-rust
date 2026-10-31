//! `/resume` 分页：对拍 **ref 真实 `SessionSelectorComponent`** 产出的 oracle 夹具
//! （`tools/difftest/tui-oracle/gen-session-selector.mjs`，禁止手改）。
//!
//! 判据：可见条目（窗口）与 `(n/total)` 提示行必须与 ref 逐行一致；
//! 夹具里的按键序列（↓ / PageDown / PageUp）逐条重放。
use std::path::PathBuf;

use pi_cli::settings_selector::{ResumeSelector, SessionRow};

/// 夹具里的会话数（> ref `maxVisible = 10`）。
const COUNT: usize = 12;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/difftest/tui-oracle")
        .join(name)
}

/// 解析文本侧车：`case <id> keys=<n>` + 若干 `line <base64>`。
fn load_cases() -> Vec<(String, Vec<String>, Vec<String>)> {
    let text = std::fs::read_to_string(fixture_path("fixtures-session-selector.lines.txt"))
        .expect("oracle 夹具缺失（先跑 gen-session-selector.mjs）");
    let mut cases: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("case ") {
            let mut parts = rest.split_whitespace();
            let id = parts.next().unwrap_or_default().to_string();
            let keys = parts
                .next()
                .and_then(|item| item.strip_prefix("keys="))
                .map(|joined| {
                    joined
                        .split(',')
                        .filter(|key| !key.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            cases.push((id, keys, Vec::new()));
        } else if let Some(payload) = line.strip_prefix("line ") {
            let bytes = base64_decode(payload);
            let text = String::from_utf8(bytes).expect("夹具行必须是 UTF-8");
            cases
                .last_mut()
                .expect("line 必须在 case 之后")
                .2
                .push(text);
        }
    }
    cases
}

/// 极简 base64 解码（只解夹具里的标准字母表 + `=` 填充）。
fn base64_decode(input: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255_u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        lookup[*byte as usize] = index as u8;
    }
    let mut out = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u32;
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let value = lookup[byte as usize];
        assert_ne!(value, 255, "非法 base64 字符：{:?}", byte as char);
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

/// 与夹具同构的 12 条会话：`modified` 取"现在 - 400 天"⇒ 两侧都渲染成 `1y`（稳定）。
fn rows() -> Vec<SessionRow> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    let long_ago = now_ms.saturating_sub(400 * 86_400_000);
    (0..COUNT)
        .map(|index| SessionRow {
            path: PathBuf::from(format!("/sessions/s{index:02}.jsonl")),
            name: None,
            summary: Some(format!("会话 {}：分页夹具", index + 1)),
            cwd: "/work/project".to_string(),
            message_count: 1 + index,
            modified_ms: long_ago,
        })
        .collect()
}

fn selector() -> ResumeSelector {
    let mut selector = ResumeSelector::new();
    selector.filter_cwd = false;
    selector.sessions = rows();
    selector
}

/// 只取"会话行 + 可能的 `(n/total)` 提示"：面板边框/标题由渲染层补，不参与比对。
fn rows_and_hint(lines: &[String]) -> (Vec<String>, Option<String>) {
    let mut rows = Vec::new();
    let mut hint = None;
    for (index, line) in lines.iter().enumerate() {
        if line.contains("分页夹具") {
            rows.push(line.trim_end().to_string());
            continue;
        }
        if line.trim_start().starts_with('(') && line.contains('/') && rows_are_done(lines, index) {
            hint = Some(line.trim_end().to_string());
        }
    }
    (rows, hint)
}

fn rows_are_done(lines: &[String], index: usize) -> bool {
    lines
        .get(index + 1)
        .is_none_or(|next| !next.contains("分页夹具"))
}

#[test]
fn resume_window_matches_ref_oracle() {
    for (id, keys, ref_lines) in load_cases() {
        let mut selector = selector();
        for key in &keys {
            match key.as_str() {
                "pageup" => selector.page_up(ResumeSelector::MAX_VISIBLE),
                "pagedown" => selector.page_down(ResumeSelector::MAX_VISIBLE),
                "down" => selector.move_down(),
                other => panic!("夹具里有未知按键：{other}"),
            }
        }
        let (ref_rows, ref_hint) = rows_and_hint(&ref_lines);
        let ours_lines = selector.lines(100);
        let (our_rows, our_hint) = rows_and_hint(&ours_lines);
        assert_eq!(our_rows, ref_rows, "用例 {id} 的可见窗口不一致");
        assert_eq!(our_hint, ref_hint, "用例 {id} 的 (n/total) 不一致");
    }
}

/// 无裁剪（会话数 ≤ maxVisible）时**不得**出现 `(n/total)`（ref `session-selector.ts:519-525`）。
#[test]
fn no_hint_when_all_sessions_fit() {
    let mut selector = ResumeSelector::new();
    selector.filter_cwd = false;
    selector.sessions = rows()
        .into_iter()
        .take(ResumeSelector::MAX_VISIBLE)
        .collect();
    let (_, hint) = rows_and_hint(&selector.lines(100));
    assert_eq!(hint, None);
    assert_eq!(
        rows_and_hint(&selector.lines(100)).0.len(),
        ResumeSelector::MAX_VISIBLE
    );
}
