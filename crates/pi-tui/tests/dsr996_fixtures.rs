//! P7-6af/D：DSR 996/997 对照 `fixtures-dsr996.lines.txt`（侧车由 `gen-dsr996-lines.mjs` 生成，
//! 生成时已断言与 `fixtures-dsr996.json` 15/15 一致）。
#![forbid(unsafe_code)]

use pi_tui::dsr996::{on_sequence, parse_report, ColorScheme, NOTIFICATIONS_OFF, NOTIFICATIONS_ON};

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
    chunks: Vec<String>,
    scheme: Option<ColorScheme>,
    events: Vec<String>,
}

fn load() -> Vec<Case> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-dsr996.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-dsr996.lines.txt");
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
                    case.chunks.push(
                        String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or("")))
                            .to_string(),
                    );
                }
            }
            "scheme" => {
                if let Some(case) = cases.last_mut() {
                    case.scheme = match parts.next() {
                        Some("dark") => Some(ColorScheme::Dark),
                        Some("light") => Some(ColorScheme::Light),
                        other => {
                            let _ = other;
                            None
                        }
                    };
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

/// 逐片喂入：命中 997 报告即消费（进 `scheme`），其余原样落到 `events`（= 会漏进编辑器）。
fn drive(case: &Case) -> (Option<ColorScheme>, Vec<String>) {
    let mut scheme = None;
    let mut events = Vec::new();
    for chunk in &case.chunks {
        match on_sequence(chunk) {
            Some(found) => scheme = Some(found),
            None => events.push(chunk.clone()),
        }
    }
    (scheme, events)
}

#[test]
fn dsr996_fixtures_all_pass() {
    let cases = load();
    assert_eq!(cases.len(), 15, "夹具应为 15 例（5 类 × 3）");
    let mut failures = Vec::new();
    for case in &cases {
        let (scheme, events) = drive(case);
        if events != case.events {
            failures.push(format!(
                "{}: 未消费事件不同\n  期望 {:?}\n  实际 {:?}",
                case.id, case.events, events
            ));
            continue;
        }
        // 超时类用例（无应答/应答晚到）不比 scheme（时序语义）
        if case.chunks.is_empty() {
            continue;
        }
        if scheme != case.scheme {
            failures.push(format!(
                "{}: 方案不同 期望 {:?} 实际 {:?}",
                case.id, case.scheme, scheme
            ));
        }
    }
    if !failures.is_empty() {
        panic!(
            "DSR996 夹具未全过：{}/{} 失败\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
    println!("DSR996 夹具全过：{} 例", cases.len());
}

/// 通知开关序列与 ref 一致（`tui.ts:887-888` / `:936`）。
#[test]
fn notification_sequences_match_ref() {
    assert_eq!(NOTIFICATIONS_ON, "\u{1b}[?2031h");
    assert_eq!(NOTIFICATIONS_OFF, "\u{1b}[?2031l");
    assert_eq!(parse_report("\u{1b}[?997;1n"), Some(ColorScheme::Dark));
    assert_eq!(parse_report("\u{1b}[?997;2n"), Some(ColorScheme::Light));
    assert_eq!(parse_report("\u{1b}[?997;3n"), None);
    assert_eq!(parse_report("\u{1b}[?996;1n"), None);
}
