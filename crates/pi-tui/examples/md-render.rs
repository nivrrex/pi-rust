//! F34/M1b：把 markdown 渲染成带 SGR 的行（供 `tools/difftest/md-oracle` 对拍）。
//!
//! 用法：`cargo run -q -p pi-tui --example md-render -- --theme dark --width 80 <file.md>`
//! 输出：每行一条 `JSON.stringify` 字符串（换行转义），便于与 TS oracle 逐行 diff。

use std::io::Read;

fn main() {
    let mut theme = "dark".to_string();
    let mut width = 80usize;
    let mut file: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--theme" => theme = args.next().unwrap_or_else(|| "dark".to_string()),
            "--width" => width = args.next().and_then(|v| v.parse().ok()).unwrap_or(80),
            other => file = Some(other.to_string()),
        }
    }
    let mut input = String::new();
    match file {
        Some(path) => input = std::fs::read_to_string(path).expect("读输入"),
        None => {
            std::io::stdin()
                .read_to_string(&mut input)
                .expect("读 stdin");
        }
    }
    let palette = pi_tui::theme::Palette::of(&pi_tui::theme::Theme::builtin(&theme));
    let (lines, styles, spans) =
        // TS 的 `Markdown(text, paddingX=1, …)` 在 `width` 里扣掉左右各 1 列（ref
        // `assistant-message.ts:114` + `text.ts` 的 padding 处理）⇒ 内容宽度是 `width-2`。
        pi_tui::render_markdown_styled_with_width(
            input.trim(),
            &palette,
            Some(width.saturating_sub(2)),
        );
    let spans_by_line: std::collections::HashMap<usize, Vec<pi_tui::style::Span>> =
        spans.into_iter().collect();
    for (index, line) in lines.iter().enumerate() {
        let style = styles.get(index).copied().flatten();
        let mut rendered = String::new();
        if let Some(line_spans) = spans_by_line.get(&index) {
            pi_tui::diff::write_spans(&mut rendered, line, line_spans, 0);
        } else {
            rendered.push_str(line);
        }
        // 整行样式（标题/引用/代码块）作为兜底：没有逐段 span 时整行染色。
        if !spans_by_line.contains_key(&index) {
            if let Some(style) = style {
                rendered = format!("{}{}{}", style.prefix(), rendered, style.reset());
            }
        }
        println!("{}", serde_json::to_string(&rendered).expect("序列化"));
    }
}
