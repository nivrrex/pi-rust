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
/// 已匹配的标记前缀，多久没有新字符才放弃（毫秒）。
///
/// 必须远大于 `MARKER_WINDOW_MS`：真机上 `ESC[20` 与 `1~` 之间常隔一次正文冲刷
/// （> 120 ms），按 Esc 时刻算窗口会把前缀丢掉、把尾巴 `1~` 漏进正文。
const SEQ_IDLE_MS: u64 = 500;

#[derive(Debug, Default)]
pub struct BracketPaste {
    /// 已暂扣、尚未决定归属的 `Esc`（时间戳）。
    held_esc_at: Option<u64>,
    /// 刚把 `Esc` 放给编辑器的时刻：其后 `MARKER_WINDOW_MS` 内若出现 `[`，
    /// 仍可能是"迟到的标记起点"。
    released_esc_at: Option<u64>,
    /// `Esc` 之后已匹配的字符（"[" + "20x~" 的前缀）。
    seq: String,
    /// `seq` 最后一次变化的时刻：窗口按"最后一次匹配"算，而不是按 `Esc` 的时刻算。
    seq_at: u64,
    /// 收集中：正文 + 结束标记已匹配长度 + 是否刚见到 `Esc`。
    collecting: Option<(String, usize, bool)>,
}

impl BracketPaste {
    pub fn feed(&mut self, input: BracketInput, now_ms: u64) -> BracketOut {
        // ── 收集中 ────────────────────────────────────────────────
        if let Some((text, matched, esc_pending)) = self.collecting.as_mut() {
            match input {
                BracketInput::Esc => {
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
        // 窗口按**最后一次匹配**算（`seq_at`）：正文冲刷造成的大间隔不会把已匹配的
        // 前缀丢掉、把剩下的 `1~` 漏成正文。
        if !self.seq.is_empty() {
            if now_ms.saturating_sub(self.seq_at) > SEQ_IDLE_MS {
                let prefix = std::mem::take(&mut self.seq);
                self.held_esc_at = None;
                self.released_esc_at = None;
                // 明显不是标记：按正文吐回，不静默丢字
                return BracketOut::Paste(prefix);
            }
            match input {
                BracketInput::Char(ch) => {
                    self.seq.push(ch);
                    self.seq_at = now_ms;
                    if self.seq == START_TAIL || self.seq == END_TAIL {
                        let start = self.seq == START_TAIL;
                        self.seq.clear();
                        self.held_esc_at = None;
                        self.released_esc_at = None;
                        if start {
                            self.collecting = Some((String::new(), 0, false));
                        }
                        return BracketOut::Consumed;
                    }
                    if START_TAIL.starts_with(self.seq.as_str())
                        || END_TAIL.starts_with(self.seq.as_str())
                    {
                        return BracketOut::Consumed;
                    }
                    // 不是标记：把已匹配的部分（含当前字符）按正文还给编辑器
                    let text = std::mem::take(&mut self.seq);
                    self.held_esc_at = None;
                    self.released_esc_at = None;
                    return BracketOut::Paste(text);
                }
                BracketInput::Esc => {
                    self.seq.clear();
                    self.seq_at = now_ms;
                    self.held_esc_at = Some(now_ms);
                    self.released_esc_at = None;
                    return BracketOut::Consumed;
                }
                BracketInput::Other => {
                    self.seq.clear();
                    self.held_esc_at = None;
                    self.released_esc_at = None;
                    return BracketOut::Pass(input);
                }
            }
        }

        // ── 空闲 ──────────────────────────────────────────────────
        match input {
            BracketInput::Esc => {
                self.held_esc_at = Some(now_ms);
                self.seq.clear();
                self.seq_at = now_ms;
                BracketOut::Consumed
            }
            BracketInput::Char('[') => {
                let fresh_hold = self
                    .held_esc_at
                    .is_some_and(|at| now_ms.saturating_sub(at) <= MARKER_WINDOW_MS);
                // Esc 已被放行（> ESC_HOLD_MS），但 `[` 在窗口内赶到：仍按标记起点处理，
                // 否则整段 `[200~…` 会作为正文进编辑器。
                let fresh_release = self
                    .released_esc_at
                    .is_some_and(|at| now_ms.saturating_sub(at) <= MARKER_WINDOW_MS);
                if fresh_hold || fresh_release {
                    self.seq.clear();
                    self.seq.push('[');
                    self.seq_at = now_ms;
                    self.held_esc_at = None;
                    self.released_esc_at = None;
                    BracketOut::Consumed
                } else {
                    BracketOut::Pass(input)
                }
            }
            BracketInput::Char(_) | BracketInput::Other => {
                self.held_esc_at = None;
                self.released_esc_at = None;
                BracketOut::Pass(input)
            }
        }
    }

    /// 定时调用：暂扣的 `Esc` 超时未成标记 ⇒ 放行；陈旧前缀 ⇒ 按正文吐回。
    pub fn flush(&mut self, now_ms: u64) -> Option<BracketOut> {
        if self.collecting.is_some() {
            return None;
        }
        if !self.seq.is_empty() {
            if now_ms.saturating_sub(self.seq_at) > SEQ_IDLE_MS {
                let prefix = std::mem::take(&mut self.seq);
                self.held_esc_at = None;
                self.released_esc_at = None;
                return Some(BracketOut::Paste(prefix));
            }
            return None;
        }
        match self.held_esc_at {
            Some(at) if now_ms.saturating_sub(at) >= ESC_HOLD_MS => {
                self.held_esc_at = None;
                self.released_esc_at = Some(now_ms);
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
        // 第三个字符不构成标记 ⇒ 已匹配的 `[A` 必须**原样交回正文**（旧实现会把它丢掉、
        // 只返回 EscReleased，属于静默丢字）。Esc 本身由 `flush` 在 40 ms 后放行。
        assert_eq!(out[2], BracketOut::Paste("[A".to_string()), "{:?}", out[2]);
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

#[cfg(test)]
mod split_marker_matrix_tests {
    //! 夹具只保留**事件类型与间隔**（内容用 `PAYLOAD`），覆盖 ConPTY 逐字节送标记、
    //! 且两批之间被正文冲刷拉开 > `MARKER_WINDOW_MS` 的情形（真机 `1~` 残留）。
    use super::{BracketInput, BracketOut, BracketPaste, ESC_HOLD_MS, MARKER_WINDOW_MS};

    fn feed_all(events: &[(BracketInput, u64)]) -> (String, Vec<BracketOut>) {
        let mut paste = BracketPaste::default();
        let mut outcomes = Vec::new();
        let mut now = 0u64;
        for (input, at) in events {
            // 模拟主循环的定时驱动：每步之间按 10 ms 粒度 flush
            while now + 10 <= *at {
                now += 10;
                if let Some(out) = paste.flush(now) {
                    outcomes.push(out);
                }
            }
            outcomes.push(paste.feed(*input, *at));
        }
        let mut text = String::new();
        for outcome in &outcomes {
            if let BracketOut::Paste(body) = outcome {
                text.push_str(body);
            }
        }
        (text, outcomes)
    }

    fn chars(text: &str, start: u64, gap: u64) -> Vec<(BracketInput, u64)> {
        text.chars()
            .enumerate()
            .map(|(index, ch)| (BracketInput::Char(ch), start + index as u64 * gap))
            .collect()
    }

    /// 起始标记拆在 `Esc` 与其余之间：中间隔 41 / 119 / 121 ms（40 与 120 的两侧）。
    #[test]
    fn start_marker_split_after_esc_does_not_leak() {
        for gap in [41u64, 119, 121] {
            let mut events = vec![(BracketInput::Esc, 0u64)];
            events.extend(chars("[200~PAYLOAD[201~", gap, 1));
            // 结尾标记也一起喂，避免测试把它的字符算进正文
            let (text, _out) = feed_all(&events);
            assert_eq!(text, "PAYLOAD", "gap={gap} 时正文被污染：{text:?}");
        }
    }

    /// 起始标记拆在 `Esc[` 之后（`200~PAYLOAD` 会整段落进正文 —— Linux 侧矩阵的第二种泄漏）。
    #[test]
    fn start_marker_split_after_bracket_does_not_leak() {
        let gap = MARKER_WINDOW_MS + 1;
        let mut events = vec![(BracketInput::Esc, 0u64), (BracketInput::Char('['), 1)];
        events.extend(chars("200~PAYLOAD", gap, 1));
        events.extend(chars("[201~", gap + 20, 1));
        let (text, _out) = feed_all(&events);
        assert_eq!(text, "PAYLOAD", "分片前缀被当正文：{text:?}");
    }

    /// 真机形态：结束标记拆在 `Esc[20` 之后，间隔 > 窗口 ⇒ 旧实现漏出 `1~`。
    #[test]
    fn end_marker_split_after_20_does_not_leak_tail() {
        let gap = MARKER_WINDOW_MS + 1;
        // 正文走括号收集
        let mut events = vec![(BracketInput::Esc, 0u64)];
        events.extend(chars("[200~PAYLOAD", 1, 1));
        let split_at = 1 + 11u64; // "[200~PAYLOAD" 共 11 个字符
        events.push((BracketInput::Esc, split_at));
        events.extend(chars("[20", split_at + 1, 1));
        events.extend(chars("1~", split_at + 1 + 3 + gap, 1));
        let (text, outcomes) = feed_all(&events);
        assert_eq!(text, "PAYLOAD", "尾部分片落进正文：{text:?}");
        let pastes = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BracketOut::Paste(_)))
            .count();
        assert_eq!(pastes, 1, "应恰好产生一次 Paste");
        let holds = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BracketOut::EscReleased))
            .count();
        assert_eq!(holds, 0, "标记拆分不应把 Esc 放给编辑器");
    }

    /// 对照：先是一个**真** Esc 键（40 ms 后放行），随后才是完整标记 ⇒ 只看正文是否干净。
    #[test]
    fn lone_esc_then_marker_still_delivers_payload() {
        let mut events = vec![(BracketInput::Esc, 0u64)];
        events.extend(chars("[200~PAYLOAD[201~", ESC_HOLD_MS + 1, 1));
        let (text, _out) = feed_all(&events);
        assert_eq!(text, "PAYLOAD", "{text:?}");
    }
}
