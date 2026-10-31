//! P7-6af/A.3 + C：键名对照 `fixtures-keymap.lines.txt`（由 ref 真实 `parseKey` 产出，非 Rust 反推）。
#![forbid(unsafe_code)]

use pi_tui::input::parse_key;

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

#[test]
fn keymap_fixtures_all_pass() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-keymap.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-keymap.lines.txt");
    let mut failures = Vec::new();
    let mut total = 0;
    for line in text.lines() {
        let mut parts = line.split(' ');
        if parts.next() != Some("case") {
            continue;
        }
        let id = parts.next().unwrap_or("").to_string();
        let kitty = parts.next() == Some("1");
        let seq = String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or(""))).to_string();
        let expected = parts.next().unwrap_or("-");
        let expected = if expected == "-" {
            None
        } else {
            Some(expected.to_string())
        };
        let actual = parse_key(&seq, kitty);
        total += 1;
        if actual != expected {
            failures.push(format!(
                "{id}: 序列 {seq:?}（kitty={kitty}）期望 {expected:?} 实际 {actual:?}"
            ));
        }
    }
    assert!(total >= 84, "夹具用例数不足：{total}");
    if !failures.is_empty() {
        panic!(
            "键名夹具未全过：{}/{} 失败\n{}",
            failures.len(),
            total,
            failures.join("\n")
        );
    }
    println!("键名夹具全过：{total} 例");
}

/// M2：kitty 激活态下的 `\n` 族键名（`fixtures-keys-mode.lines.txt`，由 ref `parseKey` 产出）。
///
/// P7-6ah/I：`input.rs::parse_key` 已按 `keys.ts:1264-1267` / `1283-1291` / `1296-1300` / `1318` 修正，
/// 原先 8 个红例已全部转绿（`\r\n`、`\x1b\r`、`\x1b\n`、`\x1b `、`\x1b\x7f`、kitty 态 `\x1b\r` 等）。
#[test]
fn keymap_kitty_mode_fixtures_pass() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/difftest/tui-oracle/fixtures-keys-mode.lines.txt"
    );
    let text = std::fs::read_to_string(path).expect("读不到 fixtures-keys-mode.lines.txt");
    let mut failures = Vec::new();
    let mut total = 0;
    for line in text.lines() {
        let mut parts = line.split(' ');
        if parts.next() != Some("case") {
            continue;
        }
        let id = parts.next().unwrap_or("").to_string();
        let kitty = parts.next() == Some("1");
        let seq = String::from_utf8_lossy(&b64_decode(parts.next().unwrap_or(""))).to_string();
        let expected = parts.next().unwrap_or("-");
        let expected = if expected == "-" {
            None
        } else {
            Some(expected.to_string())
        };
        let actual = parse_key(&seq, kitty);
        total += 1;
        if actual != expected {
            failures.push(format!(
                "{id}: 序列 {seq:?}（kitty={kitty}）期望 {expected:?} 实际 {actual:?}"
            ));
        }
    }
    assert!(total >= 12, "kitty 模式夹具用例数不足：{total}");
    if !failures.is_empty() {
        panic!(
            "kitty 模式夹具未全过：{}/{} 失败\n{}",
            failures.len(),
            total,
            failures.join("\n")
        );
    }
    println!("kitty 模式夹具全过：{total} 例");
}
