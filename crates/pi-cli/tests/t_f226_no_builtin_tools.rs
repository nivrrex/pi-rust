//! `--no-builtin-tools` / `-nbt`（ref `cli/args.ts` 的 `noBuiltinTools`）。
//!
//! 语义：**禁用内置工具，但保留扩展/自定义工具**。
//!
//! 此前这个 flag 被解析、`default_tools*` **从未读过它** ⇒ 传了内置工具照样全在，
//! flag 形同虚设（与 `--session-id` 同一类"解析了却不消费"的缺陷）。

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn no_builtin_tools_drops_the_builtin_set() {
    let parsed = pi_cli::args::parse(&owned(&["--no-builtin-tools", "-p", "hi"]));
    assert!(parsed.no_builtin_tools, "flag 应置真");
    let tools = pi_cli::print_mode::default_tools(&parsed);
    // **按名字断言会被扩展污染**：已装的扩展可以注册同名工具（如 `pi-bash-guard` 的
    // `bash`），而那些属于"扩展工具"、按语义**应当保留**。
    // 所以这里比对**不含扩展**的内置集（`default_tools_without_extensions` 不对外，
    // 改用"内置四点名工具是否出现在最终集合"的方式无法区分来源）⇒ 改判：
    // 内置的 `read`/`edit`/`write` 必然不在（扩展不会注册这三个），
    // `bash` 若在则必须是扩展来源。
    for name in ["read", "edit", "write"] {
        assert!(
            !tools.iter().any(|tool| tool.name() == name),
            "--no-builtin-tools 后不应有内置 {name}"
        );
    }
}

/// 不带该 flag 时四个内置工具都在（回归）。
#[test]
fn builtin_tools_present_by_default() {
    let parsed = pi_cli::args::parse(&owned(&["-p", "hi"]));
    let tools = pi_cli::print_mode::default_tools(&parsed);
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    for expected in ["read", "bash", "edit", "write"] {
        assert!(names.contains(&expected), "默认应有 {expected}：{names:?}");
    }
}
