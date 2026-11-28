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

// ---------------------------------------------------------------------------
// F63/A：思考档位的**取值链**（ref `core/sdk.ts:230-254`）
// ---------------------------------------------------------------------------

/// 除会话外可用的档位集合（推理模型）。非推理模型只有 `off`。
pub const EXTENDED_THINKING_LEVELS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// 模型缺省档位（ref `sdk.ts` 的 `DEFAULT_THINKING_LEVEL`）。
pub const DEFAULT_THINKING_LEVEL: &str = "medium";

/// 档位被 clamp 后的结果与来源（供 `PI_TEST_THINKING_TRACE` 与排障）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingLevel {
    /// 生效档位。
    pub level: String,
    /// 来源：`session`（会话里的 `thinking_level_change`）/ `per_model`
    /// （`settings.modelThinkingLevels`）/ `default`（`settings.defaultThinkingLevel`）/
    /// `builtin`（内置缺省）/ `forced_off`（模型不支持思考）。
    pub source: &'static str,
    /// clamp 之前的原始值。
    pub raw: String,
}

/// 按 ref `sdk.ts:230-254` 的**完整顺序**解析思考档位。
///
/// 顺序（与 ref 逐行对应）：
/// 1. `explicit`（命令行/调用方显式指定，对应 `options.thinkingLevel`）；
/// 2. 会话里有 `thinking_level_change` ⇒ 用会话值（`:237`）；
/// 3. 每模型覆盖 `modelThinkingLevels["provider/model"]`（`:241-246`）；
/// 4. `settings.defaultThinkingLevel`（`:248`）；
/// 5. 内置缺省 `DEFAULT_THINKING_LEVEL`（`:248` 的 `??`）；
/// 6. 按模型能力 **clamp**（`:251-254`）：非推理模型强制 `off`，越界按 ref 规则回退。
///
/// 为什么要这条统一函数：此前四个入口各自内联一段（`main.rs` 三处写死
/// `if resolved.reasoning { "medium" }`），于是 `settings.json` 的
/// `defaultThinkingLevel` 只影响页脚、不影响下发 provider 的 `reasoning_effort`。
#[must_use]
pub fn resolve_thinking_level(
    explicit: Option<&str>,
    session_level: Option<&str>,
    per_model_level: Option<&str>,
    settings_level: Option<&str>,
    model_reasoning: bool,
) -> ThinkingLevel {
    resolve_thinking_level_with_map(
        explicit,
        session_level,
        per_model_level,
        settings_level,
        model_reasoning,
        None,
    )
}

/// [`resolve_thinking_level`] 的完整版：额外按模型的 `thinkingLevelMap` 夹档位
/// （ref `ai/src/models.ts:921-955`）。
///
/// 与简化版的关系：简化版等价于"映射表为 `None`"，此时可用集合是
/// `off/minimal/low/medium/high`（`xhigh`/`max` 未声明 ⇒ 排除），
/// 与旧实现"七档全给"的差别只是那两个**扩展档**——正是本卡修的内容。
#[must_use]
pub fn resolve_thinking_level_with_map(
    explicit: Option<&str>,
    session_level: Option<&str>,
    per_model_level: Option<&str>,
    settings_level: Option<&str>,
    model_reasoning: bool,
    thinking_level_map: Option<&std::collections::BTreeMap<String, Option<String>>>,
) -> ThinkingLevel {
    let mut source = "builtin";
    let raw = explicit
        .or_else(|| session_level.inspect(|_| source = "session"))
        .or_else(|| per_model_level.inspect(|_| source = "per_model"))
        .or_else(|| settings_level.inspect(|_| source = "default"))
        .unwrap_or(DEFAULT_THINKING_LEVEL)
        .to_string();
    if let Some(value) = explicit {
        let _ = value;
        source = "explicit";
    }
    if !model_reasoning {
        // ref `:251-253`：`!model` ⇒ `off`；非推理模型同理（`clampThinkingLevel` 只留 `off`）。
        return ThinkingLevel {
            level: "off".to_string(),
            source: "forced_off",
            raw,
        };
    }
    let available = supported_levels(model_reasoning, thinking_level_map);
    let level = clamp_against(&available, &raw);
    ThinkingLevel { level, source, raw }
}

/// 该模型**受支持**的档位集合（ref `models.ts:921-935` 的 `getSupportedThinkingLevels`）。
///
/// 规则与 ref 逐行对应：
/// - 非推理模型 ⇒ 只有 `off`；
/// - 推理模型按 [`EXTENDED_THINKING_LEVELS`] 过滤：
///   - `map[level]` 为显式 `null` ⇒ **排除**该档（模型声明不支持）；
///   - `xhigh`/`max` **只在该档有声明时**出现（`mapped !== undefined`）；
///   - 其余档恒保留（未声明也保留）。
///
/// 这条规则此前完全没有实现：pi-rs 只按 `reasoning` 给固定七档，于是
/// 声明了 `xhigh`/`max` 的模型少给最高两档、把某档声明为 `null` 的模型多给该档
/// （`LIMITATIONS.md` "思考档位映射表"）。
#[must_use]
pub fn supported_levels(
    model_reasoning: bool,
    map: Option<&std::collections::BTreeMap<String, Option<String>>>,
) -> Vec<&'static str> {
    if !model_reasoning {
        return vec!["off"];
    }
    EXTENDED_THINKING_LEVELS
        .iter()
        .copied()
        .filter(|level: &&str| {
            let mapped = map.and_then(|entries| entries.get(*level));
            // `map[level] === null` ⇒ 排除。注意与"键不存在"（外层 `None`）区分。
            if let Some(None) = mapped {
                return false;
            }
            // `xhigh`/`max` 只有在声明了非 `null` 值时才保留。
            if *level == "xhigh" || *level == "max" {
                return mapped.is_some();
            }
            true
        })
        .collect()
}

/// 把请求档位夹到**给定集合**（ref `models.ts:937-955` 的 `clampThinkingLevel`）。
///
/// 不在集合内时：先往**更高**档找第一个可用的，再往**更低**档找；
/// 都找不到 ⇒ 集合首项（`availableLevels[0] ?? "off"`）。
#[must_use]
pub fn clamp_against(available: &[&'static str], requested: &str) -> String {
    if available.contains(&requested) {
        return requested.to_string();
    }
    let Some(requested_index) = EXTENDED_THINKING_LEVELS
        .iter()
        .position(|level| *level == requested)
    else {
        return available.first().copied().unwrap_or("off").to_string();
    };
    for candidate in &EXTENDED_THINKING_LEVELS[requested_index..] {
        if available.contains(candidate) {
            return (*candidate).to_string();
        }
    }
    for candidate in EXTENDED_THINKING_LEVELS[..requested_index].iter().rev() {
        if available.contains(candidate) {
            return (*candidate).to_string();
        }
    }
    available.first().copied().unwrap_or("off").to_string()
}

/// 按 ref `models.ts:942-952` 把请求档位夹到受支持的集合。
///
/// 本文实现只在"推理模型"上下文中被调用，可用集合即
/// [`EXTENDED_THINKING_LEVELS`]；未知值 ⇒ `off`（`availableLevels[0] ?? "off"`）。
#[must_use]
pub fn clamp_to_supported(requested: &str) -> String {
    if EXTENDED_THINKING_LEVELS.contains(&requested) {
        return requested.to_string();
    }
    "off".to_string()
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    #[test]
    fn settings_default_is_used_when_nothing_else_specifies() {
        let got = resolve_thinking_level(None, None, None, Some("high"), true);
        assert_eq!(got.level, "high", "{got:?}");
        assert_eq!(got.source, "default", "{got:?}");
        assert_eq!(got.raw, "high");
    }

    #[test]
    fn session_level_beats_settings() {
        let got = resolve_thinking_level(None, Some("low"), None, Some("high"), true);
        assert_eq!(got.level, "low", "ref `sdk.ts:237`：会话值优先");
        assert_eq!(got.source, "session");
    }

    #[test]
    fn per_model_beats_settings_default() {
        // 无映射表的模型不支持 `xhigh`（ref `models.ts:929`：扩展档只在声明时出现）
        // ⇒ 夹到最近的 `high`。此前这条断言 `xhigh`，与 ref 相反。
        let got = resolve_thinking_level(None, None, Some("xhigh"), Some("high"), true);
        assert_eq!(got.level, "high", "ref `models.ts:929` 与 `:942-952`");
        assert_eq!(got.source, "per_model");

        // 声明了 `xhigh` 的模型才真的拿到 `xhigh`。
        let map: std::collections::BTreeMap<String, Option<String>> =
            [("xhigh".to_string(), Some("xhigh".to_string()))]
                .into_iter()
                .collect();
        let got = resolve_thinking_level_with_map(
            None,
            None,
            Some("xhigh"),
            Some("high"),
            true,
            Some(&map),
        );
        assert_eq!(got.level, "xhigh", "ref `models.ts:921-935`");
        assert_eq!(got.source, "per_model");
    }

    #[test]
    fn session_beats_per_model() {
        let got = resolve_thinking_level(None, Some("minimal"), Some("max"), Some("high"), true);
        assert_eq!(got.level, "minimal");
    }

    #[test]
    fn explicit_beats_everything() {
        let got = resolve_thinking_level(Some("low"), Some("max"), Some("max"), Some("high"), true);
        assert_eq!(got.level, "low");
        assert_eq!(got.source, "explicit");
    }

    #[test]
    fn non_reasoning_model_is_forced_off() {
        let got = resolve_thinking_level(None, None, None, Some("high"), false);
        assert_eq!(got.level, "off", "非推理模型只有 off");
        assert_eq!(got.source, "forced_off");
        assert_eq!(got.raw, "high", "原始值要留下来供排障");
    }

    #[test]
    fn builtin_default_applies_when_settings_is_silent() {
        let got = resolve_thinking_level(None, None, None, None, true);
        assert_eq!(got.level, DEFAULT_THINKING_LEVEL);
        assert_eq!(got.source, "builtin");
    }

    #[test]
    fn invalid_settings_value_clamps_to_off() {
        let got = resolve_thinking_level(None, None, None, Some("bogus"), true);
        assert_eq!(
            got.level, "off",
            "未知档位 ⇒ off（ref `models.ts:942-943`）"
        );
        assert_eq!(got.raw, "bogus");
    }
}

#[cfg(test)]
mod thinking_level_map_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn map(pairs: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.map(str::to_string)))
            .collect()
    }

    /// 非推理模型只有 `off`（`models.ts:923`）。
    #[test]
    fn non_reasoning_models_only_support_off() {
        assert_eq!(supported_levels(false, None), vec!["off"]);
        assert_eq!(
            supported_levels(false, Some(&map(&[("high", Some("high"))]))),
            vec!["off"]
        );
    }

    /// 没有映射表时是全部七档（`models.ts:926-933`：`mapped === undefined` 且
    /// 非 `xhigh`/`max` ⇒ 保留；`xhigh`/`max` 因 `mapped !== undefined` 为假而排除）。
    #[test]
    fn no_map_excludes_the_two_extended_levels() {
        let levels = supported_levels(true, None);
        assert!(!levels.contains(&"xhigh"), "{levels:?}");
        assert!(!levels.contains(&"max"), "{levels:?}");
        assert_eq!(levels, vec!["off", "minimal", "low", "medium", "high"]);
    }

    /// 声明了 `xhigh`/`max` 才出现（这正是 LIMITATIONS 记的"少给最高两档"）。
    #[test]
    fn extended_levels_appear_only_when_declared() {
        let levels = supported_levels(
            true,
            Some(&map(&[("xhigh", Some("xhigh")), ("max", Some("max"))])),
        );
        assert!(levels.contains(&"xhigh"), "{levels:?}");
        assert!(levels.contains(&"max"), "{levels:?}");
    }

    /// `null` 表示不支持该档（LIMITATIONS 的"多给该档"）。
    #[test]
    fn null_excludes_a_level() {
        let levels = supported_levels(true, Some(&map(&[("minimal", None)])));
        assert!(!levels.contains(&"minimal"), "{levels:?}");
        assert!(levels.contains(&"low"), "{levels:?}");
    }

    /// clamp 先向后、再向前找最近档（`models.ts:942-952`）。
    #[test]
    fn clamp_prefers_the_next_higher_then_lower_level() {
        let available = ["off", "low", "high"];
        // `medium` 不在集合内 ⇒ 向后找 `high`。
        assert_eq!(clamp_against(&available, "medium"), "high");
        // `xhigh` 越界 ⇒ 向前找 `high`。
        assert_eq!(clamp_against(&available, "xhigh"), "high");
        // `minimal` ⇒ 向后找 `low`。
        assert_eq!(clamp_against(&available, "minimal"), "low");
    }
}
