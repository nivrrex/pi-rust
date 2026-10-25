//! T2.12：凭据解析（对齐 TS 的四类来源与顺序）。
//!
//! 依据 `ref/pi-0.87.1/packages/ai/src/providers/anthropic.ts:19-39` 与
//! `packages/ai/src/providers/openai.ts:11`。

use std::collections::BTreeMap;

use pi_ai::credentials::{resolve_credential, AuthSource, CredentialSources, EnvReader};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

struct MapEnv(BTreeMap<String, String>);

impl EnvReader for MapEnv {
    fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

#[test]
fn stored_credential_wins_over_env() {
    let sources = CredentialSources {
        stored: Some("from-auth-json".to_string()),
        cli_api_key: None,
        models_json_api_key: None,
        oauth: None,
        env_auth_token_vars: vec!["ANTHROPIC_AUTH_TOKEN".to_string()],
        env_api_key_vars: vec!["ANTHROPIC_API_KEY".to_string()],
    };
    let env = MapEnv(env(&[
        ("ANTHROPIC_AUTH_TOKEN", "tok"),
        ("ANTHROPIC_API_KEY", "k"),
    ]));
    let resolved = resolve_credential("anthropic", &sources, &env).expect("应能解析");
    assert_eq!(resolved.value, "from-auth-json");
    assert_eq!(resolved.source, AuthSource::StoredCredential);
}

/// **注意**：stored 与 `--api-key` 的相对优先级在 TS 里**未验证**——
/// `providers/anthropic.ts:20-22` 只证明 stored 排在环境变量之前，
/// 两者谁先要追 CLI 到 resolver 的传参才能定。本实现按 stored → cli 取，属工程判断。
#[test]
fn cli_flag_beats_env_and_auth_token() {
    let sources = CredentialSources {
        stored: Some("stored".to_string()),
        cli_api_key: Some("from-cli".to_string()),
        models_json_api_key: None,
        oauth: None,
        env_auth_token_vars: vec!["ANTHROPIC_AUTH_TOKEN".to_string()],
        env_api_key_vars: vec!["ANTHROPIC_API_KEY".to_string()],
    };
    let env = MapEnv(env(&[("ANTHROPIC_AUTH_TOKEN", "tok")]));
    let resolved = resolve_credential("anthropic", &sources, &env).expect("应能解析");
    // stored 同时存在时本实现取 stored（未验证的工程判断），断言的是"绝不取环境变量"。
    assert_ne!(resolved.source, AuthSource::EnvAuthToken);
    assert_ne!(resolved.value, "tok");
}

#[test]
fn auth_token_env_is_bearer_and_precedes_api_key_env() {
    let sources = CredentialSources {
        stored: None,
        cli_api_key: None,
        models_json_api_key: None,
        oauth: None,
        env_auth_token_vars: vec!["ANTHROPIC_AUTH_TOKEN".to_string()],
        env_api_key_vars: vec![
            "ANTHROPIC_OAUTH_TOKEN".to_string(),
            "ANTHROPIC_API_KEY".to_string(),
        ],
    };
    let env = MapEnv(env(&[
        ("ANTHROPIC_AUTH_TOKEN", "tok"),
        ("ANTHROPIC_OAUTH_TOKEN", "oauth"),
        ("ANTHROPIC_API_KEY", "k"),
    ]));
    let resolved = resolve_credential("anthropic", &sources, &env).expect("应能解析");
    assert_eq!(resolved.source, AuthSource::EnvAuthToken);
    assert_eq!(resolved.value, "tok");
    assert_eq!(
        resolved.header_hint(),
        Some(("authorization", "Bearer tok".to_string())),
        "auth token 走 Bearer，与 TS 的 headers 形态一致"
    );
}

#[test]
fn models_json_api_key_is_used_when_nothing_else_is_available() {
    let sources = CredentialSources {
        stored: None,
        cli_api_key: None,
        models_json_api_key: Some("from-models-json".to_string()),
        oauth: None,
        env_auth_token_vars: vec![],
        env_api_key_vars: vec!["OPENAI_API_KEY".to_string()],
    };
    let env = MapEnv(env(&[]));
    let resolved = resolve_credential("openai", &sources, &env).expect("应能解析");
    assert_eq!(resolved.source, AuthSource::ModelsJson);
    assert_eq!(resolved.value, "from-models-json");
}

#[test]
fn oauth_credential_is_recognised_but_not_an_api_key() {
    let sources = CredentialSources {
        stored: None,
        cli_api_key: None,
        models_json_api_key: None,
        oauth: Some("radius".to_string()),
        env_auth_token_vars: vec![],
        env_api_key_vars: vec![],
    };
    let env = MapEnv(env(&[]));
    assert!(
        resolve_credential("p", &sources, &env).is_none(),
        "oauth 不是 api key，交由 oauth 流程处理"
    );
}

#[test]
fn nothing_available_yields_none_rather_than_empty_key() {
    let sources = CredentialSources {
        stored: None,
        cli_api_key: None,
        models_json_api_key: None,
        oauth: None,
        env_auth_token_vars: vec!["ANTHROPIC_AUTH_TOKEN".to_string()],
        env_api_key_vars: vec!["ANTHROPIC_API_KEY".to_string()],
    };
    let env = MapEnv(env(&[]));
    assert!(resolve_credential("anthropic", &sources, &env).is_none());
}

#[test]
fn empty_env_value_does_not_count_as_a_credential() {
    let sources = CredentialSources {
        stored: None,
        cli_api_key: None,
        models_json_api_key: None,
        oauth: None,
        env_auth_token_vars: vec![],
        env_api_key_vars: vec!["OPENAI_API_KEY".to_string()],
    };
    let env = MapEnv(env(&[("OPENAI_API_KEY", "   ")]));
    assert!(
        resolve_credential("openai", &sources, &env).is_none(),
        "空白值不算凭据，否则会发出空 Authorization"
    );
}
