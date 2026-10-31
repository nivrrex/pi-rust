//! `/login` 的 provider 列表数据（ref `modes/interactive/interactive-mode.ts:5629-5663`
//! `getLoginProviderOptions()`：遍历运行时 provider，凡有 `auth.apiKey` 收 api_key 型、
//! 有 `auth.oauth` 收 oauth 型，最后 `sort((a,b)=>a.name.localeCompare(b.name))`）。
//!
//! pi-rs 的 provider 来自 `models.json`（与 ref 的"运行时 provider"同义），显示名取
//! ref `packages/ai/src/providers/*.ts` 里 **provider 级** `name`（不是 auth 方法名）。

/// ref 中带 `auth.oauth` 的 provider id（`packages/ai/src/providers/` 内含 `oauth:` 的文件）。
/// 这些 provider 在本版本**只提示暂不支持**（OAuth 后置，见 LIMITATIONS.md）。
pub const OAUTH_CAPABLE: &[&str] = &[
    "anthropic",         // providers/anthropic.ts:46-51
    "github-copilot",    // providers/github-copilot.ts:12-16
    "kimi-coding",       // providers/kimi-coding.ts:10-15
    "meta",              // providers/meta.ts:10-15
    "openai-codex",      // providers/openai-codex.ts:10-14
    "openrouter",        // providers/openrouter.ts:11-16
    "openrouter-images", // providers/openrouter-images.ts
    "radius",            // providers/radius.ts
    "xai",               // providers/xai.ts:10-15
];

/// provider 级显示名（ref 同名列，见各文件行号注释）；未列出的回退为 provider id。
const DISPLAY_NAMES: &[(&str, &str)] = &[
    ("anthropic", "Anthropic"),           // providers/anthropic.ts:46
    ("github-copilot", "GitHub Copilot"), // providers/github-copilot.ts:12
    ("google", "Google"),                 // providers/google.ts:9
    ("kimi-coding", "Kimi For Coding"),   // providers/kimi-coding.ts:10
    ("meta", "Meta"),                     // providers/meta.ts:10
    ("openai", "OpenAI"),                 // providers/openai.ts:9
    ("openai-codex", "OpenAI Codex"),     // providers/openai-codex.ts:10
    ("openrouter", "OpenRouter"),         // providers/openrouter.ts:11
    ("xai", "xAI"),                       // providers/xai.ts:10
];

/// 认证类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthType {
    ApiKey,
    Oauth,
}

impl AuthType {
    /// ref `AuthSelectorProvider.authType` 的字面值。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AuthType::ApiKey => "api_key",
            AuthType::Oauth => "oauth",
        }
    }
}

/// 列表项：`id` / 显示名 / 认证类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOption {
    pub id: String,
    pub name: String,
    pub auth_type: AuthType,
}

/// 给定已配置的 provider id 列表，产出 `/login` 列表（按显示名排序，同 ref 的 localeCompare
/// ——这里用不区分大小写的字典序，ASCII 范围内等价）。
#[must_use]
pub fn login_options(provider_ids: &[String]) -> Vec<LoginOption> {
    let mut options: Vec<LoginOption> = provider_ids
        .iter()
        .flat_map(|id| {
            let name = display_name(id).to_string();
            let mut items = vec![LoginOption {
                id: id.clone(),
                name: name.clone(),
                auth_type: AuthType::ApiKey,
            }];
            if OAUTH_CAPABLE.contains(&id.as_str()) {
                // ref 对同一 provider 先推 oauth、再推 api_key（:5641-5661）
                items.insert(
                    0,
                    LoginOption {
                        id: id.clone(),
                        name,
                        auth_type: AuthType::Oauth,
                    },
                );
            }
            items
        })
        .collect();
    options.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    options
}

/// `/login` 的候选 provider：`models.json` 里配置过的 + 内置目录里的，去重后排序。
///
/// ref 的 `getLoginProviderOptions()` 遍历的是**运行时 provider registry**（含内置），
/// 因此"零配置"的新机器也能 `/login`——这正是 pi-rs 之前缺失的一环。
#[must_use]
pub fn merged_provider_ids(configured: &[String], catalog_ids: &[&str]) -> Vec<String> {
    let mut ids: Vec<String> = configured.to_vec();
    for id in catalog_ids {
        if !ids.iter().any(|known| known == id) {
            ids.push((*id).to_string());
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

/// provider 显示名（未登记的回退为 id 本身，与 ref `?? providerId` 一致）。
#[must_use]
pub fn display_name(id: &str) -> &str {
    DISPLAY_NAMES
        .iter()
        .find(|(key, _)| *key == id)
        .map(|(_, name)| *name)
        .unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_follow_ref() {
        assert_eq!(display_name("anthropic"), "Anthropic");
        assert_eq!(display_name("openai"), "OpenAI");
        assert_eq!(display_name("xai"), "xAI");
        assert_eq!(display_name("kimi-coding"), "Kimi For Coding");
        assert_eq!(display_name("my-custom"), "my-custom", "未登记回退为 id");
    }

    #[test]
    fn ordering_is_by_name_case_insensitive() {
        // 只用非 OAuth provider，避免同一 provider 出现两条（ref 对 oauth+api_key 会各推一条）
        let ids = vec![
            "openai".to_string(),
            "google".to_string(),
            "my-custom".to_string(),
        ];
        let names: Vec<_> = login_options(&ids).into_iter().map(|o| o.name).collect();
        assert_eq!(names, ["Google", "my-custom", "OpenAI"]);
    }

    #[test]
    fn oauth_capable_provider_lists_oauth_first() {
        let ids = vec!["xai".to_string()];
        let options = login_options(&ids);
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].auth_type, AuthType::Oauth);
        assert_eq!(options[1].auth_type, AuthType::ApiKey);
        assert_eq!(options[0].as_str_pair(), ("xai", "xAI"));
    }

    impl LoginOption {
        fn as_str_pair(&self) -> (&str, &str) {
            (&self.id, &self.name)
        }
    }

    #[test]
    fn plain_provider_is_api_key_only() {
        let ids = vec!["google".to_string(), "my-proxy".to_string()];
        let options = login_options(&ids);
        assert!(options.iter().all(|o| o.auth_type == AuthType::ApiKey));
        assert_eq!(options.len(), 2);
    }

    #[test]
    fn merged_ids_include_builtin_and_keep_order() {
        let configured = vec!["zeta".to_string(), "anthropic".to_string()];
        let builtin = ["openai", "anthropic", "google"];
        let merged = merged_provider_ids(&configured, &builtin);
        assert_eq!(merged, ["anthropic", "google", "openai", "zeta"]);
        assert_eq!(
            merged.iter().filter(|id| *id == "anthropic").count(),
            1,
            "重复 id 只保留一份"
        );
    }

    #[test]
    fn auth_type_literals_match_ref() {
        assert_eq!(AuthType::ApiKey.as_str(), "api_key");
        assert_eq!(AuthType::Oauth.as_str(), "oauth");
    }
}
