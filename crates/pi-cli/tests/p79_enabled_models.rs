//! P7-9：`settings.enabledModels` → 可切换模型列表（跨 provider）。
//!
//! ref 里该设置形如 `provider/modelId`，而模型 id 自身可能含 `/`
//! （用户真机配置：`command-code/deepseek/deepseek-v4.1-flash`）⇒ **按第一个 `/` 切分**。

use pi_cli::settings::Settings;

fn choices(settings_json: &str) -> Vec<(String, String)> {
    let settings: Settings = serde_json::from_str(settings_json).expect("settings 合法");
    settings
        .enabled_models
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| entry.split_once('/'))
        .map(|(provider, model)| (provider.to_string(), model.to_string()))
        .collect()
}

/// 用户真机配置的三条（模型 id 带 `/`）。
#[test]
fn splits_on_the_first_slash_only() {
    let parsed = choices(
        r#"{"enabledModels":["openrouter/openrouter/free",
                             "command-code/deepseek/deepseek-v4.1-flash",
                             "opencode-go/deepseek-v4.1-flash"]}"#,
    );
    assert_eq!(
        parsed,
        vec![
            ("openrouter".to_string(), "openrouter/free".to_string()),
            (
                "command-code".to_string(),
                "deepseek/deepseek-v4.1-flash".to_string()
            ),
            ("opencode-go".to_string(), "deepseek-v4.1-flash".to_string()),
        ]
    );
}

/// 没有该设置时为 `None`（调用方回落到当前 provider 的 models）。
#[test]
fn absent_setting_is_none() {
    let settings: Settings = serde_json::from_str(r#"{"defaultProvider":"x"}"#).expect("合法");
    assert!(settings.enabled_models.is_none());
}

/// `powershell` 不在默认激活集里，但 `--tools powershell` 必须能选中它
/// （上游 `core/tools/index.ts:148/217` 把它放在工具表里，默认集是 `["read","bash","edit","write"]`）。
#[test]
fn powershell_tool_is_selectable_but_not_default() {
    use pi_cli::args::Args;
    let defaults = pi_cli::print_mode::default_tools(&Args::default());
    let names: Vec<String> = defaults
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(
        names,
        vec!["read", "bash", "edit", "write"],
        "默认工具集不应包含 powershell"
    );

    let parsed = Args {
        tools: vec!["powershell".to_string()],
        ..Args::default()
    };
    let selected = pi_cli::print_mode::default_tools(&parsed);
    let names: Vec<String> = selected
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, vec!["powershell"]);
    assert!(selected[0]
        .description()
        .starts_with("Execute a PowerShell command"));
}

/// 只读工具 `ls` 同样只在 `--tools` 里选中，不在默认集内。
#[test]
fn ls_tool_is_selectable_but_not_default() {
    use pi_cli::args::Args;
    let parsed = Args {
        tools: vec!["ls".to_string()],
        ..Args::default()
    };
    let selected = pi_cli::print_mode::default_tools(&parsed);
    let names: Vec<String> = selected
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, vec!["ls"]);
    assert!(selected[0]
        .description()
        .starts_with("List directory contents"));
}
