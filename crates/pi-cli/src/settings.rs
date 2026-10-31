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
    /// settings.json 的 `compaction` 段（ref settings-manager：`{enabled, reserveTokens, keepRecentTokens}`）。
    /// 三个键都可缺省，缺省项由 `compaction_settings()` 用默认值补。
    #[serde(default)]
    pub compaction: Option<CompactionSettingsJson>,
    /// `showHardwareCursor`（ref `core/settings-manager.ts` 的同名设置；默认 `false`，
    /// 由 `cli/startup-ui.ts:89` 传给 `TuiMainScreen` ⇒ `tui.ts:478` 的 `showHardwareCursor`）。
    #[serde(default, rename = "showHardwareCursor")]
    pub show_hardware_cursor: Option<bool>,
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
    /// `modelThinkingLevels`：`{"provider/modelId": "high"}` 形式的**每模型档位覆盖**
    /// （ref `settings-manager.ts:809-811` 的 `getModelThinkingLevel`）。
    #[serde(default, rename = "modelThinkingLevels")]
    pub model_thinking_levels: Option<std::collections::BTreeMap<String, String>>,
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

/// 解析 settings.json 的文本；空文本或缺失文件按"没有设置"处理。
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
                "没有可用的模型：需要 --provider 与 --model，或在 settings.json 里设置 defaultProvider 与 defaultModel（内置默认表与\"第一个可用模型\"的回落本版本未实现）"
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
    if let Err(error) = std::fs::create_dir_all(agent_dir) {
        return Err(format!("创建 {} 失败：{error}", agent_dir.display()));
    }
    std::fs::write(&path, format!("{text}\n"))
        .map_err(|error| format!("写入 {} 失败：{error}", path.display()))
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
    if let Err(error) = std::fs::create_dir_all(agent_dir) {
        return Err(format!("创建 {} 失败：{error}", agent_dir.display()));
    }
    std::fs::write(&path, format!("{text}\n"))
        .map_err(|error| format!("写入 {} 失败：{error}", path.display()))
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
