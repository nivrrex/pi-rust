//! 主题层：**照抄**上游 pi 的主题文件与解析方式。
//!
//! ref：`packages/coding-agent/src/modes/interactive/theme/theme.ts` 的
//! `loadTheme` / `resolveColors`（`vars` 先解析，`colors` 里可以用变量名或
//! 直接 `#rrggbb`），以及 `theme-json.ts` 的 schema。
//!
//! 我们**不做**自有配色表：`dark` / `light` 两份数据直接内嵌上游 JSON，
//! 保证与上游逐字节同源（值不同步时一眼可见 diff）。

use std::collections::BTreeMap;

/// 主题名（上游只内置这两个；自定义主题文件路径的支持见 `LOAD_LIMITS` 事项）。
pub const BUILTIN: [&str; 2] = ["dark", "light"];

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
