//! `--use-theme <name>`（ref `cli/args.ts:190-195` 的 `useTheme`）。
//!
//! 此前它和 `--theme`/`--no-themes` 一起被塞进 `unsupported` ⇒ 传了就报错，
//! 而 `--help` 里明明列着。pi-rs 有内置 `dark`/`light` 两个主题，
//! `--use-theme` 有直接价值：临时换个主题而不改 `settings.json`。

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn use_theme_takes_a_value() {
    let parsed = pi_cli::args::parse(&owned(&["--use-theme", "light", "-p", "hi"]));
    assert_eq!(parsed.use_theme.as_deref(), Some("light"));
}

#[test]
fn use_theme_is_absent_by_default() {
    let parsed = pi_cli::args::parse(&owned(&["-p", "hi"]));
    assert_eq!(parsed.use_theme, None);
}

/// 缺值时报错（ref `args.ts:191-194`：`--use-theme requires a theme name`）。
#[test]
fn use_theme_without_a_value_is_rejected() {
    let parsed = pi_cli::args::parse(&owned(&["--use-theme"]));
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message.contains("--use-theme")),
        "缺值应记诊断：{:?}",
        parsed.diagnostics
    );
}
