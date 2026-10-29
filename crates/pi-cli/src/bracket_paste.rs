//! 括号粘贴标记的**重组器**（Windows / ConPTY 专用修复）。
//!
//! 真机取证（WezTerm + Rime，2026-10-29，`PI_TUI_EVENT_DUMP` 日志）：
//! 我们启动时确实发了 `ESC[?2004h`，但 ConPTY 把终端回送的括号粘贴标记
//! **拆成了普通按键**——日志里能看到
//!
//! ```text
//! 27564ms  Key(Esc, Press)
//! 27566ms  Char('[') 27568ms Char('2') 27569ms Char('0') 27571ms Char('1')
//! 27573ms  Char('~')
//! ```
//!
//! 也就是说 `ESC[201~` 被当成了「Esc + `[201~` 五个字符」，crossterm 永远不会
//! 给出 `Event::Paste`。后果正是真机报告的三条现象：粘贴内容被拆成一串按键
//! （折叠成缩略图与否取决于字符间隔）、残留的 `[201~` 变成垃圾输入、以及
//! 开头那个 `Esc` 把编辑器状态搞乱。
//!
//! 这里按字节语义在**按键层**把它们重组回来：识别 `Esc [ 2 0 0 ~` 开始收集、
//! 遇到 `Esc [ 2 0 1 ~` 结束，中间的内容作为一段文本输出。
//!
//! 纯状态机，不碰 I/O，可单测。

/// 输入事件里本模块关心的三种形态。
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
    /// 该事件与粘贴标记无关，按原逻辑处理。
    Pass(BracketInput),
    /// 该事件属于标记，已吞掉（不产生任何输入）。
    Consumed,
    /// 收集到一段完整的括号粘贴内容。
    Paste(String),
}

/// 标记匹配窗口：`Esc` 之后多少毫秒内到达的字符仍算同一标记。
const MARKER_WINDOW_MS: u64 = 120;

/// 单独的 `Esc` 什么时候才真的交给上层（等这么久没等到 `[` 就放行）。
pub const ESC_HOLD_MS: u64 = 40;

/// 标记起始 `ESC[200~` 与结束 `ESC[201~` 的其余部分（不含 Esc）。
const START_TAIL: &str = "[200~";
const END_TAIL: &str = "[201~";

#[derive(Debug, Default)]
pub struct BracketPaste {
    /// 刚看到 Esc 的时间戳（标记可能从下一个字符开始）。
    esc_at: Option<u64>,
    /// 已挂起、尚未决定归属的 Esc（`Esc` 被暂扣，等 `[` 或超时）。
    held_esc_at: Option<u64>,
    /// 已匹配的 `START_TAIL` 前缀长度。
    start_matched: usize,
    /// 收集中：正文 + 已匹配的 `END_TAIL` 前缀长度 + 「刚见到 Esc」。
    collecting: Option<(String, usize, bool)>,
}

impl BracketPaste {
    /// 喂一个事件（带毫秒时间戳），返回该事件的处理结果。
    pub fn feed(&mut self, input: BracketInput, now_ms: u64) -> BracketOut {
        // 收集中：只关心正文与结束标记。
        if let Some((text, matched, esc_pending)) = self.collecting.as_mut() {
            match input {
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
                            let _ = matched;
                            self.collecting = None;
                            return BracketOut::Paste(body);
                        }
                        return BracketOut::Consumed;
                    }
                    // 不是结束标记：把已匹配的前缀当作正文的一部分
                    if *matched > 0 {
                        for byte in END_TAIL.as_bytes().iter().take(*matched) {
                            text.push(*byte as char);
                        }
                        *matched = 0;
                    }
                    text.push(ch);
                    return BracketOut::Consumed;
                }
                BracketInput::Esc => {
                    // Esc 可能是结束标记的起点：先挂起，下一个字符再判定
                    if let Some(state) = self.collecting.as_mut() {
                        if state.1 > 0 {
                            for byte in END_TAIL.as_bytes().iter().take(state.1) {
                                state.0.push(*byte as char);
                            }
                            state.1 = 0;
                        }
                        state.2 = true;
                    }
                    return BracketOut::Consumed;
                }
                BracketInput::Other => {
                    // 收集中的其它按键：丢弃这段不完整的收集（不把按键吞进正文）
                    self.collecting = None;
                    return BracketOut::Pass(input);
                }
            }
        }
        // 起始标记匹配中（Esc 之后）。
        if self.start_matched > 0 {
            let fresh = self
                .esc_at
                .is_some_and(|at| now_ms.saturating_sub(at) <= MARKER_WINDOW_MS);
            if fresh {
                match input {
                    BracketInput::Char(ch) => {
                        if ch == START_TAIL.as_bytes()[self.start_matched] as char {
                            self.start_matched += 1;
                            if self.start_matched == START_TAIL.len() {
                                self.start_matched = 0;
                                self.esc_at = None;
                                self.collecting = Some((String::new(), 0, false));
                            }
                            return BracketOut::Consumed;
                        }
                        // 匹配失败：把已匹配的部分当普通字符还原，然后处理当前字符
                        let mut out = String::new();
                        for byte in START_TAIL.as_bytes().iter().take(self.start_matched) {
                            out.push(*byte as char);
                        }
                        self.start_matched = 0;
                        self.esc_at = None;
                        // 已吞掉的 Esc/`[` 等按原样还给上层（作为普通输入）
                        return if out.is_empty() {
                            BracketOut::Pass(input)
                        } else {
                            BracketOut::Pass(BracketInput::Char('\u{fffd}'))
                        };
                    }
                    BracketInput::Esc => {
                        self.start_matched = 0;
                        self.esc_at = Some(now_ms);
                        return BracketOut::Pass(input);
                    }
                    BracketInput::Other => {
                        self.start_matched = 0;
                        self.esc_at = None;
                        return BracketOut::Pass(input);
                    }
                }
            }
            self.start_matched = 0;
            self.esc_at = None;
        }
        match input {
            BracketInput::Esc => {
                // 先暂扣：真要是单独按下，`flush` 会在 `ESC_HOLD_MS` 后放行。
                self.esc_at = Some(now_ms);
                self.held_esc_at = Some(now_ms);
                BracketOut::Consumed
            }
            BracketInput::Char('[') => {
                if self
                    .esc_at
                    .is_some_and(|at| now_ms.saturating_sub(at) <= MARKER_WINDOW_MS)
                {
                    self.start_matched = 1;
                    BracketOut::Consumed
                } else {
                    BracketOut::Pass(input)
                }
            }
            BracketInput::Char(_) | BracketInput::Other => {
                self.esc_at = None;
                BracketOut::Pass(input)
            }
        }
    }
}

impl BracketPaste {
    /// 定时调用：若暂扣的 `Esc` 已超时且没等到 `[`，把它放行给上层。
    ///
    /// 这样「单独按 Esc」不会被吞掉，而「`Esc [` 出现在标记里」会被识别。
    pub fn flush(&mut self, now_ms: u64) -> Option<BracketOut> {
        if self.collecting.is_some() {
            return None;
        }
        if self.start_matched > 0 {
            // 正在匹配 `[200~`，超时则整段丢弃（标记残缺，不吐垃圾字符）
            if self
                .esc_at
                .is_some_and(|at| now_ms.saturating_sub(at) > MARKER_WINDOW_MS)
            {
                self.start_matched = 0;
                self.esc_at = None;
                self.held_esc_at = None;
            }
            return None;
        }
        match self.held_esc_at {
            Some(at) if now_ms.saturating_sub(at) >= ESC_HOLD_MS => {
                self.held_esc_at = None;
                self.esc_at = None;
                Some(BracketOut::Pass(BracketInput::Esc))
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
        assert_eq!(
            filter.flush(ESC_HOLD_MS + 1),
            Some(BracketOut::Pass(BracketInput::Esc))
        );
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
        assert!(matches!(out[2], BracketOut::Pass(_)), "{:?}", out[2]);
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
