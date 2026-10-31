//! OSC 11（终端背景色查询）与 DSR 996（彩色方案查询）的**纯函数**状态（本批不接线）。
//!
//! 规格来源：`ref/pi-0.87.1/packages/tui/src/tui.ts`
//! - 查询：`queryTerminalBackgroundColor` `:1406-1428`（写 `\x1b]11;?\x07` 于 `:1427`）
//! - 配额：`pendingOsc11BackgroundReplies += 1`（`:1426`）
//! - 消费：`consumeOsc11BackgroundResponse` `:1084-1095` —— **必须先有配额**（`:1085`），
//!   命中后 `pendingOsc11BackgroundReplies -= 1`（`:1094`）
//! - 调用点：`handleTerminalInput` `:1007`（在按键分派之前）
//!
//! **与 `term_negotiate` 是两个独立状态机**：kitty/DA1 在 `ProcessTerminal` 层（`terminal.ts:218`），
//! OSC 11 在 `TuiMainScreen` 层且依赖配额（P7-6y 裁决）。
//!
//! **线程安全**：纯数据，`Send + Sync`。

/// OSC 11 查询序列（`tui.ts:1427`）。
pub const OSC11_QUERY: &str = "\u{1b}]11;?\u{7}";
/// DSR 996 彩色方案查询序列（`tui.ts:1434` 起，`CSI ? 996 n`）。
pub const COLOR_SCHEME_QUERY: &str = "\u{1b}[?996n";

/// 解析出的 RGB。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    /// 红。
    pub r: u8,
    /// 绿。
    pub g: u8,
    /// 蓝。
    pub b: u8,
}

/// 一次序列喂入的结果（**区分"被消费"与"解析成功"**）。
///
/// ref 的语义是：形状匹配即消费（配额 -1），解析失败只让 Promise 得到 `undefined`
/// —— 见 `terminal-colors.ts:28`（形状正则不校验载荷）、`tui.ts:1084-1095`（形状通过就消费）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Osc11Outcome {
    /// 已被本状态消费（配额已 -1）；内含解析出的颜色，解析失败为 `None`。
    Consumed(Option<Rgb>),
    /// 未命中（无配额，或不是 OSC 11 应答形状）⇒ 调用方应继续按普通输入分派。
    NotConsumed,
}

/// OSC 11 的**已发查询配额**状态（对应 ref 的 `pendingOsc11BackgroundReplies`）。
#[derive(Debug, Default, Clone)]
pub struct Osc11State {
    pending: u32,
}

impl Osc11State {
    /// 新建（无配额）。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前配额。
    #[must_use]
    pub fn pending(&self) -> u32 {
        self.pending
    }

    /// 发出一次背景色查询：返回要写出的字节并把配额 +1（`tui.ts:1425-1427`）。
    pub fn query_background(&mut self) -> &'static str {
        self.pending = self.pending.saturating_add(1);
        OSC11_QUERY
    }

    /// 发出一次彩色方案查询（`tui.ts:1434` 的 DSR）。
    #[must_use]
    pub fn query_color_scheme(&self) -> &'static str {
        COLOR_SCHEME_QUERY
    }

    /// 喂入一个序列：
    /// - 无配额 ⇒ `NotConsumed`（`tui.ts:1085`）；
    /// - 有配额但**不是 OSC 11 应答形状** ⇒ `NotConsumed`（`:1089`）；
    /// - 形状匹配 ⇒ **消费**（配额 -1，`:1094`），`Consumed(解析结果)`；解析失败时颜色为 `None`
    ///   （ref 的 `parseOsc11BackgroundColor` 返回 `undefined`，Promise 得 `undefined`）。
    pub fn on_sequence(&mut self, seq: &str) -> Osc11Outcome {
        if self.pending == 0 {
            return Osc11Outcome::NotConsumed;
        }
        if !is_osc11_shape(seq) {
            return Osc11Outcome::NotConsumed;
        }
        self.pending -= 1;
        Osc11Outcome::Consumed(parse_osc11(seq))
    }
}

/// 形状判定（`terminal-colors.ts:28`
/// `OSC11_BACKGROUND_COLOR_RESPONSE_PATTERN = /^\x1b\]11;([^\x07\x1b]*)(?:\x07|\x1b\\\\)$/i`）。
///
/// **载荷不做任何格式校验** —— d1/d2 这类非法载荷也满足形状（这正是 ref 仍会消费它们的原因）。
fn is_osc11_shape(seq: &str) -> bool {
    let Some(body) = seq.strip_prefix("\u{1b}]11;") else {
        return false;
    };
    let Some(payload) = body
        .strip_suffix('\u{7}')
        .or_else(|| body.strip_suffix("\u{1b}\\"))
    else {
        return false;
    };
    !payload.contains('\u{7}') && !payload.contains('\u{1b}')
}

/// 解析 `ESC ] 11 ; rgb:RRRR/GGGG/BBBB BEL|ST`。
///
/// ref 的 `parseOsc11BackgroundColor` 取每个分量的**高 8 位**（4 位十六进制 ⇒ `0x1111 >> 8 = 0x11`）。
fn parse_osc11(seq: &str) -> Option<Rgb> {
    let body = seq.strip_prefix("\u{1b}]11;")?;
    let body = body
        .strip_suffix('\u{7}')
        .or_else(|| body.strip_suffix("\u{1b}\\"))?;
    let rgb = body.strip_prefix("rgb:")?;
    let mut parts = rgb.split('/');
    let r = part(parts.next()?)?;
    let g = part(parts.next()?)?;
    let b = part(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(Rgb { r, g, b })
}

/// 单个分量：必须是 2/4 位十六进制；4 位时取高 8 位。
fn part(text: &str) -> Option<u8> {
    if !text.chars().all(|c| c.is_ascii_hexdigit()) || text.len() < 2 || text.len() > 4 {
        return None;
    }
    let value = u32::from_str_radix(text, 16).ok()?;
    if text.len() == 4 {
        u8::try_from(value >> 8).ok()
    } else {
        u8::try_from(value).ok()
    }
}
