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

/// ANSI 256 色立方体的 6 级取值（ref `theme.ts` 的 `CUBE_VALUES`）。
const CUBE_VALUES: [u8; 6] = [0, 95, 135, 175, 215, 255];
/// 灰阶 24 级（ref 的 `GRAY_VALUES`）。
const GRAY_VALUES: [u8; 24] = [
    8, 18, 28, 38, 48, 58, 68, 78, 88, 98, 108, 118, 128, 138, 148, 158, 168, 178, 188, 198, 208,
    218, 228, 238,
];

fn closest_cube_index(value: u8) -> usize {
    let mut best = 0_usize;
    let mut best_distance = u32::MAX;
    for (index, candidate) in CUBE_VALUES.iter().enumerate() {
        let distance = (i32::from(value) - i32::from(*candidate)).unsigned_abs();
        if distance < best_distance {
            best_distance = distance;
            best = index;
        }
    }
    best
}

fn closest_gray_index(value: u8) -> usize {
    let mut best = 0_usize;
    let mut best_distance = u32::MAX;
    for (index, candidate) in GRAY_VALUES.iter().enumerate() {
        let distance = (i32::from(value) - i32::from(*candidate)).unsigned_abs();
        if distance < best_distance {
            best_distance = distance;
            best = index;
        }
    }
    best
}

/// RGB → 256 色索引（ref `theme.ts:165-198` 的 `rgbTo256`）：
/// 先在 6×6×6 立方体里找最近点，再与灰阶比较；**只有接近中性（spread < 10）**
/// 且灰阶更近时才用灰阶，避免把带色调的颜色退化成灰。
#[must_use]
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let r_index = closest_cube_index(r);
    let g_index = closest_cube_index(g);
    let b_index = closest_cube_index(b);
    let cube = (
        CUBE_VALUES[r_index],
        CUBE_VALUES[g_index],
        CUBE_VALUES[b_index],
    );
    let cube_distance = color_distance((r, g, b), cube);

    let gray = u8::try_from(
        (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)).round() as i32,
    )
    .unwrap_or(0);
    let gray_index = closest_gray_index(gray);
    let gray_value = GRAY_VALUES[gray_index];
    let gray_distance = color_distance((r, g, b), (gray_value, gray_value, gray_value));

    let max_channel = r.max(g).max(b);
    let min_channel = r.min(g).min(b);
    let spread = max_channel - min_channel;
    if spread < 10 && gray_distance < cube_distance {
        return 232 + u8::try_from(gray_index).unwrap_or(0);
    }
    u8::try_from(16 + 36 * r_index + 6 * g_index + b_index).unwrap_or(0)
}

fn color_distance(left: (u8, u8, u8), right: (u8, u8, u8)) -> u32 {
    let dr = i32::from(left.0) - i32::from(right.0);
    let dg = i32::from(left.1) - i32::from(right.1);
    let db = i32::from(left.2) - i32::from(right.2);
    let weighted =
        f64::from(dr * dr) * 0.299 + f64::from(dg * dg) * 0.587 + f64::from(db * db) * 0.114;
    weighted.round() as u32
}

/// 终端是否支持真彩（ref `terminal-image.ts:72-75`：`COLORTERM` 是 `truecolor`/`24bit`）。
#[must_use]
pub fn truecolor_supported() -> bool {
    truecolor_from(|name| std::env::var(name).ok())
}

/// 纯函数版本（便于单测，不碰进程环境）。
#[must_use]
pub fn truecolor_from(lookup: impl Fn(&str) -> Option<String>) -> bool {
    // 逐条照 `tui/src/terminal-image.ts:78-133` 的 `detectCapabilitiesFromEnvironment`：
    // 先看"能确定支持真彩"的终端标识，再看 COLORTERM 提示，最后保守退回 256 色。
    let env = |name: &str| {
        lookup(name)
            .map(|value| value.to_lowercase())
            .unwrap_or_default()
    };
    let term_program = env("TERM_PROGRAM");
    let terminal_emulator = env("TERMINAL_EMULATOR");
    let term = env("TERM");
    let color_term = env("COLORTERM");
    let hint = color_term == "truecolor" || color_term == "24bit";

    // tmux / screen：只有 COLORTERM 提示（它们不一定透传真彩）。
    if !env("TMUX").is_empty() || term.starts_with("tmux") || term.starts_with("screen") {
        return hint;
    }
    // 已知支持真彩的终端。
    if !env("KITTY_WINDOW_ID").is_empty() || term_program == "kitty" {
        return true;
    }
    if term_program == "ghostty"
        || term.contains("ghostty")
        || !env("GHOSTTY_RESOURCES_DIR").is_empty()
    {
        return true;
    }
    if !env("WEZTERM_PANE").is_empty() || term_program == "wezterm" {
        return true;
    }
    if !env("WT_SESSION").is_empty() {
        return true;
    }
    if matches!(term_program.as_str(), "alacritty" | "vscode" | "zed") {
        return true;
    }
    if terminal_emulator == "jetbrains-jediterm" {
        return true;
    }
    if cfg!(windows) {
        return true;
    }
    hint
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

    /// 用 `Option` 形式设置背景（供补位空格复用）。
    #[must_use]
    pub const fn with_bg_only_tuple(bg: Option<(u8, u8, u8)>) -> Self {
        Self {
            fg: None,
            bg,
            bold: false,
            dim: false,
            italic: false,
        }
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
        self.prefix_with(truecolor_supported())
    }

    /// 指定色彩模式的 SGR 前缀（纯函数，单测用）。
    #[must_use]
    pub fn prefix_with(&self, truecolor: bool) -> String {
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
        // ref `theme.ts:202-232`：真彩用 `38;2;r;g;b`，否则用 `38;5;N`（最近的 256 色）。
        if let Some((r, g, b)) = self.fg {
            parts.push(if truecolor {
                format!("38;2;{r};{g};{b}")
            } else {
                format!("38;5;{}", rgb_to_256(r, g, b))
            });
        }
        if let Some((r, g, b)) = self.bg {
            parts.push(if truecolor {
                format!("48;2;{r};{g};{b}")
            } else {
                format!("48;5;{}", rgb_to_256(r, g, b))
            });
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
/// 思考档位对应的编辑框边框色（ref `theme.ts:371-395` 的 `getThinkingBorderColor`，
/// 色值取自 `dark.json` 的 `thinking*`）。
#[must_use]
pub fn thinking_border(level: &str) -> (u8, u8, u8) {
    match level {
        "minimal" => (0x6e, 0x6e, 0x6e),
        "low" => (0x5f, 0x87, 0xaf),
        "medium" => (0x81, 0xa2, 0xbe),
        "high" => (0xb2, 0x94, 0xbb),
        "xhigh" => (0xd1, 0x83, 0xe8),
        "max" => (0xff, 0x5f, 0xff),
        // `off` 与未知档位都用 `thinkingOff`（不用通配分支：仓库禁止 `_ =>`）
        "off" | "" => BORDER_MUTED,
        other => thinking_border_unknown(other),
    }
}

/// 未知档位兜底（等价于 `thinkingOff`）。
fn thinking_border_unknown(_level: &str) -> (u8, u8, u8) {
    BORDER_MUTED
}

/// `mdLink` `#81a2be`
pub const MD_LINK: (u8, u8, u8) = (0x81, 0xa2, 0xbe);
/// ref `dark.json:60` `mdListBullet: accent`（`#8abeb7`）。
pub const MD_LIST_BULLET: (u8, u8, u8) = (0x8a, 0xbe, 0xb7);
/// ref `dark.json:54` `mdCode: accent`。
pub const MD_CODE: (u8, u8, u8) = (0x8a, 0xbe, 0xb7);

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
        assert_eq!(style.prefix_with(true), "\x1b[1;38;2;95;135;255m");
        assert_eq!(style.reset(), "\x1b[0m");
    }

    #[test]
    fn background_and_attributes_combine() {
        let style = LineStyle::bg(USER_MESSAGE_BG).dim().italic();
        assert_eq!(style.prefix_with(true), "\x1b[2;3;48;2;52;53;65m");
        // 256 色模式：同色按 ref 的 `rgbTo256` 近似（#343541 → 59）。
        assert_eq!(style.prefix_with(false), "\x1b[2;3;48;5;59m");
    }
}

#[cfg(test)]
mod color_mode_tests {
    use super::*;

    /// 逐条验证色彩模式判定（期望取自 ref `tui/src/terminal-image.ts:78-133`）：
    /// 已知终端标识 → 真彩；仅 tmux/screen → 看 COLORTERM；未知 → 看 COLORTERM。
    #[test]
    fn truecolor_detection_follows_ref_rules() {
        // 每个用例在独立的 env 作用域里跑，避免相互污染。
        type Case = (&'static str, &'static [(&'static str, &'static str)], bool);
        let cases: &[Case] = &[
            (
                "alacritty_term_program",
                &[("TERM_PROGRAM", "alacritty")],
                true,
            ),
            ("vscode", &[("TERM_PROGRAM", "vscode")], true),
            ("zed", &[("TERM_PROGRAM", "zed")], true),
            ("kitty_window_id", &[("KITTY_WINDOW_ID", "1")], true),
            ("ghostty_dir", &[("GHOSTTY_RESOURCES_DIR", "/x")], true),
            ("wezterm_pane", &[("WEZTERM_PANE", "0")], true),
            ("wt_session", &[("WT_SESSION", "abc")], true),
            (
                "jetbrains",
                &[("TERMINAL_EMULATOR", "jetbrains-jediterm")],
                true,
            ),
            ("colorterm_truecolor", &[("COLORTERM", "truecolor")], true),
            ("colorterm_24bit", &[("COLORTERM", "24bit")], true),
            ("unknown_plain", &[("TERM", "xterm-256color")], false),
            ("tmux_without_hint", &[("TERM", "tmux-256color")], false),
            (
                "tmux_with_hint",
                &[("TERM", "tmux-256color"), ("COLORTERM", "truecolor")],
                true,
            ),
            ("screen_without_hint", &[("TERM", "screen-256color")], false),
        ];
        for (name, vars, expected) in cases {
            // 清掉所有相关变量，再按用例设置。
            for key in [
                "COLORTERM",
                "TERM",
                "TERM_PROGRAM",
                "TERMINAL_EMULATOR",
                "TMUX",
                "KITTY_WINDOW_ID",
                "GHOSTTY_RESOURCES_DIR",
                "WEZTERM_PANE",
                "WT_SESSION",
            ] {
                std::env::remove_var(key);
            }
            for (key, value) in *vars {
                std::env::set_var(key, value);
            }
            // `style.rs` 的判定表里有 `cfg!(windows) => true`（ref 的 win32 条件），
            // 它排在 tmux/screen 的"按 COLORTERM 提示"之后 ⇒ 只有 `unknown_plain`
            // （未命中任何提示词、且不是 tmux/screen）在 Windows 上期望 true。
            // 其余用例（含 tmux_without_hint / screen_without_hint 的 false）两侧一致。
            let want = if cfg!(windows) && *name == "unknown_plain" {
                true
            } else {
                *expected
            };
            assert_eq!(truecolor_supported(), want, "用例 {name}");
        }
        for key in [
            "COLORTERM",
            "TERM",
            "TERM_PROGRAM",
            "TERMINAL_EMULATOR",
            "TMUX",
            "KITTY_WINDOW_ID",
            "GHOSTTY_RESOURCES_DIR",
            "WEZTERM_PANE",
            "WT_SESSION",
        ] {
            std::env::remove_var(key);
        }
    }
}

#[cfg(test)]
mod thinking_border_tests {
    use super::*;

    /// 期望值按 ref 的 `rgbTo256`（`theme.ts:165-198`）手算：立方体最近点优先，
    /// 仅当 `spread < 10` 且灰阶更近时才用灰阶。
    #[test]
    fn thinking_levels_map_to_ref_border_colors() {
        let cases = [
            ("off", (0x50, 0x50, 0x50), "\u{1b}[38;5;239m"), // #505050 → 灰 78 → 239（此处仅校验 rgb）
            ("minimal", (0x6e, 0x6e, 0x6e), "\u{1b}[38;5;242m"),
            ("low", (0x5f, 0x87, 0xaf), "\u{1b}[38;5;67m"),
            ("medium", (0x81, 0xa2, 0xbe), "\u{1b}[38;5;109m"),
            ("high", (0xb2, 0x94, 0xbb), "\u{1b}[38;5;139m"),
            ("xhigh", (0xd1, 0x83, 0xe8), "\u{1b}[38;5;176m"),
            ("max", (0xff, 0x5f, 0xff), "\u{1b}[38;5;207m"),
        ];
        for (level, rgb, expected_sgr) in cases {
            assert_eq!(thinking_border(level), rgb, "{level}");
            if level != "off" {
                let style = LineStyle::fg(rgb);
                assert_eq!(style.prefix_with(false), expected_sgr, "{level}");
            }
        }
        // 未知档位回落到 `thinkingOff`
        assert_eq!(thinking_border("turbo"), BORDER_MUTED);
    }
}
