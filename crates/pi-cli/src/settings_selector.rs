//! `/settings` 选择器的**纯逻辑**部分（P7-7c1）。
//!
//! 规格来源：
//! - 条目顺序 = ref `settings-selector.ts:464-724` 的基础数组 + `:730-836` 的 9 次
//!   `items.splice(<固定下标>, 0, ...)`。`capabilities.images=false` 时 `show-images`/
//!   `image-width-cells` 不入列（`:728`），于是基础 22 条 + 插入 9 条 = **31 条**，与 golden 的
//!   `(1/31)` 一致；插入下标从 `1` 开始（`:747`）⇒ 前 10 行是"autocompact + 插入的 9 条"，
//!   这正是 golden 里的顺序（Auto-compact, Auto-resize images, Block images, … Terminal progress）。
//! - 渲染 = pi-tui `components/settings-list.ts:131-175`：
//!   `maxLabelWidth = min(36, 全部条目最大 label 宽)`；行 = `prefix + label 补空格 + "  " + 当前值`；
//!   需要滚动时追加 `  (i/n)`；随后空行 + `description` 按 `width-4` 折行（每行前两空格）；
//!   最后是提示行 `  Type to search · Enter/Space to change · Esc to cancel`。
//!
//! 本模块只做"状态 → 行"的纯函数，按键与 I/O 留在 `tui.rs`。

/// 一个设置条目。
#[derive(Debug, Clone, Copy)]
pub struct SettingsItem {
    /// 稳定 id（回调 switch 用），`switch (id)` 见 `settings-selector.ts:842-953`。
    pub id: &'static str,
    /// 行内 label。
    pub label: &'static str,
    /// 选中时展示的描述。
    pub description: &'static str,
    /// 当前值（字符串形态；ref 的 `currentValue`）。
    pub current: &'static str,
    /// 可选值（空 slice = 走自定义子菜单，ref 的 `warnings`/`model-thinking`/`theme` 等）。
    pub values: &'static [&'static str],
}

/// 条目表（顺序见模块文档；`capabilities.images=false` 口径）。
pub const SETTINGS_ITEMS: &[SettingsItem] = &[
    SettingsItem {
        id: "autocompact",
        label: "Auto-compact",
        description: "Automatically compact context when it gets too large",
        current: "true",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "auto-resize-images",
        label: "Auto-resize images",
        description: "Resize large images to 2000x2000 max for better model compatibility",
        current: "true",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "block-images",
        label: "Block images",
        description: "Prevent images from being sent to LLM providers",
        current: "false",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "skill-commands",
        label: "Skill commands",
        description: "Register skills as /skill:name commands",
        current: "true",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "show-hardware-cursor",
        label: "Show hardware cursor",
        description: "Show the terminal cursor while still positioning it for IME support",
        current: "false",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "editor-padding",
        label: "Editor padding",
        description: "Horizontal padding for input editor (0-3)",
        current: "0",
        values: &["0", "1", "2", "3"],
    },
    SettingsItem {
        id: "output-padding",
        label: "Output padding",
        description: "Horizontal padding for user messages, assistant messages, and thinking",
        current: "1",
        values: &["0", "1"],
    },
    SettingsItem {
        id: "autocomplete-max-visible",
        label: "Autocomplete max items",
        description: "Max visible items in autocomplete dropdown (3-20)",
        current: "5",
        values: &["3", "5", "7", "10", "15", "20"],
    },
    SettingsItem {
        id: "clear-on-shrink",
        label: "Clear on shrink",
        description: "Clear empty rows when content shrinks (may cause flicker)",
        current: "false",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "terminal-progress",
        label: "Terminal progress",
        description: "Show OSC 9;4 progress indicators in the terminal tab bar",
        current: "false",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "steering-mode",
        label: "Steering mode",
        description: "Enter while streaming queues steering messages. 'one-at-a-time': deliver one, wait for response. 'all': deliver all at once.",
        current: "one-at-a-time",
        values: &["one-at-a-time", "all"],
    },
    SettingsItem {
        id: "follow-up-mode",
        label: "Follow-up mode",
        description: "Esc queues follow-up messages until agent stops. 'one-at-a-time': deliver one, wait for response. 'all': deliver all at once.",
        current: "one-at-a-time",
        values: &["one-at-a-time", "all"],
    },
    SettingsItem {
        id: "transport",
        label: "Transport",
        description: "Preferred transport for providers that support multiple transports",
        current: "auto",
        values: &["sse", "websocket", "websocket-cached", "auto"],
    },
    SettingsItem {
        id: "http-idle-timeout",
        label: "HTTP idle timeout",
        description: "Maximum idle gap while waiting for HTTP headers or body chunks. Disable for local models that pause longer than five minutes.",
        current: "5 min",
        values: &["30 sec", "1 min", "2 min", "5 min", "disabled"],
    },
    SettingsItem {
        id: "cache-warming-mode",
        label: "Cache warming",
        description: "off; streaming while the agent runs; idle also between runs while continuation stays profitable",
        current: "streaming",
        values: &["off", "streaming", "idle"],
    },
    SettingsItem {
        id: "hide-thinking",
        label: "Hide thinking",
        description: "Hide thinking blocks in assistant responses",
        current: "false",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "mermaid-rendering",
        label: "Mermaid diagrams",
        description: "Render Mermaid code blocks as Unicode diagrams",
        current: "streaming",
        values: &["off", "final", "streaming"],
    },
    SettingsItem {
        id: "cache-miss-notices",
        label: "Cache miss notices",
        description: "Show transcript notices for cache costs and provider recovery diagnostics",
        current: "false",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "collapse-changelog",
        label: "Collapse changelog",
        description: "Show condensed changelog after updates",
        current: "false",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "quiet-startup",
        label: "Quiet startup",
        description: "Disable verbose printing at startup",
        current: "false",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "install-telemetry",
        label: "Install telemetry",
        description: "Send an anonymous version/update ping after changelog-detected updates",
        current: "true",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "default-project-trust",
        label: "Default project trust",
        description: "Fallback behavior when no extension or saved trust decision decides project trust",
        current: "Ask",
        values: &[],
    },
    SettingsItem {
        id: "double-escape-action",
        label: "Double-escape action",
        description: "Action when pressing Escape twice with empty editor",
        current: "tree",
        values: &["tree", "fork", "none"],
    },
    SettingsItem {
        id: "tree-filter-mode",
        label: "Tree filter mode",
        description: "Default filter when opening /tree",
        current: "default",
        values: &["default", "no-tools", "user-only", "labeled-only", "all"],
    },
    SettingsItem {
        id: "warnings",
        label: "Warnings",
        description: "Enable or disable individual warnings",
        current: "configure",
        values: &[],
    },
    SettingsItem {
        id: "model-thinking",
        label: "Default thinking level per model",
        description: "Override the default thinking level for specific models. ${cycleThinkingKey} cycles in-session.",
        current: "none",
        values: &[],
    },
    SettingsItem {
        id: "tui-mode",
        label: "TUI mode",
        description: "Interface layout; fullscreen mode is experimental",
        current: "regular",
        values: &["regular", "fullscreen"],
    },
    SettingsItem {
        id: "fullscreen-exit-output",
        label: "Fullscreen exit output",
        description: "Print the transcript or only a session resume hint when exiting fullscreen mode",
        current: "transcript",
        values: &["transcript", "resume-hint"],
    },
    SettingsItem {
        id: "fullscreen-scrollbar",
        label: "Fullscreen scrollbar",
        description: "Scrollbar behavior in fullscreen mode; has no effect in regular mode",
        current: "auto",
        values: &["auto", "always", "hidden"],
    },
    SettingsItem {
        id: "fullscreen-copy-on-select",
        label: "Fullscreen copy on select",
        description: "Automatically copy selected text in fullscreen mode; disable to copy selections with Ctrl+X",
        current: "true",
        values: &["true", "false"],
    },
    SettingsItem {
        id: "theme",
        label: "Theme",
        description: "Color theme for the interface",
        current: "dark",
        values: &[],
    },
];

/// 选择器状态：搜索框内容 + 选中下标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSelector {
    /// 搜索框内容（`enableSearch: true`，ref `:955`）。
    pub query: String,
    /// 选中下标（**过滤后**列表里的下标）。
    pub selected: usize,
    /// 用户改过的值（条目下标 → 新值）；未改过的用 `SETTINGS_ITEMS[i].current`。
    pub overrides: std::collections::BTreeMap<usize, String>,
    /// 刚被 Enter 打开的子菜单条目 id（`None` = 没有）。由调用点消费。
    pub pending_submenu: Option<&'static str>,
    /// Theme 的当前值（子菜单需要；settings.json 的 `theme` 默认 `dark`）。
    pub current_of_theme: String,
    /// 当前模型 id（`per-model thinking` 子菜单要列出它；由调用点填入）。
    pub last_model_id: Option<String>,
    /// 当前 provider（同上）。
    pub last_provider_id: Option<String>,
}

impl SettingsSelector {
    /// 新建选择器（默认选中第 0 条）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            overrides: std::collections::BTreeMap::new(),
            pending_submenu: None,
            current_of_theme: "dark".to_string(),
            last_model_id: None,
            last_provider_id: None,
        }
    }

    /// 过滤后的条目下标（大小写无关子串匹配 id/label/description）。
    #[must_use]
    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return (0..SETTINGS_ITEMS.len()).collect();
        }
        SETTINGS_ITEMS
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.id.to_lowercase().contains(&query)
                    || item.label.to_lowercase().contains(&query)
                    || item.description.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 追加一个字符到搜索框（过滤后选中下标归零，对齐"搜索即重置"）。
    pub fn push(&mut self, ch: char) {
        self.query.push(ch);
        self.selected = 0;
    }

    /// 条目当前值（用户改过就取改后的）。下标越界时返回 `""`（**不 panic**）。
    ///
    /// fuzz 用例 `p78a_selector_fuzz` 曾经用越界下标触发
    /// `index out of bounds: the len is 31 but the index is 38`。
    #[must_use]
    pub fn current_of(&self, index: usize) -> &str {
        if let Some(value) = self.overrides.get(&index) {
            return value.as_str();
        }
        SETTINGS_ITEMS.get(index).map_or("", |item| item.current)
    }

    /// 上移选中项（ref `settings-list.ts:233-235`：**到头环绕到末行**）。
    pub fn move_up(&mut self) {
        let len = self.filtered_indices().len();
        if len == 0 {
            return;
        }
        self.selected = if self.selected == 0 {
            len - 1
        } else {
            self.selected - 1
        };
    }

    /// 下移选中项（ref `settings-list.ts:236-238`：**到尾环绕到首行**）。
    pub fn move_down(&mut self) {
        let len = self.filtered_indices().len();
        if len == 0 {
            return;
        }
        self.selected = if self.selected == len - 1 {
            0
        } else {
            self.selected + 1
        };
    }

    /// 激活当前条目（ref `settings-list.ts:264-292` 的 `activateItem`）。
    ///
    /// - 有 `values` ⇒ 循环到**下一个**值并回调（这里只改内部状态，写盘见 c2 的后续切片）；
    /// - 无 `values` ⇒ ref 打开子菜单（`warnings`/`model-thinking`/`theme`），本切片不实现 ⇒ 无动作。
    pub fn activate(&mut self) -> Option<(&'static str, String)> {
        let indices = self.filtered_indices();
        let index = *indices.get(self.selected)?;
        let item = &SETTINGS_ITEMS[index];
        if item.values.is_empty() {
            // 子菜单条目（ref `settings-list.ts:268-277`）：交给调用点打开对应子菜单。
            match item.id {
                "theme" => self.pending_submenu = Some("theme"),
                "warnings" => self.pending_submenu = Some("warnings"),
                "model-thinking" => self.pending_submenu = Some("model-thinking"),
                other => {
                    let _ = other;
                }
            }
            return None;
        }
        self.pending_submenu = None;
        let current = self.current_of(index).to_string();
        let pos = item.values.iter().position(|v| *v == current).unwrap_or(0);
        let next = item.values[(pos + 1) % item.values.len()].to_string();
        self.overrides.insert(index, next.clone());
        Some((item.id, next))
    }

    /// 删除搜索框最后一个字符。
    pub fn pop(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    /// 面板行（**不含**上下边框）。
    #[must_use]
    pub fn lines(&self, width: usize) -> Vec<String> {
        const MAX_VISIBLE: usize = 10;
        let indices = self.filtered_indices();
        let max_label_width = SETTINGS_ITEMS
            .iter()
            .map(|item| pi_tui::display_width(item.label))
            .max()
            .unwrap_or(0)
            .min(36);
        let total = indices.len();
        let selected = self.selected.min(total.saturating_sub(1));
        // 可见窗口：ref `getVisibleRange` 让选中项居中并夹在两端。
        let start = selected
            .saturating_sub(MAX_VISIBLE / 2)
            .min(total.saturating_sub(MAX_VISIBLE));
        let end = (start + MAX_VISIBLE).min(total);
        // ref 的 `SettingsList` 自带搜索输入行（`enableSearch: true`，`settings-selector.ts:955`），
        // 渲染在列表之前：golden 的 `>` 行 + 一个空行（T17/T29 行 8/9）。
        let mut lines = vec![
            if self.query.is_empty() {
                ">".to_string()
            } else {
                format!("> {}", self.query)
            },
            String::new(),
        ];
        for pos in start..end {
            let item = &SETTINGS_ITEMS[indices[pos]];
            let prefix = if pos == selected { "→ " } else { "  " };
            let pad = max_label_width.saturating_sub(pi_tui::display_width(item.label));
            let current = self.current_of(indices[pos]);
            lines.push(format!(
                "{prefix}{}{}{}",
                item.label,
                " ".repeat(pad),
                format_args!("  {current}")
            ));
        }
        if start > 0 || end < total {
            lines.push(format!("  ({}/{total})", selected + 1));
        }
        if let Some(&index) = indices.get(selected) {
            let item = &SETTINGS_ITEMS[index];
            if !item.description.is_empty() {
                lines.push(String::new());
                for wrapped in pi_tui::wrap_to_width(item.description, width.saturating_sub(4)) {
                    lines.push(format!("  {wrapped}"));
                }
            }
        }
        lines.push(String::new());
        lines.push("  Type to search · Enter/Space to change · Esc to cancel".to_string());
        lines
    }
}

impl Default for SettingsSelector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsSelector, SETTINGS_ITEMS};

    /// T17/T29 口径：首屏 10 行 + `(1/31)` + 描述 + 提示，逐字节对齐 golden。
    #[test]
    fn lines_match_t29_panel() {
        let expected = [
            ">",
            "",
            "→ Auto-compact                      true",
            "  Auto-resize images                true",
            "  Block images                      false",
            "  Skill commands                    true",
            "  Show hardware cursor              false",
            "  Editor padding                    0",
            "  Output padding                    1",
            "  Autocomplete max items            5",
            "  Clear on shrink                   false",
            "  Terminal progress                 false",
            "  (1/31)",
            "",
            "  Automatically compact context when it gets too large",
            "",
            "  Type to search · Enter/Space to change · Esc to cancel",
        ];
        assert_eq!(SettingsSelector::new().lines(100), expected);
    }

    /// 条目表 31 条、顺序与 golden 前 10 行一致（防 splice 顺序写错）。
    #[test]
    fn item_table_order_and_size() {
        assert_eq!(
            SETTINGS_ITEMS.len(),
            31,
            "capabilities.images=false 口径应为 31 条"
        );
        let labels: Vec<&str> = SETTINGS_ITEMS.iter().take(10).map(|i| i.label).collect();
        assert_eq!(
            labels,
            vec![
                "Auto-compact",
                "Auto-resize images",
                "Block images",
                "Skill commands",
                "Show hardware cursor",
                "Editor padding",
                "Output padding",
                "Autocomplete max items",
                "Clear on shrink",
                "Terminal progress",
            ]
        );
    }

    /// 31 条当前值：全部来自 **TS 实测**（T29 首屏 10 行 + T30 三屏滚动的其余 21 行）。
    #[test]
    fn current_values_match_ts_recordings() {
        let expected = [
            ("autocompact", "true"),
            ("auto-resize-images", "true"),
            ("block-images", "false"),
            ("skill-commands", "true"),
            ("show-hardware-cursor", "false"),
            ("editor-padding", "0"),
            ("output-padding", "1"),
            ("autocomplete-max-visible", "5"),
            ("clear-on-shrink", "false"),
            ("terminal-progress", "false"),
            ("steering-mode", "one-at-a-time"),
            ("follow-up-mode", "one-at-a-time"),
            ("transport", "auto"),
            ("http-idle-timeout", "5 min"),
            ("cache-warming-mode", "streaming"),
            ("hide-thinking", "false"),
            ("mermaid-rendering", "streaming"),
            ("cache-miss-notices", "false"),
            ("collapse-changelog", "false"),
            ("quiet-startup", "false"),
            ("install-telemetry", "true"),
            ("default-project-trust", "Ask"),
            ("double-escape-action", "tree"),
            ("tree-filter-mode", "default"),
            ("warnings", "configure"),
            ("model-thinking", "none"),
            ("tui-mode", "regular"),
            ("fullscreen-exit-output", "transcript"),
            ("fullscreen-scrollbar", "auto"),
            ("fullscreen-copy-on-select", "true"),
            ("theme", "dark"),
        ];
        let actual: Vec<(&str, &str)> = SETTINGS_ITEMS.iter().map(|i| (i.id, i.current)).collect();
        assert_eq!(actual, expected);
    }

    /// 导航：ref `settings-list.ts:233-238` 是**环绕**（首行上移⇒末行、末行下移⇒首行）。
    #[test]
    fn navigation_wraps_at_ends() {
        let mut selector = SettingsSelector::new();
        selector.move_up();
        assert_eq!(
            selector.selected,
            SETTINGS_ITEMS.len() - 1,
            "首行上移应环绕到末行"
        );
        let lines = selector.lines(100);
        assert!(lines.iter().any(|l| l == "  (31/31)"), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("→ Theme")), "{lines:?}");
        selector.move_down();
        assert_eq!(selector.selected, 0, "末行下移应环绕到首行");
    }

    /// 激活（Enter/Space）：ref `:278-288` 循环到下一个值；无 values 的条目（子菜单）不动。
    #[test]
    fn activate_cycles_values() {
        let mut selector = SettingsSelector::new();
        assert_eq!(selector.current_of(0), "true");
        assert_eq!(
            selector.activate(),
            Some(("autocompact", "false".to_string()))
        );
        assert_eq!(selector.current_of(0), "false");
        selector.activate();
        assert_eq!(selector.current_of(0), "true");
        let editor = SETTINGS_ITEMS
            .iter()
            .position(|i| i.id == "editor-padding")
            .expect("editor-padding");
        selector.selected = selector
            .filtered_indices()
            .iter()
            .position(|i| *i == editor)
            .expect("idx");
        selector.activate();
        assert_eq!(selector.current_of(editor), "1");
        selector.activate();
        assert_eq!(selector.current_of(editor), "2");
        let theme = SETTINGS_ITEMS
            .iter()
            .position(|i| i.id == "theme")
            .expect("theme");
        selector.selected = selector
            .filtered_indices()
            .iter()
            .position(|i| *i == theme)
            .expect("idx");
        selector.activate();
        assert_eq!(
            selector.current_of(theme),
            "dark",
            "子菜单条目不应被循环改值"
        );
    }

    /// 过滤：查询 `image` 只剩两条 image 相关且选中项归零；无匹配时没有行、也没有描述。
    #[test]
    fn filter_narrows_list() {
        let mut selector = SettingsSelector::new();
        for ch in "image".chars() {
            selector.push(ch);
        }
        let lines = selector.lines(100);
        assert!(
            lines.iter().any(|l| l.contains("Auto-resize images")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("Block images")),
            "{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("Auto-compact")),
            "{lines:?}"
        );
        let mut none = SettingsSelector::new();
        for ch in "zzzz".chars() {
            none.push(ch);
        }
        let empty = none.lines(100);
        assert!(
            !empty.iter().any(|l| l.contains("Auto-compact")),
            "{empty:?}"
        );
        assert!(
            empty.iter().all(|l| !l.contains("  (")),
            "空结果不应有滚动标记：{empty:?}"
        );
    }
}

/// `/settings` → Theme 子菜单（ref `settings-selector.ts:236-330` 的 `ThemeSubmenu` 的 single 模式 +
/// `components/settings-submenu.ts:31-100` 的 `SelectSubmenu`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSubmenu {
    /// 可选主题名（不含 `Automatic` 项）。
    pub themes: Vec<String>,
    /// 当前主题（决定 `✓`）。
    pub current: String,
    /// 选中下标（第 0 项是 `Automatic`）。
    pub selected: usize,
}

impl ThemeSubmenu {
    /// 新建；选中项默认落在当前主题。
    #[must_use]
    pub fn new(themes: Vec<String>, current: String) -> Self {
        let selected = themes
            .iter()
            .position(|t| *t == current)
            .map_or(0, |i| i + 1);
        Self {
            themes,
            current,
            selected,
        }
    }

    /// 行：(当前标记, 标签, 描述)。第一项是 `Automatic`。
    ///
    /// 标记与标签分开保存：SelectList 把标记画在 label 之前，主列宽度按**标签**算
    /// （T33 golden：`    Automatic  Use …` —— 标签 9 列 + 2 空格分隔）。
    fn rows(&self) -> Vec<(bool, String, Option<&'static str>)> {
        let mut rows = vec![(
            false,
            "Automatic".to_string(),
            Some("Use separate themes for light and dark terminal appearance"),
        )];
        for theme in &self.themes {
            rows.push((*theme == self.current, theme.clone(), None));
        }
        rows
    }

    /// 当前选中的主题名；选中 `Automatic` 那一行时返回 `None`（自动模式未实现）。
    #[must_use]
    pub fn picked(&self) -> Option<String> {
        if self.selected == 0 {
            None
        } else {
            self.themes.get(self.selected - 1).cloned()
        }
    }

    /// 上移（环绕，同 `SelectList`）。
    pub fn move_up(&mut self) {
        let len = self.rows().len();
        self.selected = if self.selected == 0 {
            len - 1
        } else {
            self.selected - 1
        };
    }

    /// 下移（环绕）。
    pub fn move_down(&mut self) {
        let len = self.rows().len();
        self.selected = if self.selected == len - 1 {
            0
        } else {
            self.selected + 1
        };
    }

    /// 面板行（不含上下边框）。
    ///
    /// 布局照 `SelectSubmenu`（`components/settings-submenu.ts:59-95`）：
    /// 标题 / 空行 / 描述 / 空行 / 列表 / 空行 / 提示 `  Enter to select · Esc to go back`。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let rows = self.rows();
        let primary = rows
            .iter()
            .map(|(_, label, _)| pi_tui::display_width(label))
            .max()
            .unwrap_or(0);
        let mut lines = vec![
            "Theme".to_string(),
            String::new(),
            "Select a theme, or choose Automatic to follow terminal appearance.".to_string(),
            String::new(),
        ];
        for (index, (current, label, description)) in rows.iter().enumerate() {
            let prefix = if index == self.selected { "→ " } else { "  " };
            let marker = if *current { "✓ " } else { "  " };
            let pad = primary.saturating_sub(pi_tui::display_width(label));
            let base = format!("{prefix}{marker}{label}{}", " ".repeat(pad));
            match description {
                Some(text) => lines.push(format!("{base}  {text}")),
                None => lines.push(base.trim_end().to_string()),
            }
        }
        lines.push(String::new());
        lines.push("  Enter to select · Esc to go back".to_string());
        lines
    }
}

#[cfg(test)]
mod theme_tests {
    use super::ThemeSubmenu;

    /// T33 golden：`/settings` → Theme 子菜单的 10 行逐字节对齐。
    #[test]
    fn lines_match_t33_submenu() {
        let submenu = ThemeSubmenu::new(
            vec!["dark".to_string(), "light".to_string()],
            "dark".to_string(),
        );
        let expected = [
            "Theme",
            "",
            "Select a theme, or choose Automatic to follow terminal appearance.",
            "",
            "    Automatic  Use separate themes for light and dark terminal appearance",
            "→ ✓ dark",
            "    light",
            "",
            "  Enter to select · Esc to go back",
        ];
        assert_eq!(submenu.lines(), expected);
    }

    /// 选中项默认落在当前主题；上下移动环绕。
    #[test]
    fn selection_starts_at_current_and_wraps() {
        let mut submenu = ThemeSubmenu::new(
            vec!["dark".to_string(), "light".to_string()],
            "light".to_string(),
        );
        assert_eq!(submenu.selected, 2, "light 在 Automatic 之后的第 2 项");
        submenu.move_down();
        assert_eq!(submenu.selected, 0, "末项下移应环绕到 Automatic");
        submenu.move_up();
        assert_eq!(submenu.selected, 2, "首项上移应环绕到末项");
    }
}

/// `/settings` → Warnings 子菜单（ref `settings-selector.ts:136-176` 的 `WarningSettingsSubmenu`）。
///
/// 只有一个条目 `anthropic-extra-usage`（`values: ["true","false"]`），当前值取
/// `warnings.anthropicExtraUsage ?? true`；子菜单**没有搜索框**（`SettingsList` 未传 `enableSearch`），
/// 因此没有 `>` 行，提示是 `  Enter/Space to change · Esc to cancel`（`settings-list.ts:314-322` 的非搜索分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningsSubmenu {
    /// `warnings.anthropicExtraUsage`（缺省 `true`，ref `:152`）。
    pub anthropic_extra_usage: bool,
}

impl WarningsSubmenu {
    /// 新建（默认 `true`）。
    #[must_use]
    pub fn new(anthropic_extra_usage: bool) -> Self {
        Self {
            anthropic_extra_usage,
        }
    }

    /// 切换并返回新值（ref `:161-166` 的 onChange）。
    pub fn toggle(&mut self) -> bool {
        self.anthropic_extra_usage = !self.anthropic_extra_usage;
        self.anthropic_extra_usage
    }

    /// 面板行（不含上下边框）。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        const LABEL: &str = "Anthropic extra usage";
        const DESCRIPTION: &str = "Warn when Anthropic subscription auth may use paid extra usage";
        let value = if self.anthropic_extra_usage {
            "true"
        } else {
            "false"
        };
        let mut lines = vec![
            format!("→ {LABEL}  {value}"),
            String::new(),
            format!("  {DESCRIPTION}"),
            String::new(),
            "  Enter/Space to change · Esc to cancel".to_string(),
        ];
        // 只有一条、必然放得下 ⇒ 不出现 `(i/n)`（ref `:166-169` 的条件不成立）。
        lines.shrink_to_fit();
        lines
    }
}

#[cfg(test)]
mod warnings_tests {
    use super::WarningsSubmenu;

    /// T34 golden：Warnings 子菜单 5 行逐字节对齐。
    #[test]
    fn lines_match_t34_submenu() {
        let submenu = WarningsSubmenu::new(true);
        let expected = [
            "→ Anthropic extra usage  true",
            "",
            "  Warn when Anthropic subscription auth may use paid extra usage",
            "",
            "  Enter/Space to change · Esc to cancel",
        ];
        assert_eq!(submenu.lines(), expected);
    }

    /// 切换：true → false → true，且行内值同步变化。
    #[test]
    fn toggle_flips_value_and_line() {
        let mut submenu = WarningsSubmenu::new(true);
        assert!(!submenu.toggle());
        assert!(
            submenu.lines()[0].ends_with("false"),
            "{:?}",
            submenu.lines()
        );
        assert!(submenu.toggle());
        assert!(
            submenu.lines()[0].ends_with("true"),
            "{:?}",
            submenu.lines()
        );
    }
}

/// `/settings` → `Default thinking level per model` 的两步子菜单（P7-7c3d）。
///
/// 规格来自 ref `settings-selector.ts:589-690`（`submenu: SteppedSubmenu([model, level], …, {loop:true})`）：
/// - 第 1 步：标题 `Per-Model Thinking Level`、描述 `Step 1/2 · Select a model to configure`，
///   列表项 label = `<id> [<provider>]`，可搜索（有 `>` 行，提示 `  Type to filter · Enter to select · Esc to go back`）；
/// - 第 2 步：标题 `Thinking Level for <id> [<provider>]`、描述
///   `Step 2/2 · Select default thinking level for this model`，列表项 `${current?"✓ ":"  "}${level}` + 描述
///   （非 reasoning 模型只有 `off` 一级，`THINKING_DESCRIPTIONS` 里 `off`= `No reasoning`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelThinkingSubmenu {
    /// 可选模型：`(id, provider)`。
    pub models: Vec<(String, String)>,
    /// 第 1 步搜索框内容。
    pub query: String,
    /// 第 1 步选中下标。
    pub selected: usize,
    /// 已进入第 2 步时记录选中的模型下标。
    pub step: Option<usize>,
}

impl ModelThinkingSubmenu {
    /// 新建（只有一步可选模型）。
    #[must_use]
    pub fn new(models: Vec<(String, String)>) -> Self {
        Self {
            models,
            query: String::new(),
            selected: 0,
            step: None,
        }
    }

    /// 第 1 步的过滤结果（大小写无关子串，匹配 id/provider）。
    #[must_use]
    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return (0..self.models.len()).collect();
        }
        self.models
            .iter()
            .enumerate()
            .filter(|(_, (id, provider))| {
                id.to_lowercase().contains(&query) || provider.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 描述文字（第 1 步查搜索框，第 2 步忽略）。
    pub fn push(&mut self, ch: char) {
        if self.step.is_none() {
            self.query.push(ch);
            self.selected = 0;
        }
    }

    /// 删除搜索框末字符。
    pub fn pop(&mut self) {
        if self.step.is_none() {
            self.query.pop();
            self.selected = 0;
        }
    }

    /// 上移（环绕）。
    pub fn move_up(&mut self) {
        let len = if self.step.is_none() {
            self.filtered_indices().len()
        } else {
            1
        };
        if len == 0 {
            return;
        }
        self.selected = if self.selected == 0 {
            len - 1
        } else {
            self.selected - 1
        };
    }

    /// 下移（环绕）。
    pub fn move_down(&mut self) {
        let len = if self.step.is_none() {
            self.filtered_indices().len()
        } else {
            1
        };
        if len == 0 {
            return;
        }
        self.selected = if self.selected == len - 1 {
            0
        } else {
            self.selected + 1
        };
    }

    /// Enter：第 1 步进第 2 步；第 2 步返回 `(model_index, level)` 供调用点落盘。
    pub fn activate(&mut self) -> Option<(usize, &'static str)> {
        if self.step.is_none() {
            let indices = self.filtered_indices();
            if let Some(&index) = indices.get(self.selected) {
                self.step = Some(index);
                self.selected = 0;
            }
            return None;
        }
        // 非 reasoning 模型只有 `off` 一档（ref `:638-642`）。
        self.step.map(|index| (index, "off"))
    }

    /// Esc：第 2 步回第 1 步；第 1 步返回 `false` 表示交给外层关闭子菜单。
    pub fn back(&mut self) -> bool {
        if self.step.is_some() {
            self.step = None;
            self.selected = 0;
            true
        } else {
            false
        }
    }

    /// 面板行（不含上下边框）。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        match self.step {
            None => {
                let mut lines = vec![
                    "Per-Model Thinking Level".to_string(),
                    String::new(),
                    "Step 1/2 · Select a model to configure".to_string(),
                    String::new(),
                    if self.query.is_empty() {
                        ">".to_string()
                    } else {
                        format!("> {}", self.query)
                    },
                    String::new(),
                ];
                let indices = self.filtered_indices();
                if indices.is_empty() {
                    lines.push("  No matching models".to_string());
                } else {
                    for (pos, index) in indices.iter().enumerate() {
                        let (id, provider) = &self.models[*index];
                        let prefix = if pos == self.selected { "→ " } else { "  " };
                        lines.push(format!("{prefix}{id} [{provider}]"));
                    }
                }
                lines.push(String::new());
                lines.push("  Type to filter · Enter to select · Esc to go back".to_string());
                lines
            }
            Some(index) => {
                let (id, provider) = &self.models[index];
                let mut lines = vec![
                    format!("Thinking Level for {id} [{provider}]"),
                    String::new(),
                    "Step 2/2 · Select default thinking level for this model".to_string(),
                    String::new(),
                ];
                // 主列宽：ref 的 `SUBMENU_SELECT_LIST_LAYOUT` 写 12，但 SelectList 会把 label 前的
                // 2 列标记**算进**主列，于是实际留给 label 的是 10（T35 golden：`→   off` 后共 7 空格
                // = pad 5 + 分隔 2）。
                const PRIMARY: usize = 10;
                let label = "  off";
                let pad = PRIMARY.saturating_sub(pi_tui::display_width(label));
                lines.push(format!("→ {label}{}  No reasoning", " ".repeat(pad)));
                lines.push(String::new());
                lines.push("  Enter to select · Esc to go back".to_string());
                lines
            }
        }
    }
}

#[cfg(test)]
mod model_thinking_tests {
    use super::ModelThinkingSubmenu;

    fn submenu() -> ModelThinkingSubmenu {
        ModelThinkingSubmenu::new(vec![(
            "mock-model".to_string(),
            "openai-completions".to_string(),
        )])
    }

    /// T35 第 1 步 golden（10 行）逐字节对齐。
    #[test]
    fn step1_matches_t35() {
        let expected = [
            "Per-Model Thinking Level",
            "",
            "Step 1/2 · Select a model to configure",
            "",
            ">",
            "",
            "→ mock-model [openai-completions]",
            "",
            "  Type to filter · Enter to select · Esc to go back",
        ];
        assert_eq!(submenu().lines(), expected);
    }

    /// T35 第 2 步 golden（8 行）逐字节对齐：只有 `off` 一档、主列 12 宽。
    #[test]
    fn step2_matches_t35() {
        let mut sub = submenu();
        assert_eq!(sub.activate(), None, "第 1 步 Enter 只进第 2 步");
        let expected = [
            "Thinking Level for mock-model [openai-completions]",
            "",
            "Step 2/2 · Select default thinking level for this model",
            "",
            "→   off       No reasoning",
            "",
            "  Enter to select · Esc to go back",
        ];
        assert_eq!(sub.lines(), expected);
    }

    /// 第 2 步 Enter 返回 (模型下标, 档位)；Esc 从第 2 步退回第 1 步。
    #[test]
    fn activate_and_back() {
        let mut sub = submenu();
        sub.activate();
        assert_eq!(sub.activate(), Some((0, "off")));
        assert!(sub.back(), "第 2 步 Esc 应退回第 1 步");
        assert!(!sub.back(), "第 1 步 Esc 应交回外层");
        assert_eq!(sub.lines()[0], "Per-Model Thinking Level");
    }
}

/// `/resume` 会话选择器（P7-7d1：**只做首屏 + 空列表**）。
///
/// 规格来源 `modes/interactive/components/session-selector.ts`：
/// - 布局 `buildBaseLayout:742-755`：空行 → 顶部边框 → 空行 → 头部 → 空行 → 内容 → 空行 → 底部边框；
/// - 头部 `SessionSelectorHeader.render:150-189`：第 1 行 `左标题 + 空格 + 右状态`，
///   第 2/3 行是两条提示（`tab scope · re:<pattern> regex · "phrase" exact` /
///   `ctrl+s sort · ctrl+n named · ctrl+d delete · ctrl+p path (off) · ctrl+r rename`）；
/// - 空列表文案（T36 golden）：`  No sessions in current folder. Press Tab to view all.`。
///
/// 会话列表里的一行（`/resume`）。
///
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// 会话文件路径（`/resume` 选中后要用它重放并接管写入）。
    pub path: std::path::PathBuf,
    /// 会话名（`session_info` 条目的 `name`；有名字时列表显示名字而不是首条消息）。
    pub name: Option<String>,
    /// 摘要：第一条**用户**消息的文本；没有则 `None`（渲染成 `(no messages)`）。
    pub summary: Option<String>,
    /// 会话所属工作目录（`All` 作用域下显示，经 `shorten_path`）。
    pub cwd: String,
    /// 消息条数（`"type":"message"` 的行数）。
    pub message_count: usize,
    /// 最后修改时间（毫秒时间戳，用于相对时间）。
    pub modified_ms: u128,
}

/// 相对时间（ref `session-selector.ts:35-52` 的 `formatSessionDate`）。
#[must_use]
pub fn format_session_age(now_ms: u128, then_ms: u128) -> String {
    let diff_ms = now_ms.saturating_sub(then_ms);
    let mins = diff_ms / 60_000;
    let hours = diff_ms / 3_600_000;
    let days = diff_ms / 86_400_000;
    if mins < 1 {
        "now".to_string()
    } else if mins < 60 {
        format!("{mins}m")
    } else if hours < 24 {
        format!("{hours}h")
    } else if days < 7 {
        format!("{days}d")
    } else if days < 30 {
        format!("{}w", days / 7)
    } else if days < 365 {
        format!("{}mo", days / 30)
    } else {
        format!("{}y", days / 365)
    }
}

/// 把绝对路径缩成 `~/…`（ref `session-selector.ts:24-33` 的 `shortenPath`）。
#[must_use]
pub fn shorten_path(path: &str, home: &str) -> String {
    if !home.is_empty() {
        if path == home {
            return "~".to_string();
        }
        if let Some(rest) = path.strip_prefix(home) {
            if rest.starts_with('/') {
                return format!("~{rest}");
            }
        }
    }
    path.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSelector {
    /// 是否 `All` 作用域（`Tab` 切换；false = `Current Folder`）。
    pub all_scopes: bool,
    /// 排序模式（ref `session-selector.ts` 的 `Sort: Threaded | Recent`，`ctrl+s` 切换）。
    pub sort_recent: bool,
    /// 是否显示完整路径（`ctrl+p`，头部提示 `path (on|off)`）。
    pub show_path: bool,
    /// 名称过滤是否只看命名会话（`ctrl+n`，头部 `Name: All | Named`）。
    pub named_only: bool,
    /// 重命名模式（`ctrl+r`）下的输入文本；`None` = 不在重命名模式。
    pub rename: Option<String>,
    /// 是否处于删除确认态（`ctrl+d`，ref `session-selector.ts:543-557`）。
    pub confirming_delete: bool,
    /// 已加载的会话（`All` 作用域用；`Current Folder` 按 `cwd` 过滤）。
    pub sessions: Vec<SessionRow>,
    /// 当前会话的 `cwd`（`Current Folder` 过滤依据）。
    pub current_cwd: String,
    /// 是否按 `current_cwd` 过滤（ref `session-manager.ts:1904-1906` 的 `filterCwd`）：
    /// 只有**自定义会话目录**才过滤；默认分区目录里列出目录内全部会话
    /// （目录改名后 header 里的旧 cwd 不会让会话消失）。
    pub filter_cwd: bool,
    /// `HOME`（`shorten_path` 用）。
    pub home: String,
    /// 选中下标（过滤后列表）。
    pub selected: usize,
}

impl ResumeSelector {
    /// 新建（默认 `Current Folder` 作用域，ref `:716` 的 `scope: "current"`）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            all_scopes: false,
            sort_recent: false,
            show_path: false,
            named_only: false,
            rename: None,
            confirming_delete: false,
            sessions: Vec::new(),
            current_cwd: String::new(),
            filter_cwd: false,
            home: String::new(),
            selected: 0,
        }
    }

    /// 当前作用域下可见的会话下标（`All` = 全部；`Current Folder` = `cwd` 相同）。
    #[must_use]
    pub fn visible_indices(&self) -> Vec<usize> {
        // ref `session-selector.ts:459-470`：作用域（Current Folder / All）+ `ctrl+n`
        // 的 `Name: Named` 过滤要**叠加**（此前只做了标签、没做过滤）。
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, row)| self.all_scopes || !self.filter_cwd || row.cwd == self.current_cwd)
            .filter(|(_, row)| !self.named_only || row.name.is_some())
            .map(|(i, _)| i)
            .collect()
    }

    /// 可见窗口上限（ref `session-selector.ts:311`：`maxVisible = 10`）。
    pub const MAX_VISIBLE: usize = 10;

    /// 可见条数变化后把选中项夹回合法范围（ref `session-selector.ts:394`）。
    fn clamp_selection(&mut self) {
        let len = self.visible_indices().len();
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    /// 上移选中项（ref `session-selector.ts:614`：`max(0, selectedIndex - 1)`，**到头夹住**不环绕）。
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// 下移选中项（ref `session-selector.ts:618`：`min(len - 1, selectedIndex + 1)`）。
    pub fn move_down(&mut self) {
        let len = self.visible_indices().len();
        if len > 0 {
            self.selected = (self.selected + 1).min(len - 1);
        }
    }

    /// 翻页（ref `:622-626`：`selectedIndex ∓ maxVisible`）。
    pub fn page_up(&mut self, page: usize) {
        self.selected = self.selected.saturating_sub(page.max(1));
    }

    /// 翻页（向下）。
    pub fn page_down(&mut self, page: usize) {
        let len = self.visible_indices().len();
        if len > 0 {
            self.selected = (self.selected + page.max(1)).min(len - 1);
        }
    }

    /// 切换排序模式（`ctrl+s`，ref `app.session.toggleSort`）。
    pub fn toggle_sort(&mut self) {
        self.sort_recent = !self.sort_recent;
        self.clamp_selection();
    }

    /// 进入删除确认态（`ctrl+d`，ref `:543-557`）。
    pub fn enter_delete_confirm(&mut self) {
        self.confirming_delete = true;
    }

    /// 退出删除确认态（Esc / Enter 之后）。
    pub fn exit_delete_confirm(&mut self) {
        self.confirming_delete = false;
    }

    /// 当前选中的会话文件路径（删除用）。
    #[must_use]
    pub fn selected_path(&self) -> Option<std::path::PathBuf> {
        let visible = self.visible_indices();
        let pos = self.selected.min(visible.len().saturating_sub(1));
        visible
            .get(pos)
            .map(|index| self.sessions[*index].path.clone())
    }

    /// 进入重命名模式（`ctrl+r`，ref `session-selector.ts:885-896` 的 `enterRenameMode`）。
    pub fn enter_rename(&mut self) {
        self.rename = Some(String::new());
    }

    /// 退出重命名模式（Esc / 保存后）。
    pub fn exit_rename(&mut self) {
        self.rename = None;
    }

    /// 重命名输入框追加字符。
    pub fn rename_push(&mut self, ch: char) {
        if let Some(text) = self.rename.as_mut() {
            text.push(ch);
        }
    }

    /// 重命名输入框退格。
    pub fn rename_pop(&mut self) {
        if let Some(text) = self.rename.as_mut() {
            text.pop();
        }
    }

    /// 切换"显示完整路径"（`ctrl+p`，ref `app.session.togglePath`）。
    pub fn toggle_path(&mut self) {
        self.show_path = !self.show_path;
    }

    /// 切换"只看命名会话"（`ctrl+n`，ref `app.session.toggleNamedFilter`）。
    pub fn toggle_named(&mut self) {
        self.named_only = !self.named_only;
        self.clamp_selection();
    }

    /// 切换作用域（ref `tui.input.tab`）。
    pub fn toggle_scope(&mut self) {
        self.all_scopes = !self.all_scopes;
        self.clamp_selection();
    }

    /// 面板行（**不含**上下边框；边框由 `render_all` 画）。
    #[must_use]
    pub fn lines(&self, width: usize) -> Vec<String> {
        if let Some(text) = self.rename.as_ref() {
            // ref `:885-900`：重命名模式**整块替换**成 `Rename Session` 面板。
            return vec![
                String::new(),
                " Rename Session".to_string(),
                String::new(),
                if text.is_empty() {
                    ">".to_string()
                } else {
                    format!("> {text}")
                },
                String::new(),
                " enter to save · escape/ctrl+c to cancel".to_string(),
                String::new(),
            ];
        }
        let title = if self.all_scopes {
            // golden 实测（T37）：All 作用域标题是 `Resume Session (All)`，不是 `(All Folders)`。
            "Resume Session (All)"
        } else {
            "Resume Session (Current Folder)"
        };
        // 右状态：作用域单选 + 名称过滤 + 排序（ref `:150-189`）。
        let scope = if self.all_scopes {
            "○ Current Folder | ◉ All"
        } else {
            "◉ Current Folder | ○ All"
        };
        let sort = if self.sort_recent {
            "Recent"
        } else {
            "Threaded"
        };
        let name = if self.named_only { "Named" } else { "All" };
        let right = format!("{scope}  Name: {name}  Sort: {sort}");
        let left_width = pi_tui::display_width(title);
        let right_width = pi_tui::display_width(&right);
        let spacing = width.saturating_sub(left_width + right_width);
        let header =
            pi_tui::clamp_to_width(&format!("{title}{}{right}", " ".repeat(spacing)), width);
        let mut lines = vec![
            String::new(),
            header,
            if self.confirming_delete {
                // ref `:158-161`：确认态把第 1 行提示换成红色确认文案，第 2 行留空。
                "Delete session? enter confirm · escape/ctrl+c cancel".to_string()
            } else {
                "tab scope · re:<pattern> regex · \"phrase\" exact".to_string()
            },
            if self.confirming_delete {
                String::new()
            } else {
                format!(
                    "ctrl+s sort · ctrl+n named · ctrl+d delete · ctrl+p path ({}) · ctrl+r rename",
                    if self.show_path { "on" } else { "off" }
                )
            },
            String::new(),
        ];
        lines.push(">".to_string());
        lines.push(String::new());
        let visible = self.visible_indices();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        if visible.is_empty() {
            if self.all_scopes {
                // golden 实测（T37）：无会话时文案是 `  No sessions found`（**无句号**）。
                lines.push("  No sessions found".to_string());
            } else if self.named_only {
                // T54 golden：开了 `ctrl+n` 之后空态文案不同。
                lines.push(
                    "  No named sessions in current folder. Press ctrl+n to show all, or Tab to view all."
                        .to_string(),
                );
            } else {
                lines.push("  No sessions in current folder. Press Tab to view all.".to_string());
            }
        } else {
            // ref `session-selector.ts:450-454`：可见窗口 10 条，选中项尽量居中
            // （`max(0, min(selected - maxVisible/2, total - maxVisible))`）。
            let selected = self.selected.min(visible.len().saturating_sub(1));
            let (start, end) = pi_tui::selector_window(visible.len(), selected, Self::MAX_VISIBLE);
            for (pos, index) in visible.iter().enumerate() {
                if pos < start || pos >= end {
                    continue;
                }
                let row = &self.sessions[*index];
                let cursor = if pos == selected { "› " } else { "  " };
                // ref `:468-470`：有名字显示名字，否则显示首条消息。
                // ref `session-selector.ts:469-471`：名字优先，其次首条消息；
                // 两种来源都做"控制字符 → 空格 + trim"（多行首消息否则会把换行带进列表行）。
                let summary = row
                    .name
                    .clone()
                    .or_else(|| row.summary.clone())
                    .unwrap_or_else(|| "(no messages)".to_string())
                    .chars()
                    .map(|ch| if ch.is_control() { ' ' } else { ch })
                    .collect::<String>()
                    .trim()
                    .to_string();
                let age = format_session_age(now_ms, row.modified_ms);
                let mut right = format!("{} {age}", row.message_count);
                if self.all_scopes && !row.cwd.is_empty() {
                    right = format!("{} {right}", shorten_path(&row.cwd, &self.home));
                }
                let left_width = pi_tui::display_width(cursor) + pi_tui::display_width(&summary);
                let right_width = pi_tui::display_width(&right);
                let spacing = width.saturating_sub(left_width + right_width).max(1);
                lines.push(pi_tui::clamp_to_width(
                    &format!("{cursor}{summary}{}{right}", " ".repeat(spacing)),
                    width,
                ));
            }
            // ref `:519-525`：**仅当**窗口被裁剪（`start > 0 || end < total`）才追加
            // `  (n/total)`；不裁剪时不加，保证单会话/少量会话的面板与旧 golden 一致。
            if start > 0 || end < visible.len() {
                lines.push(pi_tui::selector_scroll_hint(selected, visible.len()));
            }
        }
        lines.push(String::new());
        lines
    }
}

impl Default for ResumeSelector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod resume_tests {
    use super::ResumeSelector;

    /// T36 golden：会话选择器首屏（含头部两行提示、`>`、空态文案）逐字节对齐。
    #[test]
    fn lines_match_t36_first_screen() {
        let lines = ResumeSelector::new().lines(100);
        let expected = [
            "",
            "Resume Session (Current Folder)                  ◉ Current Folder | ○ All  Name: All  Sort: Threa",
            "tab scope · re:<pattern> regex · \"phrase\" exact",
            "ctrl+s sort · ctrl+n named · ctrl+d delete · ctrl+p path (off) · ctrl+r rename",
            "",
            ">",
            "",
            "  No sessions in current folder. Press Tab to view all.",
            "",
        ];
        assert_eq!(lines.len(), expected.len(), "{lines:#?}");
        // 头部被宽度截断（golden 行 12 在 100 列处截断）⇒ 逐字符前缀比较
        assert_eq!(
            lines[1].chars().count(),
            100,
            "头部应截断到 100 列：{:?}",
            lines[1]
        );
        assert!(lines[1].starts_with("Resume Session (Current Folder)"));
        assert_eq!(&lines[2..], &expected[2..]);
    }

    /// 作用域切换（`Tab`）：标题与单选状态同步变化。
    #[test]
    fn scope_toggle_updates_title_and_radios() {
        let mut selector = ResumeSelector::new();
        selector.toggle_scope();
        let lines = selector.lines(120);
        assert!(
            lines[1].starts_with("Resume Session (All)"),
            "{:?}",
            lines[1]
        );
        assert!(
            lines[1].contains("○ Current Folder | ◉ All"),
            "{:?}",
            lines[1]
        );
    }
}

/// 消息文本（ref `session-manager.ts:774-783` 的 `extractTextContent`）：
/// `content` 是字符串就用它；是分块数组就把所有 `type == "text"` 的块用空格拼起来。
/// 会话文件里的 content **几乎总是数组**，早先只认字符串 ⇒ `/resume` 列表一律
/// `(no messages)`（真机：有内容也显示无内容）。
fn message_text(message: &serde_json::Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.trim().to_string());
    }
    let parts: Vec<&str> = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("text"))
        .filter_map(|block| block.get("text").and_then(|v| v.as_str()))
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(" ").trim().to_string())
}

/// 扫描会话目录（`*.jsonl`），产出行数据（`/resume` 用）。
///
/// 每条会话：首行的 `cwd`/`timestamp` 决定工作目录与时间；`"type":"message"` 行数决定消息数；
/// 摘要取**第一条 user 消息**的文本（ref 的 `displayText`；取不到 ⇒ `None`）。
///
/// 解析失败的**单个文件**被跳过（不 panic、不中断整个列表）：会话文件是外部输入。
#[must_use]
pub fn scan_sessions(dir: &std::path::Path) -> Vec<SessionRow> {
    let mut rows = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return rows;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut cwd = String::new();
        let mut header_ms: Option<u128> = None;
        let mut last_activity_ms: Option<u128> = None;
        let mut message_count = 0usize;
        let mut summary: Option<String> = None;
        let mut name: Option<String> = None;
        let mut header_seen = false;
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            // ref `session-manager.ts:809-813`：**第一条可解析条目必须是 `session`**，
            // 否则这个文件不算会话（返回 null，不列出来）。
            if !header_seen {
                if record.get("type").and_then(|v| v.as_str()) != Some("session") {
                    break;
                }
                header_seen = true;
                cwd = record
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                header_ms = record
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(parse_rfc3339_ms);
                continue;
            }
            let kind = record.get("type").and_then(|v| v.as_str());
            if kind == Some("session_info") {
                name = record
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string);
            }
            if kind.is_some_and(|kind| kind == "message") {
                message_count += 1;
                // ref `getMessageActivityTime`：条目时间戳参与"最后活动时间"，
                // 列表排序与 `3d`/`2h` 相对时间都按它算（此前只取了首行时间）。
                if let Some(stamp) = record.get("timestamp").and_then(|v| v.as_str()) {
                    if let Some(ms) = parse_rfc3339_ms(stamp) {
                        last_activity_ms = Some(last_activity_ms.map_or(ms, |prev| prev.max(ms)));
                    }
                }
                if summary.is_none() {
                    let message = record.get("message");
                    if message.and_then(|m| m.get("role")).and_then(|r| r.as_str()) == Some("user")
                    {
                        summary = message.and_then(message_text).filter(|s| !s.is_empty());
                    }
                }
            }
        }
        if !header_seen {
            continue;
        }
        // ref `session-manager.ts:860-868`：最后活动时间 → 首行时间 → 文件 mtime。
        let modified_ms = last_activity_ms
            .or(header_ms)
            .or_else(|| {
                std::fs::metadata(&path)
                    .ok()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|since| since.as_millis())
            })
            .unwrap_or(0);
        rows.push(SessionRow {
            path: path.clone(),
            name,
            summary,
            cwd,
            message_count,
            modified_ms,
        });
    }
    // 最近修改的在前（ref `buildSessionTree` 之后按 `latestActivity` 降序）。
    rows.sort_by_key(|row| std::cmp::Reverse(row.modified_ms));
    rows
}

/// 解析 `2026-01-01T00:00:00.000Z` 形式的 RFC3339 时间戳为毫秒。
///
/// 只支持会话文件里出现的固定形态（`YYYY-MM-DDTHH:MM:SS[.mmm]Z`）；其它形态返回 `None`
/// （调用方按 0 处理 ⇒ 时间显示为最旧，而不是 panic）。
#[must_use]
pub fn parse_rfc3339_ms(text: &str) -> Option<u128> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let parse =
        |range: std::ops::Range<usize>| -> Option<i64> { text.get(range)?.parse::<i64>().ok() };
    let year = parse(0..4)?;
    let month = parse(5..7)?;
    let day = parse(8..10)?;
    let hour = parse(11..13)?;
    let minute = parse(14..16)?;
    let second = parse(17..19)?;
    let millis = text
        .get(19..)
        .filter(|rest| rest.starts_with('.'))
        .and_then(|rest| rest.get(1..4))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // 天数换算（Howard Hinnant 的 days_from_civil 简化版；不需要处理闰秒）。
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u128::try_from(secs).ok().map(|s| s * 1000 + millis as u128)
}

#[cfg(test)]
mod resume_list_tests {
    use super::{format_session_age, scan_sessions, shorten_path, ResumeSelector, SessionRow};

    /// `10mo` 对应的毫秒数（`format_session_age` 用 `days / 30` 算月）。
    const AGE_10_MONTHS_MS: u128 = 300 * 86_400_000;

    fn now_millis_for_test() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or(0)
    }

    /// 会话文件里的 `content` 是**分块数组**（真机全是这种）：`/resume` 必须取出文本，
    /// 不能显示 `(no messages)`（P8e-19 真机缺陷）。字符串 content 也兼容。
    #[test]
    fn scan_sessions_reads_text_from_content_blocks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.jsonl");
        let body = [
            r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-10-01T00:00:00.000Z","cwd":"/w"}"#,
            r#"{"type":"message","timestamp":"2026-10-01T00:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"先 ls"},{"type":"text","text":"再读 AGENTS.md"}]}}"#,
            r#"{"type":"message","timestamp":"2026-10-01T00:00:02.000Z","message":{"role":"user","content":"第二句"}}"#,
        ]
        .join("\n");
        std::fs::write(&path, body).expect("写会话");

        let rows = scan_sessions(dir.path());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].summary.as_deref(), Some("先 ls 再读 AGENTS.md"));
        assert_eq!(rows[0].message_count, 2);
    }

    /// `modified` 取**最后一条消息**的时间（ref `getMessageActivityTime`），不是首行时间。
    #[test]
    fn scan_sessions_uses_last_message_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("b.jsonl");
        let body = [
            r#"{"type":"session","version":3,"id":"s2","timestamp":"2026-10-01T00:00:00.000Z","cwd":"/w"}"#,
            r#"{"type":"message","timestamp":"2026-10-02T00:00:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"x"}]}}"#,
        ]
        .join("\n");
        std::fs::write(&path, body).expect("写会话");

        let rows = scan_sessions(dir.path());
        assert_eq!(rows.len(), 1);
        let header_ms = super::parse_rfc3339_ms("2026-10-01T00:00:00.000Z").expect("时间可解析");
        let last_ms = super::parse_rfc3339_ms("2026-10-02T00:00:00.000Z").expect("时间可解析");
        assert_eq!(rows[0].modified_ms, last_ms);
        assert!(rows[0].modified_ms > header_ms);
    }

    /// 首行不是 `session` 的文件不算会话（ref `session-manager.ts:809-813` 返回 null）。
    #[test]
    fn scan_sessions_skips_files_without_session_header() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("c.jsonl"),
            r#"{"type":"message","timestamp":"2026-10-01T00:00:00.000Z","message":{"role":"user","content":"x"}}"#,
        )
        .expect("写会话");
        assert!(scan_sessions(dir.path()).is_empty());
    }

    /// 多行首条消息在列表行里必须压成一行（ref 把控制字符替换成空格）。
    #[test]
    fn row_normalizes_newlines_in_summary() {
        let mut selector = ResumeSelector::new();
        selector.sessions = vec![SessionRow {
            path: std::path::PathBuf::from("/a.jsonl"),
            name: None,
            summary: Some("第一行\n第二行".to_string()),
            cwd: String::new(),
            message_count: 1,
            modified_ms: now_millis_for_test(),
        }];
        let lines = selector.lines(60);
        let row = lines
            .iter()
            .find(|l| l.starts_with("› "))
            .expect("应有选中行");
        assert!(!row.contains('\n'), "{row:?}");
        assert!(row.starts_with("› 第一行 第二行"), "{row:?}");
    }

    /// T38 golden：`All` 作用域下列出一行（`› (no messages) … <cwd> 1 9mo`）。
    #[test]
    fn all_scope_row_matches_t38() {
        let mut selector = ResumeSelector::new();
        selector.all_scopes = true;
        selector.home = "/root".to_string();
        selector.sessions = vec![SessionRow {
            path: std::path::PathBuf::from("/tmp/s.jsonl"),
            name: None,
            summary: None,
            cwd: "/tmp/tui-T38-resume-list-abc/work".to_string(),
            message_count: 1,
            // 时间戳用**相对现在**的固定偏移：绝对日期会让"9mo"这类断言随时间失效
            // （2026-10-28 起 2026-01-01 已跨到 10mo，测试就会假失败）。
            modified_ms: now_millis_for_test() - AGE_10_MONTHS_MS,
        }];
        let lines = selector.lines(100);
        let row = lines
            .iter()
            .find(|l| l.starts_with("› "))
            .expect("应有选中行");
        assert!(row.starts_with("› (no messages) "), "{row:?}");
        assert!(
            row.ends_with("/tmp/tui-T38-resume-list-abc/work 1 10mo"),
            "{row:?}"
        );
        assert_eq!(row.chars().count(), 100, "行宽应等于面板宽度：{row:?}");
    }

    /// 相对时间分档（ref `formatSessionDate:35-52`）：now / Nm / Nh / Nd / Nw / Nmo / Ny。
    #[test]
    fn age_buckets_match_ref() {
        let now = 1_000_000_000_000u128;
        assert_eq!(format_session_age(now, now - 30_000), "now");
        assert_eq!(format_session_age(now, now - 5 * 60_000), "5m");
        assert_eq!(format_session_age(now, now - 3 * 3_600_000), "3h");
        assert_eq!(format_session_age(now, now - 2 * 86_400_000), "2d");
        assert_eq!(format_session_age(now, now - 14 * 86_400_000), "2w");
        assert_eq!(format_session_age(now, now - 100 * 86_400_000), "3mo");
        assert_eq!(format_session_age(now, now - 400 * 86_400_000), "1y");
    }

    /// `~` 缩短只替换 HOME 前缀；路径不是 HOME 子目录时原样返回。
    #[test]
    fn shorten_path_only_under_home() {
        assert_eq!(shorten_path("/root/projects/x", "/root"), "~/projects/x");
        assert_eq!(shorten_path("/tmp/x", "/root"), "/tmp/x");
        assert_eq!(shorten_path("/root", "/root"), "~");
    }

    /// `Current Folder` 作用域只保留 `cwd` 相同的会话。
    #[test]
    fn current_folder_filters_by_cwd() {
        let mut selector = ResumeSelector::new();
        selector.current_cwd = "/w1".to_string();
        // ref `session-manager.ts:1904-1906`：只有**自定义**会话目录才按 header cwd 过滤；
        // 默认分区目录里列出全部（目录改名后旧 header 的 cwd 不同也照样出现）。
        selector.filter_cwd = true;
        selector.sessions = vec![
            SessionRow {
                path: std::path::PathBuf::from("/a.jsonl"),
                name: None,
                summary: Some("a".into()),
                cwd: "/w1".into(),
                message_count: 2,
                modified_ms: 2,
            },
            SessionRow {
                path: std::path::PathBuf::from("/b.jsonl"),
                name: None,
                summary: Some("b".into()),
                cwd: "/w2".into(),
                message_count: 3,
                modified_ms: 1,
            },
        ];
        assert_eq!(selector.visible_indices(), vec![0]);
        selector.all_scopes = true;
        assert_eq!(selector.visible_indices(), vec![0, 1]);
    }

    /// 上下键必须能在 `/resume` 列表里移动选中项（真机：多个会话时按上下键没反应）。
    /// ref `session-selector.ts:614-626`：到头**夹住**、不环绕。
    #[test]
    fn resume_selection_moves_with_arrow_keys() {
        let mut selector = ResumeSelector::new();
        selector.filter_cwd = false;
        selector.sessions = (0..3)
            .map(|i| SessionRow {
                path: std::path::PathBuf::from(format!("/s{i}.jsonl")),
                name: None,
                summary: Some(format!("会话 {i}")),
                cwd: "/w".into(),
                message_count: 1,
                modified_ms: 10 - i as u128,
            })
            .collect();

        let cursor_row = |sel: &ResumeSelector| {
            sel.lines(80)
                .into_iter()
                .find(|line| line.starts_with("› "))
                .unwrap_or_default()
        };
        assert!(
            cursor_row(&selector).starts_with("› 会话 0"),
            "初始选中第 1 条"
        );

        selector.move_down();
        assert!(
            cursor_row(&selector).starts_with("› 会话 1"),
            "下键到第 2 条"
        );
        selector.move_down();
        selector.move_down();
        assert!(cursor_row(&selector).starts_with("› 会话 2"), "到底后夹住");
        selector.move_up();
        assert!(
            cursor_row(&selector).starts_with("› 会话 1"),
            "上键回第 2 条"
        );
        selector.move_up();
        selector.move_up();
        assert!(cursor_row(&selector).starts_with("› 会话 0"), "到顶后夹住");
    }

    /// 空列表时上下键不 panic、`selected` 保持 0。
    #[test]
    fn resume_selection_on_empty_list_is_safe() {
        let mut selector = ResumeSelector::new();
        selector.move_down();
        selector.move_up();
        selector.page_down(10);
        assert_eq!(selector.selected, 0);
        assert!(selector.selected_path().is_none());
    }

    /// 默认会话目录（`filter_cwd = false`）：**不**按 header cwd 过滤。
    /// 真机缺陷（OI-2）：`~/Desktop/Colibrì` 改名成 `colibri` 后，旧会话 header 里还是
    /// 老路径，`/resume` 里整个消失。ref 的行为是照列（`list()` 在默认目录下不过滤）。
    #[test]
    fn default_session_dir_does_not_filter_by_header_cwd() {
        let mut selector = ResumeSelector::new();
        selector.current_cwd = "/root/Desktop/colibri".to_string();
        selector.filter_cwd = false;
        selector.sessions = vec![SessionRow {
            path: std::path::PathBuf::from("/a.jsonl"),
            name: None,
            summary: Some("old".into()),
            cwd: "/root/Desktop/Colibrì".into(),
            message_count: 7,
            modified_ms: 2,
        }];
        assert_eq!(selector.visible_indices(), vec![0]);
    }
}

/// 读取会话文件里"要重放到屏幕上的消息"（`/resume` 用）：`(是否助手, 文本)`。
///
/// 只取**有正文**的 user / assistant 消息（ref 重放历史时也是按消息渲染；
/// tool 结果、system、thinking 等本切片不重放，登记为待办）。
///
/// # Errors
/// 文件读不了或不是合法会话 JSONL 时返回错误描述。
pub fn replay_messages(path: &std::path::Path) -> Result<Vec<(bool, String)>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if record.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        let Some(message) = record.get("message") else {
            continue;
        };
        let role = message
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let text_of = |content: &serde_json::Value| -> Option<String> {
            if let Some(s) = content.as_str() {
                return Some(s.trim().to_string());
            }
            let blocks = content.as_array()?;
            let mut parts = Vec::new();
            for block in blocks {
                if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(s) = block.get("text").and_then(|v| v.as_str()) {
                        parts.push(s.to_string());
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n").trim().to_string())
            }
        };
        match role {
            "user" => {
                if let Some(parts) = message.get("content").and_then(text_of) {
                    if !parts.is_empty() {
                        out.push((false, parts));
                    }
                }
            }
            "assistant" => {
                if let Some(parts) = message.get("content").and_then(text_of) {
                    if !parts.is_empty() {
                        out.push((true, parts));
                    }
                }
            }
            other => {
                let _ = other;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod replay_tests {
    use super::replay_messages;

    /// 只取 user/assistant 的正文；system、tool、thinking 不重放。
    #[test]
    fn replays_only_user_and_assistant_text() {
        let dir = std::env::temp_dir().join(format!("pi-rs-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建目录");
        let path = dir.join("s.jsonl");
        let lines = [
            r#"{"type":"session","version":3,"id":"x","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/w"}"#,
            r#"{"type":"message","id":"1","parentId":null,"timestamp":"t","message":{"role":"system","content":"","timestamp":1}}"#,
            r#"{"type":"message","id":"2","parentId":"1","timestamp":"t","message":{"role":"user","content":"hello from seeded session","timestamp":2}}"#,
            r#"{"type":"message","id":"3","parentId":"2","timestamp":"t","message":{"role":"assistant","content":[{"type":"text","text":"seeded assistant reply"}],"api":"openai-completions","provider":"p","model":"m","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"reasoning":0,"totalTokens":2,"cost":{"input":0.0,"output":0.0,"cacheRead":0.0,"cacheWrite":0.0,"total":0.0}},"stopReason":"stop","timestamp":3}}"#,
            r#"{"type":"message","id":"4","parentId":"3","timestamp":"t","message":{"role":"tool","content":"tool out","timestamp":4}}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").expect("写文件");
        let replayed = replay_messages(&path).expect("可读");
        assert_eq!(
            replayed,
            vec![
                (false, "hello from seeded session".to_string()),
                (true, "seeded assistant reply".to_string()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 会话累计用量（`/resume` 之后 footer 要显示它）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SessionUsageTotals {
    /// 输入 token 累计。
    pub input: u64,
    /// 输出 token 累计。
    pub output: u64,
    /// 费用累计（美元）。
    pub cost: f64,
}

/// 累加会话里所有 assistant 消息的 `usage`（input/output/`cost.total`）。
///
/// # Errors
/// 文件读不了时返回错误描述；单行解析失败**跳过**（会话文件是外部输入）。
pub fn session_usage(path: &std::path::Path) -> Result<SessionUsageTotals, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    let mut totals = SessionUsageTotals::default();
    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(message) = record.get("message") else {
            continue;
        };
        if message.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let Some(usage) = message.get("usage") else {
            continue;
        };
        totals.input += usage
            .get("input")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        totals.output += usage
            .get("output")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        totals.cost += usage
            .get("cost")
            .and_then(|c| c.get("total"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
    }
    Ok(totals)
}

/// `/tree` 的一行（P7-7e1/e2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// 行前缀里的类型标记：`[system]` / `user:` / `assistant:` 等。
    pub kind: TreeRowKind,
    /// 该条目上的标签（ref 的 `labelsById`，来自 `label` 条目）；`None` = 无标签。
    pub label: Option<String>,
    /// 条目 id（`label` 条目的 `targetId` 指向它）。
    pub id: String,
    /// 父条目 id（会话条目的 `parentId`；根为 `None`）。
    pub parent_id: Option<String>,
    /// 标签写入时间（`label` 条目的 `timestamp`，已格式化为 `HH:MM` 或 `M/D HH:MM`）。
    pub label_time: Option<String>,
    /// 内容摘要（已去除换行并截断到适合宽度前）。
    pub text: String,
}

/// `/tree` 行类型（决定前缀文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeRowKind {
    /// system 消息：`[system]`。
    System,
    /// 用户消息：`user: …`。
    User,
    /// 助手消息：`assistant: …`。
    Assistant,
    /// 其它（tool 结果等）：`tool: …`。
    Other,
}

/// `/tree` 头部帮助项（ref `tree-selector.ts:1225-1244` 的 `TREE_HELP_ITEMS`，按 ref 顺序）。
const TREE_HELP_ITEMS: &[&str] = &[
    "↑/↓ move",
    "←/→ page",
    "ctrl+←/→ branch",
    "ctrl+x copy",
    "shift+l label",
    "shift+t label time",
    "filters ctrl+d/t/u/l/a",
    "cycle ctrl+o/shift+ctrl+o",
];

/// 把 label 条目的 ISO 时间戳格式化成 `HH:MM`（同日）/ `M/D HH:MM`（同年）/ `YY/M/D HH:MM`
/// （ref `tree-selector.ts:862-884` 的 `formatLabelTimestamp`）。解析失败时返回空串。
#[must_use]
pub fn format_label_timestamp(iso: &str, offset_minutes: i32, now_ms: u128) -> String {
    // ref `tree-selector.ts:862-884` 的 `formatLabelTimestamp`，三档：
    // 同一天 ⇒ `HH:MM`；同年 ⇒ `M/D HH:MM`；其它 ⇒ `YY/M/D HH:MM`。时间按**本地时区**显示。
    let Some(ms) = parse_rfc3339_ms(iso) else {
        return String::new();
    };
    let shifted = shift_minutes(ms, offset_minutes);
    let local_now = shift_minutes(now_ms, offset_minutes);
    let (year, month, day, hour, minute) = civil_from_ms(shifted);
    let (now_year, now_month, now_day, _, _) = civil_from_ms(local_now);
    if (year, month, day) == (now_year, now_month, now_day) {
        format!("{hour:02}:{minute:02}")
    } else if year == now_year {
        format!("{month}/{day} {hour:02}:{minute:02}")
    } else {
        format!("{:02}/{month}/{day} {hour:02}:{minute:02}", year % 100)
    }
}

/// 把时间戳按分钟偏移平移（负值按 0 处理，避免溢出）。
fn shift_minutes(ms: u128, offset_minutes: i32) -> u128 {
    let offset = i64::from(offset_minutes) * 60_000;
    let shifted = ms as i128 + i128::from(offset);
    u128::try_from(shifted.max(0)).unwrap_or(0)
}

/// epoch 毫秒 → `(年, 月, 日, 时, 分)`（civil，UTC 语义；时区由调用方先平移）。
#[must_use]
pub fn civil_from_ms(ms: u128) -> (i64, u32, u32, u32, u32) {
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = (rem / 3600) as u32;
    let minute = ((rem % 3600) / 60) as u32;
    (year, month, day, hour, minute)
}

/// 天数（自 1970-01-01）→ 公历日期（Howard Hinnant 的 `civil_from_days`）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

/// `+0800` / `-0430` → 分钟；**按字符**解析，非"符号 + 4 位 ASCII 数字"一律 `None`。
///
/// 旧实现按**字节**切片（`digits[..2]`），`date +%z` 若返回含多字节字符的内容就会
/// panic（`end byte index 2 is not a char boundary`）——见 `p78a_offset_panic_tests`。
fn parse_utc_offset_compact(text: &str) -> Option<i32> {
    let mut chars = text.chars();
    let sign = match chars.next()? {
        '+' => 1,
        '-' => -1,
        _other => return None,
    };
    let digits: Vec<i32> = chars
        .map(|ch| ch.to_digit(10).and_then(|value| i32::try_from(value).ok()))
        .collect::<Option<Vec<i32>>>()?;
    if digits.len() != 4 {
        return None;
    }
    Some(sign * (digits[0] * 600 + digits[1] * 60 + digits[2] * 10 + digits[3]))
}

/// 本机 UTC 偏移（分钟）。通过 `date +%z` **取一次**并缓存；取不到时按 0（UTC）处理。
///
/// 用外部 `date` 是因为本项目不允许引入时区库/`libc`；`%z` 是 POSIX 标准输出。
#[must_use]
pub fn local_utc_offset_minutes() -> i32 {
    static CACHED: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        let output = std::process::Command::new("date").arg("+%z").output();
        let Ok(output) = output else {
            return 0;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        parse_utc_offset_compact(text.trim()).unwrap_or(0)
    })
}

/// `/tree` 头部块（标题 + 折行帮助 + 搜索行 + 边框 + 空行）；`/tree` 与其子面板共用。
fn tree_header_lines(width: usize) -> Vec<String> {
    let border = "─".repeat(width);
    let mut lines = vec!["   Session Tree".to_string()];
    lines.extend(wrap_help_items(TREE_HELP_ITEMS, width));
    lines.push("  Type to search:".to_string());
    lines.push(border);
    lines.push(String::new());
    lines
}

/// 把帮助项按 `width` 折行（ref `tree-selector.ts:1196-1220` 的 `TreeHelp.render`）：
/// 每行以两空格开头、项之间用 ` · ` 连接；放不下就换行（**换行而不是截断**）。
fn wrap_help_items(items: &[&str], width: usize) -> Vec<String> {
    const INDENT: &str = "  ";
    const SEPARATOR: &str = " · ";
    let mut lines = Vec::new();
    let mut current = String::new();
    for item in items {
        let candidate = if current.is_empty() {
            format!("{INDENT}{item}")
        } else {
            format!("{current}{SEPARATOR}{item}")
        };
        if current.is_empty() || pi_tui::display_width(&candidate) <= width {
            current = candidate;
            continue;
        }
        lines.push(current.trim_end().to_string());
        current = format!("{INDENT}{item}");
    }
    if !current.is_empty() {
        lines.push(current.trim_end().to_string());
    }
    lines
}

/// `/tree` 选择器状态（本切片只做首屏与行渲染）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSelector {
    /// 已加载的树行。
    pub rows: Vec<TreeRow>,
    /// 选中下标（末行 = 最新）。
    pub selected: usize,
    /// 过滤模式（ref `tree-selector.ts:112` 的 `filterMode`，默认 `default`）。
    pub filter_mode: TreeFilterMode,
    /// 是否显示标签时间戳（ref `:116` 的 `showLabelTimestamps`，`shift+t` 切换）。
    pub show_label_timestamps: bool,
    /// 已折叠的条目 id（ref `:117` 的 `foldedNodes`，`ctrl+←` 折叠 / `ctrl+→` 展开）。
    pub folded: std::collections::BTreeSet<String>,
    /// 一屏最多显示多少行（ref `:111` 的 `maxVisibleLines`；T61 实测 = `rows/2` = 15）。
    pub max_visible: usize,
}

/// `/tree` 的过滤模式（ref `tree-selector.ts:1049-1085` 的循环顺序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeFilterMode {
    /// 默认（含工具结果）。
    Default,
    /// 去掉工具结果。
    NoTools,
    /// 只看用户消息。
    UserOnly,
    /// 只看有 label 的条目（本项目未实现 label ⇒ 恒为空）。
    LabeledOnly,
    /// 全部条目。
    All,
}

impl TreeFilterMode {
    /// 下一个模式（ref `:1080-1085` 的正向循环）。
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            TreeFilterMode::Default => TreeFilterMode::NoTools,
            TreeFilterMode::NoTools => TreeFilterMode::UserOnly,
            TreeFilterMode::UserOnly => TreeFilterMode::LabeledOnly,
            TreeFilterMode::LabeledOnly => TreeFilterMode::All,
            TreeFilterMode::All => TreeFilterMode::Default,
        }
    }

    /// 状态标签（ref `:647-667` 的 `getStatusLabels`）：`default` 无标签。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            TreeFilterMode::Default => "",
            TreeFilterMode::NoTools => " [no-tools]",
            TreeFilterMode::UserOnly => " [user]",
            TreeFilterMode::LabeledOnly => " [labeled]",
            TreeFilterMode::All => " [all]",
        }
    }
}

impl TreeSelector {
    /// 新建；默认选中最后一行（ref 打开时定位到当前叶子）。
    #[must_use]
    pub fn new(rows: Vec<TreeRow>) -> Self {
        let selected = rows.len().saturating_sub(1);
        Self {
            rows,
            selected,
            filter_mode: TreeFilterMode::Default,
            show_label_timestamps: false,
            folded: std::collections::BTreeSet::new(),
            max_visible: 15,
        }
    }

    /// 当前过滤模式下可见的行下标（ref `:360-380` 的 `passesFilter` 分支）。
    #[must_use]
    pub fn visible_indices(&self) -> Vec<usize> {
        // 父链上任意一层被折叠 ⇒ 本行不可见（ref `foldedNodes` 的语义）。
        let hidden: std::collections::BTreeSet<&str> = self
            .rows
            .iter()
            .filter(|row| self.folded.contains(&row.id))
            .map(|row| row.id.as_str())
            .collect();
        let mut hidden_all: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for row in &self.rows {
            let mut cursor = row.parent_id.as_deref();
            while let Some(parent) = cursor {
                if hidden.contains(parent) {
                    hidden_all.insert(row.id.clone());
                    break;
                }
                cursor = self
                    .rows
                    .iter()
                    .find(|candidate| candidate.id == parent)
                    .and_then(|candidate| candidate.parent_id.as_deref());
            }
        }
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !hidden_all.contains(&row.id))
            .filter(|(_, row)| match self.filter_mode {
                TreeFilterMode::Default => true,
                TreeFilterMode::NoTools => row.kind != TreeRowKind::Other,
                TreeFilterMode::UserOnly => row.kind == TreeRowKind::User,
                // 本项目没有 label 功能 ⇒ 恒为空（与 ref 无 label 时的结果一致）。
                TreeFilterMode::LabeledOnly => false,
                TreeFilterMode::All => true,
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 当前选中条目在**全量行**里的下标（过滤切换时用它保持选中同一条）。
    fn selected_absolute(&self) -> Option<usize> {
        let visible = self.visible_indices();
        let pos = self.selected.min(visible.len().saturating_sub(1));
        visible.get(pos).copied()
    }

    /// 换过滤模式后重新定位选中项：**优先停在同一行**，该行被过滤掉才夹取。
    ///
    /// T44/T46 golden 实测：`ctrl+o` 之后光标仍在 `user: say hi`；`ctrl+d` 切回默认后
    /// 光标也仍在 `user: say hi`（不是回到第 0 行）。
    fn reselect(&mut self, absolute: Option<usize>) {
        let visible = self.visible_indices();
        self.selected = match absolute {
            Some(target) => visible
                .iter()
                .position(|index| *index == target)
                .unwrap_or_else(|| self.selected.min(visible.len().saturating_sub(1))),
            None => self.selected.min(visible.len().saturating_sub(1)),
        };
    }

    /// 上翻一页（ref `:1026-1028`：`selected - maxVisibleLines`，夹 0）。
    pub fn page_up(&mut self, step: usize) {
        self.selected = self.selected.saturating_sub(step.max(1));
    }

    /// 下翻一页（ref `:1029-1031`：`selected + maxVisibleLines`，夹末行）。
    pub fn page_down(&mut self, step: usize) {
        let last = self.visible_indices().len().saturating_sub(1);
        self.selected = (self.selected + step.max(1)).min(last);
    }

    /// 选中条目是否有子节点（`isFoldable`）。
    #[must_use]
    pub fn is_foldable(&self, index: usize) -> bool {
        // ref `:1117-1124`：有子节点，**且**（是根节点 或 其父节点有多个子节点）才算可折叠。
        // 线性链上（每个父只有一个子）中间节点**不可折叠**——T58 golden 依赖这条：
        // 选中 `user` 时 `ctrl+←` 是"跳到分支起点（system）"，而不是把 user 折叠。
        let id = self.rows[index].id.as_str();
        let has_child = self
            .rows
            .iter()
            .any(|row| row.parent_id.as_deref() == Some(id));
        if !has_child {
            return false;
        }
        // 父条目**不在树里**（如 `thinking_level_change` 这类非 message 条目）⇒ 视为根节点
        // （ref 的 `visibleParentMap` 只记录可见父节点，`parentId` 取不到就是 null）。
        let parent = self.rows[index]
            .parent_id
            .as_deref()
            .filter(|parent| self.rows.iter().any(|row| row.id == *parent));
        match parent {
            None => true,
            Some(parent) => {
                let siblings = self
                    .rows
                    .iter()
                    .filter(|row| row.parent_id.as_deref() == Some(parent))
                    .count();
                siblings > 1
            }
        }
    }

    /// `ctrl+←`（ref `:1010-1017` 的 `foldOrUp`）：
    /// 选中项可折叠且未折叠 ⇒ 折叠；否则跳到**所在分支段的起点**（最靠根的那一层）。
    pub fn fold_or_up(&mut self) {
        let visible = self.visible_indices();
        let Some(&index) = visible.get(self.selected) else {
            return;
        };
        if self.is_foldable(index) && !self.folded.contains(&self.rows[index].id) {
            let id = self.rows[index].id.clone();
            self.folded.insert(id);
            self.reselect(Some(index));
            return;
        }
        // 沿 parentId 往上找到最靠根的祖先（线性链上就是根），把选中项移到那里。
        let mut root = index;
        let mut cursor = self.rows[index].parent_id.clone();
        while let Some(parent) = cursor {
            if let Some(pos) = self.rows.iter().position(|row| row.id == parent) {
                root = pos;
                cursor = self.rows[pos].parent_id.clone();
            } else {
                break;
            }
        }
        self.reselect(Some(root));
    }

    /// `ctrl+→`（ref `:1018-1025` 的 `unfoldOrDown`）：已折叠 ⇒ 展开；否则下移到子分支起点。
    pub fn unfold_or_down(&mut self) {
        let visible = self.visible_indices();
        let Some(&index) = visible.get(self.selected) else {
            return;
        };
        if self.folded.contains(&self.rows[index].id) {
            let id = self.rows[index].id.clone();
            self.folded.remove(&id);
            self.reselect(Some(index));
            return;
        }
        // 下移：找第一个以当前项为祖先的后代（分支子段起点）。
        let mut target = None;
        for pos in index + 1..self.rows.len() {
            if self.rows[pos].parent_id.as_deref() == Some(self.rows[index].id.as_str()) {
                target = Some(pos);
                break;
            }
        }
        if let Some(pos) = target {
            self.reselect(Some(pos));
        }
    }

    /// 切换"显示标签时间戳"（ref `:1098` 的 `app.tree.toggleLabelTimestamp`）。
    pub fn toggle_label_timestamps(&mut self) {
        self.show_label_timestamps = !self.show_label_timestamps;
    }

    /// 直选/切换过滤模式（ref `:1049-1069`：再按一次同一个键就回到 `default`）。
    pub fn set_filter(&mut self, mode: TreeFilterMode) {
        let absolute = self.selected_absolute();
        self.filter_mode = if self.filter_mode == mode {
            TreeFilterMode::Default
        } else {
            mode
        };
        self.reselect(absolute);
    }

    /// 切回默认过滤（`ctrl+d`，ref `:1049-1053`）。
    pub fn reset_filter(&mut self) {
        let absolute = self.selected_absolute();
        self.filter_mode = TreeFilterMode::Default;
        self.reselect(absolute);
    }

    /// 反向循环过滤模式（ref `:1072-1077`：`default → all → labeled-only → user-only → no-tools → default`）。
    pub fn cycle_filter_back(&mut self) {
        let absolute = self.selected_absolute();
        self.filter_mode = match self.filter_mode {
            TreeFilterMode::Default => TreeFilterMode::All,
            TreeFilterMode::All => TreeFilterMode::LabeledOnly,
            TreeFilterMode::LabeledOnly => TreeFilterMode::UserOnly,
            TreeFilterMode::UserOnly => TreeFilterMode::NoTools,
            TreeFilterMode::NoTools => TreeFilterMode::Default,
        };
        self.reselect(absolute);
    }

    /// 切换过滤模式（ref `:1080-1085`：清空折叠状态后重新过滤）。
    pub fn cycle_filter(&mut self) {
        let absolute = self.selected_absolute();
        self.filter_mode = self.filter_mode.next();
        self.reselect(absolute);
    }

    /// 上移（夹在 0，不环绕：ref 的树列表是普通列表语义）。
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// 可见行数（`move_down` 的夹取上限）。
    fn visible_len(&self) -> usize {
        self.visible_indices().len()
    }

    /// 下移（夹在末行）。
    pub fn move_down(&mut self) {
        let last = self.visible_len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    /// 面板行（**不含**上下边框；边框由 `render_all` 画）。
    ///
    /// 布局照 `tree-selector.ts`：标题 `   Session Tree`、两行帮助、`  Type to search:`
    /// （都装在头部边框块里），随后空行、列表、计数 `  (i/n)`、空行。
    #[must_use]
    pub fn lines(&self, width: usize) -> Vec<String> {
        let mut lines = tree_header_lines(width);
        let visible = self.visible_indices();
        // ref `:671-676`：**过滤后**为空也走 `No entries found` + `(0/0)<labels>`。
        if visible.is_empty() {
            lines.push("  No entries found".to_string());
            lines.push(format!("  (0/0){}", self.filter_mode.label()));
            lines.push(String::new());
            return lines;
        }
        // 选中项夹在可见范围内（过滤后行数变少时不能出现 `(3/1)`）。
        let selected = self.selected.min(visible.len().saturating_sub(1));
        // 只渲染**一屏窗口**（ref `:678-690`：`start = clamp(selected - max/2, 0, len - max)`）。
        // 此前把全部行都画出来，长列表会把整屏顶掉（T61 实测：40 条时面板占满屏幕）。
        let max_visible = self.max_visible.max(1);
        let start = selected
            .saturating_sub(max_visible / 2)
            .min(visible.len().saturating_sub(max_visible));
        let end = (start + max_visible).min(visible.len());
        for (pos, index) in visible.iter().enumerate().skip(start).take(end - start) {
            let row = &self.rows[*index];
            let cursor = if pos == selected { "› " } else { "  " };
            let mut prefix = tree_row_with_label(row, row.label.as_deref());
            if self.show_label_timestamps {
                if let (Some(_), Some(stamp)) = (row.label.as_ref(), row.label_time.as_ref()) {
                    // ref `:747-749`：标签之后、正文之前插入 `HH:MM `。
                    if let Some(rest) = prefix
                        .strip_prefix(&format!("[{}] ", row.label.clone().unwrap_or_default()))
                    {
                        prefix =
                            format!("[{}] {stamp} {rest}", row.label.clone().unwrap_or_default());
                    }
                }
            }
            // 折叠标记（ref `:726-740` 的 `isFolded ? "⊞" : foldable ? "⊟" : "─"`）：
            // 折叠的节点画 `⊞`（T58 golden：`› ⊞ • [system]`）；根节点未折叠时不画标记。
            // 根节点（父条目不在树里）**只有折叠时**才画 `⊞`，展开时不画
            // （T58 golden 帧0：`› • [system]`；帧1 折叠后：`› ⊞ • [system]`）。
            let is_root = row
                .parent_id
                .as_deref()
                .is_none_or(|parent| !self.rows.iter().any(|candidate| candidate.id == parent));
            // 线性链（每个父只有 1 个子）**不画任何标记**（T58 golden 帧0 的三行都没有标记）；
            // 只有分支点（父有多个子）才画 `⊟`（可折叠）/`─`，折叠的节点画 `⊞`。
            let siblings = row.parent_id.as_deref().map_or(0, |parent| {
                self.rows
                    .iter()
                    .filter(|candidate| candidate.parent_id.as_deref() == Some(parent))
                    .count()
            });
            let fold_marker = if self.folded.contains(&row.id) {
                "⊞ "
            } else if is_root || siblings <= 1 {
                ""
            } else if self.is_foldable(*index) {
                "⊟ "
            } else {
                "─ "
            };
            lines.push(format!("{cursor}{fold_marker}• {prefix}"));
        }
        let label_time_flag = if self.show_label_timestamps {
            " [+label time]"
        } else {
            ""
        };
        lines.push(format!(
            "  ({}/{}){}{label_time_flag}",
            selected + 1,
            visible.len(),
            self.filter_mode.label()
        ));
        lines.push(String::new());
        lines
    }
}

/// 从会话文件里读出 `/tree` 的行（只取 message 条目）。
///
/// 顺序 = 文件里的追加顺序（ref 按 entry 链从旧到新渲染）。
///
/// # Errors
/// 文件读不了时返回错误描述；坏行跳过。
pub fn tree_rows_from_session(path: &std::path::Path) -> Result<Vec<TreeRow>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("读取 {} 失败：{error}", path.display()))?;
    let mut rows = Vec::new();
    // 两遍：先收齐 `label` 条目（它们可能排在消息之后），再构建行。
    let records: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect();
    let mut labels: std::collections::BTreeMap<String, (Option<String>, Option<String>)> =
        std::collections::BTreeMap::new();
    for record in &records {
        if record.get("type").and_then(|v| v.as_str()) == Some("label") {
            if let Some(target) = record.get("targetId").and_then(|v| v.as_str()) {
                let label = record
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let stamp = record
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .map(|stamp| {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_millis());
                        format_label_timestamp(stamp, local_utc_offset_minutes(), now_ms)
                    });
                labels.insert(target.to_string(), (label, stamp));
            }
        }
    }
    for record in &records {
        if record.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        let Some(message) = record.get("message") else {
            continue;
        };
        let role = message
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let text_of = |content: &serde_json::Value| -> String {
            if let Some(s) = content.as_str() {
                return s.split('\n').next().unwrap_or_default().trim().to_string();
            }
            let Some(blocks) = content.as_array() else {
                return String::new();
            };
            for block in blocks {
                if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(s) = block.get("text").and_then(|v| v.as_str()) {
                        return s.split('\n').next().unwrap_or_default().trim().to_string();
                    }
                }
            }
            String::new()
        };
        let (kind, body) = match role {
            "system" => (TreeRowKind::System, String::new()),
            "user" => (
                TreeRowKind::User,
                message.get("content").map(text_of).unwrap_or_default(),
            ),
            "assistant" => (
                TreeRowKind::Assistant,
                message.get("content").map(text_of).unwrap_or_default(),
            ),
            other => {
                let _ = other;
                (
                    TreeRowKind::Other,
                    message.get("content").map(text_of).unwrap_or_default(),
                )
            }
        };
        let entry = record
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(|id| labels.get(id).cloned());
        let label = entry.as_ref().and_then(|(label, _)| label.clone());
        let label_time = entry.and_then(|(_, stamp)| stamp);
        rows.push(TreeRow {
            kind,
            label,
            label_time,
            parent_id: record
                .get("parentId")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            id: record
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            text: body,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tree_tests {
    use super::{tree_rows_from_session, TreeRow, TreeRowKind, TreeSelector};

    /// T41 golden：空态首屏（标题前 3 空格、帮助两行、搜索行、**头部底边框**、`No entries found` / `(0/0)`）。
    #[test]
    fn empty_state_matches_t41() {
        let lines = TreeSelector::new(Vec::new()).lines(100);
        println!("{lines:#?}");
        assert_eq!(lines[1], "  ↑/↓ move · ←/→ page · ctrl+←/→ branch · ctrl+x copy · shift+l label · shift+t label time");
        assert_eq!(
            lines[2],
            "  filters ctrl+d/t/u/l/a · cycle ctrl+o/shift+ctrl+o"
        );
        assert_eq!(lines[3], "  Type to search:");
        assert!(
            lines[4].starts_with('─'),
            "第 5 行应是头部底边框：{lines:#?}"
        );
        assert_eq!(lines[5], "");
        assert_eq!(lines[6], "  No entries found");
        assert_eq!(lines[7], "  (0/0)");
    }

    /// T43 golden：三行树（system/user/assistant），选中最后一行，计数 `(3/3)`。
    #[test]
    fn rows_match_t43() {
        let selector = TreeSelector::new(vec![
            TreeRow {
                kind: TreeRowKind::System,
                text: String::new(),
                label: None,
                id: String::new(),
                label_time: None,
                parent_id: None,
            },
            TreeRow {
                kind: TreeRowKind::User,
                text: "say hi".to_string(),
                label: None,
                id: String::new(),
                label_time: None,
                parent_id: None,
            },
            TreeRow {
                kind: TreeRowKind::Assistant,
                text: "Hello world".to_string(),
                label: None,
                id: String::new(),
                label_time: None,
                parent_id: None,
            },
        ]);
        let lines = selector.lines(100);
        assert!(lines.iter().any(|l| l == "  • [system]"), "{lines:#?}");
        assert!(lines.iter().any(|l| l == "  • user: say hi"), "{lines:#?}");
        assert!(
            lines.iter().any(|l| l == "› • assistant: Hello world"),
            "选中行应有 `› ` 前缀：{lines:#?}"
        );
        assert!(lines.iter().any(|l| l == "  (3/3)"), "{lines:#?}");
    }

    /// 从会话文件读行：只取 message，system 不带正文，坏行跳过。
    #[test]
    fn rows_from_session_file() {
        let dir = std::env::temp_dir().join(format!("pi-rs-tree-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建目录");
        let path = dir.join("s.jsonl");
        std::fs::write(
            &path,
            [
                r#"{"type":"session","version":3,"id":"x"}"#,
                "不是 JSON",
                r#"{"type":"message","id":"1","message":{"role":"system","content":""}}"#,
                r#"{"type":"message","id":"2","message":{"role":"user","content":"say hi"}}"#,
                r#"{"type":"message","id":"3","message":{"role":"assistant","content":[{"type":"text","text":"Hello\nworld"}]}}"#,
            ]
            .join("\n")
                + "\n",
        )
        .expect("写文件");
        let rows = tree_rows_from_session(&path).expect("可读");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, TreeRowKind::System);
        assert_eq!(rows[1].text, "say hi");
        assert_eq!(rows[2].text, "Hello", "只取第一行文本");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tree_filter_tests {
    use super::{TreeFilterMode, TreeRow, TreeRowKind, TreeSelector};

    fn selector() -> TreeSelector {
        TreeSelector::new(vec![
            TreeRow {
                kind: TreeRowKind::System,
                text: String::new(),
                label: None,
                id: String::new(),
                label_time: None,
                parent_id: None,
            },
            TreeRow {
                kind: TreeRowKind::User,
                text: "say hi".to_string(),
                label: None,
                id: String::new(),
                label_time: None,
                parent_id: None,
            },
            TreeRow {
                kind: TreeRowKind::Assistant,
                text: "Hello world".to_string(),
                label: None,
                id: String::new(),
                label_time: None,
                parent_id: None,
            },
        ])
    }

    /// T44 golden：`ctrl+o` 一次 ⇒ `default → no-tools`，计数行追加 ` [no-tools]`。
    #[test]
    fn cycle_filter_appends_status_label() {
        let mut sel = selector();
        assert_eq!(sel.filter_mode, TreeFilterMode::Default);
        assert!(
            sel.lines(100).iter().any(|l| l == "  (3/3)"),
            "默认模式下无状态标签"
        );
        sel.cycle_filter();
        assert_eq!(sel.filter_mode, TreeFilterMode::NoTools);
        assert!(
            sel.lines(100).iter().any(|l| l == "  (3/3) [no-tools]"),
            "{:#?}",
            sel.lines(100)
        );
        // 再循环三次回到 default（no-tools → user-only → labeled-only → all → default）
        for expected in [
            TreeFilterMode::UserOnly,
            TreeFilterMode::LabeledOnly,
            TreeFilterMode::All,
            TreeFilterMode::Default,
        ] {
            sel.cycle_filter();
            assert_eq!(sel.filter_mode, expected);
        }
    }

    /// `user-only` 只留用户消息；`labeled-only` 恒空（本项目无 label）。
    #[test]
    fn filters_narrow_rows() {
        let mut sel = selector();
        sel.filter_mode = TreeFilterMode::UserOnly;
        assert_eq!(sel.visible_indices(), vec![1]);
        let lines = sel.lines(100);
        assert!(lines.iter().any(|l| l == "  (1/1) [user]"), "{lines:#?}");

        sel.filter_mode = TreeFilterMode::LabeledOnly;
        assert!(sel.visible_indices().is_empty());
        let lines = sel.lines(100);
        assert!(
            lines.iter().any(|l| l == "  No entries found"),
            "{lines:#?}"
        );
        assert!(lines.iter().any(|l| l == "  (0/0) [labeled]"), "{lines:#?}");
    }

    /// 上移把光标放到上一行（T44 `after-up` 帧的形态）。
    #[test]
    fn move_up_selects_previous_row() {
        let mut sel = selector();
        assert_eq!(sel.selected, 2, "打开时选中末行");
        sel.move_up();
        assert_eq!(sel.selected, 1);
        let lines = sel.lines(100);
        assert!(lines.iter().any(|l| l == "› • user: say hi"), "{lines:#?}");
        assert!(lines.iter().any(|l| l == "  (2/3)"), "{lines:#?}");
    }
}

/// `/tree` 里选中**非当前叶子**后弹出的 "Summarize branch?" 选择器（P7-7e e4a）。
///
/// ref `interactive-mode.ts:5426-5445`：`showExtensionSelector("Summarize branch?", ["No summary",
/// "Summarize", "Summarize with custom prompt"])`；T45 golden 的形态：
/// 空行 / ` Summarize branch?` / 空行 / 三个选项（选中项 ` → ` 前缀）/ 空行 /
/// ` ↑↓ navigate  enter select  escape/ctrl+c cancel`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummarizeSelector {
    /// 选中下标（0 = `No summary`）。
    pub selected: usize,
    /// 第三项（`Summarize with custom prompt`）打开后的输入文本；`None` = 未进入输入态。
    pub custom: Option<String>,
}

impl SummarizeSelector {
    /// 三个固定选项（顺序见 ref 调用点）。
    pub const OPTIONS: [&'static str; 3] =
        ["No summary", "Summarize", "Summarize with custom prompt"];

    /// 新建（默认选中 `No summary`）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            selected: 0,
            custom: None,
        }
    }

    /// 上移（环绕）。
    pub fn move_up(&mut self) {
        self.selected = if self.selected == 0 {
            Self::OPTIONS.len() - 1
        } else {
            self.selected - 1
        };
    }

    /// 下移（环绕）。
    pub fn move_down(&mut self) {
        self.selected = (self.selected + 1) % Self::OPTIONS.len();
    }

    /// 当前选项文本。
    #[must_use]
    pub fn picked(&self) -> &'static str {
        Self::OPTIONS[self.selected]
    }

    /// 进入"自定义指令"输入态（选中第三项并回车时）。
    pub fn enter_custom(&mut self) {
        self.custom = Some(String::new());
    }

    /// 退出"自定义指令"输入态（Esc）。
    pub fn exit_custom(&mut self) {
        self.custom = None;
    }

    /// 自定义输入：追加字符。
    pub fn custom_push(&mut self, ch: char) {
        if let Some(text) = self.custom.as_mut() {
            text.push(ch);
        }
    }

    /// 自定义输入：换行（`shift+enter`/`ctrl+j`）。
    pub fn custom_newline(&mut self) {
        if let Some(text) = self.custom.as_mut() {
            text.push('\n');
        }
    }

    /// 自定义输入：退格。
    pub fn custom_pop(&mut self) {
        if let Some(text) = self.custom.as_mut() {
            text.pop();
        }
    }

    /// 面板行（**不含**上下边框）。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        if let Some(text) = self.custom.as_ref() {
            // ref `interactive-mode.ts:5447-5449`：`showExtensionEditor("Custom summarization instructions")`，
            // T66 golden 的形态：空行 / 标题 / 空行 / 上边框 / 输入 / 下边框 / 提示 / 空行。
            let mut lines = vec![
                String::new(),
                " Custom summarization instructions".to_string(),
                String::new(),
                "────────────────────────────────────────────────────────────────────────────────────────────────────".to_string(),
            ];
            if text.is_empty() {
                lines.push(String::new());
            } else {
                for line in text.split('\n') {
                    lines.push(line.to_string());
                }
            }
            lines.push("────────────────────────────────────────────────────────────────────────────────────────────────────".to_string());
            // T66 golden：输入框下边框与提示行之间还有**一个空行**。
            lines.push(String::new());
            lines.push(
                " enter submit  shift+enter/ctrl+j newline  escape/ctrl+c cancel  ctrl+g external editor".to_string(),
            );
            lines.push(String::new());
            return lines;
        }
        let mut lines = vec![
            String::new(),
            " Summarize branch?".to_string(),
            String::new(),
        ];
        for (index, option) in Self::OPTIONS.iter().enumerate() {
            let prefix = if index == self.selected {
                " → "
            } else {
                "   "
            };
            lines.push(format!("{prefix}{option}"));
        }
        lines.push(String::new());
        lines.push(" ↑↓ navigate  enter select  escape/ctrl+c cancel".to_string());
        // golden 实测：提示行之后还有**一个空行**才到底边框。
        lines.push(String::new());
        lines
    }
}

impl Default for SummarizeSelector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod summarize_tests {
    use super::SummarizeSelector;

    /// T45 golden（prompt 一帧）：`Summarize branch?` + 三选项 + 提示行逐字节对齐。
    #[test]
    fn lines_match_t45_prompt() {
        let expected = [
            "",
            " Summarize branch?",
            "",
            " → No summary",
            "   Summarize",
            "   Summarize with custom prompt",
            "",
            " ↑↓ navigate  enter select  escape/ctrl+c cancel",
            "",
        ];
        assert_eq!(SummarizeSelector::new().lines(), expected);
    }

    /// 默认选中 `No summary`；上下移动环绕。
    #[test]
    fn selection_wraps() {
        let mut sel = SummarizeSelector::new();
        assert_eq!(sel.picked(), "No summary");
        sel.move_up();
        assert_eq!(sel.picked(), "Summarize with custom prompt");
        sel.move_down();
        assert_eq!(sel.picked(), "No summary");
        assert!(
            sel.lines().iter().any(|l| l == " → No summary"),
            "{:?}",
            sel.lines()
        );
    }
}

/// `/tree` 里 `shift+l` 打开的标签输入（P7-7e：ref `tree-selector.ts:1279-1311` 的 `LabelInput`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelInput {
    /// 已输入的标签文本。
    pub text: String,
}

impl LabelInput {
    /// 新建（空串 = 删除标签）。
    #[must_use]
    pub fn new() -> Self {
        Self {
            text: String::new(),
        }
    }

    /// 追加字符。
    pub fn push(&mut self, ch: char) {
        self.text.push(ch);
    }

    /// 删除末字符。
    pub fn pop(&mut self) {
        self.text.pop();
    }

    /// 面板行：**保留树头部块**（标题/帮助/搜索行/边框），只把列表区换成标签输入
    /// （T49 golden：头部 6 行不变，之后是 `  Label (empty to remove):` / `  >` / 提示行）。
    #[must_use]
    pub fn lines(&self, width: usize) -> Vec<String> {
        let mut lines = tree_header_lines(width);
        lines.push("  Label (empty to remove):".to_string());
        lines.push(if self.text.is_empty() {
            "  >".to_string()
        } else {
            format!("  > {}", self.text)
        });
        lines.push("  enter save  escape/ctrl+c cancel".to_string());
        // golden：提示行之后还有一个空行才到底边框。
        lines.push(String::new());
        lines
    }
}

impl Default for LabelInput {
    fn default() -> Self {
        Self::new()
    }
}

/// 给树行加上标签前缀（ref `tree-selector.ts` 的行渲染：`[label] user: …`）。
#[must_use]
pub fn tree_row_with_label(row: &TreeRow, label: Option<&str>) -> String {
    let body = match row.kind {
        TreeRowKind::System => "[system]".to_string(),
        TreeRowKind::User => format!("user: {}", row.text),
        TreeRowKind::Assistant => format!("assistant: {}", row.text),
        TreeRowKind::Other => format!("tool: {}", row.text),
    };
    match label {
        Some(text) if !text.is_empty() => format!("[{text}] {body}"),
        Some(_) | None => body,
    }
}

#[cfg(test)]
mod label_tests {
    use super::{tree_row_with_label, LabelInput, TreeRow, TreeRowKind};

    /// T49 golden（输入态）：保留树头部块，其后是标签输入三行。
    #[test]
    fn lines_match_t49_label_input() {
        let lines = LabelInput::new().lines(120);
        assert_eq!(lines[0], "   Session Tree");
        let tail = &lines[lines.len() - 4..];
        assert_eq!(tail[0], "  Label (empty to remove):");
        assert_eq!(tail[1], "  >");
        assert_eq!(tail[2], "  enter save  escape/ctrl+c cancel");
        assert_eq!(tail[3], "");
        assert!(lines.iter().any(|l| l == "  Type to search:"), "{lines:#?}");
        let mut input = LabelInput::new();
        input.push('x');
        let typed = input.lines(120);
        assert_eq!(typed[typed.len() - 3], "  > x");
        assert_eq!(typed[typed.len() - 2], "  enter save  escape/ctrl+c cancel");
        assert_eq!(typed[typed.len() - 1], "");
    }

    /// 有标签时行前缀是 `[label] `（golden 里是 `[label] user: say hi`）。
    #[test]
    fn row_prefix_carries_label() {
        let row = TreeRow {
            kind: TreeRowKind::User,
            text: "say hi".to_string(),
            label: None,
            id: String::new(),
            label_time: None,
            parent_id: None,
        };
        assert_eq!(
            tree_row_with_label(&row, Some("mylabel")),
            "[mylabel] user: say hi"
        );
        assert_eq!(tree_row_with_label(&row, None), "user: say hi");
        assert_eq!(tree_row_with_label(&row, Some("")), "user: say hi");
    }
}

#[cfg(test)]
mod label_time_tests {
    use super::{format_label_timestamp, parse_rfc3339_ms, TreeRow, TreeRowKind, TreeSelector};

    /// `format_label_timestamp` 三档（ref `tree-selector.ts:862-884`）：
    /// 同日 `HH:MM` / 同年 `M/D HH:MM` / 跨年 `YY/M/D HH:MM`，时间按**本地时区**。
    #[test]
    fn formats_three_tiers() {
        // 2026-10-24 12:46 本地（UTC+8）
        let now = parse_rfc3339_ms("2026-10-24T04:46:00.000Z").expect("now");
        assert_eq!(
            format_label_timestamp("2026-10-24T04:46:11.000Z", 480, now),
            "12:46"
        );
        assert_eq!(
            format_label_timestamp("2026-10-02T04:46:00.000Z", 480, now),
            "10/2 12:46"
        );
        // 本地时间已跨到 2026-01-01，但年份与 now 相同 ⇒ 仍走"同年"档
        assert_eq!(
            format_label_timestamp("2025-12-31T16:30:00.000Z", 480, now),
            "1/1 00:30"
        );
        // 真正跨年（2024 年）⇒ 三段式
        assert_eq!(
            format_label_timestamp("2024-06-01T02:05:00.000Z", 480, now),
            "24/6/1 10:05"
        );
        assert_eq!(format_label_timestamp("坏时间", 480, now), "");
        // 同一时刻在 UTC 下显示不同：说明偏移确实生效
        assert_eq!(
            format_label_timestamp("2026-10-24T04:46:11.000Z", 0, now),
            "04:46"
        );
    }

    /// `shift+t` 打开后：行里插入 `HH:MM `，计数行追加 ` [+label time]`。
    #[test]
    fn toggle_inserts_timestamp_and_flag() {
        let mut selector = TreeSelector::new(vec![TreeRow {
            kind: TreeRowKind::User,
            text: "say hi".to_string(),
            label: Some("mylabel".to_string()),
            id: "m1".to_string(),
            label_time: Some("05:46".to_string()),
            parent_id: None,
        }]);
        assert!(
            selector
                .lines(100)
                .iter()
                .any(|l| l == "› • [mylabel] user: say hi"),
            "{:#?}",
            selector.lines(100)
        );
        selector.toggle_label_timestamps();
        let lines = selector.lines(100);
        assert!(
            lines
                .iter()
                .any(|l| l == "› • [mylabel] 05:46 user: say hi"),
            "{lines:#?}"
        );
        assert!(
            lines.iter().any(|l| l == "  (1/1) [+label time]"),
            "{lines:#?}"
        );
    }
}

#[cfg(test)]
mod p78a_offset_panic_tests {
    use super::parse_utc_offset_compact;

    /// C 阶段：`date +%z` 输出若含多字节字符（例如被本地化/替换），旧实现按**字节**切片
    /// `digits[..2]` 会 panic（`end byte index 2 is not a char boundary`）。
    #[test]
    fn multibyte_offset_does_not_panic() {
        assert_eq!(parse_utc_offset_compact("+0±0"), None);
        assert_eq!(parse_utc_offset_compact("+±00"), None);
        assert_eq!(parse_utc_offset_compact("±0800"), None);
        assert_eq!(parse_utc_offset_compact("＋0800"), None);
    }

    /// 正常形态与边界。
    #[test]
    fn plain_offsets_parse() {
        assert_eq!(parse_utc_offset_compact("+0800"), Some(480));
        assert_eq!(parse_utc_offset_compact("-0430"), Some(-270));
        assert_eq!(parse_utc_offset_compact("+0000"), Some(0));
        assert_eq!(parse_utc_offset_compact("+08"), None);
        assert_eq!(parse_utc_offset_compact(""), None);
        assert_eq!(parse_utc_offset_compact("+ab00"), None);
    }
}

#[cfg(test)]
mod p79_limits_tests {
    use super::{format_label_timestamp, ResumeSelector, SessionRow};

    fn row(name: Option<&str>, index: usize) -> SessionRow {
        SessionRow {
            path: std::path::PathBuf::from(format!("/tmp/s{index}.jsonl")),
            name: name.map(str::to_string),
            summary: Some(format!("摘要 {index}")),
            cwd: "/tmp/work".to_string(),
            message_count: index,
            modified_ms: 1_700_000_000_000 + index as u128,
        }
    }

    /// ref `session-selector.ts:459-470`：`ctrl+n` 打开 `Name: Named` 后**只列有名字的会话**，
    /// 且行文本优先显示名字（P7-7 的 T67 已对拍；这里补过滤逻辑与空态文案的单测）。
    #[test]
    fn named_filter_keeps_only_named_sessions() {
        let mut selector = ResumeSelector::new();
        selector.sessions = vec![row(Some("有名字"), 1), row(None, 2), row(Some("另一个"), 3)];
        selector.current_cwd = "/tmp/work".to_string();
        selector.home = "/tmp".to_string();
        assert_eq!(selector.visible_indices().len(), 3, "默认不过滤");

        selector.toggle_named();
        assert_eq!(selector.visible_indices(), vec![0, 2], "只保留有名字的会话");
        let lines = selector.lines(100);
        let joined = lines.join("\n");
        assert!(joined.contains("Name: Named"), "标签应变成 Named：{joined}");
        assert!(joined.contains("有名字"), "行文本应优先显示名字");

        // 全都没有名字时走 ref 的空态文案（T54 golden）。
        let mut empty = ResumeSelector::new();
        empty.sessions = vec![row(None, 1)];
        empty.current_cwd = "/tmp/work".to_string();
        empty.home = "/tmp".to_string();
        empty.toggle_named();
        let text = empty.lines(100).join("\n");
        assert!(
            text.contains("No named sessions in current folder"),
            "空态文案不对：{text}"
        );
    }

    /// 标签时间戳的三档格式在**固定时区**下稳定（跨日/跨年边界，`TZ` 由 `date +%z` 反映）。
    #[test]
    fn label_timestamp_formats_are_stable_under_fixed_offset() {
        let base = 1_767_225_600_000_u128; // 2026-01-01T00:00:00Z
        let same_day = format_label_timestamp("2026-01-01T03:04:05.000Z", 480, base);
        assert_eq!(same_day, "11:04", "同日档应为 HH:MM");
        let same_year = format_label_timestamp("2026-03-05T01:00:00.000Z", 480, base);
        assert_eq!(same_year, "3/5 09:00", "同年档应为 M/D HH:MM");
        // 本地时间跨年（base 的本地日期是 2026-01-01，这条是 2025-11-20）⇒ 走跨年档。
        let other_year = format_label_timestamp("2025-11-20T00:00:00.000Z", 480, base);
        assert_eq!(other_year, "25/11/20 08:00", "跨年档应为 YY/M/D HH:MM");
    }
}
