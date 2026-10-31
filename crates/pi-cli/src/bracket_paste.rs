//! 括号粘贴标记的**重组器**（Windows / ConPTY 专用修复）。
//!
//! ConPTY 会把终端回送的 `ESC[200~` / `ESC[201~` 拆成普通按键，甚至拆成多段、段间
//! 隔一次正文冲刷。这里按字节语义在按键层重组：`ESC [ 2 0 0 ~` 开始收集，
//! `ESC [ 2 0 1 ~` 结束；只有结尾残留时整体吞掉。
//!
//! **记账式重组（2026-10-31）**：旧实现"超时就丢前缀"，于是 `ESC[20` 被丢、`1~`
//! 落进正文（真机截图里的 `1~`）；多段拆分时还会整段 `200~` 漏出来。现在改成
//! **不按时间丢任何字节**：
//!
//! - 已匹配的前缀一直记着（`pending`），直到"下一个字符补不齐"或"遇到非字符键"；
//! - 补得齐 ⇒ 吞掉并继续；补满 ⇒ 按标记处理；
//! - 补不齐 ⇒ 把前缀（必要时带上暂扣的 `Esc`）**原样按正文交回**，不静默丢字；
//! - 时间只做两件事：`ESC_HOLD_MS` 后把孤立 `Esc` 放给编辑器；`PENDING_IDLE_MS`
//!   之后把长期没人理的残片按正文吐回。
//!
//! 纯状态机，不碰 I/O，可单测。

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
    /// 先把这段正文交给编辑器（标记残片按正文吐回），再处理这个按键。
    TextThen(String, BracketInput),
    /// 属于标记，已吞掉。
    Consumed,
    /// 收集到一段完整的括号粘贴内容。
    Paste(String),
    /// 暂扣的 `Esc` 到点放行（`flush` 返回）。
    EscReleased,
}

/// 暂扣的 `Esc` 多久后真的交给编辑器（宁可早放，也不让 Esc 失灵）。
pub const ESC_HOLD_MS: u64 = 40;

/// 残片最长记忆时间：超过它就把残片按正文吐回（正常情况下不会有这么长的静默）。
pub const PENDING_IDLE_MS: u64 = 5_000;

const START_TAIL: &str = "[200~";
const END_TAIL: &str = "[201~";

#[derive(Debug, Default)]
pub struct BracketPaste {
    /// 正在匹配的标记前缀：`(已匹配的字符, Esc 是否已经放给编辑器)`。
    ///
    /// `matched` 不含 `Esc` 本身（`""` / `"["` / `"[2"` / `"[20"` / `"[200"` …）。
    /// **不按时间丢弃**：只要下一个字符还能把它补成标记，就一直吞；补不齐才吐回正文。
    pending: Option<(String, bool)>,
    /// 暂扣的 `Esc` 时刻（到 `ESC_HOLD_MS` 放行；放行后 `pending` 仍然有效）。
    held_esc_at: Option<u64>,
    /// 最后一次活动（`Esc` 或补前缀）的时刻，用于 `PENDING_IDLE_MS`。
    last_at: u64,
    /// 收集中：正文 + 结束标记已匹配长度 + 结束标记是否刚见到 `Esc`。
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
                    // 收集中遇到其它按键：正文已收，按 Paste 交回，再处理该按键
                    let body = std::mem::take(text);
                    self.collecting = None;
                    if body.is_empty() {
                        return BracketOut::Pass(input);
                    }
                    return BracketOut::TextThen(body, input);
                }
            }
        }

        // ── 标记匹配中：记账式，不按时间丢字节 ────────────────────
        if let Some((matched, esc_emitted)) = self.pending.clone() {
            self.last_at = now_ms;
            match input {
                BracketInput::Char(ch) => {
                    let candidate = format!("{matched}{ch}");
                    if candidate == START_TAIL {
                        self.pending = None;
                        self.held_esc_at = None;
                        self.collecting = Some((String::new(), 0, false));
                        return BracketOut::Consumed;
                    }
                    if candidate == END_TAIL {
                        // 只有结尾残留：整体吞掉
                        self.pending = None;
                        self.held_esc_at = None;
                        return BracketOut::Consumed;
                    }
                    if START_TAIL.starts_with(candidate.as_str())
                        || END_TAIL.starts_with(candidate.as_str())
                    {
                        self.pending = Some((candidate, esc_emitted));
                        return BracketOut::Consumed;
                    }
                    // 补不齐：把（未放行的 Esc +）已匹配前缀 + 当前字符原样交回正文
                    self.pending = None;
                    self.held_esc_at = None;
                    let mut text = String::new();
                    if !esc_emitted {
                        text.push('\u{1b}');
                    }
                    text.push_str(&candidate);
                    return BracketOut::Paste(text);
                }
                BracketInput::Esc => {
                    // 前一段残片到此为止：按正文交回，再从新的 Esc 重新记账
                    self.held_esc_at = Some(now_ms);
                    self.pending = Some((String::new(), false));
                    if matched.is_empty() && esc_emitted {
                        return BracketOut::Consumed;
                    }
                    let mut text = String::new();
                    if !esc_emitted {
                        text.push('\u{1b}');
                    }
                    text.push_str(&matched);
                    if text.is_empty() {
                        return BracketOut::Consumed;
                    }
                    return BracketOut::Paste(text);
                }
                BracketInput::Other => {
                    // 非字符键：残片按正文交回，再把按键放过去
                    self.pending = None;
                    self.held_esc_at = None;
                    if matched.is_empty() {
                        return BracketOut::Pass(input);
                    }
                    let mut text = String::new();
                    if !esc_emitted {
                        text.push('\u{1b}');
                    }
                    text.push_str(&matched);
                    return BracketOut::TextThen(text, input);
                }
            }
        }

        // ── 空闲 ──────────────────────────────────────────────────
        match input {
            BracketInput::Esc => {
                self.held_esc_at = Some(now_ms);
                self.last_at = now_ms;
                self.pending = Some((String::new(), false));
                BracketOut::Consumed
            }
            BracketInput::Char(_) | BracketInput::Other => BracketOut::Pass(input),
        }
    }

    /// 定时调用：放行孤立的 `Esc`；把长期没人理的残片按正文吐回。
    pub fn flush(&mut self, now_ms: u64) -> Option<BracketOut> {
        if self.collecting.is_some() {
            return None;
        }
        let (matched, esc_emitted) = self.pending.clone()?;
        // 孤立 Esc：到点放行（放行后残片仍可继续补全，只是不再重复吐 Esc）
        if !esc_emitted
            && self
                .held_esc_at
                .is_some_and(|at| now_ms.saturating_sub(at) >= ESC_HOLD_MS)
        {
            self.held_esc_at = None;
            self.pending = Some((matched, true));
            return Some(BracketOut::EscReleased);
        }
        // 残片长期没人理：按正文吐回
        if now_ms.saturating_sub(self.last_at) > PENDING_IDLE_MS {
            self.pending = None;
            self.held_esc_at = None;
            if matched.is_empty() {
                return None;
            }
            let mut text = String::new();
            if !esc_emitted {
                text.push('\u{1b}');
            }
            text.push_str(&matched);
            return Some(BracketOut::Paste(text));
        }
        None
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
        // 记账式：暂扣的 Esc 也一并交回（不再丢 Esc，也不再丢 `[`）。
        assert_eq!(
            out[2],
            BracketOut::Paste("\u{1b}[A".to_string()),
            "{:?}",
            out[2]
        );
    }

    /// 记账式：`Esc` 500 ms 后才来 `[` —— 仍按标记起点处理（真机 600 ms 间隔也不该把
    /// `[200~` 漏进正文）。窗口不再按时间丢字节。
    #[test]
    fn late_bracket_after_escape_still_starts_a_marker() {
        let mut filter = BracketPaste::default();
        assert_eq!(filter.feed(BracketInput::Esc, 0), BracketOut::Consumed);
        assert_eq!(
            filter.feed(BracketInput::Char('['), 500),
            BracketOut::Consumed
        );
    }

    #[test]
    fn other_key_aborts_collection_without_swallowing_it() {
        let mut events = vec![(BracketInput::Esc, 0)];
        events.extend(chars("[200~abc", 1));
        events.push((BracketInput::Other, 20));
        let out = run(&events);
        // 正文按 Paste 交回，非字符键照样放过去（旧实现把已收的 `abc` 整段丢掉）
        assert_eq!(
            out.last(),
            Some(&BracketOut::TextThen(
                "abc".to_string(),
                BracketInput::Other
            ))
        );
    }
}

#[cfg(test)]
mod split_marker_matrix_tests {
    //! 夹具只保留**事件类型与间隔**（内容用 `PAYLOAD`），覆盖 ConPTY 逐字节送标记、
    //! 且两批之间被正文冲刷拉开 > `MARKER_WINDOW_MS` 的情形（真机 `1~` 残留）。
    use super::{BracketInput, BracketOut, BracketPaste, ESC_HOLD_MS};

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
        let gap = ESC_HOLD_MS + 1;
        let mut events = vec![(BracketInput::Esc, 0u64), (BracketInput::Char('['), 1)];
        events.extend(chars("200~PAYLOAD", gap, 1));
        events.extend(chars("[201~", gap + 20, 1));
        let (text, _out) = feed_all(&events);
        assert_eq!(text, "PAYLOAD", "分片前缀被当正文：{text:?}");
    }

    /// 真机形态：结束标记拆在 `Esc[20` 之后，间隔 > 窗口 ⇒ 旧实现漏出 `1~`。
    #[test]
    fn end_marker_split_after_20_does_not_leak_tail() {
        let gap = ESC_HOLD_MS + 1;
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

#[cfg(test)]
mod accounting_matrix_tests {
    //! 记账式重组的穷举矩阵（只含事件类型与间隔，内容用 PAYLOAD）。
    use super::{BracketInput, BracketOut, BracketPaste, PENDING_IDLE_MS};

    /// 把事件按时间喂进去，途中每 10 ms 驱动一次 `flush()`；返回收集到的正文与全部输出。
    fn drive(events: &[(BracketInput, u64)], extra_flush: &[u64]) -> (String, Vec<BracketOut>) {
        let mut paste = BracketPaste::default();
        let mut outs = Vec::new();
        let mut now = 0u64;
        for (input, at) in events {
            while now + 10 <= *at {
                now += 10;
                if let Some(out) = paste.flush(now) {
                    outs.push(out);
                }
            }
            // 显式 flush 点（模拟"正文冲刷把它拆开"）
            for at_flush in extra_flush {
                if *at_flush >= now && *at_flush < *at {
                    if let Some(out) = paste.flush(*at_flush) {
                        outs.push(out);
                    }
                }
            }
            outs.push(paste.feed(*input, *at));
        }
        let mut text = String::new();
        for out in &outs {
            match out {
                BracketOut::Paste(body) => text.push_str(body),
                BracketOut::TextThen(body, _) => text.push_str(body),
                _ => {}
            }
        }
        (text, outs)
    }

    fn payload_events(marker_bytes: &str, start_ms: u64, gap_ms: u64) -> Vec<(BracketInput, u64)> {
        let mut out = Vec::new();
        let mut at = start_ms;
        for ch in marker_bytes.chars() {
            if ch == '\u{1b}' {
                out.push((BracketInput::Esc, at));
            } else {
                out.push((BracketInput::Char(ch), at));
            }
            at += gap_ms;
        }
        out
    }

    /// 两段拆分：在 `ESC[200~PAYLOAD[201~` 的每个位置断开，后半段延后 `gap` 毫秒。
    #[test]
    fn two_segment_split_never_leaks_markers() {
        let full = "\u{1b}[200~PAYLOAD\u{1b}[201~";
        let chars: Vec<char> = full.chars().collect();
        for split in 0..=chars.len() {
            for gap in [0u64, 41, 121, 600] {
                let mut events = payload_events(&chars[..split].iter().collect::<String>(), 0, 1);
                let tail: String = chars[split..].iter().collect();
                events.extend(payload_events(&tail, split as u64 + gap, 1));
                let (text, _outs) = drive(&events, &[]);
                assert_eq!(
                    text, "PAYLOAD",
                    "split={split} gap={gap} 正文被污染：{text:?}"
                );
                assert!(
                    !text.contains("200~") && !text.contains("201~") && !text.contains("1~"),
                    "split={split} gap={gap} 泄漏标记残片：{text:?}"
                );
            }
        }
    }

    /// 三段拆分：`(i, j)` 全覆盖 × 四个间隔。
    #[test]
    fn three_segment_split_never_leaks_markers() {
        let full = "\u{1b}[200~PAYLOAD\u{1b}[201~";
        let chars: Vec<char> = full.chars().collect();
        for gap in [0u64, 41, 121, 600] {
            for i in 0..=chars.len() {
                for j in i..=chars.len() {
                    let first: String = chars[..i].iter().collect();
                    let second: String = chars[i..j].iter().collect();
                    let third: String = chars[j..].iter().collect();
                    let mut events = payload_events(&first, 0, 1);
                    events.extend(payload_events(&second, i as u64 + gap, 1));
                    events.extend(payload_events(
                        &third,
                        i as u64 + gap + second.chars().count() as u64 + gap,
                        1,
                    ));
                    let (text, _outs) = drive(&events, &[]);
                    assert_eq!(
                        text, "PAYLOAD",
                        "i={i} j={j} gap={gap} 正文被污染：{text:?}"
                    );
                }
            }
        }
    }

    /// 正文冲刷变体：在 `[200~` 之后、`[201~` 之前各插一次显式 `flush()`。
    #[test]
    fn body_flush_variants_do_not_break_the_paste() {
        for gap in [41u64, 121, 600] {
            for flush_at in [gap, gap * 2, gap * 3] {
                let events = payload_events("\u{1b}[200~PAYLOAD\u{1b}[201~", 0, gap);
                let (text, outs) = drive(&events, &[flush_at]);
                assert_eq!(text, "PAYLOAD", "gap={gap} flush@{flush_at}: {text:?}");
                let pastes = outs
                    .iter()
                    .filter(|out| matches!(out, BracketOut::Paste(_)))
                    .count();
                assert_eq!(pastes, 1, "gap={gap} flush@{flush_at}：应恰好一次 Paste");
            }
        }
    }

    /// 残留标记：只有结尾标记 ⇒ 整体吞掉；只有前缀 ⇒ 作正文交回且不含 `200~`/`201~`。
    #[test]
    fn residual_markers_do_not_leak() {
        let (text, _) = drive(&payload_events("\u{1b}[201~", 0, 1), &[]);
        assert!(text.is_empty(), "结尾残留应被吞掉：{text:?}");

        for residual in ["\u{1b}[20", "\u{1b}1~", "[201~"] {
            let (text, _) = drive(&payload_events(residual, 0, 1), &[]);
            assert!(
                !text.contains("200~") && !text.contains("201~"),
                "{residual:?} 漏出完整标记：{text:?}"
            );
        }
    }

    /// 超过 `PENDING_IDLE_MS` 没人理的残片按正文交回（不静默丢字）。
    #[test]
    fn stale_prefix_is_returned_as_text() {
        let events = payload_events("\u{1b}[20", 0, 1);
        let mut patched = events.clone();
        patched.push((BracketInput::Char('x'), PENDING_IDLE_MS + 100));
        let (text, _) = drive(&patched, &[]);
        assert!(text.contains("[20"), "陈旧前缀不应丢字：{text:?}");
        assert!(!text.contains("200~"), "{text:?}");
    }
}
