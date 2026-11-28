//! P7-4g/W0(a)：**独立屏幕模型**诊断工具（dev 用途）。
//!
//! 读 `--raw <raw.bin> --meta <meta.json> --cols N --rows M`，按 `meta.json` 的最后一个
//! `snapshotOffsets[i].offset` 回退到其之前最后一个 `\x1b[?2026l` 的结尾（与
//! `tools/difftest/compare-tui.mjs` 的口径一致），再把这段字节喂给 `vt100` 渲染并打印屏幕文本。
//! 用途：与 JS 模型逐行 diff，判断"少一行"是模型缺陷还是真实屏幕。
use std::io::Read;

fn main() -> Result<(), String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let get = |name: &str| -> Option<String> {
        let index = argv.iter().position(|item| item == name)?;
        argv.get(index + 1).cloned()
    };
    let raw_path = get("--raw").ok_or("缺 --raw")?;
    let meta_path = get("--meta").ok_or("缺 --meta")?;
    let cols: u16 = get("--cols")
        .ok_or("缺 --cols")?
        .parse()
        .map_err(|e| format!("{e}"))?;
    let rows: u16 = get("--rows")
        .ok_or("缺 --rows")?
        .parse()
        .map_err(|e| format!("{e}"))?;

    let mut raw = Vec::new();
    std::fs::File::open(&raw_path)
        .map_err(|e| format!("{raw_path}: {e}"))?
        .read_to_end(&mut raw)
        .map_err(|e| format!("{raw_path}: {e}"))?;
    let meta_text = std::fs::read_to_string(&meta_path).map_err(|e| format!("{meta_path}: {e}"))?;
    let meta: serde_json::Value = serde_json::from_str(&meta_text).map_err(|e| e.to_string())?;
    let offset = meta
        .get("snapshotOffsets")
        .and_then(|value| value.as_array())
        .and_then(|list| list.last())
        .and_then(|last| last.get("offset"))
        .and_then(serde_json::Value::as_u64)
        .ok_or("meta.json 里没有 snapshotOffsets[last].offset")? as usize;

    // 回退到偏移之前**最后一个** `?2026l` 的结尾（要求整个序列早于偏移），与 JS 口径一致。
    const SYNC_END: &[u8] = b"\x1b[?2026l";
    let mut last_end: Option<usize> = None;
    let mut cursor = 0usize;
    while let Some(found) = find(&raw[cursor..], SYNC_END) {
        let start = cursor + found;
        let end = start + SYNC_END.len();
        if end <= offset {
            last_end = Some(end);
            cursor = end;
        } else {
            break;
        }
    }
    let cut = last_end.unwrap_or(offset);

    // scrollback 长度从 `--scrollback`（默认 1000）取：用于验证"滚出屏幕的行是否被保留"。
    let scrollback: usize = get("--scrollback")
        .map(|value| value.parse().unwrap_or(1000))
        .unwrap_or(1000);
    let mut parser = vt100::Parser::new(rows, cols, scrollback);
    parser.process(&raw[..cut]);
    let screen = parser.screen();
    if argv.iter().any(|item| item == "--dump-scrollback") {
        let text = screen.scrollback().to_string();
        println!(
            "[scrollback 行数={}]",
            text.lines().filter(|line| !line.trim().is_empty()).count()
        );
        for line in text.lines().filter(|line| !line.trim().is_empty()).take(12) {
            println!("[sb] {}", line.trim_end());
        }
    }
    for row in 0..rows {
        let mut line = String::new();
        for col in 0..cols {
            // **宽字符处理**：`contents()` 对宽字符的续格返回空串 ⇒ 直接拼接、不要补空格，
            // 否则 CJK 会渲染成“标 题”（首版这里补了空格，造成与 JS 模型的全场景假分歧）。
            line.push_str(
                &screen
                    .cell(row, col)
                    .map(|cell| cell.contents().to_string())
                    .unwrap_or_default(),
            );
        }
        println!("{}", line.trim_end());
    }
    Ok(())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
