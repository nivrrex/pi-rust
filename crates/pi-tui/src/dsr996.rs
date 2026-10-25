//! DSR 996 / 997（终端彩色方案查询）的**纯函数**状态（P7-6af/D）。
//!
//! 规格：`ref/pi-0.87.1/packages/tui/src/` —
//! - 查询：`tui.ts:1453` 写 `\x1b[?996n`（`queryTerminalColorScheme`，`:1434` 起）
//! - 通知开关：`tui.ts:887-888` 写 `\x1b[?2031h`、`:936` 写 `\x1b[?2031l`
//! - 应答：`\x1b[?997;1n`（dark）/ `\x1b[?997;2n`（light）
//! - 消费：`tui.ts:1010` → `consumeTerminalColorSchemeReport`（`:1108-1118`）
//!   → `parseTerminalColorSchemeReport`（`terminal-colors.ts:67-73`），正则
//!   `^(?:\x1b\[\?997;(1|2)n)+$`（`terminal-colors.ts:29`）
//! - **无配额**：只要形状匹配就消费（与 OSC 11 不同）。
//!
//! **线程安全**：纯数据（无状态），`Send + Sync`。

/// DSR 996 查询（`tui.ts:1453`）。
pub const COLOR_SCHEME_QUERY: &str = "\u{1b}[?996n";
/// 彩色方案通知开启（`tui.ts:887-888`）。
pub const NOTIFICATIONS_ON: &str = "\u{1b}[?2031h";
/// 彩色方案通知关闭（`tui.ts:936`）。
pub const NOTIFICATIONS_OFF: &str = "\u{1b}[?2031l";

/// 终端彩色方案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    /// 深色（`997;1`）。
    Dark,
    /// 浅色（`997;2`）。
    Light,
}

/// 解析应答：`^\x1b\[\?997;(1|2)n$`（`terminal-colors.ts:29/67-73`）。
///
/// 注：ref 的正则允许**连续多条**拼接（`(?:\x1b\[\?997;(1|2)n)+`），取最后一次匹配；
/// 本实现按"一条一个序列"的现实场景实现，遇到多条时取最后一个 `997;<n>` 片段。
#[must_use]
pub fn parse_report(seq: &str) -> Option<ColorScheme> {
    let mut scheme = None;
    let mut rest = seq;
    while !rest.is_empty() {
        let tail = rest.strip_prefix("\u{1b}[?997;")?;
        let mut chars = tail.chars();
        let value = match chars.next() {
            Some('1') => ColorScheme::Dark,
            Some('2') => ColorScheme::Light,
            other => {
                let _ = other;
                return None;
            }
        };
        if chars.next() != Some('n') {
            return None;
        }
        scheme = Some(value);
        rest = chars.as_str();
    }
    scheme
}

/// 喂入一个序列：命中 997 报告 ⇒ 返回方案（调用方应**消费**，不转发）；否则 `None`。
#[must_use]
pub fn on_sequence(seq: &str) -> Option<ColorScheme> {
    parse_report(seq)
}
