//! 终端协商状态机（**纯函数**移植，规格：`ref/pi-0.87.1/packages/tui/src/terminal.ts`）。
//!
//! **本批不接线**：启动/退出序列仍由 `pi-cli` 现有代码写；本模块只提供可单测的状态机。
//!
//! 覆盖：start 序列、kitty 应答识别与消费、DA1 应答降级（`modifyOtherKeys`）、
//! 分片聚合（150 ms，`terminal.ts:13`）与超时把缓冲当普通输入、kitty 状态时序、stop 序列。
//!
//! **不在范围内**：OSC 11 / DSR 996 的应答消费（消费点在 `tui.ts:1007/1084-1095`，
//! 属 `TuiMainScreen` 层）⇒ 登记为 P7-6x 另取 oracle。
//!
//! 设计：无定时器、无全局可变状态。时间以显式 `now_ms` 传入；`on_data` 返回"下次唤醒时刻"，
//! 调用方到期后调用 [`NegotiateState::on_timeout`]。
//!
//! **线程安全**：`NegotiateState` 是纯数据（`Send + Sync`），函数无共享状态。

/// 分片聚合超时（`terminal.ts:13` `KEYBOARD_PROTOCOL_RESPONSE_FRAGMENT_TIMEOUT_MS = 150`）。
pub const FRAGMENT_TIMEOUT_MS: u64 = 150;

/// 期望的 kitty 协议标志位（`terminal.ts:12` `DESIRED_KITTY_KEYBOARD_PROTOCOL_FLAGS = 7`）。
pub const DESIRED_KITTY_FLAGS: u32 = 7;

const SYNC_START: &str = "\u{1b}[?2004h";
const SYNC_STOP: &str = "\u{1b}[?2004l";
const KITTY_POP: &str = "\u{1b}[<u";
const MODIFY_OTHER_KEYS_ON: &str = "\u{1b}[>4;2m";
const MODIFY_OTHER_KEYS_OFF: &str = "\u{1b}[>4;0m";

/// 状态机的输出动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 原样交给输入处理器（`terminal.ts:226` `forwardInputSequence`）。
    Forward(String),
    /// 写字节到终端（应答处理引起的写出）。
    Write(&'static str),
    /// 应答被消费（不产生键事件）。
    Consume,
}

/// 协商状态（对应 ref `ProcessTerminal` 的相关字段）。
#[derive(Debug, Default, Clone)]
pub struct NegotiateState {
    /// 协商应答的前缀缓冲（`terminal.ts:284` `keyboardProtocolNegotiationBuffer`）。
    pending: String,
    kitty_active: bool,
    modify_other_keys: bool,
    pushed: bool,
}

impl NegotiateState {
    /// 新建状态（未 start）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// kitty 协议当前是否激活（对应 `keys.ts` 的 `isKittyProtocolActive`）。
    #[must_use]
    pub fn kitty_active(&self) -> bool {
        self.kitty_active
    }

    /// `modifyOtherKeys` 当前是否开启。
    #[must_use]
    pub fn modify_other_keys(&self) -> bool {
        self.modify_other_keys
    }

    /// `start()` 写出的序列（`terminal.ts:185` + `:14`，顺序：括号粘贴 → kitty push → 查询 → DA1 哨兵）。
    #[must_use]
    pub fn start_sequences(&mut self) -> String {
        self.pushed = true;
        format!("{SYNC_START}\u{1b}[>{DESIRED_KITTY_FLAGS}u\u{1b}[?u\u{1b}[c")
    }

    /// 喂入**一个序列事件**（`input.rs` 的输出单位）。返回本批动作与"下次唤醒时刻"。
    ///
    /// 依据 `terminal.ts:218`：ref 的协商判断挂在 `stdinBuffer.on("data", (sequence) => …)` 上，
    /// 输入单位是**已切好的序列**；150 ms（`terminal.ts:13`）聚合的是**序列拼接**
    /// （`keyboardProtocolNegotiationBuffer`，`:288-317`），不是字节。
    pub fn on_sequence(&mut self, seq: &str, now_ms: u64) -> (Vec<Action>, Option<u64>) {
        let mut actions = Vec::new();
        if self.pending.is_empty() {
            if self.consume_or_park(seq, now_ms, &mut actions) {
                return (actions, self.wake_for(now_ms));
            }
            actions.push(Action::Forward(seq.to_string()));
            return (actions, None);
        }
        // 已有前缀：拼接后重新判定
        let joined = format!("{}{}", self.pending, seq);
        self.pending.clear();
        if self.consume_or_park(&joined, now_ms, &mut actions) {
            return (actions, self.wake_for(now_ms));
        }
        // 不是应答、也不是应答前缀 ⇒ 之前缓存的"半个应答"按普通输入交出，再处理当前序列
        if !joined.is_empty() && joined != *seq {
            actions.push(Action::Forward(
                joined[..joined.len() - seq.len()].to_string(),
            ));
        }
        actions.push(Action::Forward(seq.to_string()));
        (actions, None)
    }

    /// 命中应答 ⇒ 消费并写状态；是前缀 ⇒ 挂起（返回"已处理"，由调用方读 `wake_for`）。
    fn consume_or_park(&mut self, seq: &str, _now_ms: u64, actions: &mut Vec<Action>) -> bool {
        if let Some(flags) = parse_kitty_flags(seq) {
            if flags != 0 {
                if self.modify_other_keys {
                    self.modify_other_keys = false;
                    actions.push(Action::Write(MODIFY_OTHER_KEYS_OFF));
                }
                self.kitty_active = true;
            } else {
                self.enable_modify_other_keys(actions);
            }
            actions.push(Action::Consume);
            return true;
        }
        if is_da1(seq) {
            if !self.kitty_active {
                self.enable_modify_other_keys(actions);
            }
            actions.push(Action::Consume);
            return true;
        }
        if is_negotiation_prefix(seq) {
            self.pending = seq.to_string();
            return true;
        }
        false
    }

    fn wake_for(&self, now_ms: u64) -> Option<u64> {
        if self.pending.is_empty() {
            None
        } else {
            Some(now_ms + FRAGMENT_TIMEOUT_MS)
        }
    }

    /// 超时：把残留缓冲**当普通输入**交出（`terminal.ts:331-340`）。
    pub fn on_timeout(&mut self) -> Vec<Action> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let rest = std::mem::take(&mut self.pending);
        vec![Action::Forward(rest)]
    }

    /// `stop()` 写出的序列（`terminal.ts:428` → `:434-440` → `:441`）。
    #[must_use]
    pub fn stop_sequences(&mut self) -> String {
        let mut out = String::from(SYNC_STOP);
        if self.pushed || self.kitty_active {
            out.push_str(KITTY_POP);
            self.pushed = false;
            self.kitty_active = false;
        }
        if self.modify_other_keys {
            out.push_str(MODIFY_OTHER_KEYS_OFF);
            self.modify_other_keys = false;
        }
        out
    }

    fn enable_modify_other_keys(&mut self, actions: &mut Vec<Action>) {
        if self.kitty_active || self.modify_other_keys {
            return;
        }
        self.modify_other_keys = true;
        actions.push(Action::Write(MODIFY_OTHER_KEYS_ON));
    }
}

/// `\x1b[?<digits>u` ⇒ flags（`terminal.ts` 的 `parseKeyboardProtocolNegotiationSequence`，kitty-flags 分支）。
fn parse_kitty_flags(seq: &str) -> Option<u32> {
    let body = seq.strip_prefix("\u{1b}[?")?.strip_suffix('u')?;
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    body.parse().ok()
}

/// `\x1b[?<p>;<p>c` ⇒ DA1（`terminal.ts` 同一解析函数的 DA 分支）。
fn is_da1(seq: &str) -> bool {
    let Some(body) = seq
        .strip_prefix("\u{1b}[?")
        .and_then(|s| s.strip_suffix('c'))
    else {
        return false;
    };
    !body.is_empty() && body.chars().all(|c| c.is_ascii_digit() || c == ';')
}

/// 是否是"协商应答的前缀"——**严格对齐** `terminal.ts:33-35`：
/// ```js
/// return sequence === "\x1b[" || /^\x1b\[\?[\d;]*$/.test(sequence);
/// ```
/// 注意：**裸 `\x1b` 不是前缀**（ref 会立刻把它当普通输入转发）；`\x1b[1;5A` 这类非协商 CSI 也不是。
fn is_negotiation_prefix(seq: &str) -> bool {
    if seq == "\u{1b}[" {
        return true;
    }
    let Some(rest) = seq.strip_prefix("\u{1b}[?") else {
        return false;
    };
    rest.chars().all(|c| c.is_ascii_digit() || c == ';')
}

#[cfg(test)]
mod esc_probe {
    use super::{Action, NegotiateState};

    #[test]
    fn lone_esc_is_forwarded_immediately() {
        let mut state = NegotiateState::new();
        let (actions, wake) = state.on_sequence("\u{1b}", 10);
        println!("actions={actions:?} wake={wake:?}");
        assert!(actions.contains(&Action::Forward("\u{1b}".to_string())));
        assert!(wake.is_none());
    }
}
