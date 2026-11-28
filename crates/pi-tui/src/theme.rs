//! 主题层：**照抄**上游 pi 的主题文件与解析方式。
//!
//! ref：`packages/coding-agent/src/modes/interactive/theme/theme.ts` 的
//! `loadTheme` / `resolveColors`（`vars` 先解析，`colors` 里可以用变量名或
//! 直接 `#rrggbb`），以及 `theme-json.ts` 的 schema。
//!
//! 我们**不做**自有配色表：`dark` / `light` 两份数据直接内嵌上游 JSON，
//! 保证与上游逐字节同源（值不同步时一眼可见 diff）。

use std::collections::BTreeMap;

/// 内置主题名（自定义主题经 [`register_custom`] 注册后可被 [`resolve`] 找到）。
pub const BUILTIN: [&str; 2] = ["dark", "light"];

/// 自定义主题注册表（`--theme <dir>` 载入）。
///
/// 进程级单例：TUI 渲染时按名字取主题（`theme::resolve`），而渲染点有十几处，
/// 逐一传注册表不现实。只在**启动期**注册一次，之后只读。
static CUSTOM_THEMES: std::sync::OnceLock<std::sync::Mutex<Vec<Theme>>> =
    std::sync::OnceLock::new();

fn custom_themes() -> &'static std::sync::Mutex<Vec<Theme>> {
    CUSTOM_THEMES.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// 注册一批自定义主题（后注册的同名主题覆盖前面的）。
///
/// **线程安全**：`Mutex` 保护；启动期调用一次，之后只读。
pub fn register_custom(themes: Vec<Theme>) {
    if let Ok(mut list) = custom_themes().lock() {
        for theme in themes {
            if let Some(slot) = list.iter_mut().find(|item| item.name == theme.name) {
                *slot = theme;
            } else {
                list.push(theme);
            }
        }
    }
}

/// 列出**全部可用主题名**：内置两个在前，随后是已注册的自定义主题（按名字排序）。
///
/// 供 `/settings` 的主题子菜单用——不只列内置两套，否则 `--theme <dir>` 载入的
/// 自定义主题在界面上根本看不到，用户只能靠 `--use-theme` 手输名字。
#[must_use]
pub fn available_names() -> Vec<String> {
    let mut names: Vec<String> = BUILTIN.iter().map(|name| (*name).to_string()).collect();
    if let Ok(list) = custom_themes().lock() {
        for theme in list.iter() {
            if !names.contains(&theme.name) {
                names.push(theme.name.clone());
            }
        }
    }
    names
}

/// 按名字取主题：**先查自定义、再回落内置**（ref `getThemeByName` 同一顺序）。
///
/// 名字未知时回落 `dark`（与 `Theme::builtin` 的兜底一致）。
#[must_use]
pub fn resolve(name: &str) -> Theme {
    if let Ok(list) = custom_themes().lock() {
        if let Some(theme) = list.iter().find(|item| item.name == name) {
            return theme.clone();
        }
    }
    Theme::builtin(name)
}

/// 缺省主题（ref `settings-manager.ts:782-791`：没有 `theme` 设置时上游用 `dark`）。
pub const DEFAULT_NAME: &str = "dark";

const DARK_JSON: &str = include_str!("theme-dark.json");
const LIGHT_JSON: &str = include_str!("theme-light.json");

/// 解析后的主题：`colors` 里所有键 → RGB。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    name: String,
    colors: BTreeMap<String, (u8, u8, u8)>,
}

impl Theme {
    /// 按名字加载内置主题（`dark` / `light`）。
    ///
    /// 名字未知时**回落到 dark**（与上游 `loadTheme` 的兜底一致）。
    #[must_use]
    pub fn builtin(name: &str) -> Self {
        let raw = if name == "light" {
            LIGHT_JSON
        } else {
            DARK_JSON
        };
        let fallback_name = if name == "light" {
            "light"
        } else {
            DEFAULT_NAME
        };
        Self::parse(raw).unwrap_or_else(|()| Self {
            name: fallback_name.to_string(),
            colors: BTreeMap::new(),
        })
    }

    /// 主题名（`dark` / `light`）。
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 取某个色键的 RGB；缺失返回 `None`（调用点自行决定回落）。
    #[must_use]
    pub fn rgb(&self, key: &str) -> Option<(u8, u8, u8)> {
        self.colors.get(key).copied()
    }

    /// 从**主题 JSON 文本**加载（上游格式：`{name, vars?, colors:{…}}`）。
    ///
    /// 内置 dark/light 走的就是这条路；`--theme <dir>` 的自定义主题也复用它。
    pub fn from_json(raw: &str) -> Result<Self, String> {
        Self::parse(raw).map_err(|()| "主题 JSON 缺少 `colors` 对象或不是合法 JSON".to_string())
    }

    /// 从**主题 JSON 文件**加载（`--theme <dir>` 里的 `*.json`）。
    ///
    /// 自动剥 UTF-8 BOM（Windows 编辑器会加）；读取失败或格式不符都返回
    /// `Err(原因)`，由调用方决定跳过并报告——不 panic。
    pub fn from_file(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
        Self::from_json(raw).map_err(|error| format!("{}: {error}", path.display()))
    }

    /// 扫描一个**目录**下的 `*.json`，返回所有能解析的主题。
    ///
    /// 与 ref `theme.ts:455-477` 的 `getCustomThemeInfos` 同语义：非 `.json` 忽略、
    /// 坏文件**跳过**（不因一个坏主题让整个启动失败）。目录不存在 ⇒ 空表。
    #[must_use]
    pub fn from_dir(dir: &std::path::Path) -> Vec<Self> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut themes: Vec<Self> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .filter_map(|path| Self::from_file(&path).ok())
            .collect();
        // 顺序稳定（`read_dir` 无保证）：按名字排。
        themes.sort_by(|a, b| a.name.cmp(&b.name));
        themes
    }

    /// 解析上游主题 JSON：先取 `vars`，再把 `colors` 里的引用解析成 RGB。
    ///
    /// 返回 `Err(())` 表示 JSON 结构不符（缺 `colors` 对象）。
    fn parse(raw: &str) -> Result<Self, ()> {
        let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| ())?;
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(DEFAULT_NAME)
            .to_string();
        let mut vars: BTreeMap<String, (u8, u8, u8)> = BTreeMap::new();
        if let Some(map) = value.get("vars").and_then(serde_json::Value::as_object) {
            for (key, raw_value) in map {
                if let Some(text) = raw_value.as_str() {
                    if let Some(rgb) = parse_hex(text) {
                        vars.insert(key.clone(), rgb);
                    }
                }
            }
        }
        let mut colors: BTreeMap<String, (u8, u8, u8)> = BTreeMap::new();
        let map = value
            .get("colors")
            .and_then(serde_json::Value::as_object)
            .ok_or(())?;
        for (key, raw_value) in map {
            let Some(text) = raw_value.as_str() else {
                continue;
            };
            if let Some(rgb) = parse_hex(text).or_else(|| vars.get(text).copied()) {
                colors.insert(key.clone(), rgb);
            }
        }
        Ok(Self { name, colors })
    }
}

/// `#rrggbb` → RGB（上游 `parseColor` 只接受 6 位十六进制）。
fn parse_hex(text: &str) -> Option<(u8, u8, u8)> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
    let green = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
    let blue = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
    Some((red, green, blue))
}

#[cfg(test)]
mod tests {
    use super::Theme;

    /// 内嵌的两份 JSON 必须与上游 `theme/dark.json`、`theme/light.json` 同源：
    /// 这里抽查几处**关键键**，值不对就说明有人手改过内嵌数据。
    #[test]
    fn builtin_themes_match_upstream_values() {
        let dark = Theme::builtin("dark");
        assert_eq!(dark.name(), "dark");
        assert_eq!(dark.rgb("userMessageBg"), Some((0x34, 0x35, 0x41)));
        assert_eq!(dark.rgb("toolSuccessBg"), Some((0x28, 0x32, 0x28)));
        assert_eq!(dark.rgb("toolErrorBg"), Some((0x3c, 0x28, 0x28)));
        assert_eq!(dark.rgb("toolOutput"), Some((0x80, 0x80, 0x80)));
        assert_eq!(dark.rgb("accent"), Some((0x8a, 0xbe, 0xb7)));
        assert_eq!(dark.rgb("mdHeading"), Some((0xf0, 0xc6, 0x74)));

        let light = Theme::builtin("light");
        assert_eq!(light.name(), "light");
        assert_eq!(light.rgb("userMessageBg"), Some((0xe8, 0xe8, 0xe8)));
        assert_eq!(light.rgb("toolSuccessBg"), Some((0xe8, 0xf0, 0xe8)));
        assert_eq!(light.rgb("toolErrorBg"), Some((0xf0, 0xe8, 0xe8)));
        assert_eq!(light.rgb("text"), Some((0x1f, 0x23, 0x28)));
        // light 的 `accent` 引用 `vars.teal`，解析链必须走通。
        assert_eq!(light.rgb("accent"), Some((0x5a, 0x80, 0x80)));
    }

    /// 主题名未知时回落 dark（上游 `loadTheme` 的兜底）。
    #[test]
    fn unknown_theme_falls_back_to_dark() {
        let theme = Theme::builtin("solarized");
        assert_eq!(theme.name(), "dark");
        assert_eq!(theme.rgb("userMessageBg"), Some((0x34, 0x35, 0x41)));
    }
}

/// 渲染层用的调色板：把上游 `colors` 的键名固定成字段，缺键时回落到 dark 的值，
/// 这样新增主题不会因为少一个键就让某个元素没颜色（与上游 `resolveColors`
/// 的"缺省继承"精神一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// `text`
    pub text: (u8, u8, u8),
    /// `muted`（= gray）
    pub muted: (u8, u8, u8),
    /// `dim`（= dimGray）
    pub dim: (u8, u8, u8),
    /// `accent`
    pub accent: (u8, u8, u8),
    /// `warning`
    pub warning: (u8, u8, u8),
    /// `error`
    pub error: (u8, u8, u8),
    /// `border` / `borderMuted`
    pub border: (u8, u8, u8),
    /// `borderMuted`
    pub border_muted: (u8, u8, u8),
    /// `userMessageBg`
    pub user_message_bg: (u8, u8, u8),
    /// `customMessageBg`（压缩/分支摘要块与自定义消息块的底色，ref `customMessageBg`）
    pub custom_message_bg: (u8, u8, u8),
    /// `toolSuccessBg`
    pub tool_success_bg: (u8, u8, u8),
    /// `toolErrorBg`
    pub tool_error_bg: (u8, u8, u8),
    /// `mdHeading`
    pub md_heading: (u8, u8, u8),
    /// `mdCodeBlock`
    pub md_code_block: (u8, u8, u8),
    /// `mdCodeBlockBorder`
    pub md_code_block_border: (u8, u8, u8),
    /// 语法高亮各色（ref `theme.ts:1036-1064` 的 `buildCliHighlightTheme` 映射目标）。
    pub syntax_keyword: (u8, u8, u8),
    pub syntax_type: (u8, u8, u8),
    pub syntax_string: (u8, u8, u8),
    pub syntax_number: (u8, u8, u8),
    pub syntax_comment: (u8, u8, u8),
    pub syntax_function: (u8, u8, u8),
    pub syntax_variable: (u8, u8, u8),
    pub syntax_punctuation: (u8, u8, u8),
    pub syntax_operator: (u8, u8, u8),
    /// `mdLink`
    pub md_link: (u8, u8, u8),
    /// `mdListBullet`（dark 指向 accent、light 指向 green）
    pub md_list_bullet: (u8, u8, u8),
    /// `mdCode`（行内代码；dark 指向 accent、light 指向 teal）
    pub md_code: (u8, u8, u8),
    /// `thinkingOff`…`thinkingMax`（编辑框边框随档位）
    pub thinking: [Option<(u8, u8, u8)>; 7],
}

impl Palette {
    /// 从主题取调色板；缺键回落 dark 的同名字段。
    #[must_use]
    pub fn of(theme: &Theme) -> Self {
        let dark = Theme::builtin(DEFAULT_NAME);
        let get = |theme: &Theme, key: &str| -> Option<(u8, u8, u8)> {
            theme.rgb(key).or_else(|| dark.rgb(key))
        };
        let md_accent = get(theme, "mdListBullet");
        Self {
            text: get(theme, "text").unwrap_or((0xd4, 0xd4, 0xd4)),
            muted: get(theme, "muted").unwrap_or((0x80, 0x80, 0x80)),
            dim: get(theme, "dim").unwrap_or((0x66, 0x66, 0x66)),
            accent: get(theme, "accent").unwrap_or((0x8a, 0xbe, 0xb7)),
            warning: get(theme, "warning").unwrap_or((0xff, 0xff, 0x00)),
            error: get(theme, "error").unwrap_or((0xcc, 0x66, 0x66)),
            border: get(theme, "border").unwrap_or((0x50, 0x50, 0x50)),
            border_muted: get(theme, "borderMuted").unwrap_or((0x50, 0x50, 0x50)),
            user_message_bg: get(theme, "userMessageBg").unwrap_or((0x34, 0x35, 0x41)),
            custom_message_bg: get(theme, "customMessageBg").unwrap_or((0x2d, 0x28, 0x38)),
            tool_success_bg: get(theme, "toolSuccessBg").unwrap_or((0x28, 0x32, 0x28)),
            tool_error_bg: get(theme, "toolErrorBg").unwrap_or((0x3c, 0x28, 0x28)),
            md_heading: get(theme, "mdHeading").unwrap_or((0xf0, 0xc6, 0x74)),
            md_code_block: get(theme, "mdCodeBlock").unwrap_or((0xb5, 0xbd, 0x68)),
            md_code_block_border: get(theme, "mdCodeBlockBorder").unwrap_or((0x80, 0x80, 0x80)),
            syntax_keyword: get(theme, "syntaxKeyword").unwrap_or((0x56, 0x9c, 0xd6)),
            syntax_type: get(theme, "syntaxType").unwrap_or((0x4e, 0xc9, 0xb0)),
            syntax_string: get(theme, "syntaxString").unwrap_or((0xce, 0x91, 0x78)),
            syntax_number: get(theme, "syntaxNumber").unwrap_or((0xb5, 0xce, 0xa8)),
            syntax_comment: get(theme, "syntaxComment").unwrap_or((0x6a, 0x99, 0x55)),
            syntax_function: get(theme, "syntaxFunction").unwrap_or((0xdc, 0xdc, 0xaa)),
            syntax_variable: get(theme, "syntaxVariable").unwrap_or((0x9c, 0xdc, 0xfe)),
            syntax_punctuation: get(theme, "syntaxPunctuation").unwrap_or((0xd4, 0xd4, 0xd4)),
            syntax_operator: get(theme, "syntaxOperator").unwrap_or((0xd4, 0xd4, 0xd4)),
            md_link: get(theme, "mdLink").unwrap_or((0x81, 0xa2, 0xbe)),
            md_list_bullet: get(theme, "mdListBullet")
                .or(md_accent)
                .unwrap_or((0x8a, 0xbe, 0xb7)),
            md_code: get(theme, "mdCode")
                .or(md_accent)
                .unwrap_or((0x8a, 0xbe, 0xb7)),
            thinking: [
                get(theme, "thinkingOff"),
                get(theme, "thinkingMinimal"),
                get(theme, "thinkingLow"),
                get(theme, "thinkingMedium"),
                get(theme, "thinkingHigh"),
                get(theme, "thinkingXhigh"),
                get(theme, "thinkingMax"),
            ],
        }
    }
}
