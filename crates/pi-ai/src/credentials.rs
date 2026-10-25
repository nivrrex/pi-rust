//! 凭据解析，对齐 `ref/pi-0.87.1/packages/ai/src/providers/anthropic.ts:19-39`
//! 与 `packages/ai/src/providers/openai.ts:11` 的解析顺序。
//!
//! **头名不在本文件决定**：TS 把 apiKey 交给厂商 SDK，由 SDK 设置具体的头
//! （OpenAI 系是 `Authorization: Bearer`，Anthropic 系是 `x-api-key`）。
//! 这里是 SDK 约定，不在 pi 源码里，本模块只把"哪种形态"标出来。

/// 读取环境变量的抽象；测试用 map 替身，生产用 `std::env`。
pub trait EnvReader {
    fn get(&self, name: &str) -> Option<String>;
}

/// 读取进程环境变量。
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnv;

impl EnvReader for ProcessEnv {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// 凭据来源，按 TS 的解析顺序排列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    /// 已存凭据（auth.json）。
    StoredCredential,
    /// 命令行 `--api-key`。
    CliApiKey,
    /// 环境变量里的 auth token（转成 `Authorization: Bearer`）。
    EnvAuthToken,
    /// 环境变量里的 api key。
    EnvApiKey,
    /// models.json 的 `provider.apiKey`。
    ModelsJson,
}

/// 解析凭据时的候选来源。
#[derive(Debug, Clone, Default)]
pub struct CredentialSources {
    pub stored: Option<String>,
    pub cli_api_key: Option<String>,
    pub models_json_api_key: Option<String>,
    /// `provider.oauth`；本版本只支持 `"radius"`，不是 api key。
    pub oauth: Option<String>,
    /// 按顺序试的环境变量名（auth token 形态）。
    pub env_auth_token_vars: Vec<String>,
    /// 按顺序试的环境变量名（api key 形态）。
    pub env_api_key_vars: Vec<String>,
}

/// 解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCredential {
    pub source: AuthSource,
    /// 凭据原文。
    pub value: String,
    /// 来源说明，便于排障时对齐 TS 的 `source` 字段。
    pub detail: String,
}

impl ResolvedCredential {
    /// 该凭据应使用的请求头；`None` 表示按 api key 形态交给 SDK。
    ///
    /// 只有 auth token 形态在 pi 侧显式组头（与 TS 的 `headers: { Authorization: Bearer … }` 一致）。
    pub fn header_hint(&self) -> Option<(&'static str, String)> {
        match self.source {
            AuthSource::EnvAuthToken => Some(("authorization", format!("Bearer {}", self.value))),
            AuthSource::StoredCredential
            | AuthSource::CliApiKey
            | AuthSource::EnvApiKey
            | AuthSource::ModelsJson => None,
        }
    }
}

/// 按 TS 的顺序解析凭据：`oauth` 不参与，空白值视为缺失。
///
/// 顺序依据 `providers/anthropic.ts:19-39`：stored 排在环境变量之前，
/// auth token 排在 api key 之前。**stored 与 `--api-key` 的相对次序未验证**
/// （TS 的 resolver 只看到 `credential.key`），本实现取 stored 优先，属工程判断。
pub fn resolve_credential(
    provider: &str,
    sources: &CredentialSources,
    env: &dyn EnvReader,
) -> Option<ResolvedCredential> {
    /// 空白值视为缺失：否则会发出空的认证头。
    fn take(value: &Option<String>) -> Option<String> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    }

    if let Some(value) = take(&sources.stored) {
        return Some(ResolvedCredential {
            source: AuthSource::StoredCredential,
            value,
            detail: "stored credential".to_string(),
        });
    }
    if let Some(value) = take(&sources.cli_api_key) {
        return Some(ResolvedCredential {
            source: AuthSource::CliApiKey,
            value,
            detail: "--api-key".to_string(),
        });
    }
    for name in &sources.env_auth_token_vars {
        if let Some(value) = env.get(name).and_then(|v| take(&Some(v))) {
            return Some(ResolvedCredential {
                source: AuthSource::EnvAuthToken,
                value,
                detail: name.clone(),
            });
        }
    }
    for name in &sources.env_api_key_vars {
        if let Some(value) = env.get(name).and_then(|v| take(&Some(v))) {
            return Some(ResolvedCredential {
                source: AuthSource::EnvApiKey,
                value,
                detail: name.clone(),
            });
        }
    }
    if let Some(value) = take(&sources.models_json_api_key) {
        return Some(ResolvedCredential {
            source: AuthSource::ModelsJson,
            value,
            detail: format!("models.json 的 {provider}.apiKey"),
        });
    }
    None
}
