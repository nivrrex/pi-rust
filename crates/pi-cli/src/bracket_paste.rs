//! 括号粘贴标记的**重组器**（Windows / ConPTY 专用修复）。
//!
//! 真机取证（WezTerm + Rime，2026-10-29，`PI_TUI_EVENT_DUMP` 日志）：
//! 启动时我们发了 `ESC[?2004h`，但 ConPTY 把终端回送的标记**拆成了普通按键**：
//!
//! ```text
//! 27564ms  Key(Esc, Press)
//! 27566ms  Char('[')  27568ms Char('2')  27569ms Char('0')
//! 27571ms  Char('1')  27573ms Char('~')
//! ```
//!
//! 于是 crossterm 永远不会给出 `Event::Paste`，正文被拆成一串按键，残留的
//! `[201~` 变成垃圾输入，开头的裸 `Esc` 还会打乱编辑器状态。
//!
//! 这里按字节语义在按键层把它们重组：`Esc [ 2 0 0 ~` 开始收集，
//! `Esc [ 2 0 1 ~` 结束；孤立的 `[201~`（只有结尾残留）整体吞掉。
//!
//! 纯状态机，不碰 I/O，可单测。

/// 本模块关心的输入形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BracketInput {
    /// 一个 `Esc` 键（Press）。
    Esc,
    /// 一个字符键。
    Char(char),
    /// 其它按键（方向键、功能键、修饰键……）——原样放过。
    Other,
}

/// 状态机输出。
#[derive(Debug, PartialEq, Eq)]
pub enum BracketOut {
    /// 与粘贴标记无关：按原逻辑处理（`Esc` 可能已被暂扣）。
    Pass(BracketInput),
    /// 属于标记，已吞掉。
    Consumed,
    /// 收集到一段完整的括号粘贴内容。
    Paste(String),
    /// 暂扣的 `Esc` 到点放行（`flush` 返回）。
    EscReleased,
}

/// 标记匹配窗口：`Esc` 之后多少毫秒内到达的字符仍算同一标记。
const MARKER_WINDOW_MS: u64 = 120;

/// 单独按下的 `Esc` 什么时候真的交给上层。
pub const ESC_HOLD_MS: u64 = 40;

const START_TAIL: &str = "[200~";
const END_TAIL: &str = "[201~";

#[derive(Debug, Default)]
pub struct BracketPaste {
    /// 已暂扣、尚未决定归属的 `Esc`（时间戳）。
    held_esc_at: Option<u64>,
    /// `Esc` 之后已匹配的字符（"[" + "20x~" 的前缀）。
    seq: String,
    /// 收集中：正文 + 结束标记已匹配长度 + 是否刚见到 `Esc`。
    collecting: Option<(String, usize, bool)>,
}

impl BracketPaste {
    pub fn feed(&mut self, input: BracketInput, now_ms: u64) -> BracketOut {
        // ── 收集中 ────────────────────────────────────────────────
        if let Some((text, matched, esc_pending)) = self.collecting.as_mut() {
            match input {
                BracketInput::Esc => {
                    // 结束标记的起点：先挂起
                    if *matched > 0 {
                        for byte in END_TAIL.as_bytes().iter().take(*matched) {
                            text.push(*byte as char);
                        }
                        *matched = 0;
                    }
                    *esc_pending = true;
                    return BracketOut::Consumed;
                }
                BracketInput::Char(ch) => {
                    if *esc_pending {
                        *esc_pending = false;
                        if ch == '[' {
                            *matched = 1;
                            return BracketOut::Consumed;
                        }
                        text.push('\u{1b}');
                        text.push(ch);
                        return BracketOut::Consumed;
                    }
                    if *matched == 0 && ch == '[' {
                        *matched = 1;
                        return BracketOut::Consumed;
                    }
                    if *matched > 0 && ch == END_TAIL.as_bytes()[*matched] as char {
                        *matched += 1;
                        if *matched == END_TAIL.len() {
                            let body = std::mem::take(text);
                            self.collecting = None;
                            return BracketOut::Paste(body);
                        }
                        return BracketOut::Consumed;
                    }
                    if *matched > 0 {
                        for byte in END_TAIL.as_bytes().iter().take(*matched) {
                            text.push(*byte as char);
                        }
                        *matched = 0;
                    }
                    text.push(ch);
                    return BracketOut::Consumed;
                }
                BracketInput::Other => {
                    // 收集中遇到其它按键：放弃这次收集（不把按键吞进正文）
                    self.collecting = None;
                    return BracketOut::Pass(input);
                }
            }
        }
        // ── 标记匹配中 ────────────────────────────────────────────
        if !self.seq.is_empty() {
            let fresh = self
                .held_esc_at
                .is_some_and(|at| now_ms.saturating_sub(at) <= MARKER_WINDOW_MS);
            if fresh {
                match input {
                    BracketInput::Char(ch) => {
                        self.seq.push(ch);
                        if self.seq == START_TAIL {
                            self.seq.clear();
                            self.held_esc_at = None;
                            self.collecting = Some((String::new(), 0, false));
                            return BracketOut::Consumed;
                        }
                        if self.seq == END_TAIL {
                            // 只有结尾残留（起始标记在窗口外/被吞）：整体丢弃
                            self.seq.clear();
                            self.held_esc_at = None;
                            return BracketOut::Consumed;
                        }
                        if START_TAIL.starts_with(self.seq.as_str())
                            || END_TAIL.starts_with(self.seq.as_str())
                        {
                            return BracketOut::Consumed;
                        }
                        // 不是标记：把暂扣的 Esc 还原给上层，然后放行这个字符
                        self.seq.clear();
                        self.held_esc_at = None;
                        return BracketOut::EscReleased;
                    }
                    BracketInput::Esc => {
                        self.seq.clear();
                        self.held_esc_at = Some(now_ms);
                        return BracketOut::Consumed;
                    }
                    BracketInput::Other => {
                        self.seq.clear();
                        self.held_esc_at = None;
                        return BracketOut::EscReleased;
                    }
                }
            }
            self.seq.clear();
            self.held_esc_at = None;
        }
        // ── 空闲 ──────────────────────────────────────────────────
        match input {
            BracketInput::Esc => {
                self.held_esc_at = Some(now_ms);
                self.seq.clear();
                BracketOut::Consumed
            }
            BracketInput::Char('[') => {
                if self
                    .held_esc_at
                    .is_some_and(|at| now_ms.saturating_sub(at) <= MARKER_WINDOW_MS)
                {
                    self.seq.push('[');
                    BracketOut::Consumed
                } else {
                    BracketOut::Pass(input)
                }
            }
            BracketInput::Char(_) | BracketInput::Other => {
                self.held_esc_at = None;
                BracketOut::Pass(input)
            }
        }
    }

    /// 定时调用：暂扣的 `Esc` 超时未成标记 ⇒ 放行。
    pub fn flush(&mut self, now_ms: u64) -> Option<BracketOut> {
        if self.collecting.is_some() {
            return None;
        }
        // 残缺的标记：超时后整段丢弃，避免把 `[20` 之类吐进正文
        if !self.seq.is_empty()
            && self
                .held_esc_at
                .is_some_and(|at| now_ms.saturating_sub(at) > MARKER_WINDOW_MS)
        {
            self.seq.clear();
            self.held_esc_at = None;
            return None;
        }
        match self.held_esc_at {
            Some(at) if now_ms.saturating_sub(at) >= ESC_HOLD_MS && self.seq.is_empty() => {
                self.held_esc_at = None;
                Some(BracketOut::EscReleased)
            }
            Some(_) | None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(events: &[(BracketInput, u64)]) -> Vec<BracketOut> {
        let mut filter = BracketPaste::default();
        events
            .iter()
            .map(|(input, at)| filter.feed(*input, *at))
            .collect()
    }

    fn chars(text: &str, start_ms: u64) -> Vec<(BracketInput, u64)> {
        text.chars()
            .enumerate()
            .map(|(index, ch)| (BracketInput::Char(ch), start_ms + index as u64))
            .collect()
    }

    #[test]
    fn rejoins_marker_split_into_keys() {
        let mut events = vec![(BracketInput::Esc, 0)];
        events.extend(chars("[200~hello\nworld", 1));
        events.push((BracketInput::Esc, 40));
        events.extend(chars("[201~", 41));
        let out = run(&events);
        // 只有最后一条产出 Paste，其余全部 Consumed
        assert_eq!(
            out.last(),
            Some(&BracketOut::Paste("hello\nworld".to_string()))
        );
        assert!(out[..out.len() - 1]
            .iter()
            .all(|item| *item == BracketOut::Consumed));
    }

    #[test]
    fn lone_escape_is_passed_through_after_hold() {
        let mut filter = BracketPaste::default();
        // Esc 先被暂扣（Consumed），没有 `[` 跟随
        assert_eq!(filter.feed(BracketInput::Esc, 0), BracketOut::Consumed);
        assert_eq!(filter.flush(10), None, "未到 40ms 不放行");
        assert_eq!(filter.flush(ESC_HOLD_MS + 1), Some(BracketOut::EscReleased));
    }

    #[test]
    fn escape_then_unrelated_bracket_does_not_lose_typing() {
        // `Esc [` 之后不是 "200~" ⇒ 后续字符必须继续作为普通输入（不丢字）
        let out = run(&[
            (BracketInput::Esc, 0),
            (BracketInput::Char('['), 3),
            (BracketInput::Char('A'), 6),
        ]);
        assert_eq!(out[0], BracketOut::Consumed, "Esc 被暂扣");
        assert_eq!(out[1], BracketOut::Consumed, "`[` 进入标记匹配");
        // 第三个字符不构成标记 ⇒ 先把暂扣的 Esc 放出去，再把该字符作为普通输入
        assert_eq!(out[2], BracketOut::EscReleased, "{:?}", out[2]);
    }

    #[test]
    fn stale_escape_does_not_start_a_marker() {
        let mut filter = BracketPaste::default();
        assert_eq!(filter.feed(BracketInput::Esc, 0), BracketOut::Consumed);
        assert_eq!(
            filter.feed(BracketInput::Char('['), 500),
            BracketOut::Pass(BracketInput::Char('['))
        );
    }

    #[test]
    fn other_key_aborts_collection_without_swallowing_it() {
        let mut events = vec![(BracketInput::Esc, 0)];
        events.extend(chars("[200~abc", 1));
        events.push((BracketInput::Other, 20));
        let out = run(&events);
        assert_eq!(out.last(), Some(&BracketOut::Pass(BracketInput::Other)));
    }
}
