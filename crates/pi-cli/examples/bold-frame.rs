//! F58/B：把**一条助手 markdown 消息**单独渲染成带 SGR 的帧，供与 TS 侧逐行比对粗体。
//!
//! 用法：`cargo run -p pi-cli --example bold-frame -- <message.txt> <cols>`
//!
//! 输出：`diff_render` 的原始字节（含 `?2026h` 帧同步与每行 SGR），直接落 stdout。
//! 之所以不复用整帧渲染：整帧会带上头部/页脚/会话历史，粗体差异会被其他行淹没。

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("缺 <message.txt>")?;
    let cols: usize = args
        .next()
        .unwrap_or_else(|| "110".to_string())
        .parse()
        .map_err(|error| format!("cols 解析失败：{error}"))?;
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("读 {path} 失败：{error}"))?;

    let mut state = pi_tui::TuiState::new("0.87.1", "/tmp/work", "0.0");
    state.quiet_startup = true;
    state
        .transcript
        .push(pi_tui::TranscriptItem::Assistant(text));

    // 行数取足够大：这里只关心"这一条消息"的排版，不做视口裁剪。
    let rows = 2000;
    let mut cache = pi_tui::RenderCache::new();
    let frame = pi_tui::render_all_marker_cached_styled(&state, cols, rows, &mut cache);
    let mut diff = pi_tui::diff::DiffState::new();
    let (bytes, cursor) = pi_tui::diff::diff_render_with_cursor_styled(
        &mut diff,
        &frame.lines,
        &frame.styles,
        &frame.spans,
        cols,
        rows,
    )
    .map_err(|error| format!("渲染失败：{error}"))?;
    use std::io::Write as _;
    let mut out = std::io::stdout();
    out.write_all(bytes.as_bytes())
        .and_then(|()| out.write_all(cursor.as_bytes()))
        .map_err(|error| format!("写 stdout 失败：{error}"))?;
    Ok(())
}
