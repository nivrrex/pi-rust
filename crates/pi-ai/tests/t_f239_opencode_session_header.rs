//! `opencode` / `opencode-go` 必须带 `x-opencode-session`（F239）。
//!
//! ref `ai/src/providers/opencode-headers.ts:11-31` 给这两个 provider 的**所有**
//! adapter 套一层 `withOpenCodeSessionHeader`：没配 `sessionId` 就不加；已有同名头
//! 就不覆盖（`hasHeader` 大小写不敏感）。
//!
//! 真机症状：opencode-go 的请求被服务端拒成
//! `400 Request is missing x-opencode-session and cannot be routed efficiently`。

use pi_ai::provider::{needs_opencode_session_header, opencode_session_header};

#[test]
fn only_opencode_providers_need_the_header() {
    assert!(needs_opencode_session_header("opencode"));
    assert!(needs_opencode_session_header("opencode-go"));
    for other in ["command-code", "openrouter", "anthropic", "ant-ling", ""] {
        assert!(
            !needs_opencode_session_header(other),
            "{other} 不应要求该头"
        );
    }
}

#[test]
fn header_carries_the_session_id() {
    let header = opencode_session_header("opencode-go", Some("sess-1"), &[]);
    assert_eq!(
        header,
        Some(("x-opencode-session".to_string(), "sess-1".to_string()))
    );
}

#[test]
fn no_session_id_means_no_header() {
    assert_eq!(opencode_session_header("opencode-go", None, &[]), None);
}

/// 用户在 `models.json` 里配了同名头 ⇒ 以配置为准，不覆盖（ref `hasHeader`）。
#[test]
fn configured_header_wins_and_comparison_is_case_insensitive() {
    for name in ["x-opencode-session", "X-OpenCode-Session"] {
        let configured = vec![(name.to_string(), "from-config".to_string())];
        assert_eq!(
            opencode_session_header("opencode-go", Some("sess-1"), &configured),
            None,
            "{name} 已配置时不应再追加"
        );
    }
}

/// 非 opencode provider 即使有 session id 也不加（否则会污染其它服务的请求）。
#[test]
fn other_providers_never_get_it() {
    assert_eq!(
        opencode_session_header("command-code", Some("sess-1"), &[]),
        None
    );
}
