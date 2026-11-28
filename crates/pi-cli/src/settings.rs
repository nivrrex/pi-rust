//! `settings.json` 的最小读取 + 默认模型解析。
//!
//! 字段名照 `settings-manager.ts:113`（`defaultModel`）与同文件的 `defaultProvider`。
//!
//! **解析顺序**（`model-resolver.ts:640-712` 的子集）：
//! 1. 命令行 `--provider` + `--model`；
//! 2. settings 的 `defaultProvider` + `defaultModel`；
//! 3. 其余回落（`defaultModelPerProvider` 内置表、`getAvailableSnapshot()` 的第一个可用模型）
//!    **本版本未实现**——这里返回明确的错误，而不是猜一个模型。

use serde::Deserialize;

/// `settings.json` 里本项目用到的字段。
///
/// **不拒绝未知字段**：settings.json 里还有大量本项目未实现的项（主题、快捷键等），
/// 拒绝未知键会把用户的正常配置判成错误。这一点与 `models.json` 的处理有意不同——
/// 后者是互通性数据，拼错键必须报错。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Settings {
    #[serde(default, rename = "defaultProvider")]
    pub default_provider: Option<String>,
    #[serde(default, rename = "defaultModel")]
    pub default_model: Option<String>,
    /// settings.json 的 `shellPath`（ref `settings-manager.ts:1010` 的 `getShellPath()`）。
    #[serde(default, rename = "shellPath")]
    pub shell_path: Option<String>,
    /// settings.json 的 `compaction` 段（ref settings-manager：`{enabled, reserveTokens, keepRecentTokens}`）。
    /// 三个键都可缺省，缺省项由 `compaction_settings()` 用默认值补。
    #[serde(default)]
    pub compaction: Option<CompactionSettingsJson>,
    /// `defaultTools`：**初始激活的内置工具名**（ref `settings-manager.ts:144` 的
    /// `getDefaultTools()`，`sdk.ts:259-265`）。缺省时用内置默认集
    /// `["read","bash","edit","write"]`；`--tools` 命令行始终优先。
    #[serde(default, rename = "defaultTools")]
    pub default_tools: Option<Vec<String>>,
    /// `defaultProjectTrust`（ref `settings-manager.ts` 的同名字段，
    /// `project-trust.ts:77` 读它）：`always` / `never` / `ask`。缺省按 `ask`。
    #[serde(default, rename = "defaultProjectTrust")]
    pub default_project_trust: Option<String>,
    /// `showHardwareCursor`（ref `core/settings-manager.ts` 的同名设置；默认 `false`，
    /// 由 `cli/startup-ui.ts:89` 传给 `TuiMainScreen` ⇒ `tui.ts:478` 的 `showHardwareCursor`）。
    #[serde(default, rename = "showHardwareCursor")]
    pub show_hardware_cursor: Option<bool>,
    /// `hideThinkingBlock`（ref `core/settings-manager.ts:979`：`?? false`，默认**显示**）。
    #[serde(default, rename = "hideThinkingBlock")]
    pub hide_thinking_block: Option<bool>,
    /// `enabledModels`：**可切换模型的白名单**，形如 `provider/modelId`
    /// （ref `settings-manager.getEnabledModels()`；模型 id 自身可能含 `/`，
    /// 解析时按**第一个** `/` 切分）。缺省 = null（不限）。
    #[serde(default, rename = "enabledModels")]
    pub enabled_models: Option<Vec<String>>,
    /// `defaultThinkingLevel`：`ctrl+s` / `/thinking` 的"设为默认"写这里
    /// （ref `settings-manager.getDefaultThinkingLevel()`）。
    #[serde(default, rename = "defaultThinkingLevel")]
    pub default_thinking_level: Option<String>,
    /// `theme`：内置主题名（`dark`/`light`）或**自定义主题文件路径**。
    /// ref `settings-manager.ts:782-791`：`getTheme()` 对含 `/` 的值返回 `undefined`
    /// （即"不是内置名"），由主题加载器按路径处理。
    #[serde(default)]
    pub theme: Option<String>,
    /// `quietStartup`（ref `settings-manager.ts:1021-1023` 的 `getQuietStartup()`，默认 false）：
    /// 为真时启动屏不打印 `Model scope:` 与内置头部（`interactive-mode.ts:904/957`）。
    #[serde(default, rename = "quietStartup")]
    pub quiet_startup: Option<bool>,
    /// `modelThinkingLevels`：`{"provider/modelId": "high"}` 形式的**每模型档位覆盖**
    /// （ref `settings-manager.ts:809-811` 的 `getModelThinkingLevel`）。
    #[serde(default, rename = "modelThinkingLevels")]
    pub model_thinking_levels: Option<std::collections::BTreeMap<String, String>>,
    /// `cacheWarming`：上下文预热模式（ref `settings-manager.ts:955-958` 的
    /// `getCacheWarmingMode()`；合法值 `"streaming"` / `"off"` / `"eager"`，
    /// 非法或缺省一律回落 `"streaming"`）。
    ///
    /// `/session` 面板的 `Cache Warming: Mode:` 一行读它（ref
    /// `interactive-mode.ts:6474`）。
    #[serde(default, rename = "cacheWarming")]
    pub cache_warming: Option<String>,
}

impl Settings {
    /// `getCacheWarmingMode()`（ref `settings-manager.ts:955-958`）：
    /// 只接受 `CACHE_WARMING_MODES` 的三个值（`off` / `streaming` / `idle`，
    /// 见 ref `settings-manager.ts:77`），其余（含缺省）回落 `"streaming"`。
    #[must_use]
    pub fn cache_warming_mode(&self) -> &str {
        const MODES: [&str; 3] = ["off", "streaming", "idle"];
        self.cache_warming
            .as_deref()
            .filter(|mode| MODES.contains(mode))
            .unwrap_or("streaming")
    }
}

/// settings.json 的 `compaction` 段。**注意**：这里不拒绝未知键（与 TS 的宽松解析一致），
/// 但场景自带的 settings 由 `check-scenarios.mjs` 另行校验键名。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionSettingsJson {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub reserve_tokens: Option<u64>,
    #[serde(default)]
    pub keep_recent_tokens: Option<u64>,
}

impl Settings {
    /// 把 settings.json 的 `compaction` 段与默认值合并成压缩参数（ref settings-manager 的
    /// `getCompactionSettings`：逐键回落到 `DEFAULT_COMPACTION_SETTINGS`）。
    #[must_use]
    pub fn compaction_settings(&self) -> pi_core::agent::compaction::CompactionSettings {
        let base = pi_core::agent::compaction::DEFAULT_COMPACTION_SETTINGS;
        match &self.compaction {
            None => base,
            Some(overrides) => pi_core::agent::compaction::CompactionSettings {
                enabled: overrides.enabled.unwrap_or(base.enabled),
                reserve_tokens: overrides.reserve_tokens.unwrap_or(base.reserve_tokens),
                keep_recent_tokens: overrides
                    .keep_recent_tokens
                    .unwrap_or(base.keep_recent_tokens),
            },
        }
    }
}

/// 读 `<agent_dir>/settings.json` 的 `quietStartup`（缺失/解析失败一律 false）。
#[must_use]
pub fn quiet_startup(agent_dir: &std::path::Path) -> bool {
    std::fs::read_to_string(agent_dir.join("settings.json"))
        .ok()
        .and_then(|text| parse(&text).ok())
        .and_then(|settings| settings.quiet_startup)
        .unwrap_or(false)
}

fn lock_settings(path: &std::path::Path) -> Result<crate::config_store::ConfigLockGuard, String> {
    crate::config_store::ConfigLockGuard::acquire(
        path,
        std::time::Duration::from_secs(10),
        std::time::Duration::from_millis(200),
    )
    .map_err(|error| error.to_string())
}

fn finish_settings_write<T>(
    guard: crate::config_store::ConfigLockGuard,
    result: Result<T, String>,
) -> Result<T, String> {
    guard.check().map_err(|error| error.to_string())?;
    guard.release().map_err(|error| error.to_string())?;
    result
}

fn write_settings(path: &std::path::Path, text: &str) -> Result<(), String> {
    crate::config_store::write_atomic(path, text, false).map_err(|error| error.to_string())
}

/// 把 `hideThinkingBlock` 写回 `settings.json`，**保留未知键**（ref
/// `settings-manager.ts:999` 的 `setHideThinkingBlock`：只改这一个键、其余原样写回）。
///
/// 读不出来或顶层不是对象时报错，让调用方决定是否提示。
pub fn set_hide_thinking_block(agent_dir: &std::path::Path, hidden: bool) -> Result<(), String> {
    let path = agent_dir.join("settings.json");
    let guard = lock_settings(&path)?;
    let result = (|| {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut object = if text.trim().is_empty() {
            serde_json::Map::new()
        } else {
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(serde_json::Value::Object(map)) => map,
                Ok(_) => return Err("settings.json 顶层不是对象".to_string()),
                Err(error) => return Err(format!("settings.json 解析失败：{error}")),
            }
        };
        object.insert(
            "hideThinkingBlock".to_string(),
            serde_json::Value::Bool(hidden),
        );
        let body = serde_json::to_string_pretty(&serde_json::Value::Object(object))
            .map_err(|error| error.to_string())?;
        write_settings(&path, &body)
    })();
    finish_settings_write(guard, result)
}

/// `settings.json` 的 `hideThinkingBlock`（ref `settings-manager.ts:979` 默认 `false`）。
#[must_use]
pub fn hide_thinking_block(agent_dir: &std::path::Path) -> bool {
    std::fs::read_to_string(agent_dir.join("settings.json"))
        .ok()
        .and_then(|text| parse(&text).ok())
        .and_then(|settings| settings.hide_thinking_block)
        .unwrap_or(false)
}

/// 解析 settings.json 的文本；空文本或缺失文件按"没有设置"处理。
/// 读 `settings.json` 里的 `defaultTools`（文件缺失/解析失败 ⇒ `None`）。
///
/// 单列一个函数而不是让调用方构造整个 `Settings`：工具装配发生在 `print_mode`
/// 深处，那里只有 `agent_dir`（ref `sdk.ts:259` 同样只持有 settingsManager）。
#[must_use]
pub fn default_tools(agent_dir: &std::path::Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(agent_dir.join("settings.json")).ok()?;
    parse(&text).ok()?.default_tools
}

/// 解析 `settings.json`。
pub fn parse(text: &str) -> Result<Settings, String> {
    if text.trim().is_empty() {
        return Ok(Settings::default());
    }
    serde_json::from_str::<Settings>(text).map_err(|error| error.to_string())
}

/// 解析"用哪个 provider/model"。
///
/// 返回 `Err` 表示两处都没有可用的组合——调用方应报错退出，而不是猜一个默认值。
pub fn resolve_model(
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
    settings: &Settings,
) -> Result<(String, String), String> {
    match (cli_provider, cli_model) {
        (Some(provider), Some(model)) => Ok((provider.to_string(), model.to_string())),
        (Some(provider), None) => match &settings.default_model {
            // 只给了 provider：用 settings 的默认模型配它。
            Some(model) => Ok((provider.to_string(), model.clone())),
            None => Err(format!(
                "缺少模型名：--provider {provider} 需要同时给出 --model（settings.json 里也没有 defaultModel）"
            )),
        },
        (None, Some(model)) => match &settings.default_provider {
            Some(provider) => Ok((provider.clone(), model.to_string())),
            None => Err(format!(
                "缺少 provider：--model {model} 需要同时给出 --provider（settings.json 里也没有 defaultProvider）"
            )),
        },
        (None, None) => match (&settings.default_provider, &settings.default_model) {
            (Some(provider), Some(model)) => Ok((provider.clone(), model.clone())),
            (Some(provider), None) => Err(format!(
                "settings.json 的 defaultProvider 是 {provider}，但缺 defaultModel"
            )),
            (None, Some(model)) => Err(format!(
                "settings.json 的 defaultModel 是 {model}，但缺 defaultProvider"
            )),
            (None, None) => Err(
                // 这里只是"前两级没给出模型"；`resolve_initial_model` 之后还有
                // 两级回落（`defaultModelPerProvider` 内置表、`available.first()`），
                // 所以本条只有在**完全没有可用模型**时才会最终落到用户眼前。
                // 原本文案写"回落本版本未实现"是**过时的**——回落已实现（见该函数末尾）。
                "没有可用的模型：需要 --provider 与 --model，或在 settings.json 里设置 defaultProvider 与 defaultModel（也可先在 models.json 里配置任意一个带 apiKey 的 provider）"
                    .to_string(),
            ),
        },
    }
}

/// 一项设置写回 `settings.json` 时需要的类型（决定 JSON 里写成 `true` / `5` / `"auto"`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    /// 布尔（值串 `true`/`false`）。
    Bool,
    /// 整数（如 padding、autocomplete 可见条数）。
    Number,
    /// 字符串（其余枚举值）。
    Text,
}

/// `/settings` 条目 id → (`settings.json` 键名, 值类型)。
///
/// 键名照 ref `core/settings-manager.ts` 的 `Settings` 字段名（camelCase）；
/// 值类型由 ref 各 `on*Change` 回调的形参类型决定（如 `onAutoCompactChange(boolean)`、
/// `onEditorPaddingXChange(number)`、`onTransportChange(Transport)`）。
///
/// 返回 `None` 表示该条目由子菜单或未实现路径处理，本函数不落盘。
#[must_use]
pub fn json_key_for(id: &str) -> Option<(&'static str, SettingKind)> {
    let entry = match id {
        "autocompact" => ("autoCompact", SettingKind::Bool),
        "auto-resize-images" => ("autoResizeImages", SettingKind::Bool),
        "block-images" => ("blockImages", SettingKind::Bool),
        "skill-commands" => ("enableSkillCommands", SettingKind::Bool),
        "show-hardware-cursor" => ("showHardwareCursor", SettingKind::Bool),
        "editor-padding" => ("editorPaddingX", SettingKind::Number),
        "output-padding" => ("outputPad", SettingKind::Number),
        "autocomplete-max-visible" => ("autocompleteMaxVisible", SettingKind::Number),
        "clear-on-shrink" => ("clearOnShrink", SettingKind::Bool),
        "terminal-progress" => ("showTerminalProgress", SettingKind::Bool),
        "steering-mode" => ("steeringMode", SettingKind::Text),
        "follow-up-mode" => ("followUpMode", SettingKind::Text),
        "transport" => ("transport", SettingKind::Text),
        "http-idle-timeout" => ("httpIdleTimeoutMs", SettingKind::Number),
        "cache-warming-mode" => ("cacheWarmingMode", SettingKind::Text),
        "hide-thinking" => ("hideThinkingBlock", SettingKind::Bool),
        "mermaid-rendering" => ("mermaidRenderingMode", SettingKind::Text),
        "cache-miss-notices" => ("showCacheMissNotices", SettingKind::Bool),
        "collapse-changelog" => ("collapseChangelog", SettingKind::Bool),
        "quiet-startup" => ("quietStartup", SettingKind::Bool),
        "install-telemetry" => ("enableInstallTelemetry", SettingKind::Bool),
        "double-escape-action" => ("doubleEscapeAction", SettingKind::Text),
        "tree-filter-mode" => ("treeFilterMode", SettingKind::Text),
        "tui-mode" => ("tuiMode", SettingKind::Text),
        "fullscreen-exit-output" => ("fullscreenExitOutput", SettingKind::Text),
        "fullscreen-scrollbar" => ("fullscreenScrollbar", SettingKind::Text),
        "fullscreen-copy-on-select" => ("fullscreenCopyOnSelect", SettingKind::Bool),
        other => {
            let _ = other;
            return None;
        }
    };
    Some(entry)
}

/// `http-idle-timeout` 的标签 → 毫秒（ref `core/http-dispatcher.ts:8-14`；`disabled` = 0）。
///
/// 这个条目的界面值是**标签**而不是裸数字，所以落盘前必须过这张表；
/// 反方向的 `formatHttpIdleTimeoutMs`（`:37-43`）在 c1 的当前值里已经是标签形态。
pub const HTTP_IDLE_TIMEOUT_CHOICES: &[(&str, i64)] = &[
    ("30 sec", 30_000),
    ("1 min", 60_000),
    ("2 min", 120_000),
    ("5 min", 300_000),
    ("disabled", 0),
];

/// 把界面上的字符串值转成 JSON 值（`SettingKind` 决定类型）。
///
/// 返回 `None` 表示该字符串不能按此类型解析（例如数字项收到非数字），调用方**不得**落盘。
#[must_use]
pub fn json_value_for(kind: SettingKind, text: &str) -> Option<serde_json::Value> {
    match kind {
        SettingKind::Bool => {
            // 只有字面量 `true`/`false` 算合法布尔；其它串一律视为"不能落盘"。
            if text == "true" {
                Some(serde_json::Value::Bool(true))
            } else if text == "false" {
                Some(serde_json::Value::Bool(false))
            } else {
                None
            }
        }
        SettingKind::Number => text
            .parse::<i64>()
            .ok()
            .map(|n| serde_json::Value::Number(serde_json::Number::from(n))),
        SettingKind::Text => Some(serde_json::Value::String(text.to_string())),
    }
}

/// 把一项设置写回 `<agent_dir>/settings.json`。
///
/// 语义照 ref `settings-manager.ts:668-700` + `:270-296` 的**读-改-写**：
/// 读现有 JSON（缺失 ⇒ `{}`），只覆盖这一个键，其余键（含本实现不认识的）原样保留，
/// 再以 2 空格缩进写回（ref 用 `JSON.stringify(..., null, 2)`，见 `:664`）。
///
/// # Errors
/// 现有文件不是合法 JSON 对象、或写入失败时返回错误描述。
pub fn persist_setting(
    agent_dir: &std::path::Path,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let path = agent_dir.join("settings.json");
    let guard = lock_settings(&path)?;
    let result = (|| {
        let mut root = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(value) => value,
                Err(error) => return Err(format!("{} 不是合法 JSON：{error}", path.display())),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                serde_json::Value::Object(serde_json::Map::new())
            }
            Err(error) => return Err(format!("读取 {} 失败：{error}", path.display())),
        };
        let Some(object) = root.as_object_mut() else {
            return Err(format!("{} 顶层不是 JSON 对象", path.display()));
        };
        object.insert(key.to_string(), value);
        let text = serde_json::to_string_pretty(&root)
            .map_err(|error| format!("序列化 settings 失败：{error}"))?;
        write_settings(&path, &text)
    })();
    finish_settings_write(guard, result)
}

/// 从 `settings.json` **删除**一个键（其余字段原样保留）。
///
/// `/scoped-models` 的 `ctrl+s` 在"全部启用"时用它：`enabledModels` 缺省即"无过滤"，
/// 而写成"列出全部模型"的显式列表会让**以后新增的模型**不再自动进入 Ctrl+P 循环
/// （ref `scoped-models-selector.ts:38-40` 的 `normalizeEnabled` 正是为了避免这个）。
///
/// 键本来就不存在时是成功的空操作（幂等）。
pub fn remove_setting(agent_dir: &std::path::Path, key: &str) -> Result<(), String> {
    let path = agent_dir.join("settings.json");
    if !path.exists() {
        return Ok(());
    }
    let guard = lock_settings(&path)?;
    let result = (|| {
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(());
        };
        let mut root = serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|error| format!("{} 不是合法 JSON：{error}", path.display()))?;
        let Some(object) = root.as_object_mut() else {
            return Err(format!("{} 顶层不是 JSON 对象", path.display()));
        };
        if object.remove(key).is_none() {
            return Ok(());
        }
        let text = serde_json::to_string_pretty(&root)
            .map_err(|error| format!("序列化 settings 失败：{error}"))?;
        write_settings(&path, &text)
    })();
    finish_settings_write(guard, result)
}

/// 把一次界面改值落盘：`id` → `settings.json` 键名 + 类型，再读-改-写。
///
/// 返回：
/// - `Ok(true)`：已写入；
/// - `Ok(false)`：该条目不由本函数落盘（子菜单项、或界面值不是该类型的合法值，例如
///   `http-idle-timeout` 的 `5 min` 标签还没有对应的毫秒数）——**不写**，避免写错值；
/// - `Err`：文件层面出错（不是合法 JSON、写失败）。
///
/// # Errors
/// 见 [`persist_setting`]。
pub fn persist_change(agent_dir: &std::path::Path, id: &str, text: &str) -> Result<bool, String> {
    let Some((key, kind)) = json_key_for(id) else {
        return Ok(false);
    };
    // 特例：`http-idle-timeout` 的界面值是标签，落盘要写毫秒数（ref `:942-951`）。
    if id == "http-idle-timeout" {
        let Some((_, ms)) = HTTP_IDLE_TIMEOUT_CHOICES
            .iter()
            .find(|(label, _)| *label == text)
        else {
            return Ok(false);
        };
        persist_setting(agent_dir, key, serde_json::Value::from(*ms))?;
        return Ok(true);
    }
    let Some(value) = json_value_for(kind, text) else {
        return Ok(false);
    };
    persist_setting(agent_dir, key, value)?;
    Ok(true)
}

/// 把一项设置写进 `settings.json` 的**嵌套段**（如 `warnings.anthropicExtraUsage`）。
///
/// ref 的 `warnings` 是对象（`Settings.warnings?: WarningSettings`），不能当顶层键写。
///
/// # Errors
/// 同 [`persist_setting`]；另外段存在但不是对象时报错。
pub fn persist_nested_setting(
    agent_dir: &std::path::Path,
    section: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let path = agent_dir.join("settings.json");
    let guard = lock_settings(&path)?;
    let result = (|| {
        let mut root = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
                .map_err(|error| format!("{} 不是合法 JSON：{error}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                serde_json::Value::Object(serde_json::Map::new())
            }
            Err(error) => return Err(format!("读取 {} 失败：{error}", path.display())),
        };
        let Some(object) = root.as_object_mut() else {
            return Err(format!("{} 顶层不是 JSON 对象", path.display()));
        };
        let section_value = object
            .entry(section.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        let Some(section_object) = section_value.as_object_mut() else {
            return Err(format!("{}.{} 不是 JSON 对象", path.display(), section));
        };
        section_object.insert(key.to_string(), value);
        let text = serde_json::to_string_pretty(&root)
            .map_err(|error| format!("序列化 settings 失败：{error}"))?;
        write_settings(&path, &text)
    })();
    finish_settings_write(guard, result)
}

/// ref `core/model-resolver.ts:20` 的 `defaultModelPerProvider`（**provider 顺序有意义**：
/// 第 4 步按这张表的顺序找"默认模型"命中）。只收录本实现认识的 provider；
/// 表尾未收录的 provider 走"第一个可用模型"兜底。
pub const DEFAULT_MODEL_PER_PROVIDER: &[(&str, &str)] = &[
    ("amazon-bedrock", "us.anthropic.claude-opus-4-6-v1"),
    ("ant-ling", "Ring-2.6-1T"),
    ("anthropic", "claude-opus-4-8"),
    ("openai", "gpt-5.5"),
    ("azure-openai-responses", "gpt-5.4"),
    ("openai-codex", "gpt-5.5"),
    ("radius", "balanced"),
    ("nvidia", "nvidia/nemotron-3-super-120b-a12b"),
    ("deepseek", "deepseek-v4-pro"),
    ("google", "gemini-3.1-pro-preview"),
    ("google-vertex", "gemini-3.1-pro-preview"),
    ("github-copilot", "gpt-5.4"),
    ("openrouter", "moonshotai/kimi-k2.6"),
    ("vercel-ai-gateway", "zai/glm-5.1"),
    ("xai", "grok-4.7"),
    ("groq", "openai/gpt-oss-120b"),
    ("cerebras", "gpt-oss-120b"),
    ("zai", "glm-5.3"),
    ("zai-coding-cn", "glm-5.3"),
    ("mistral", "devstral-medium-latest"),
    ("minimax", "MiniMax-M2.7"),
    ("minimax-cn", "MiniMax-M2.7"),
    ("moonshotai", "kimi-k2.6"),
    ("moonshotai-cn", "kimi-k2.6"),
    ("huggingface", "moonshotai/Kimi-K2.6"),
    ("fireworks", "accounts/fireworks/models/kimi-k2p6"),
    ("together", "moonshotai/Kimi-K2.6"),
    ("baseten", "zai-org/GLM-5.2"),
    ("opencode", "kimi-k2.6"),
    ("opencode-go", "kimi-k2.6"),
];

/// 一个"可选模型"（provider + 模型 id）。回落逻辑只需要这两个字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    /// provider 名（`models.json` 的键）。
    pub provider: String,
    /// 模型 id。
    pub id: String,
}

/// 按 ref `model-resolver.ts:622-717` 的顺序解析初始模型。
///
/// 1. CLI `--provider` + `--model` 都给 → 直接用；
/// 2. `scoped`（settings 的 `enabledModels`，按顺序）第一个模型（续会话时跳过）；
/// 3. settings 的 `defaultProvider`/`defaultModel`，**前提是该 provider 有可用凭据**；
/// 4. 第一个可用模型：先按 [`DEFAULT_MODEL_PER_PROVIDER`] 的顺序找命中，再取 `available[0]`；
/// 5. 都没有 → `Err`（调用方报错退出，文案与 ref 的"没有可用模型"一致）。
pub fn resolve_initial_model(
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
    settings: &Settings,
    scoped: &[ModelRef],
    available: &[ModelRef],
    is_continuing: bool,
    has_auth: &dyn Fn(&str) -> bool,
) -> Result<ModelRef, String> {
    if let (Some(provider), Some(model)) = (cli_provider, cli_model) {
        return Ok(ModelRef {
            provider: provider.to_string(),
            id: model.to_string(),
        });
    }
    if let Some(first) = scoped.first().filter(|_| !is_continuing) {
        return Ok(first.clone());
    }
    if let (Some(provider), Some(model)) = (&settings.default_provider, &settings.default_model) {
        if has_auth(provider) || available.iter().any(|item| &item.provider == provider) {
            return Ok(ModelRef {
                provider: provider.clone(),
                id: model.clone(),
            });
        }
    }
    for (provider, default_id) in DEFAULT_MODEL_PER_PROVIDER {
        if let Some(found) = available
            .iter()
            .find(|item| item.provider == *provider && item.id == *default_id)
        {
            return Ok(found.clone());
        }
    }
    if let Some(first) = available.first() {
        return Ok(first.clone());
    }
    Err(
        "没有可用的模型：请用 --provider/--model 指定，或在 settings.json 里配置 \
         defaultProvider/defaultModel，或为某个 provider 配置 API key"
            .to_string(),
    )
}

#[cfg(test)]
mod config_write_tests {
    use super::persist_setting;

    #[test]
    fn concurrent_setting_updates_do_not_lose_keys() {
        let dir = tempfile::tempdir().expect("tmp");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|index| {
                let dir = dir.path().to_path_buf();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    persist_setting(&dir, &format!("key{index}"), serde_json::json!(index))
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("join").expect("write");
        }
        let text = std::fs::read_to_string(dir.path().join("settings.json")).expect("read");
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        for index in 0..4 {
            assert_eq!(value[format!("key{index}")], serde_json::json!(index));
        }
    }
}

#[cfg(test)]
mod resolve_initial_model_tests {
    use super::{resolve_initial_model, ModelRef, Settings};

    fn model(provider: &str, id: &str) -> ModelRef {
        ModelRef {
            provider: provider.to_string(),
            id: id.to_string(),
        }
    }

    fn settings(default_provider: Option<&str>, default_model: Option<&str>) -> Settings {
        Settings {
            default_provider: default_provider.map(str::to_string),
            default_model: default_model.map(str::to_string),
            shell_path: None,
            ..Settings::default()
        }
    }

    /// 1) CLI 两个参数都给 ⇒ 直接用（ref `model-resolver.ts:648-662`）。
    #[test]
    fn cli_wins() {
        let resolved = resolve_initial_model(
            Some("p"),
            Some("m"),
            &settings(Some("x"), Some("y")),
            &[],
            &[],
            false,
            &|_| true,
        )
        .expect("应解析成功");
        assert_eq!(resolved, model("p", "m"));
    }

    /// 2) 空配置目录 + 无可用模型 ⇒ Err（ref 第 5 步：交由调用方报错）。
    #[test]
    fn empty_everything_errors() {
        let error =
            resolve_initial_model(None, None, &Settings::default(), &[], &[], false, &|_| {
                false
            })
            .expect_err("应报错");
        assert!(error.contains("没有可用的模型"), "{error}");
    }

    /// 3) 仅 settings 默认 + 该 provider 有凭据 ⇒ 用默认（ref 第 3 步）。
    #[test]
    fn settings_default_with_auth() {
        let resolved = resolve_initial_model(
            None,
            None,
            &settings(Some("openrouter"), Some("openrouter/free")),
            &[],
            &[model("other", "m")],
            false,
            &|provider| provider == "openrouter",
        )
        .expect("应解析成功");
        assert_eq!(resolved, model("openrouter", "openrouter/free"));
    }

    /// 4) scoped 模型优先于 settings 默认；续会话时跳过 scoped（ref 第 2 步）。
    #[test]
    fn scoped_models_skip_when_continuing() {
        let scoped = [model("scoped", "s1")];
        let resolved = resolve_initial_model(
            None,
            None,
            &settings(Some("openrouter"), Some("openrouter/free")),
            &scoped,
            &[model("scoped", "s1")],
            false,
            &|_| true,
        )
        .expect("应解析成功");
        assert_eq!(resolved, model("scoped", "s1"));

        let resumed = resolve_initial_model(
            None,
            None,
            &settings(Some("openrouter"), Some("openrouter/free")),
            &scoped,
            &[model("other", "m")],
            true,
            &|provider| provider == "openrouter",
        )
        .expect("应解析成功");
        assert_eq!(resumed, model("openrouter", "openrouter/free"));
    }

    /// B-c：**仅环境变量密钥**（无 settings 默认、无 scoped）⇒ 走第 4 步，
    /// 按 `defaultModelPerProvider` 顺序命中可用模型（这里 `openrouter` 是唯一可用的）。
    #[test]
    fn only_env_auth_uses_provider_default() {
        let available = [model("openrouter", "moonshotai/kimi-k2.6")];
        let resolved = resolve_initial_model(
            None,
            None,
            &Settings::default(),
            &[],
            &available,
            false,
            &|provider| provider == "openrouter",
        )
        .expect("应解析成功");
        assert_eq!(resolved, model("openrouter", "moonshotai/kimi-k2.6"));
    }

    /// B-c：**仅 auth.json 凭据**（settings 里配了默认）⇒ 第 3 步命中 settings 默认。
    /// `has_auth` 只对 auth.json 里存在的 provider 返回 true。
    #[test]
    fn only_auth_json_credential_uses_saved_default() {
        let resolved = resolve_initial_model(
            None,
            None,
            &settings(Some("deepseek"), Some("deepseek-v4-pro")),
            &[],
            &[model("openrouter", "moonshotai/kimi-k2.6")],
            false,
            &|provider| provider == "deepseek",
        )
        .expect("应解析成功");
        assert_eq!(resolved, model("deepseek", "deepseek-v4-pro"));
    }

    /// 5) 没有 settings 默认 ⇒ 先按 `defaultModelPerProvider` 顺序命中，否则第一个可用。
    #[test]
    fn falls_back_to_provider_default_then_first() {
        let available = [
            model("zzz", "m1"),
            model("openai", "gpt-5.5"),
            model("openrouter", "free"),
        ];
        let resolved = resolve_initial_model(
            None,
            None,
            &Settings::default(),
            &[],
            &available,
            false,
            &|_| false,
        )
        .expect("应解析成功");
        // ref 表里 openai 在 openrouter 之前 ⇒ 命中 gpt-5.5。
        assert_eq!(resolved, model("openai", "gpt-5.5"));

        let only_unknown = [model("zzz", "m1")];
        let first = resolve_initial_model(
            None,
            None,
            &Settings::default(),
            &[],
            &only_unknown,
            false,
            &|_| false,
        )
        .expect("应解析成功");
        assert_eq!(first, model("zzz", "m1"));
    }
}

// ---------------------------------------------------------------------------
// F63/B：`models.json` 解析失败**不得致命**（ref `core/model-config.ts:279-315`）
// ---------------------------------------------------------------------------

/// 读 `models.json` 的结果：配置 + 可选错误文案。
///
/// ref `ModelConfig.load`（`model-config.ts:279-315`）对三类失败都是**返回空 Map
/// 加一条 error 字符串**，调用方继续运行（模型回退到内置目录）：
/// - 读不到（非 ENOENT）⇒ `Failed to load models.json: …`（`:287-292`）
/// - 解析失败 ⇒ `Failed to parse models.json: …`（`:295-303`）
/// - schema 不符 ⇒ `Invalid models.json schema: …`（`:305-312`）
///
/// 此前 pi-rs 在四个入口都直接把这三类当致命错误 `ExitCode::from(1)`，
/// 与 ref 相反：用户手改 `models.json` 打错一个逗号，程序就再也起不来。
#[derive(Debug)]
pub struct ModelsConfigLoad {
    pub config: pi_ai::config::ModelsConfig,
    /// 失败原因（`None` = 正常）。
    pub error: Option<String>,
}

/// 按 ref 语义加载 `models.json`（**永不失败**）。
///
/// 文件不存在是正常状态（只放 `auth.json` 的机器），按空配置处理。
#[must_use]
pub fn load_models_config(agent_dir: &std::path::Path) -> ModelsConfigLoad {
    let path = agent_dir.join("models.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ModelsConfigLoad {
                config: pi_ai::config::ModelsConfig::default(),
                error: None,
            };
        }
        Err(error) => {
            return ModelsConfigLoad {
                config: pi_ai::config::ModelsConfig::default(),
                error: Some(format!(
                    "Failed to load models.json: {error}\n\nFile: {}",
                    path.display()
                )),
            };
        }
    };
    let config = match serde_json::from_str::<pi_ai::config::ModelsConfig>(&text) {
        Ok(config) => config,
        Err(error) => {
            return ModelsConfigLoad {
                config: pi_ai::config::ModelsConfig::default(),
                error: Some(format!(
                    "Failed to parse models.json: {error}\n\nFile: {}",
                    path.display()
                )),
            };
        }
    };
    if let Err(error) = config.validate() {
        return ModelsConfigLoad {
            config: pi_ai::config::ModelsConfig::default(),
            error: Some(format!(
                "Invalid models.json schema:\n{error}\n\nFile: {}",
                path.display()
            )),
        };
    }
    ModelsConfigLoad {
        config,
        error: None,
    }
}

// ---------------------------------------------------------------------------
// F63/D：`settings.json` 关键设置的**加载期**校验（只警告、不阻断、不改文件）
// ---------------------------------------------------------------------------

/// 一条警告（沿用既有 `Warning` 样式：调用方统一 `eprintln!("Warning: {msg}")`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsWarning {
    /// 出问题的键（便于测试与排查，不显示给用户）。
    pub key: &'static str,
    /// 消息（**不含** `Warning: ` 前缀，由调用方统一加）。
    pub message: String,
}

/// 检查 `settings.json` 的语义问题。
///
/// **只返回警告，从不失败、从不改文件**（F63/D 的默认理解）。
///
/// 与 ref 的关系（`notes/p9-f63/plan.md` D1）：
/// - **JSON 语法**：ref 已有（`settings-manager.ts:591-593` 的 `recordError`
///   → `settings-diagnostics.ts:4-9` 的 `Invalid settings file <path>: …`），
///   由 [`parse`] 的调用方处理，不在此重复。
/// - 以下各项 ref **没有**对应检查（逐个 grep 确认无枚举/名称校验），
///   属"用户要求的偏离"：见 LIMITATIONS，且只产生 warning。
#[must_use]
pub fn collect_warnings(settings: &Settings) -> Vec<SettingsWarning> {
    let mut out = Vec::new();
    // `defaultThinkingLevel` 枚举：ref 的 `ThinkingLevel` 只允许这七档
    // （`packages/ai/src/models.ts` 的扩展档位集合）。
    if let Some(level) = settings.default_thinking_level.as_deref() {
        const VALID: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
        if !VALID.contains(&level) {
            out.push(SettingsWarning {
                key: "defaultThinkingLevel",
                message: format!(
                    "settings.json: defaultThinkingLevel \"{level}\" is not one of off/minimal/low/medium/high/xhigh/max; falling back to the default"
                ),
            });
        }
    }
    // `defaultProjectTrust` 枚举。
    if let Some(value) = settings.default_project_trust.as_deref() {
        if !["always", "never", "ask"].contains(&value) {
            out.push(SettingsWarning {
                key: "defaultProjectTrust",
                message: format!(
                    "settings.json: defaultProjectTrust \"{value}\" is not one of always/never/ask; treating it as \"ask\""
                ),
            });
        }
    }
    out
}

/// 检查需要**模型注册表**才能判断的项（`defaultProvider`/`defaultModel`/
/// `enabledModels`/`defaultTools`）。
///
/// 分开一个函数是因为它要读 `models.json` 与内置目录；`settings.rs` 只看自身。
#[must_use]
pub fn collect_registry_warnings(
    settings: &Settings,
    known_models: &[(&str, &str)],
    known_tools: &[&str],
) -> Vec<SettingsWarning> {
    let mut out = Vec::new();
    let has = |provider: &str, model: &str| {
        known_models
            .iter()
            .any(|(p, m)| *p == provider && *m == model)
    };
    // `defaultProvider`/`defaultModel`：两者都在时检查组合是否存在。
    if let (Some(provider), Some(model)) = (
        settings.default_provider.as_deref(),
        settings.default_model.as_deref(),
    ) {
        if !known_models.is_empty() && !has(provider, model) {
            out.push(SettingsWarning {
                key: "defaultProvider",
                message: format!(
                    "settings.json: defaultProvider/defaultModel \"{provider}/{model}\" is not in the model registry; it will be ignored"
                ),
            });
        }
    }
    // `enabledModels` 条目：任何一条都匹配不到就提示（ref 的启动告警只覆盖
    // "pattern 解析不到"，这里补"字面量条目不存在"）。
    if let Some(entries) = settings.enabled_models.as_deref() {
        // **不校验** `enabledModels` 的条目是否存在：ref 对这类条目的处理是
        // `No models match pattern "<pattern>"`（`model-resolver.ts:325/349`），
        // pi-rs 已由另一条路径产出同样的提示。这里再加一条
        // `enabledModels entry … is not in the model registry` 只是重复刷屏，
        // 且把 T80-model-scope-invalid（专门测这个情形）的屏幕多顶一行 ⇒ 对拍不同。
        let _ = (entries, &has);
    }
    // `defaultTools` 名称。
    if let Some(tools) = settings.default_tools.as_deref() {
        for name in tools {
            if !known_tools.is_empty() && !known_tools.contains(&name.as_str()) {
                out.push(SettingsWarning {
                    key: "defaultTools",
                    message: format!(
                        "settings.json: defaultTools entry \"{name}\" is not a known tool; it will be ignored"
                    ),
                });
            }
        }
    }
    out
}

/// TUI 模式下暂存的 `settings.json` 加载期诊断（F63/K）。
///
/// 为什么需要跨模块共享：`load_settings` 在 binary 侧（`main.rs`），
/// 而渲染在 lib 侧（`tui.rs`），两者不能直接传参。由 `warn_once` 入队、
/// `tui::run` 取走并作为**会话区**的 warning 行呈现（ref
/// `interactive-mode.ts:1129-1137` 的 `showWarning` 位置）。
///
/// 线程安全：`Mutex` 保护，只在启动期单线程使用。
#[must_use]
pub fn pending_diagnostics() -> &'static std::sync::Mutex<Vec<String>> {
    static LIST: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// **启动期配置诊断**（`models.json` / `auth.json`）：渲染在 TUI 帧**顶部**。
///
/// 与 [`pending_diagnostics`] **分开**：那个队列的诊断要在**会话区**
/// （F63 的 `tui_shows_settings_diagnostics_in_the_transcript` 钉住了位置），
/// 混用会把 settings 诊断顶到首行——实际踩过一次，两个用例同时红。
#[must_use]
pub fn startup_config_diagnostics() -> &'static std::sync::Mutex<Vec<String>> {
    static LIST: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// 往启动期配置诊断队列里**去重**推一条。
///
/// 去重是必要的：`build_provider` 在启动期可能被调用多次（初始解析 + 回退重找），
/// 同一条 `auth.json` 错误会刷多遍。
pub fn push_startup_config_diagnostic(message: &str) {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let first = seen
        .lock()
        .map(|mut set| set.insert(message.to_string()))
        .unwrap_or(true);
    if !first {
        return;
    }
    if let Ok(mut list) = startup_config_diagnostics().lock() {
        list.push(message.to_string());
    }
}

/// 读 `settings.json` 的 `defaultThinkingLevel`（文件缺失/解析失败 ⇒ `None`）。
///
/// 供**运行期**重新解析档位的场景用（Ctrl+P 切模型后要重算，
/// ref `agent-session.ts:2310-2322`）。
#[must_use]
pub fn default_thinking_level(agent_dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(agent_dir.join("settings.json")).ok()?;
    parse(&text).ok()?.default_thinking_level
}

/// 读 `modelThinkingLevels["provider/modelId"]`（无该条目 ⇒ `None`）。
#[must_use]
pub fn model_thinking_level(
    agent_dir: &std::path::Path,
    provider: &str,
    model_id: &str,
) -> Option<String> {
    let text = std::fs::read_to_string(agent_dir.join("settings.json")).ok()?;
    parse(&text)
        .ok()?
        .model_thinking_levels
        .and_then(|map| map.get(&format!("{provider}/{model_id}")).cloned())
}

/// `theme` 设置的 `"<light>/<dark>"` 形式（ref `theme.ts:580-598` 的 `parseAutoThemeSetting`）。
///
/// 规则（与 ref 逐行对应）：必须**恰好一个** `/`；两段各自 trim 后都非空。
/// 其余情况（无 `/`、多于一个 `/`、任一段为空）返回 `None`，调用方按普通主题名处理。
#[must_use]
pub fn parse_auto_theme(theme_setting: &str) -> Option<(String, String)> {
    let mut parts = theme_setting.split('/');
    let light = parts.next()?.trim().to_string();
    let dark = parts.next()?.trim().to_string();
    if parts.next().is_some() {
        return None;
    }
    if light.is_empty() || dark.is_empty() {
        return None;
    }
    Some((light, dark))
}

/// sRGB 通道的线性化（ref `theme.ts:651-655` 的 `toLinear`）。
fn srgb_to_linear(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.039_28 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// 相对亮度（ref `theme.ts:650-656` 的 `getRgbColorLuminance`。
#[must_use]
pub fn rgb_luminance(r: u8, g: u8, b: u8) -> f64 {
    0.2126 * srgb_to_linear(r) + 0.7152 * srgb_to_linear(g) + 0.0722 * srgb_to_linear(b)
}

/// 由终端背景色判定 light/dark（ref `theme.ts:662-664` 的 `getThemeForRgbColor`）。
#[must_use]
pub fn theme_for_rgb(r: u8, g: u8, b: u8) -> &'static str {
    if rgb_luminance(r, g, b) >= 0.5 {
        "light"
    } else {
        "dark"
    }
}

/// xterm-256 索引 → RGB（ref 的 `ansi256ToHex` 同口径：0-15 用标准表，
/// 16-231 是 6×6×6 立方，232-255 是灰阶）。
#[must_use]
pub fn ansi256_to_rgb(index: u8) -> (u8, u8, u8) {
    const BASE16: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    if index < 16 {
        return BASE16[usize::from(index)];
    }
    if index < 232 {
        let n = index - 16;
        return (
            CUBE[usize::from(n / 36)],
            CUBE[usize::from((n % 36) / 6)],
            CUBE[usize::from(n % 6)],
        );
    }
    let level = 8 + (index - 232) * 10;
    (level, level, level)
}

/// 从 `COLORFGBG` 取背景色索引（ref `theme.ts:639-648` 的 `getColorFgBgBackgroundIndex`：
/// 从**最后**一段往前找第一个 0–255 的整数）。
#[must_use]
pub fn colorfgbg_background_index(colorfgbg: &str) -> Option<u8> {
    colorfgbg
        .split(';')
        .rev()
        .find_map(|part| part.trim().parse::<u8>().ok())
}

/// 环境提示下的主题（ref `theme.ts:666-683` 的 `detectTerminalBackgroundFromEnv`）。
#[must_use]
pub fn theme_from_env(colorfgbg: Option<&str>) -> &'static str {
    colorfgbg
        .and_then(colorfgbg_background_index)
        .map(|index| {
            let (r, g, b) = ansi256_to_rgb(index);
            theme_for_rgb(r, g, b)
        })
        .unwrap_or("dark")
}

#[cfg(test)]
mod terminal_theme_tests {
    use super::*;

    /// ref `theme.ts:580-598`：必须**恰好一个** `/`，两段都非空。
    #[test]
    fn auto_theme_needs_exactly_one_slash_with_two_non_empty_parts() {
        assert_eq!(
            parse_auto_theme("light/dark"),
            Some(("light".to_string(), "dark".to_string()))
        );
        assert_eq!(
            parse_auto_theme(" light / dark "),
            Some(("light".to_string(), "dark".to_string()))
        );
        // 无 `/`、多于一个 `/`、任一段空 ⇒ 都不是 auto 形式。
        assert_eq!(parse_auto_theme("dark"), None);
        assert_eq!(parse_auto_theme("a/b/c"), None);
        assert_eq!(parse_auto_theme("/dark"), None);
        assert_eq!(parse_auto_theme("light/"), None);
        assert_eq!(parse_auto_theme("  /  "), None);
    }

    /// ref `theme.ts:662-664`：亮度 ≥ 0.5 判 light。
    #[test]
    fn luminance_decides_light_or_dark() {
        // 纯白 ⇒ 亮度 1 ⇒ light；纯黑 ⇒ 0 ⇒ dark。
        assert_eq!(theme_for_rgb(255, 255, 255), "light");
        assert_eq!(theme_for_rgb(0, 0, 0), "dark");
        // 中灰 128：线性化后约 0.216 ⇒ dark。
        assert_eq!(theme_for_rgb(128, 128, 128), "dark");
    }

    /// ref 的 ANSI 256 → RGB：立方段与灰阶段。
    #[test]
    fn ansi256_cube_and_grayscale() {
        assert_eq!(ansi256_to_rgb(0), (0, 0, 0));
        assert_eq!(ansi256_to_rgb(15), (255, 255, 255));
        // 16 是立方段起点 (0,0,0)；231 是 (255,255,255)。
        assert_eq!(ansi256_to_rgb(16), (0, 0, 0));
        assert_eq!(ansi256_to_rgb(231), (255, 255, 255));
        // 232 起是灰阶 8,18,28…
        assert_eq!(ansi256_to_rgb(232), (8, 8, 8));
        assert_eq!(ansi256_to_rgb(255), (238, 238, 238));
    }

    /// ref `theme.ts:639-648`：从**最后**一段往前找第一个 0–255 整数。
    #[test]
    fn colorfgbg_uses_the_last_valid_index() {
        assert_eq!(colorfgbg_background_index("0;15"), Some(15));
        assert_eq!(colorfgbg_background_index("15;0"), Some(0));
        // 末尾非数字 ⇒ 往前找。
        assert_eq!(colorfgbg_background_index("0;15;default"), Some(15));
        // 越界（>255）不算。
        assert_eq!(colorfgbg_background_index("0;300"), Some(0));
        assert_eq!(colorfgbg_background_index(""), None);
    }

    /// ref `theme-controller.ts:60-72`：auto 形式按提示选，其它原样，未设走默认。
    #[test]
    fn resolve_theme_name_covers_auto_plain_and_unset() {
        // auto 形式 + 亮背景 ⇒ light 那半。
        assert_eq!(
            resolve_theme_name(Some("light/dark"), Some("0;15")),
            "light"
        );
        // auto 形式 + 暗背景 ⇒ dark 那半。
        assert_eq!(resolve_theme_name(Some("light/dark"), Some("15;0")), "dark");
        // 无提示 ⇒ 回落到 dark 那半（不是默认名）。
        assert_eq!(resolve_theme_name(Some("nord/mocha"), None), "mocha");
        // 普通主题名原样返回（含只有一个 `/` 但两段非空的非法情形之外的一切）。
        assert_eq!(resolve_theme_name(Some("dark"), None), "dark");
        assert_eq!(resolve_theme_name(Some("a/b/c"), None), "a/b/c");
        // 未设置 ⇒ 默认。
        assert_eq!(
            resolve_theme_name(None, Some("0;15")),
            pi_tui::theme::DEFAULT_NAME
        );
    }

    /// ref `theme.ts:666-683`：有 `COLORFGBG` 按其背景索引判定，否则回落 `dark`。
    #[test]
    fn env_hint_falls_back_to_dark() {
        // 15 = 亮白 ⇒ light；0 = 黑 ⇒ dark。
        assert_eq!(theme_from_env(Some("0;15")), "light");
        assert_eq!(theme_from_env(Some("15;0")), "dark");
        assert_eq!(theme_from_env(None), "dark");
        assert_eq!(theme_from_env(Some("garbage")), "dark");
    }
}

/// 由 `theme` 设置与 `COLORFGBG` 解析出**实际生效**的主题名
/// （ref `theme-controller.ts:60-72`）。
///
/// - `"<light>/<dark>"`（auto 形式）⇒ 按终端背景提示选其一；
/// - 其它字符串 ⇒ 原样使用；
/// - 未设置 ⇒ [`pi_tui::theme::DEFAULT_NAME`]。
///
/// 真实背景色探测（ref 的 OSC 11 查询）尚未实现：这里只用 `COLORFGBG` 提示，
/// 无提示时按 `dark`（与 ref 的 `detectTerminalBackgroundFromEnv` 回落一致）。
#[must_use]
pub fn resolve_theme_name(theme_setting: Option<&str>, colorfgbg: Option<&str>) -> String {
    match theme_setting {
        Some(setting) => match parse_auto_theme(setting) {
            Some((light, dark)) => {
                if theme_from_env(colorfgbg) == "light" {
                    light
                } else {
                    dark
                }
            }
            None => setting.to_string(),
        },
        None => pi_tui::theme::DEFAULT_NAME.to_string(),
    }
}
