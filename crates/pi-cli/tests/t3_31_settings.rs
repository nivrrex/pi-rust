//! P3.5：默认模型解析顺序（`model-resolver.ts:640-712` 的子集）。

use pi_cli::settings::{parse, resolve_model, Settings};

#[test]
fn cli_flags_win_over_settings() {
    let settings = Settings {
        default_thinking_level: None,
        theme: None,
        model_thinking_levels: None,
        enabled_models: None,
        show_hardware_cursor: None,
        default_provider: Some("from-settings".to_string()),
        default_model: Some("settings-model".to_string()),
        compaction: None,
    };
    let resolved = resolve_model(Some("cli-p"), Some("cli-m"), &settings).expect("可解析");
    assert_eq!(resolved, ("cli-p".to_string(), "cli-m".to_string()));
}

#[test]
fn settings_are_used_when_the_cli_is_silent() {
    let settings = Settings {
        default_thinking_level: None,
        theme: None,
        model_thinking_levels: None,
        enabled_models: None,
        show_hardware_cursor: None,
        default_provider: Some("p".to_string()),
        default_model: Some("m".to_string()),
        compaction: None,
    };
    assert_eq!(
        resolve_model(None, None, &settings).expect("可解析"),
        ("p".to_string(), "m".to_string())
    );
}

#[test]
fn a_lone_provider_borrows_the_settings_model_and_vice_versa() {
    let settings = Settings {
        default_thinking_level: None,
        theme: None,
        model_thinking_levels: None,
        enabled_models: None,
        show_hardware_cursor: None,
        default_provider: Some("p".to_string()),
        default_model: Some("m".to_string()),
        compaction: None,
    };
    assert_eq!(
        resolve_model(Some("cli-p"), None, &settings).expect("可解析"),
        ("cli-p".to_string(), "m".to_string())
    );
    assert_eq!(
        resolve_model(None, Some("cli-m"), &settings).expect("可解析"),
        ("p".to_string(), "cli-m".to_string())
    );
}

#[test]
fn half_a_pair_is_an_error_naming_what_is_missing() {
    let empty = Settings::default();
    let error = resolve_model(None, Some("m"), &empty).expect_err("缺 provider 应报错");
    assert!(error.contains("defaultProvider"), "实际：{error}");

    let error = resolve_model(Some("p"), None, &empty).expect_err("缺模型应报错");
    assert!(error.contains("defaultModel"), "实际：{error}");
}

#[test]
fn settings_must_give_both_fields() {
    let only_provider = Settings {
        default_thinking_level: None,
        theme: None,
        model_thinking_levels: None,
        enabled_models: None,
        show_hardware_cursor: None,
        default_provider: Some("p".to_string()),
        default_model: None,
        compaction: None,
    };
    let error = resolve_model(None, None, &only_provider).expect_err("应报错");
    assert!(error.contains("缺 defaultModel"), "实际：{error}");

    let only_model = Settings {
        default_thinking_level: None,
        theme: None,
        model_thinking_levels: None,
        enabled_models: None,
        show_hardware_cursor: None,
        default_provider: None,
        default_model: Some("m".to_string()),
        compaction: None,
    };
    let error = resolve_model(None, None, &only_model).expect_err("应报错");
    assert!(error.contains("缺 defaultProvider"), "实际：{error}");
}

#[test]
fn nothing_configured_says_so_instead_of_guessing_a_model() {
    let error = resolve_model(None, None, &Settings::default()).expect_err("应报错");
    assert!(error.contains("没有可用的模型"), "实际：{error}");
    assert!(
        error.contains("未实现"),
        "要说清是回落未实现，而不是用户没配：{error}"
    );
}

#[test]
fn parsing_tolerates_absent_fields_and_empty_text() {
    assert_eq!(parse("").expect("空文本可解析"), Settings::default());
    assert_eq!(parse("   ").expect("空白可解析"), Settings::default());
    let parsed = parse(r#"{"defaultProvider":"p","defaultModel":"m"}"#).expect("可解析");
    assert_eq!(parsed.default_provider.as_deref(), Some("p"));
    assert_eq!(parsed.default_model.as_deref(), Some("m"));

    // 设定文件里还有大量本项目未实现的键：**不拒绝**未知字段。
    let with_extras = parse(r#"{"theme":"dark","defaultModel":"m","unknown":{"a":1}}"#)
        .expect("未知键不影响解析");
    assert_eq!(with_extras.default_model.as_deref(), Some("m"));
}

#[test]
fn malformed_settings_report_a_parse_error() {
    assert!(parse("{not json").is_err());
}
