//! F34/M1b：把 markdown 渲染成带 SGR 的行（供 `tools/difftest/md-oracle` 对拍）。
//!
//! 用法：`cargo run -q -p pi-tui --example md-render -- --theme dark --width 80 <file.md>`
//! 输出：每行一条 `JSON.stringify` 字符串（换行转义），便于与 TS oracle 逐行 diff。

use std::io::Read;

fn main() {
    let mut theme = "dark".to_string();
    let mut width = 80usize;
    let mut file: Option<String> = None;
    // 批量模式：`--dir <目录>`，每个 `.md` 输出一条 `{"file":…,"lines":[…]}`（F35/G0 度量用）。
    let mut dir: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--theme" => theme = args.next().unwrap_or_else(|| "dark".to_string()),
            "--width" => width = args.next().and_then(|v| v.parse().ok()).unwrap_or(80),
            "--dir" => dir = args.next(),
            other => file = Some(other.to_string()),
        }
    }
    if let Some(dir) = dir {
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .expect("读目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".md"))
            .collect();
        names.sort();
        for name in names {
            let text =
                std::fs::read_to_string(std::path::Path::new(&dir).join(&name)).expect("读文件");
            let lines = render_lines(&text, &theme, width);
            let payload = serde_json::json!({ "file": name, "lines": lines });
            println!("{}", serde_json::to_string(&payload).expect("序列化"));
        }
        return;
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
    for line in render_lines(&input, &theme, width) {
        println!("{}", serde_json::to_string(&line).expect("序列化"));
    }
}

/// 把一段 markdown 渲染成带 SGR 的行（每行补空格到可用宽度，便于逐格比较）。
fn render_lines(input: &str, theme: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let palette = pi_tui::theme::Palette::of(&pi_tui::theme::Theme::builtin(theme));
    let (lines, styles, spans) =
        // TS 的 `Markdown.render(width)` 内部按 `width - 2*paddingX` 排版（`markdown.ts` 的
        // `availableWidth`），外层再补 `paddingX` 空格 ⇒ 内容宽度是 `width - 2`。
        pi_tui::render_markdown_styled_with_width(
            input.trim(),
            &palette,
            Some(width.saturating_sub(2)),
        );
    let spans_by_line: std::collections::HashMap<usize, Vec<pi_tui::style::Span>> =
        spans.into_iter().collect();
    // 折行要在**纯文本**上做（`rendered` 里含 SGR，按显示宽度折会把转义序列算进宽度），
    // 与 `lib.rs` 的 Assistant 分支同口径：先 `wrap_to_width(line)`，再按段套 span。
    let content_width = width.saturating_sub(2);
    for (index, line) in lines.iter().enumerate() {
        let style = styles.get(index).copied().flatten();
        // 续行补回行首内边距（与 `lib.rs` 的 Assistant 分支同口径）。
        let lead: String = line.chars().take_while(|ch| *ch == ' ').collect();
        let mut segments = pi_tui::wrap_to_width(line, content_width + lead.len());
        for (index, segment) in segments.iter_mut().enumerate() {
            if index > 0 && !segment.starts_with(lead.as_str()) {
                segment.insert_str(0, &lead);
            }
        }
        let un_wrapped = segments.len() == 1;
        let line_spans_all = spans_by_line.get(&index).cloned().unwrap_or_default();
        let mut cursor = 0usize;
        for (segment_index, segment) in segments.iter().enumerate() {
            let _ = un_wrapped;
            let mut rendered = String::new();
            // 按段重映射 span（折行后仍然保留 `**粗体**`/行内代码的样式）。
            let body = if segment_index > 0 {
                segment.strip_prefix(lead.as_str()).unwrap_or(segment)
            } else {
                segment.as_str()
            };
            let start = line[cursor.min(line.len())..]
                .find(body)
                .map(|offset| cursor + offset)
                .unwrap_or(cursor);
            let end = start + body.len();
            cursor = end;
            let shift = segment.len() - body.len();
            let mapped: Vec<pi_tui::style::Span> = line_spans_all
                .iter()
                .filter_map(|span| {
                    let from = span.start.max(start);
                    let to = span.end.min(end);
                    if from >= to {
                        return None;
                    }
                    Some(pi_tui::style::Span::new(
                        from - start + shift,
                        to - start + shift,
                        span.style,
                    ))
                })
                .collect();
            if !mapped.is_empty() {
                pi_tui::diff::write_spans(&mut rendered, segment, &mapped, 0);
            }
            if rendered.is_empty() {
                rendered = match style {
                    Some(style) => format!("{}{}{}", style.prefix(), segment, style.reset()),
                    None => segment.clone(),
                };
            }
            let visible = pi_tui::diff::visible_width(&rendered);
            let padded = if visible < content_width {
                format!("{rendered}{}", " ".repeat(content_width - visible))
            } else {
                rendered
            };
            out.push(padded);
        }
    }
    out
}
