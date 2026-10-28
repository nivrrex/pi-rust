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

    // F1-e：配置值先按 ref 的形态解析（字面值 / `$VAR` / `${VAR}` / `!command`）。
    // `Empty`（如 `$VAR` 未定义）与 `CommandFailed` 都视为**没有凭据**，
    // 由调用方决定"不发请求 + 报错"，而不是把 `$VAR` 原文当 Bearer 发出去。
    let resolve = |raw: String| -> Option<String> {
        match resolve_config_value(&raw, &|name| std::env::var(name).ok(), &run_config_command) {
            ConfigValue::Value(value) => Some(value),
            ConfigValue::Empty | ConfigValue::CommandFailed(_) => None,
        }
    };
    let take = |value: &Option<String>| -> Option<String> { take(value).and_then(resolve) };

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

/// 配置值解析的结果（ref `core/resolve-config-value.ts:145-176`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigValue {
    /// 解析出的值（字面值、环境变量替换后、或命令输出）。
    Value(String),
    /// `!命令` 形式但命令执行失败（ref `:174-176` 返回错误信息）。
    CommandFailed(String),
    /// 解析后为空（`$VAR` 未定义等）——调用方应视为"没有密钥"。
    Empty,
}

/// 解析一条配置值，口径与 ref `resolveConfigValue` 一致：
/// - `!cmd` → 交给 `run_command` 执行并取 stdout（去掉尾部换行）；
/// - `$VAR` / `${VAR}` → 环境变量替换（未定义替换成空串，与 ref 的模板行为一致）；
/// - 其余 → 字面值。
///
/// `env` 与 `run_command` 都注入，便于单测（不依赖真实环境/进程）。
#[must_use]
pub fn resolve_config_value(
    raw: &str,
    env: &dyn Fn(&str) -> Option<String>,
    run_command: &dyn Fn(&str) -> Result<String, String>,
) -> ConfigValue {
    let trimmed = raw.trim();
    if let Some(command) = trimmed.strip_prefix('!') {
        return match run_command(command.trim()) {
            Ok(output) => {
                let value = output.trim_end_matches(['\n', '\r']).to_string();
                if value.is_empty() {
                    ConfigValue::Empty
                } else {
                    ConfigValue::Value(value)
                }
            }
            Err(error) => ConfigValue::CommandFailed(error),
        };
    }
    // `$VAR` / `${VAR}`：ref `:60-77` 的模板解析（只支持变量名，不支持表达式）。
    let mut output = String::with_capacity(trimmed.len());
    let mut rest = trimmed;
    while let Some(index) = rest.find('$') {
        output.push_str(&rest[..index]);
        let after = &rest[index + 1..];
        if let Some(stripped) = after.strip_prefix('{') {
            if let Some(end) = stripped.find('}') {
                let name = &stripped[..end];
                output.push_str(&env(name).unwrap_or_default());
                rest = &stripped[end + 1..];
                continue;
            }
        }
        let name_len = after
            .char_indices()
            .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '_')
            .map(|(index, ch)| index + ch.len_utf8())
            .last()
            .unwrap_or(0);
        if name_len == 0 {
            output.push('$');
            rest = after;
            continue;
        }
        output.push_str(&env(&after[..name_len]).unwrap_or_default());
        rest = &after[name_len..];
    }
    output.push_str(rest);
    if output.trim().is_empty() {
        ConfigValue::Empty
    } else {
        ConfigValue::Value(output)
    }
}

/// `!command` 的执行器：Unix 走 `sh -c`，Windows 走 `cmd /C`
/// （ref `resolve-config-value.ts:153-176` 用"配置的 shell"，未配置时即系统默认 shell）。
fn run_config_command(command: &str) -> Result<String, String> {
    #[cfg(windows)]
    let mut child = std::process::Command::new("cmd");
    #[cfg(windows)]
    child.arg("/C").arg(command);
    #[cfg(not(windows))]
    let mut child = std::process::Command::new("sh");
    #[cfg(not(windows))]
    child.arg("-c").arg(command);
    let output = child.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("命令退出码 {}", output.status.code().unwrap_or(-1)));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

#[cfg(test)]
mod resolve_config_value_tests {
    use super::{resolve_config_value, ConfigValue};

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn no_command(_: &str) -> Result<String, String> {
        Err("未执行".to_string())
    }

    /// 字面值原样返回（ref `resolve-config-value.ts:145-151`）。
    #[test]
    fn literal_passes_through() {
        assert_eq!(
            resolve_config_value("sk-abc", &no_env, &no_command),
            ConfigValue::Value("sk-abc".to_string())
        );
    }

    /// `$VAR` / `${VAR}` 替换成环境变量值；未定义 ⇒ 空 ⇒ `Empty`（调用方视为无密钥）。
    #[test]
    fn env_templates_are_expanded() {
        let env = |name: &str| match name {
            "KEY" => Some("sk-from-env".to_string()),
            _ => None,
        };
        assert_eq!(
            resolve_config_value("$KEY", &env, &no_command),
            ConfigValue::Value("sk-from-env".to_string())
        );
        assert_eq!(
            resolve_config_value("${KEY}", &env, &no_command),
            ConfigValue::Value("sk-from-env".to_string())
        );
        assert_eq!(
            resolve_config_value("prefix-$KEY", &env, &no_command),
            ConfigValue::Value("prefix-sk-from-env".to_string())
        );
        assert_eq!(
            resolve_config_value("$MISSING", &env, &no_command),
            ConfigValue::Empty
        );
    }

    /// `!command` 走注入的执行器；失败要区分出来（ref `:174-176`）。
    #[test]
    fn command_form_runs_and_trims_newline() {
        let run = |command: &str| match command {
            "echo sk-cmd" => Ok("sk-cmd\n".to_string()),
            other => Err(format!("failed: {other}")),
        };
        assert_eq!(
            resolve_config_value("!echo sk-cmd", &no_env, &run),
            ConfigValue::Value("sk-cmd".to_string())
        );
        assert_eq!(
            resolve_config_value("!false", &no_env, &run),
            ConfigValue::CommandFailed("failed: false".to_string())
        );
    }
}
