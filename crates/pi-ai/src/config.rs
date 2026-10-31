//! `models.json` 的读取，对应 `ref/pi-0.87.1/packages/coding-agent/src/core/model-config.ts`
//! 的 `ModelsJsonSchema` / `ProviderConfigSchema` / `ModelDefinitionSchema`
//! （见 `spec/f-config.md`）。
//!
//! 互通要求：所有结构都用 `#[serde(flatten)] extra` 保留未知字段，读后写回**语义等价**
//! （未启用 `preserve_order`，因此只保证语义等价、不保证字节等价；见 notes/questions.md 的 Q8）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::decoder::Rates;

/// 模型缺省的最大输出 token（`provider-composer.ts:191`）。
pub const DEFAULT_MAX_TOKENS: u64 = 16_384;

/// 模型的缺省上下文窗口（`provider-composer.ts:190`）。
pub const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;

/// `models.json` 的顶层。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelsConfig {
    /// 按 provider 名索引；`serde_json::Map` 支持非标识符键，保留原样。
    #[serde(default)]
    pub providers: Map<String, Value>,
}

impl ModelsConfig {
    /// 校验**整份** `models.json`（ref `core/model-config.ts:305-312`：`Check` 失败就整份拒绝）。
    ///
    /// 为什么需要它：`ModelsConfig.providers` 是 `Map<String, Value>`（原始 JSON，`config.rs:23-27`），
    /// 顶层解析**不做任何类型校验**；真解析只发生在 `resolve_with_catalog:94-95`（用到哪个 provider 才解析哪个）。
    /// 于是"另一个 provider 的 `cost` 写成了字符串"这类错误，在**懒校验**下永远没人发现，
    /// 而 ref 在**加载时**就对整个文件做 schema 校验。这里补上加载期校验。
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (name, raw) in &self.providers {
            let provider: ProviderConfig =
                serde_json::from_value(raw.clone()).map_err(|error: serde_json::Error| {
                    ConfigError::Invalid(format!("provider {name}: {error}"))
                })?;
            for model in &provider.models {
                // 触发每个模型条目的解析（`ModelDefinition` 已随 provider 一起解析过，
                // 这里显式引用一次，避免"只解析了 provider 忘了模型"的将来改动）。
                let _id = &model.id;
            }
        }
        Ok(())
    }

    /// 解析出一个可用的 `(provider, model)` 组合。
    ///
    /// 不带内置目录：缺 `api` / `baseUrl` 时报 [`ConfigError::MissingField`]，
    /// 与 oracle 的 `missing-api` / `missing-baseurl` 两个形态一致。
    pub fn resolve(
        &self,
        provider_name: &str,
        model_id: &str,
    ) -> Result<ResolvedModel, ConfigError> {
        self.resolve_with_catalog(provider_name, model_id, None)
    }

    /// 用内置目录里的定义解析一个模型，provider 级字段（`baseUrl` 等）仍优先。
    fn resolve_builtin(
        provider_name: &str,
        provider: &ProviderConfig,
        builtin: &BuiltInModel,
    ) -> Result<ResolvedModel, ConfigError> {
        let base_url = provider
            .base_url
            .clone()
            .or_else(|| builtin.base_url.clone())
            .ok_or_else(|| ConfigError::MissingField {
                provider: provider_name.to_string(),
                field: "baseUrl",
                // 内置目录缺 baseUrl 这条路径**没有 oracle 证据**，不猜 TS 文案。
                message: format!("provider {provider_name} 缺少字段 baseUrl"),
            })?;
        let cost = builtin.cost.unwrap_or_default();
        Ok(ResolvedModel {
            provider_name: provider_name.to_string(),
            model_id: builtin.id.clone(),
            api: builtin.api.clone(),
            base_url,
            max_tokens: builtin.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            context_window: builtin.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
            // 内置模型表里没有这些字段，按缺省。
            reasoning: false,
            name: Some(builtin.id.clone()),
            api_key: provider.api_key.clone(),
            headers: merge_headers(provider.headers.as_ref(), None),
            auth_header: provider.auth_header.unwrap_or(false),
            input_supported: vec!["text".to_string()],
            rates: Rates {
                input: cost.input,
                output: cost.output,
                cache_read: cost.cache_read,
                cache_write: cost.cache_write,
            },
        })
    }

    /// 同 [`ModelsConfig::resolve`]，但允许从内置目录回落取 `api` 与费率。
    ///
    /// TS 的行为（`api/provider-composer.ts` 的 `findModelDefaults`）：**内置 provider**
    /// 在 models.json 里没写全时用内置目录补齐。`catalog` 为 `None` 时行为与
    /// [`ModelsConfig::resolve`] 相同。
    pub fn resolve_with_catalog(
        &self,
        provider_name: &str,
        model_id: &str,
        catalog: Option<&BuiltInCatalog>,
    ) -> Result<ResolvedModel, ConfigError> {
        // `models.json` 里没有这个 provider 时，允许**只靠内置目录**解析（ref 的 provider
        // registry 本来就含内置 provider：`/login` 成功后无需手写 models.json 也能跑）。
        let raw: &Value = match self.providers.get(provider_name) {
            Some(raw) => raw,
            None => {
                let builtin = catalog
                    .and_then(|catalog| catalog.find(provider_name, model_id))
                    .or_else(|| {
                        catalog
                            .and_then(|catalog| catalog.defaults_for(provider_name, model_id, None))
                    })
                    .ok_or_else(|| ConfigError::UnknownProvider(provider_name.to_string()))?;
                // **只有目录、没有 models.json**（只放 `auth.json` 的机器）：直接用目录里的
                // 模型定义。此前这里合成 `"models": [{ "id": … }]` 走"用户声明"分支，
                // 目录字段被丢掉 ⇒ `contextWindow` 退回 128000（真机 footer 里 262k 的
                // 模型显示成 128k，`/model` 切到 1M 的模型也还是 128k）。
                // ref 等价路径是"provider 目录内置、models.json 不覆盖"（`applyModelsJson`
                // 的 `config === undefined` 分支），模型定义原样保留。
                let provider = ProviderConfig {
                    api: Some(builtin.api.clone()),
                    base_url: builtin.base_url.clone(),
                    ..ProviderConfig::default()
                };
                return Self::resolve_builtin(provider_name, &provider, builtin);
            }
        };
        let provider: ProviderConfig = serde_json::from_value(raw.clone())
            .map_err(|error: serde_json::Error| ConfigError::Invalid(error.to_string()))?;
        let builtin = catalog.and_then(|catalog| catalog.find(provider_name, model_id));
        // ref `:198-205`：精确 id 缺失时，**同一 provider 的其它内置模型**仍可提供 baseUrl/api
        // （真机 cfg-real 里 `openrouter` 只写了 models、没有 baseUrl ⇒ 必须继承）。
        let provider_defaults = catalog.and_then(|catalog| {
            provider.api.as_deref().map_or_else(
                || catalog.defaults_for(provider_name, model_id, None),
                |api| catalog.defaults_for(provider_name, model_id, Some(api)),
            )
        });
        let builtin = builtin.or(provider_defaults);
        // models.json 里没有该模型时，内置 provider 可以回落到目录定义（TS 同款）。
        let declared = provider.models.iter().find(|model| model.id == model_id);
        let model = match (declared, builtin) {
            (Some(model), _) => model,
            (None, Some(builtin)) => {
                return Self::resolve_builtin(provider_name, &provider, builtin)
            }
            (None, None) => {
                return Err(ConfigError::UnknownModel {
                    provider: provider_name.to_string(),
                    model: model_id.to_string(),
                })
            }
        };
        let api = model
            .api
            .clone()
            .or_else(|| provider.api.clone())
            .or_else(|| builtin.map(|model| model.api.clone()))
            .ok_or_else(|| ConfigError::MissingField {
                provider: provider_name.to_string(),
                field: "api",
                message: format!(
                    "Provider \"{provider_name}\": Provider {provider_name}, model {model_id}: no \"api\" specified. Set at provider or model level."
                ),
            })?;
        let base_url = model
            .base_url
            .clone()
            .or_else(|| provider.base_url.clone())
            .or_else(|| builtin.and_then(|model| model.base_url.clone()))
            .ok_or_else(|| ConfigError::MissingField {
                provider: provider_name.to_string(),
                field: "baseUrl",
                message: format!(
                    "Provider \"{provider_name}\": Provider {provider_name}: \"baseUrl\" is required when defining custom models."
                ),
            })?;
        // 缺省值逐字照 `core/provider-composer.ts:178-191`：
        // name→id、reasoning→false、input→["text"]、cost→全 0、
        // contextWindow→128000、maxTokens→16384。
        let cost = model.cost.unwrap_or_default();
        Ok(ResolvedModel {
            provider_name: provider_name.to_string(),
            model_id: model.id.clone(),
            api,
            base_url,
            max_tokens: model.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            context_window: model.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
            reasoning: model.reasoning.unwrap_or(false),
            name: model.name.clone(),
            api_key: provider.api_key.clone(),
            headers: merge_headers(provider.headers.as_ref(), model.headers.as_ref()),
            auth_header: provider.auth_header.unwrap_or(false),
            input_supported: model
                .input
                .clone()
                .unwrap_or_else(|| vec!["text".to_string()]),
            rates: Rates {
                input: cost.input,
                output: cost.output,
                cache_read: cost.cache_read,
                cache_write: cost.cache_write,
            },
        })
    }
}

/// 一个 provider 的配置。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "baseUrl", default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// provider 级 API key（`providers.<id>.apiKey`，ref `core/model-config.ts:232`）。
    ///
    /// 此前它落在 `extra` 里被**静默忽略**（"extra 吞配置"）：调用方永远传 `None` 给
    /// `build_provider`，于是 models.json 里配的 key 不生效。
    #[serde(rename = "apiKey", default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// provider 级请求头（ref `:235`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    /// `true` 时用 `Authorization: Bearer <apiKey>`（ref `:237` 的 `authHeader`；
    /// 语义见 `provider-composer.ts:297-309` 的 `withConfiguredAuth`：没有 key 就报错）。
    #[serde(
        rename = "authHeader",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub auth_header: Option<bool>,
    #[serde(default)]
    pub models: Vec<ModelDefinition>,
    /// 未识别的字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 一个模型的定义。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelDefinition {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// 模型级 `baseUrl`（ref `provider-composer.ts:171`：`definition.baseUrl ??
    /// providerConfig.baseUrl ?? defaults?.baseUrl` —— **模型级最优先**）。
    /// 此前这个键在解析时被直接丢掉 ⇒ openrouter 这类"baseUrl 写在模型条目里"的配置
    /// 会退回 provider/内置目录的值，或在无目录时报 `baseUrl` 必填。
    #[serde(rename = "baseUrl", default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(rename = "maxTokens", default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(
        rename = "contextWindow",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,
    /// 模型支持的输入模态（`models.json` 的 `input`，缺省 `["text"]`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
    /// 是否推理模型。**决定初始思考等级**：推理模型默认 `medium`、其余夹成 `off`
    /// （`defaults.ts:3` + `agent-session.ts:2321/2325` 的 `clampThinkingLevel`）。
    /// 我此前**根本没解析这个字段**，只在注释里写了"默认 false"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    /// 模型级请求头（ref `core/model-config.ts:202`；与 provider 级合并时**模型级优先**，
    /// 见 `provider-composer.ts:431-445` 的 `rawModelHeaders`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    /// 未识别的字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 模型的费率。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(rename = "cacheRead", default)]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite", default)]
    pub cache_write: f64,
}

impl ModelCost {
    /// 费率缺省时全零。
    pub fn zero() -> Self {
        Self::default()
    }
}

/// 解析出来的、可直接建客户端的模型信息。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModel {
    pub provider_name: String,
    pub model_id: String,
    /// `openai-completions` 或 `anthropic-messages`。
    pub api: String,
    /// 已含 `/v1` 的 base url。
    pub base_url: String,
    pub max_tokens: u64,
    pub context_window: u64,
    pub rates: Rates,
    /// 是否推理模型（`models.json` 的 `reasoning`，缺省 false）。
    pub reasoning: bool,
    /// 模型显示名（`getAvailableSnapshot` 报的 `name`）。
    pub name: Option<String>,
    /// provider 级 `apiKey`（ref `model-config.ts:232`；由调用方按
    /// "stored → CLI → models.json → env" 的优先级决定是否采用）。
    pub api_key: Option<String>,
    /// provider 级 + 模型级请求头的**合并结果**（模型级优先，ref `provider-composer.ts:431-445`）。
    pub headers: Vec<(String, String)>,
    /// `authHeader: true` → 用 `Authorization: Bearer <apiKey>`（ref `:237` + `provider-composer.ts:304-307`）。
    pub auth_header: bool,
    /// 支持的输入模态（缺省 `["text"]`，照 `provider-composer.ts:178-191`）。
    pub input_supported: Vec<String>,
}

/// 合并 provider 级与模型级请求头：**模型级覆盖 provider 级**（ref `provider-composer.ts:431-445`：
/// `{...config.modelOverrides[id].headers, ...definition.headers, ...extensionModel.headers}`）。
///
/// 顺序即最终发送顺序（provider 级在前，同名的模型级覆盖它）。
pub fn merge_headers(
    provider: Option<&std::collections::BTreeMap<String, String>>,
    model: Option<&std::collections::BTreeMap<String, String>>,
) -> Vec<(String, String)> {
    let mut merged: Vec<(String, String)> = Vec::new();
    for (name, value) in provider
        .into_iter()
        .flatten()
        .chain(model.into_iter().flatten())
    {
        merged.retain(|(existing, _not_value)| !existing.eq_ignore_ascii_case(name));
        merged.push((name.clone(), value.clone()));
    }
    merged
}

/// 读取 `models.json` 的失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("models.json 中没有名为 {0} 的 provider")]
    UnknownProvider(String),
    #[error("provider {provider} 下没有名为 {model} 的模型")]
    UnknownModel { provider: String, model: String },
    /// 必填字段缺失。
    ///
    /// `message` 是**与 TS 逐字一致**的完整文案，取自 oracle 记录的
    /// `fixtures/models-config/*.expected.json` 的 `loadError`。TS 的两条文案形如
    /// `Provider "p": Provider p, model m: no "api" specified. Set at provider or model level.`
    /// 与 `Provider "p": Provider p: "baseUrl" is required when defining custom models.`
    /// ——**外层都有 `Provider "<名>": ` 前缀**，这点只看我原先的写法会漏。
    #[error("{message}")]
    MissingField {
        provider: String,
        field: &'static str,
        message: String,
    },
    #[error("models.json 解析失败: {0}")]
    Invalid(String),
}

/// 按 provider 名取配置，未知名字返回 [`ConfigError::UnknownProvider`]。
pub fn provider_of(config: &ModelsConfig, name: &str) -> Result<ProviderConfig, ConfigError> {
    let raw = config
        .providers
        .get(name)
        .ok_or_else(|| ConfigError::UnknownProvider(name.to_string()))?;
    serde_json::from_value(raw.clone())
        .map_err(|error: serde_json::Error| ConfigError::Invalid(error.to_string()))
}

/// 便捷别名：provider 索引的只读视图。
pub type ProviderMap = BTreeMap<String, ProviderConfig>;

/// 内置模型目录里的一个模型。
///
/// 内容来自 oracle 导出的 `fixtures/models-config/builtin-catalog.json`
/// （TS 的 `ModelRegistry.getAll()`，在网络隔离内取得）。
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltInModel {
    pub id: String,
    pub api: String,
    pub base_url: Option<String>,
    pub max_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub cost: Option<ModelCost>,
}

/// 内置模型目录。
///
/// TS 的行为：`models.json` 未定义某个模型时回落到内置目录
/// （`core/model-runtime.ts:183-190` 建目录，`api/provider-composer.ts` 的
/// `findModelDefaults` 查回落）。
#[derive(Debug, Clone, Default)]
pub struct BuiltInCatalog {
    providers: std::collections::BTreeMap<String, Vec<BuiltInModel>>,
}

impl BuiltInCatalog {
    /// 从 oracle 导出的 JSON 解析。
    pub fn from_json(text: &str) -> Result<Self, ConfigError> {
        let raw: Map<String, Value> =
            serde_json::from_str(text).map_err(|error| ConfigError::Invalid(error.to_string()))?;
        let mut providers = std::collections::BTreeMap::new();
        for (provider_id, models) in raw {
            // P7-6am/M：兼容两种形状——
            //   ① `{provider: [model, …]}`（`fixtures/models-config/builtin-catalog.json` 的 oracle 导出形状）
            //   ② `{provider: { models: [model, …], checkedAt, etag }}`（ref 的 `models-store.json`，
            //      见 `core/models-store.ts:46-60`：`FileModelsStore` 存在 `<agentDir>/models-store.json`）
            let list = match models.as_array() {
                Some(list) => list,
                None => models
                    .get("models")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        ConfigError::Invalid(format!("{provider_id} 既不是数组也没有 models 数组"))
                    })?,
            };
            let mut parsed = Vec::with_capacity(list.len());
            for item in list {
                let get = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
                let get_u = |key: &str| item.get(key).and_then(Value::as_u64);
                parsed.push(BuiltInModel {
                    id: get("id")
                        .ok_or_else(|| ConfigError::Invalid("内置模型缺少 id".to_string()))?,
                    api: get("api")
                        .ok_or_else(|| ConfigError::Invalid("内置模型缺少 api".to_string()))?,
                    base_url: get("baseUrl"),
                    max_tokens: get_u("maxTokens"),
                    context_window: get_u("contextWindow"),
                    cost: item
                        .get("cost")
                        .and_then(|cost| serde_json::from_value(cost.clone()).ok()),
                });
            }
            providers.insert(provider_id, parsed);
        }
        Ok(Self { providers })
    }

    /// 把另一份目录叠到自己上面（`models-store.json` 的语义）。
    ///
    /// ref `core/remote-catalog-provider.ts:12-21` 的 `mergeModels`：**按模型 id 逐个覆盖**，
    /// 目录里没有的 id 追加在后面。store 是"动态刷新后的缓存"，优先级高于编译期内置目录。
    pub fn overlay(&mut self, other: &Self) {
        for (provider, models) in &other.providers {
            let target = self.providers.entry(provider.clone()).or_default();
            for model in models {
                match target.iter().position(|existing| existing.id == model.id) {
                    Some(index) => target[index] = model.clone(),
                    None => target.push(model.clone()),
                }
            }
        }
    }

    /// provider 数量。
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// 内置目录里的 provider id（升序）。`/login` 在没有 `models.json` 时用它列候选。
    #[must_use]
    pub fn provider_ids(&self) -> Vec<&str> {
        self.providers.keys().map(String::as_str).collect()
    }

    /// 某 provider 的第一个内置模型 id（供 `/login` 成功后选默认模型）。
    /// 遍历全部 `(provider, model)`（顺序：provider 名字典序、模型按目录顺序）。
    ///
    /// `--list-models` 要把内置目录与 `models.json` 并起来列出来（ref `cli/list-models.ts`
    /// 取的是运行时 registry），这里给出只读迭代器。
    pub fn entries(&self) -> impl Iterator<Item = (&str, &BuiltInModel)> {
        self.providers.iter().flat_map(|(provider, models)| {
            models.iter().map(move |model| (provider.as_str(), model))
        })
    }

    #[must_use]
    pub fn first_model(&self, provider: &str) -> Option<&str> {
        self.providers
            .get(provider)?
            .first()
            .map(|model| model.id.as_str())
    }

    /// 内置模型总数。
    pub fn total_models(&self) -> usize {
        self.providers.values().map(Vec::len).sum()
    }

    /// 查一个内置模型。
    pub fn find(&self, provider: &str, model_id: &str) -> Option<&BuiltInModel> {
        self.providers
            .get(provider)?
            .iter()
            .find(|model| model.id == model_id)
    }

    /// 照 ref `provider-composer.ts:198-205` 的 `findModelDefaults` 取"继承用"的默认模型：
    /// 1. 同 id；2. 同 `api`；3. `openai-completions`；4. **该 provider 的第一个模型**。
    ///
    /// 用途：`models.json` 只写 `models`（不写 `baseUrl`/`api`）时，ref 会从内置目录继承
    /// （`baseUrl = definition.baseUrl ?? providerConfig.baseUrl ?? defaults?.baseUrl`，`:171-172`）。
    #[must_use]
    pub fn defaults_for(
        &self,
        provider: &str,
        model_id: &str,
        api: Option<&str>,
    ) -> Option<&BuiltInModel> {
        let models = self.providers.get(provider)?;
        if let Some(found) = models.iter().find(|model| model.id == model_id) {
            return Some(found);
        }
        if let Some(api) = api {
            if let Some(found) = models.iter().find(|model| model.api == api) {
                return Some(found);
            }
        }
        if let Some(found) = models
            .iter()
            .find(|model| model.api == "openai-completions")
        {
            return Some(found);
        }
        models.first()
    }

    /// 某 provider 某模型的最大输出 token；内置未给时用 TS 的缺省值。
    pub fn max_tokens(&self, provider: &str, model_id: &str) -> u64 {
        self.find(provider, model_id)
            .and_then(|model| model.max_tokens)
            .unwrap_or(DEFAULT_MAX_TOKENS)
    }
}
