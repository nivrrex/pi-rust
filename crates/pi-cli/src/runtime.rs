//! 运行期路径与 provider 构造（路径部分为纯逻辑，可单测）。
//!
//! 出处：`spec/f-config.md` 第 1 节、`spec/i-cli.md` 第 5 节。

use std::path::PathBuf;

/// 配置目录与会话目录。
///
/// - `PI_CODING_AGENT_DIR` 优先，否则 `<home>/.pi/agent`（`CONFIG_DIR_NAME` 缺省 `.pi`）。
///   〔`config.ts:504`、`config.ts:507-509`〕
/// - 会话目录：`--session-dir` 优先于 `PI_CODING_AGENT_SESSION_DIR`；都没有时回落到配置目录
///   下的 `sessions`。**待验证**：TS 侧"都没有"时的具体回落值未从源码确认，这里取配置目录
///   下的 `sessions` 作为工程判断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub agent_dir: PathBuf,
    pub session_dir: PathBuf,
}

/// 把 `~` / `~/x` 展开成 home 下的路径（其他位置出现 `~` 不展开）。
pub fn expand_tilde(path: &str, home: &str) -> PathBuf {
    if path == "~" {
        return PathBuf::from(home);
    }
    match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(home).join(rest),
        None => PathBuf::from(path),
    }
}

/// 用户主目录（跨平台）：`HOME` → `USERPROFILE`（Windows）→
/// `HOMEDRIVE`+`HOMEPATH`（老式 Windows）。ref 用 `os.homedir()`，
/// 在 Windows 上等价于 `USERPROFILE`。
///
/// 真机问题：Windows 上 `HOME` 通常**不存在**，之前只读 `HOME` 会让
/// `home` 变成空串 ⇒ 配置目录算成 `/.pi/agent`，表现为"没关联到标准目录"。
pub fn home_from(env: &dyn Fn(&str) -> Option<String>) -> String {
    fn take(value: Option<String>) -> Option<String> {
        value
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    }
    if let Some(home) = take(env("HOME")) {
        return home;
    }
    if let Some(profile) = take(env("USERPROFILE")) {
        return profile;
    }
    if let (Some(drive), Some(path)) = (take(env("HOMEDRIVE")), take(env("HOMEPATH"))) {
        return format!("{drive}{path}");
    }
    String::new()
}

/// 解析路径。`env` 抽象出来便于单测（生产实现读 `std::env::var`）。
pub fn resolve_paths(
    env: &dyn Fn(&str) -> Option<String>,
    home: &str,
    session_dir_flag: Option<&str>,
) -> Paths {
    let agent_dir = match env("PI_CODING_AGENT_DIR") {
        Some(value) if !value.trim().is_empty() => expand_tilde(value.trim(), home),
        Some(_) | None => PathBuf::from(home).join(".pi").join("agent"),
    };
    let session_dir = match session_dir_flag {
        Some(flag) if !flag.trim().is_empty() => expand_tilde(flag.trim(), home),
        Some(_) | None => match env("PI_CODING_AGENT_SESSION_DIR") {
            Some(value) if !value.trim().is_empty() => expand_tilde(value.trim(), home),
            Some(_) | None => agent_dir.join("sessions"),
        },
    };
    Paths {
        agent_dir,
        session_dir,
    }
}

/// 按 `models.json` 解析出的模型构造 provider。
///
/// 认证走 `resolve_credential`（stored → `--api-key` → models.json 的 key → env），
/// 顺序由该函数实现，这里只负责把它算出的头交给客户端。
///
/// **provider 名取自 `resolved.provider_name`**（P4-13 步骤 2）。此前这里用
/// `Client::new(base_url, rates)`，而两个 client 的 `new` 把 `provider` 写死成 `"mock"`
/// （`openai.rs:191` / `anthropic.rs:237`）→ opencode-go 的两个特例（发送侧
/// `openai.rs:278`、解码侧 `openai.rs:325`）在生产路径上**永不触发**。现在改走
/// `from_resolved`（构造器早就有，但此前只有测试用）。
pub fn build_provider(
    resolved: &pi_ai::config::ResolvedModel,
    api_key: Option<String>,
) -> Result<std::sync::Arc<dyn pi_ai::Provider>, String> {
    use pi_ai::credentials::{resolve_credential, CredentialSources, ProcessEnv};

    let api = resolved.api.as_str();
    let env_vars: Vec<String> = match api {
        "anthropic-messages" => vec!["ANTHROPIC_API_KEY".to_string()],
        "openai-completions" | "openai-responses" => vec!["OPENAI_API_KEY".to_string()],
        other => return Err(format!("本版本不支持 api\"{other}\"")),
    };
    let sources = CredentialSources {
        // ref `core/auth-storage.ts:52-70/253-265`：`<agentDir>/auth.json` 里
        // `{provider: {type:"api_key", key}}`。此前这里恒传 `None` ⇒ **auth.json 里的
        // 凭据永远不被使用**：openrouter 这类"key 只存在 auth.json"的 provider 于是
        // 发出不带 Authorization 的请求，服务端回 `401 No cookie auth credentials found`。
        stored: stored_api_key(&resolved.provider_name),
        cli_api_key: api_key,
        // models.json 的 `providers.<id>.apiKey`（ref `core/model-config.ts:232`）——
        // 此前调用方恒传 `None`，等于把该配置吞掉了。
        models_json_api_key: resolved.api_key.clone(),
        oauth: None,
        env_auth_token_vars: Vec::new(),
        env_api_key_vars: env_vars.clone(),
    };
    let credential = resolve_credential(&resolved.provider_name, &sources, &ProcessEnv);
    // F1-e：**没有任何可用凭据时不要发请求**。ref 在构造 provider 前就要求 resolved key
    // （`provider-composer.ts:304-312`），不会发出不带 Authorization 的请求让服务端回
    // `401 No cookie auth credentials found`。
    if credential.is_none() {
        return Err(format!(
            "Provider \"{}\" 没有可用的 API key：请在 auth.json、--api-key、环境变量（{}）\
             或 models.json 的 apiKey 中提供其一（支持 $VAR / !command 形式）",
            resolved.provider_name,
            env_vars.join(" / ")
        ));
    }
    // `authHeader: true` → `Authorization: Bearer <apiKey>`（ref `:237` + `provider-composer.ts:304-307`，
    // 没有 key 时 TS 抛 `authHeader requires a resolved API key`）。
    let mut extra_headers = resolved.headers.clone();
    if resolved.auth_header {
        let key = credential
            .as_ref()
            .map(|found| found.value.clone())
            .ok_or_else(|| {
                format!(
                    "Provider \"{}\": authHeader requires a resolved API key",
                    resolved.provider_name
                )
            })?;
        extra_headers.retain(|(name, _not_value)| !name.eq_ignore_ascii_case("authorization"));
        extra_headers.push(("Authorization".to_string(), format!("Bearer {key}")));
    }

    match api {
        "anthropic-messages" => {
            let client = pi_ai::anthropic::AnthropicMessagesClient::from_resolved(
                resolved,
                pi_ai::anthropic::AuthHeader::default(),
                "",
            );
            let client = match credential {
                // 默认形态就是 `x-api-key`（`AuthHeader` 的 `#[default]`）。
                Some(found) => client.with_api_key(&found.value),
                None => client,
            };
            Ok(std::sync::Arc::new(client.with_headers(extra_headers)))
        }
        "openai-responses" => {
            // ref `api/openai-responses.ts:113-200`：同一套 `authHeader` 语义
            // （`Authorization: Bearer`），端点是 `{baseUrl}/responses`。
            let client = pi_ai::responses::ResponsesClient::from_resolved(resolved);
            let client = match credential {
                Some(found) => client.with_auth(pi_ai::openai::AuthHeader::Bearer, &found.value),
                None => client,
            };
            Ok(std::sync::Arc::new(client.with_headers(extra_headers)))
        }
        "openai-completions" => {
            let client = pi_ai::openai::ChatCompletionsClient::from_resolved(resolved);
            let client = match credential {
                Some(found) => client.with_auth(pi_ai::openai::AuthHeader::Bearer, &found.value),
                None => client,
            };
            Ok(std::sync::Arc::new(client.with_headers(extra_headers)))
        }
        other => Err(format!("本版本不支持 api\"{other}\"")),
    }
}

/// 从 `<agentDir>/auth.json` 里取某个 provider 的 api key（ref `auth-storage.ts:253-265`）。
///
/// 只处理 `type: "api_key"`；`key` 支持 `$VAR` / `${VAR}` 展开（ref `resolveConfigValue`），
/// `!command` 形态**不支持**（会执行外部命令，按 `.md` 的"不新增外部执行"约定跳过并返回 None）。
#[must_use]
pub fn auth_json_api_key(text: &str, provider: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    let entry = parsed.get(provider)?;
    if entry.get("type").and_then(|value| value.as_str()) != Some("api_key") {
        return None;
    }
    let key = entry.get("key").and_then(|value| value.as_str())?.trim();
    if key.is_empty() || key.starts_with('!') {
        return None;
    }
    Some(expand_config_value(key))
}

/// `$VAR` / `${VAR}` 展开（未定义时原样保留，和 ref 的 `resolveConfigValue` 一致）。
fn expand_config_value(value: &str) -> String {
    if let Some(name) = value
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    {
        return std::env::var(name).unwrap_or_else(|_| value.to_string());
    }
    if let Some(name) = value.strip_prefix('$') {
        return std::env::var(name).unwrap_or_else(|_| value.to_string());
    }
    value.to_string()
}

/// 读配置目录下的 `auth.json`（读不到就返回 `None`，不影响其它凭据来源）。
fn stored_api_key(provider: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let paths = resolve_paths(&|name| std::env::var(name).ok(), &home, None);
    let text = std::fs::read_to_string(paths.agent_dir.join("auth.json")).ok()?;
    auth_json_api_key(&text, provider)
}

/// 加载内置模型目录（P7-6am/M 的实现，P7-7f 从 `main.rs` 挪来以便 TUI 侧也能用）。
///
/// ref 的目录存在 `<agentDir>/models-store.json`（`core/models-store.ts:46-60`，形状为
/// `{provider: {models:[…], checkedAt, etag}}`）；也兼容 oracle 导出的 `{provider:[…]}`。
/// 缺失或解析失败都**不致命**：退回"无目录"。
#[must_use]
pub fn load_builtin_catalog(agent_dir: &std::path::Path) -> Option<pi_ai::config::BuiltInCatalog> {
    for name in ["models-store.json", "pi-official-catalog.json"] {
        let path = agent_dir.join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match pi_ai::config::BuiltInCatalog::from_json(&text) {
            Ok(catalog) => return Some(catalog),
            Err(_) => continue,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| {
            owned
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn defaults_to_dot_pi_agent_under_home() {
        let paths = resolve_paths(&env_from(&[]), "/home/u", None);
        assert_eq!(paths.agent_dir, PathBuf::from("/home/u/.pi/agent"));
        assert_eq!(
            paths.session_dir,
            PathBuf::from("/home/u/.pi/agent/sessions")
        );
    }

    /// 主目录解析（B-e）：Windows 上 `HOME` 通常不存在，必须回落到 `USERPROFILE`，
    /// 否则配置目录会算成 `/.pi/agent`（真机表现为"没关联到标准目录"）。
    #[test]
    fn home_prefers_home_then_userprofile() {
        assert_eq!(
            super::home_from(&env_from(&[
                ("HOME", "/home/u"),
                ("USERPROFILE", r"C:\\Users\\u")
            ])),
            "/home/u"
        );
        assert_eq!(
            super::home_from(&env_from(&[("USERPROFILE", r"C:\\Users\\u")])),
            r"C:\\Users\\u"
        );
        assert_eq!(
            super::home_from(&env_from(&[
                ("HOMEDRIVE", "C:"),
                ("HOMEPATH", r"\\Users\\u")
            ])),
            r"C:\\Users\\u"
        );
        assert_eq!(super::home_from(&env_from(&[("HOME", "   ")])), "");
    }

    /// Windows 形态的路径拼接：`USERPROFILE` 下拼 `.pi/agent`，且 session 目录在其下。
    #[test]
    fn windows_home_builds_standard_agent_dir() {
        let env = env_from(&[("USERPROFILE", r"C:\\Users\\Admin")]);
        let home = super::home_from(&env);
        let paths = resolve_paths(&env, &home, None);
        assert_eq!(
            paths.agent_dir,
            PathBuf::from(r"C:\\Users\\Admin").join(".pi").join("agent")
        );
        assert_eq!(paths.session_dir, paths.agent_dir.join("sessions"));
    }

    #[test]
    fn env_var_overrides_the_agent_dir_and_expands_tilde() {
        let paths = resolve_paths(
            &env_from(&[("PI_CODING_AGENT_DIR", "~/cfg")]),
            "/home/u",
            None,
        );
        assert_eq!(paths.agent_dir, PathBuf::from("/home/u/cfg"));
    }

    #[test]
    fn session_dir_flag_beats_the_env_var() {
        let env = env_from(&[
            ("PI_CODING_AGENT_DIR", "/a"),
            ("PI_CODING_AGENT_SESSION_DIR", "/from-env"),
        ]);
        let paths = resolve_paths(&env, "/home/u", Some("/from-flag"));
        assert_eq!(paths.session_dir, PathBuf::from("/from-flag"));
        let without_flag = resolve_paths(&env, "/home/u", None);
        assert_eq!(without_flag.session_dir, PathBuf::from("/from-env"));
    }

    #[test]
    fn blank_values_are_treated_as_missing() {
        let paths = resolve_paths(
            &env_from(&[
                ("PI_CODING_AGENT_DIR", "   "),
                ("PI_CODING_AGENT_SESSION_DIR", ""),
            ]),
            "/home/u",
            None,
        );
        assert_eq!(paths.agent_dir, PathBuf::from("/home/u/.pi/agent"));
        assert_eq!(
            paths.session_dir,
            PathBuf::from("/home/u/.pi/agent/sessions")
        );
    }
}

#[cfg(test)]
mod p79_auth_tests {
    use super::auth_json_api_key;

    /// ref `core/auth-storage.ts:253-265`：`{provider: {type:"api_key", key}}`。
    #[test]
    fn reads_api_key_entries() {
        let text = r#"{"openrouter":{"type":"api_key","key":"sk-or-123"}}"#;
        assert_eq!(
            auth_json_api_key(text, "openrouter"),
            Some("sk-or-123".to_string())
        );
    }

    /// `$VAR` / `${VAR}` 展开（ref `resolveConfigValue`）；变量未定义时原样返回。
    #[test]
    fn expands_env_style_keys() {
        let text = r#"{"openrouter":{"type":"api_key","key":"$PI_TEST_AUTH_KEY"}}"#;
        assert_eq!(
            auth_json_api_key(text, "openrouter"),
            Some("$PI_TEST_AUTH_KEY".to_string()),
            "未定义的变量应原样保留"
        );
        let text = r#"{"openrouter":{"type":"api_key","key":"${PI_TEST_AUTH_KEY}"}}"#;
        assert_eq!(
            auth_json_api_key(text, "openrouter"),
            Some("${PI_TEST_AUTH_KEY}".to_string())
        );
    }

    /// 只认 `api_key`；`oauth` / `!command` / 其它 provider / 空 key 都要返回 None。
    #[test]
    fn ignores_other_credential_forms() {
        let oauth = r#"{"openrouter":{"type":"oauth","access":"a","refresh":"r","expires":1}}"#;
        assert_eq!(auth_json_api_key(oauth, "openrouter"), None);
        let command = r#"{"openrouter":{"type":"api_key","key":"!op read key"}}"#;
        assert_eq!(auth_json_api_key(command, "openrouter"), None);
        let empty = r#"{"openrouter":{"type":"api_key","key":"  "}}"#;
        assert_eq!(auth_json_api_key(empty, "openrouter"), None);
        assert_eq!(
            auth_json_api_key(r#"{"other":{"type":"api_key","key":"k"}}"#, "openrouter"),
            None
        );
        assert_eq!(auth_json_api_key("not json", "openrouter"), None);
    }
}
