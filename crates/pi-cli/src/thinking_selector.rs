//! `/thinking` 选择器的**纯逻辑**部分（P7-7b）。
//!
//! 行为规格来自 ref `modes/interactive/components/thinking-selector.ts`：
//! - 构造顺序（`:74-104`）：边框 → 空行 → `Thinking Level` → 空行 →
//!   `<app.thinking.cycle 键位> cycles thinking levels in-session` → 空行 → 搜索框 →
//!   空行 → 列表 → 空行 → `  Enter to select · Ctrl+S to set as default · Escape/Ctrl+C to cancel` → 边框；
//! - 条目文本（`:66-72`）：`{✓ | 空格}{level}` + 描述（默认档追加 ` · default`）；
//! - 档位描述表（`:25-33`）：`off`=No reasoning … `max`=Maximum reasoning；
//! - 两列布局（`:18-21`）：主列宽 12–32（实测 100 列时主列 12）。
//!
//! 本模块只做"状态 → 行"的纯函数，按键与 I/O 留在 `tui.rs`。

/// 思考档位条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingEntry {
    /// 档位名（`off`/`low`/…，即 `ThinkingLevel` 字面量）。
    pub level: String,
    /// 选中标记的目标：是否为当前档位（决定 `✓ `）。
    pub current: bool,
    /// 是否为默认档位（决定描述尾部 ` · default`）。
    pub default: bool,
}

/// 档位 → 描述（ref `thinking-selector.ts:25-33`）。
///
/// 入参是 `&str` 而不是枚举（档位名来自模型目录），因此这里用 `if` 链而非 `match`：
/// lint-banned 禁止 wildcard 分支，而 open 字符串无法穷举。
#[must_use]
pub fn level_description(level: &str) -> &'static str {
    if level == "off" {
        "No reasoning"
    } else if level == "minimal" {
        "Very brief reasoning (~1k tokens)"
    } else if level == "low" {
        "Light reasoning (~2k tokens)"
    } else if level == "medium" {
        "Moderate reasoning (~8k tokens)"
    } else if level == "high" {
        "Deep reasoning (~16k tokens)"
    } else if level == "xhigh" {
        "Extra-high reasoning (~32k tokens)"
    } else if level == "max" {
        "Maximum reasoning"
    } else {
        // 未知档位不属于规格；给空描述，不得 panic。
        ""
    }
}

/// 选择器状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingSelector {
    /// 搜索框内容（`applyFilter` 的输入）。
    pub query: String,
    /// 可选档位（`availableLevels` 的顺序）。
    pub levels: Vec<ThinkingEntry>,
}

impl ThinkingSelector {
    /// 新建选择器。
    #[must_use]
    pub fn new(levels: Vec<ThinkingEntry>) -> Self {
        Self {
            query: String::new(),
            levels,
        }
    }

    /// 当前选中档位（过滤后第一项；ref 的 `getSelectedLevel()`）。
    #[must_use]
    pub fn selected_level(&self) -> Option<String> {
        self.filtered_indices()
            .first()
            .and_then(|index| self.levels.get(*index))
            .map(|entry| entry.level.clone())
    }

    /// 过滤后的下标表（ref `applyFilter:118-126` 用 `fuzzyFilter`；这里用大小写无关子串）。
    #[must_use]
    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return (0..self.levels.len()).collect();
        }
        self.levels
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.level.to_lowercase().contains(&query)
                    || level_description(&item.level)
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 追加一个字符到搜索框。
    pub fn push(&mut self, ch: char) {
        self.query.push(ch);
    }

    /// 删除搜索框最后一个字符。
    pub fn pop(&mut self) {
        self.query.pop();
    }

    /// 面板行（**不含**上下边框）。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            String::new(),
            "Thinking Level".to_string(),
            String::new(),
            "Shift+Tab cycles thinking levels in-session".to_string(),
            String::new(),
            if self.query.is_empty() {
                ">".to_string()
            } else {
                format!("> {}", self.query)
            },
            String::new(),
        ];
        // 主列宽：ref `THINKING_SELECT_LIST_LAYOUT` 是 12–32，实测 100 列时取 12。
        const PRIMARY_WIDTH: usize = 12;
        for index in self.filtered_indices() {
            let item = &self.levels[index];
            let marker = if item.current { "✓ " } else { "  " };
            let primary = format!("{marker}{}", item.level);
            let mut description = level_description(&item.level).to_string();
            if item.default {
                description.push_str(" · default");
            }
            lines.push(format!(
                "→ {primary:<width$}{description}",
                width = PRIMARY_WIDTH
            ));
        }
        lines.push(String::new());
        lines.push(
            "  Enter to select · Ctrl+S to set as default · Escape/Ctrl+C to cancel".to_string(),
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{level_description, ThinkingEntry, ThinkingSelector};

    fn off_only() -> ThinkingSelector {
        ThinkingSelector::new(vec![ThinkingEntry {
            level: "off".to_string(),
            current: true,
            default: false,
        }])
    }

    /// T28 口径：面板行必须与 golden 逐字节一致（含主列 12 宽的两列对齐）。
    #[test]
    fn lines_match_t28_panel() {
        let expected = [
            "",
            "Thinking Level",
            "",
            "Shift+Tab cycles thinking levels in-session",
            "",
            ">",
            "",
            "→ ✓ off       No reasoning",
            "",
            "  Enter to select · Ctrl+S to set as default · Escape/Ctrl+C to cancel",
        ];
        assert_eq!(off_only().lines(), expected);
    }

    /// 描述表与 ref `:25-33` 一致（七档全覆盖）。
    #[test]
    fn descriptions_match_ref_table() {
        assert_eq!(level_description("off"), "No reasoning");
        assert_eq!(
            level_description("minimal"),
            "Very brief reasoning (~1k tokens)"
        );
        assert_eq!(level_description("low"), "Light reasoning (~2k tokens)");
        assert_eq!(
            level_description("medium"),
            "Moderate reasoning (~8k tokens)"
        );
        assert_eq!(level_description("high"), "Deep reasoning (~16k tokens)");
        assert_eq!(
            level_description("xhigh"),
            "Extra-high reasoning (~32k tokens)"
        );
        assert_eq!(level_description("max"), "Maximum reasoning");
    }

    /// 默认档位追加 ` · default`；非当前档位没有 `✓`。
    #[test]
    fn default_badge_and_marker() {
        let selector = ThinkingSelector::new(vec![
            ThinkingEntry {
                level: "off".to_string(),
                current: false,
                default: true,
            },
            ThinkingEntry {
                level: "high".to_string(),
                current: true,
                default: false,
            },
        ]);
        let lines = selector.lines();
        assert!(lines
            .iter()
            .any(|l| l == "→   off       No reasoning · default"));
        assert!(lines
            .iter()
            .any(|l| l == "→ ✓ high      Deep reasoning (~16k tokens)"));
        // 过滤：查询 `deep` 命中 high、排除 off
        let mut filtered = selector.clone();
        filtered.query = "deep".to_string();
        assert_eq!(filtered.filtered_indices(), vec![1]);
        filtered.query = "zzz".to_string();
        assert!(filtered.filtered_indices().is_empty());
    }
}
