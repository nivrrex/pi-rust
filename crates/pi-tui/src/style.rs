//! 整行样式（P8d/C4）：把主题里的一类颜色映射成 **SGR 前缀**。
//!
//! 设计约束：
//! - 样式是**整行**粒度（够覆盖 ref 的用户消息底色、thinking 斜体、工具块底色、
//!   markdown 各元素、边框、footer 等可见差异），不引入 span 级排版，避免动到
//!   `render_all` 的文本产物（既有 golden 与单测一个字都不能变）。
//! - 颜色用**真彩** `38;2;r;g;b` / `48;2;r;g;b`：TS 的 `theme.fg("#rrggbb", …)` 输出的就是真彩，
//!   `tools/difftest/compare-style.mjs` 记录的也是 `rgb(r,g,b)`。
//! - 宽度计算仍以**纯文本**为准（`diff.rs` 的 `visible_width(line)` 不变），
//!   所以加样式不会影响折行与视口。

/// 行内一段（字节区间 `[start, end)`）的样式（P8d/C4，span 级）。
///
/// 为什么需要：TS 的一行里会混多种颜色（启动提示 = 键名 `dim` + 说明 `muted` +
/// 版本号 `accent` 粗体），整行粒度补不出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// 起始字节偏移（相对该行文本）。
    pub start: usize,
    /// 结束字节偏移（不含）。
    pub end: usize,
    /// 该段的样式。
    pub style: LineStyle,
}

impl Span {
    /// 构造一段。
    #[must_use]
    pub const fn new(start: usize, end: usize, style: LineStyle) -> Self {
        Self { start, end, style }
    }
}

/// 一行可用的样式组合（未设置的字段表示"终端默认"）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineStyle {
    /// 前景色（真彩）。
    pub fg: Option<(u8, u8, u8)>,
    /// 背景色（真彩）。
    pub bg: Option<(u8, u8, u8)>,
    /// 粗体（SGR 1）。
    pub bold: bool,
    /// 暗淡（SGR 2）。
    pub dim: bool,
    /// 斜体（SGR 3）。
    pub italic: bool,
}

impl LineStyle {
    /// 构造只带前景色的样式。
    #[must_use]
    pub const fn fg(rgb: (u8, u8, u8)) -> Self {
        Self {
            fg: Some(rgb),
            bg: None,
            bold: false,
            dim: false,
            italic: false,
        }
    }

    /// 构造只带背景色的样式。
    #[must_use]
    pub const fn bg(rgb: (u8, u8, u8)) -> Self {
        Self {
            fg: None,
            bg: Some(rgb),
            bold: false,
            dim: false,
            italic: false,
        }
    }

    /// 叠加背景色（链式）。
    #[must_use]
    pub const fn with_bg(mut self, rgb: (u8, u8, u8)) -> Self {
        self.bg = Some(rgb);
        self
    }

    /// 只加背景色（前景保持终端默认）。
    #[must_use]
    pub const fn with_bg_only(rgb: (u8, u8, u8)) -> Self {
        Self::bg(rgb)
    }

    /// 叠加粗体。
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// 叠加暗淡。
    #[must_use]
    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// 叠加斜体。
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// 是否没有任何效果（不需要写 SGR）。
    #[must_use]
    pub const fn is_plain(&self) -> bool {
        self.fg.is_none() && self.bg.is_none() && !self.bold && !self.dim && !self.italic
    }

    /// 生成 SGR 前缀；无效果时返回空串。
    #[must_use]
    pub fn prefix(&self) -> String {
        if self.is_plain() {
            return String::new();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.bold {
            parts.push("1".to_string());
        }
        if self.dim {
            parts.push("2".to_string());
        }
        if self.italic {
            parts.push("3".to_string());
        }
        if let Some((r, g, b)) = self.fg {
            parts.push(format!("38;2;{r};{g};{b}"));
        }
        if let Some((r, g, b)) = self.bg {
            parts.push(format!("48;2;{r};{g};{b}"));
        }
        format!("\x1b[{}m", parts.join(";"))
    }

    /// 复位序列（只有真的写过前缀才需要）。
    #[must_use]
    pub fn reset(&self) -> &'static str {
        if self.is_plain() {
            ""
        } else {
            "\x1b[0m"
        }
    }
}

// ---------------------------------------------------------------------------
// dark 主题色值（`ref/…/theme/dark.json` 的 vars，C3 取证）
// ---------------------------------------------------------------------------

/// `border` → blue `#5f87ff`
pub const BORDER: (u8, u8, u8) = (0x5f, 0x87, 0xff);
/// `accent` `#8abeb7`
pub const ACCENT: (u8, u8, u8) = (0x8a, 0xbe, 0xb7);
/// `text` `#d4d4d4`
pub const TEXT: (u8, u8, u8) = (0xd4, 0xd4, 0xd4);
/// `muted` → gray `#808080`
pub const MUTED: (u8, u8, u8) = (0x80, 0x80, 0x80);
/// `dim` → dimGray `#666666`
pub const DIM: (u8, u8, u8) = (0x66, 0x66, 0x66);
/// `borderMuted` → darkGray `#505050`
pub const BORDER_MUTED: (u8, u8, u8) = (0x50, 0x50, 0x50);
/// `error` → red `#cc6666`
pub const ERROR: (u8, u8, u8) = (0xcc, 0x66, 0x66);
/// `warning` → yellow `#ffff00`
pub const WARNING: (u8, u8, u8) = (0xff, 0xff, 0x00);
/// `success` → green `#b5bd68`
pub const SUCCESS: (u8, u8, u8) = (0xb5, 0xbd, 0x68);
/// `userMessageBg` `#343541`
pub const USER_MESSAGE_BG: (u8, u8, u8) = (0x34, 0x35, 0x41);
/// `toolPendingBg` `#282832`
pub const TOOL_PENDING_BG: (u8, u8, u8) = (0x28, 0x28, 0x32);
/// `toolSuccessBg` `#283228`
pub const TOOL_SUCCESS_BG: (u8, u8, u8) = (0x28, 0x32, 0x28);
/// `toolErrorBg` `#3c2828`
pub const TOOL_ERROR_BG: (u8, u8, u8) = (0x3c, 0x28, 0x28);
/// `mdHeading` `#f0c674`
pub const MD_HEADING: (u8, u8, u8) = (0xf0, 0xc6, 0x74);
/// `mdCodeBlock` → green `#b5bd68`
pub const MD_CODE_BLOCK: (u8, u8, u8) = (0xb5, 0xbd, 0x68);
/// `mdCodeBlockBorder` → gray `#808080`
pub const MD_CODE_BLOCK_BORDER: (u8, u8, u8) = (0x80, 0x80, 0x80);
/// `mdLink` `#81a2be`
pub const MD_LINK: (u8, u8, u8) = (0x81, 0xa2, 0xbe);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_style_emits_nothing() {
        assert!(LineStyle::default().prefix().is_empty());
        assert!(LineStyle::default().reset().is_empty());
    }

    #[test]
    fn foreground_uses_true_color_sgr() {
        let style = LineStyle::fg(BORDER).bold();
        assert_eq!(style.prefix(), "\x1b[1;38;2;95;135;255m");
        assert_eq!(style.reset(), "\x1b[0m");
    }

    #[test]
    fn background_and_attributes_combine() {
        let style = LineStyle::bg(USER_MESSAGE_BG).dim().italic();
        assert_eq!(style.prefix(), "\x1b[2;3;48;2;52;53;65m");
    }
}
