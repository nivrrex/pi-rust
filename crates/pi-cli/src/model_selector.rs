//! `/model` 选择器的**纯逻辑**部分（P7-7a）。
//!
//! 行为规格来自 ref `modes/interactive/components/model-selector.ts`：
//! - 构造顺序 `updateList`（`:297-368`）：列表行 → （有选中项时）空行 + `Model Name: …`
//!   → （有刷新状态时）空行 + 状态 → 组件级空行 + 键位提示（`:139-158`）；
//! - 过滤 `filterModels`（`:281-303`）：查询非空时过滤、并把选中项移到首行；
//! - 空结果文案 `  No matching models`（`:347`）；
//! - 列表行形态 `→ ✓ <id> [<provider>] · default`（`:316-330`）。
//!
//! 本模块只做"状态 → 行"的纯函数，I/O 与按键映射留在 `tui.rs`。

/// 选择器里的一行模型条目（渲染只需要这些字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    /// 模型 id（ref 的 `item.id`）。
    pub id: String,
    /// provider 标识（ref 的 `item.provider`）。
    pub provider: String,
    /// 展示名（ref 的 `item.model.name`）。
    pub name: String,
    /// 是否为当前模型（决定 `✓`）。
    pub current: bool,
    /// 是否为默认模型（决定 ` · default`）。
    pub default: bool,
}

/// 选择器状态：查询串 + 全部模型 + 刷新状态文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelector {
    /// 搜索框内容（`filterModels` 的输入）。
    pub query: String,
    /// 作用域内的全部模型（`activeModels`）。
    pub models: Vec<ModelEntry>,
    /// 刷新状态文案（`refreshStatusMessage`）；`None` 表示不显示。
    pub refresh_status: Option<String>,
    /// 选中下标（未过滤列表里的下标语义按 ref 简化为"过滤后下标"）。
    pub selected: usize,
}

impl ModelSelector {
    /// 新建选择器；`models` 为作用域内模型，`refresh_status` 为刷新结果文案。
    #[must_use]
    pub fn new(models: Vec<ModelEntry>, refresh_status: Option<String>) -> Self {
        let selected = models.iter().position(|m| m.current).unwrap_or(0);
        Self {
            query: String::new(),
            models,
            refresh_status,
            selected,
        }
    }

    /// 过滤后的下标表（保持原顺序）。
    ///
    /// ref 用 `fuzzyFilter`（`:282-296`），这里用**大小写无关的子串匹配**；
    /// 与 ref 的差别只在"模糊"程度，命中/不命中的判定对 T26/T27 一致。
    #[must_use]
    pub fn filtered_indices(&self) -> Vec<usize> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return (0..self.models.len()).collect();
        }
        self.models
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.id.to_lowercase().contains(&query)
                    || m.provider.to_lowercase().contains(&query)
                    || m.name.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 当前选中的模型（过滤后）。
    #[must_use]
    pub fn selected_model(&self) -> Option<&ModelEntry> {
        let indices = self.filtered_indices();
        // ref `filterModels`（`:299-302`）：有查询时选中项归零；无查询时按原下标 clamp。
        let pos = if self.query.trim().is_empty() {
            self.selected.min(indices.len().saturating_sub(1))
        } else {
            0
        };
        indices.get(pos).map(|i| &self.models[*i])
    }

    /// 上移选中项（ref `model-selector.ts:377-384`：到头环绕到末项）。
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

    /// 下移选中项（ref `model-selector.ts:385-392`：到尾环绕到首项）。
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

    /// 追加一个字符到搜索框。
    pub fn push(&mut self, ch: char) {
        self.query.push(ch);
    }

    /// 删除搜索框最后一个字符。
    pub fn pop(&mut self) {
        self.query.pop();
    }

    /// 选择器面板行（**不含**上下边框，边框由 `render_all` 画）。
    ///
    /// 顺序严格按 ref `updateList`（`:297-368`）+ 构造（`:139-158`）。
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            String::new(),
            "Only showing models from configured providers. Use /login to add providers."
                .to_string(),
            String::new(),
            if self.query.is_empty() {
                ">".to_string()
            } else {
                format!("> {}", self.query)
            },
            String::new(),
        ];
        let indices = self.filtered_indices();
        // 列表：ref `model-selector.ts:308-337` —— 最多 10 项，窗口以选中项为中心，
        // 被裁剪时补 `  (n/total)`。取前 10 项（旧实现）会让选中项滚出可视区。
        let position = if self.query.trim().is_empty() {
            self.selected.min(indices.len().saturating_sub(1))
        } else {
            0
        };
        let (start, end) = pi_tui::selector_window(indices.len(), position, 10);
        for index in &indices[start..end] {
            let item = &self.models[*index];
            let cursor = if Some(item) == self.selected_model() {
                "→ "
            } else {
                "  "
            };
            let marker = if item.current { "✓ " } else { "  " };
            let default_badge = if item.default { " · default" } else { "" };
            lines.push(format!(
                "{cursor}{marker}{} [{}]{default_badge}",
                item.id, item.provider
            ));
        }
        if indices.is_empty() {
            lines.push("  No matching models".to_string());
        } else {
            if start > 0 || end < indices.len() {
                lines.push(pi_tui::selector_scroll_hint(position, indices.len()));
            }
            if let Some(selected) = self.selected_model() {
                lines.push(String::new());
                lines.push(format!("  Model Name: {}", selected.name));
            }
        }
        if let Some(status) = &self.refresh_status {
            lines.push(String::new());
            lines.push(format!("  {status}"));
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
    use super::{ModelEntry, ModelSelector};

    fn single() -> ModelSelector {
        ModelSelector::new(
            vec![ModelEntry {
                id: "mock-model".to_string(),
                provider: "openai-completions".to_string(),
                name: "mock-model".to_string(),
                current: true,
                default: true,
            }],
            Some("Model catalogs refreshed.".to_string()),
        )
    }

    /// T26 口径：无查询时的面板行必须与 golden 的 12 行逐字节一致。
    #[test]
    fn lines_match_t26_panel() {
        let expected = [
            "",
            "Only showing models from configured providers. Use /login to add providers.",
            "",
            ">",
            "",
            "→ ✓ mock-model [openai-completions] · default",
            "",
            "  Model Name: mock-model",
            "",
            "  Model catalogs refreshed.",
            "",
            "  Enter to select · Ctrl+S to set as default · Escape/Ctrl+C to cancel",
        ];
        assert_eq!(single().lines(), expected);
    }

    /// T27 口径：输入 `xyz` 后列表为空 ⇒ 只剩 `No matching models`，且**没有** `Model Name:` 行。
    #[test]
    fn query_without_match_shows_no_matching_models() {
        let mut selector = single();
        for ch in "xyz".chars() {
            selector.push(ch);
        }
        let lines = selector.lines();
        assert!(
            lines.iter().any(|l| l == "> xyz"),
            "搜索行应显示查询：{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "  No matching models"),
            "应显示无匹配文案：{lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("Model Name:")),
            "空结果不得显示 Model Name：{lines:?}"
        );
        // 反向对照：同样的断言在**有查询但不为空结果**时必须失败（防"永远不显示"的假通过）。
        let mut hit = single();
        hit.push('m');
        assert!(
            hit.lines().iter().any(|l| l.contains("Model Name:")),
            "有匹配时必须显示 Model Name"
        );
    }

    /// 匹配大小写无关；provider 也参与匹配（ref `getModelSelectorSearchText` 含 provider）。
    #[test]
    fn filter_is_case_insensitive_and_covers_provider() {
        let mut selector = single();
        selector.query = "OPENAI".to_string();
        assert_eq!(selector.filtered_indices(), vec![0]);
        selector.query = "nomatch".to_string();
        assert!(selector.filtered_indices().is_empty());
    }
}

#[cfg(test)]
mod window_tests {
    use super::{ModelEntry, ModelSelector};

    fn models(n: usize, current: usize) -> Vec<ModelEntry> {
        (0..n)
            .map(|i| ModelEntry {
                id: format!("model-{i:02}"),
                provider: "openrouter".to_string(),
                name: format!("model {i:02}"),
                current: i == current,
                default: false,
            })
            .collect()
    }

    /// 选中项滚出窗口的问题回归：25 个模型、选中第 18 个时，窗口必须包含它并给出 `(19/25)`。
    #[test]
    fn window_follows_selected_model() {
        let mut selector = ModelSelector::new(models(25, 0), None);
        selector.selected = 17;
        let lines = selector.lines();
        assert!(
            lines.iter().any(|line| line.contains("model-17")),
            "选中项必须在窗口内：{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("(18/25)")),
            "需要滚动提示：{lines:?}"
        );
        assert_eq!(
            lines.iter().filter(|line| line.contains("model-")).count(),
            10,
            "窗口最多 10 项：{lines:?}"
        );
    }
}
