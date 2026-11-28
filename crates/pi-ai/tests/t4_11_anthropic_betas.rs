//! P4-13 步骤 4.2：`anthropic-beta` 头与触发条件。
//!
//! ref `api/anthropic-messages.ts`：
//! - `:181` `FINE_GRAINED_TOOL_STREAMING_BETA = "fine-grained-tool-streaming-2025-05-14"`
//! - `:182` `INTERLEAVED_THINKING_BETA = "interleaved-thinking-2025-05-14"`
//! - `:991-1033` `getBetaFeatures(model, context, isOAuthToken, nativeToolChanges, options)`
//! - `:1074` `...(betaFeatures.length > 0 ? { betas: betaFeatures } : {})`
//!   —— `betas` 由 SDK 转成请求头（golden/S12-anthropic 的请求体里**没有** `betas`，
//!   路径是 `/v1/v1/messages?beta=true`，说明走的是 SDK 的 beta 端点）。
//! - `:1019-1026` 交错思考的条件：`model.reasoning && options.thinkingEnabled === true &&
//!   (options.interleavedThinking ?? true) && compat.forceAdaptiveThinking !== true`
//! - `:1005-1014` 用户配了 `anthropic-beta` → 按 `,` 拆分去重后原样采用
use pi_ai::anthropic::{
    beta_features, build_request_headers, thinking_config_for_level, INTERLEAVED_THINKING_BETA,
};
use pi_ai::{Context, Message, ProviderRequest};

fn request(thinking: Option<pi_ai::anthropic::ThinkingConfig>) -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: Context::from_messages(
            Some("sys".to_string()),
            &[Message::User {
                content: pi_ai::UserContent::Text("hi".to_string()),
                timestamp: 0,
                extra: serde_json::Map::new(),
            }],
        ),
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking,
        headers: Vec::new(),
    }
}

#[test]
fn reasoning_model_with_thinking_enabled_asks_for_interleaved_thinking() {
    // golden 的场景：推理模型 + 等级 medium → thinking Enabled → 该 beta 触发。
    let thinking = thinking_config_for_level(Some("medium"), None, 4096);
    assert!(
        matches!(
            thinking,
            Some(pi_ai::anthropic::ThinkingConfig::Enabled { .. })
        ),
        "样本自检：等级 medium 应给出 Enabled"
    );
    let features = beta_features(&request(thinking), &[]);
    assert_eq!(features, vec![INTERLEAVED_THINKING_BETA.to_string()]);
}

#[test]
fn no_thinking_means_no_beta_feature() {
    // 负向：非推理模型（`reasoning_effort` 为 None）→ 不发 thinking，也不发 beta。
    let features = beta_features(&request(None), &[]);
    assert!(features.is_empty(), "{features:?}");
}

#[test]
fn level_off_means_no_beta_feature() {
    // 负向：等级 off → `thinkingEnabled === false`（ref `:1021` 只认 `=== true`）。
    let thinking = thinking_config_for_level(Some("off"), None, 4096);
    assert!(matches!(
        thinking,
        Some(pi_ai::anthropic::ThinkingConfig::Disabled)
    ));
    let features = beta_features(&request(thinking), &[]);
    assert!(features.is_empty(), "{features:?}");
}

#[test]
fn configured_anthropic_beta_header_wins() {
    // ref `:1005-1014`：用户配了该头 → 原样采用（拆分 + 去空白 + 去重）。
    let mut request = request(thinking_config_for_level(Some("medium"), None, 4096));
    request.headers.push((
        "anthropic-beta".to_string(),
        " my-beta ,other ,my-beta ".to_string(),
    ));
    let features = beta_features(&request, &[]);
    assert_eq!(features, vec!["my-beta".to_string(), "other".to_string()]);
}

#[test]
fn header_name_is_matched_case_insensitively() {
    let mut request = request(None);
    request
        .headers
        .push(("Anthropic-Beta".to_string(), "x".to_string()));
    assert_eq!(beta_features(&request, &[]), vec!["x".to_string()]);
}

#[test]
fn non_empty_features_become_the_anthropic_beta_header() {
    let features = vec![INTERLEAVED_THINKING_BETA.to_string()];
    let headers = build_request_headers(
        &[("x-api-key".to_string(), "k".to_string())],
        &[],
        &features,
    );
    assert!(
        headers.contains(&(
            "anthropic-beta".to_string(),
            INTERLEAVED_THINKING_BETA.to_string()
        )),
        "{headers:?}"
    );
    assert!(
        headers.contains(&("x-api-key".to_string(), "k".to_string())),
        "{headers:?}"
    );
}

#[test]
fn empty_features_do_not_add_the_header() {
    // ref `:1074`：空列表整段不发 → 不该出现 `anthropic-beta` 头。
    let headers = build_request_headers(&[], &[], &[]);
    assert!(
        !headers
            .iter()
            .any(|(name, _not_value)| name.eq_ignore_ascii_case("anthropic-beta")),
        "{headers:?}"
    );
}

#[test]
fn request_level_headers_override_client_headers() {
    let headers = build_request_headers(
        &[
            ("x-a".to_string(), "1".to_string()),
            ("x-b".to_string(), "1".to_string()),
        ],
        &[("x-b".to_string(), "2".to_string())],
        &[],
    );
    assert_eq!(
        headers,
        vec![
            ("x-a".to_string(), "1".to_string()),
            ("x-b".to_string(), "2".to_string())
        ]
    );
}
