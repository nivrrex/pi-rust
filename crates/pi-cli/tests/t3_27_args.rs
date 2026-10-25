//! P3.5：CLI 参数解析的规则（出处见 `pi_cli::args` 各分支的注释）。
//!
//! 重点在两条容易被"静默忽略"的地方：未识别的 `--flag` 必须**被记下来**（由 main 报错），
//! 单横线未知选项必须进错误诊断。

use pi_cli::args::{parse, AppMode, Mode};

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[test]
fn positional_arguments_and_at_files_are_separated() {
    let args = parse(&argv(&["hello", "@file.txt", "world"]));
    assert_eq!(args.messages, vec!["hello", "world"]);
    assert_eq!(args.file_args, vec!["file.txt"]);
    assert!(!args.has_errors());
}

#[test]
fn double_dash_stops_parsing_and_keeps_everything_positional() {
    let args = parse(&argv(&["--", "--model", "@x.txt", "rest"]));
    assert_eq!(args.messages, vec!["--model", "rest"]);
    assert_eq!(args.file_args, vec!["x.txt"]);
    assert!(args.model.is_none(), "-- 之后不再解析选项");
}

#[test]
fn unknown_single_dash_option_is_an_error_diagnostic() {
    let args = parse(&argv(&["-x"]));
    assert!(args.has_errors());
    assert_eq!(args.diagnostics[0].message, "Unknown option: -x");
}

#[test]
fn unknown_long_flags_are_recorded_not_ignored() {
    let args = parse(&argv(&[
        "--whatever=1",
        "--other",
        "value",
        "--flag-only",
        "--next",
    ]));
    assert_eq!(
        args.unknown_flags.get("whatever"),
        Some(&Some("1".to_string()))
    );
    assert_eq!(
        args.unknown_flags.get("other"),
        Some(&Some("value".to_string()))
    );
    assert_eq!(args.unknown_flags.get("flag-only"), Some(&None));
    assert_eq!(args.unknown_flags.get("next"), Some(&None));
}

#[test]
fn known_flags_with_values_are_parsed() {
    let args = parse(&argv(&[
        "--provider",
        "mock",
        "--model=claude-x",
        "--api-key",
        "k",
        "--system-prompt",
        "sys",
        "--append-system-prompt",
        "more",
    ]));
    assert_eq!(args.provider.as_deref(), Some("mock"));
    assert_eq!(args.model.as_deref(), Some("claude-x"));
    assert_eq!(args.api_key.as_deref(), Some("k"));
    assert_eq!(args.system_prompt.as_deref(), Some("sys"));
    assert_eq!(args.append_system_prompt, vec!["more"]);
}

#[test]
fn mode_rejects_missing_and_invalid_values_with_the_exact_text() {
    let missing = parse(&argv(&["--mode"]));
    assert!(missing.has_errors());
    assert_eq!(
        missing.diagnostics[0].message,
        "--mode requires text, json, or rpc"
    );

    let invalid = parse(&argv(&["--mode", "xml"]));
    assert!(invalid.has_errors());
    assert_eq!(
        invalid.diagnostics[0].message,
        "Invalid mode \"xml\". Valid values: text, json, rpc"
    );

    let ok = parse(&argv(&["--mode", "rpc"]));
    assert_eq!(ok.mode, Some(Mode::Rpc));
}

#[test]
fn name_requires_a_value() {
    let args = parse(&argv(&["--name"]));
    assert!(args.has_errors());
    assert_eq!(args.diagnostics[0].message, "--name requires a value");
}

#[test]
fn print_eats_a_following_positional_but_not_a_flag() {
    let args = parse(&argv(&["--print", "do it"]));
    assert!(args.print);
    assert_eq!(args.messages, vec!["do it"]);

    let with_flag = parse(&argv(&["--print", "--verbose", "later"]));
    assert!(with_flag.print);
    assert_eq!(
        with_flag.messages,
        vec!["later"],
        "不能把 --verbose 当成位置参数"
    );
}

#[test]
fn list_flags_split_on_commas_and_trim() {
    let args = parse(&argv(&[
        "--tools",
        " read , bash ,, edit ",
        "--exclude-tools=write",
    ]));
    assert_eq!(args.tools, vec!["read", "bash", "edit"]);
    assert_eq!(args.exclude_tools, vec!["write"]);
}

#[test]
fn tool_switches_and_session_flags_set_booleans() {
    let args = parse(&argv(&[
        "--no-tools",
        "--no-session",
        "-c",
        "-r",
        "--offline",
        "--verbose",
    ]));
    assert!(args.no_tools && args.no_session && args.continue_session && args.resume);
    assert!(args.offline && args.verbose);
}

#[test]
fn recognized_but_unimplemented_flags_are_listed_for_the_caller_to_reject() {
    let args = parse(&argv(&["--no-extensions", "--theme", "dark", "hi"]));
    assert_eq!(args.unsupported, vec!["--no-extensions", "--theme"]);
    assert_eq!(args.messages, vec!["hi"], "--theme 的值不能被当成位置参数");
}

#[test]
fn app_mode_follows_the_spec_priority() {
    let rpc = parse(&argv(&["--mode", "rpc"]));
    assert_eq!(rpc.app_mode(true, true), AppMode::Rpc, "--mode rpc 优先");

    let json_mode = parse(&argv(&["--mode", "json"]));
    assert_eq!(json_mode.app_mode(true, true), AppMode::PrintJson);

    let print = parse(&argv(&["--print"]));
    assert_eq!(print.app_mode(true, true), AppMode::PrintText);

    let no_flags = parse(&argv(&[]));
    assert_eq!(no_flags.app_mode(true, true), AppMode::Interactive);
    assert_eq!(no_flags.app_mode(false, true), AppMode::PrintText);
    assert_eq!(no_flags.app_mode(true, false), AppMode::PrintText);
}

#[test]
fn thinking_accepts_any_string_at_parse_time() {
    let args = parse(&argv(&["--thinking", "bogus"]));
    assert_eq!(args.thinking.as_deref(), Some("bogus"));
}

#[test]
fn no_context_files_is_accepted_while_other_unimplemented_flags_stay_rejected() {
    for flag in ["--no-context-files", "-nc"] {
        let args = parse(&argv(&[flag]));
        assert!(args.no_context_files, "{flag} 必须置真");
        assert!(
            !args
                .unsupported
                .iter()
                .any(|item| item.contains("no-context-files")),
            "{flag} 不得再进 unsupported：{:?}",
            args.unsupported
        );
        assert!(!args.has_errors(), "{flag} 之后不得有错误诊断");
    }
    // `--tui-mode` 自 P7-1 起在 print/json/rpc 放行 ⇒ 邻近未实现 flag 改用仍未支持的 `--export`。
    let args = parse(&argv(&["--export", "x"]));
    assert!(!args.no_context_files);
    assert!(
        args.unsupported.iter().any(|item| item == "--export"),
        "--export 必须仍被拒：{:?}",
        args.unsupported
    );
    let args = parse(&argv(&["--no-context-files=1"]));
    assert!(!args.no_context_files, "带 =值 的形式不得置真");
    assert!(
        args.unknown_flags.contains_key("no-context-files"),
        "带 =值 的形式应落到 unknown_flags：{:?}",
        args.unknown_flags
    );
}
