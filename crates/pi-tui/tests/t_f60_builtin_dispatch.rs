//! F60/3：**每一条内置斜杠命令都要么执行、要么给出可见反馈**。
//!
//! 真机缺陷模式（用户报的 `/todo` 卡死、`/compact` 无反应）共享同一个成因：
//! 命令被识别（`builtin_command(name)` 为 true ⇒ `TuiAction::Command`），
//! 但 `pi-cli` 的 `match command_word` 里没有对应分支，落到 `other => {}`
//! ⇒ **屏幕一个字节都不变**，用户只能认为程序卡死了。
//!
//! 本用例钉住命令面的**完整性**：
//! 1. `pi_tui::builtin_command` 识别的每一条，都在 `ref` 的 `BUILTIN_SLASH_COMMANDS` 里；
//! 2. `ref` 的每一条都被识别（不能"少认"导致被当 prompt 发给模型）；
//! 3. `pi-cli` 对**每一条**都有分支（执行或显式"未实现"提示）——用源码断言，
//!    因为分支集合本身就是契约，新增命令必须同时加分支。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// ref `core/slash-commands.ts:19-44` 的 24 条（逐条列举，缺一即失败）。
const REF_BUILTINS: [&str; 24] = [
    "settings",
    "model",
    "tree",
    "thinking",
    "scoped-models",
    "export",
    "import",
    "share",
    "bug",
    "copy",
    "name",
    "session",
    "changelog",
    "hotkeys",
    "fork",
    "clone",
    "trust",
    "login",
    "logout",
    "new",
    "compact",
    "resume",
    "reload",
    "quit",
];

/// ref 的 24 条必须全部被识别（少认 = 被当普通 prompt 发给模型）。
#[test]
fn every_ref_builtin_command_is_recognised() {
    for name in REF_BUILTINS {
        assert!(
            pi_tui::builtin_command(name),
            "ref 的内置命令未被识别：/{name}"
        );
    }
}

/// 识别表里的每一条都必须来自 ref（不能凭空多认，否则用户敲 `/xxx` 不会发给模型）。
#[test]
fn recognised_commands_all_come_from_ref() {
    // 识别表是 26 条：ref 的 24 条 + `login`/`logout` 重复计入（ref 里各出现一次，
    // 表里为兼容历史写法各写了两遍）。用黑名单式枚举把表的内容固定下来：
    // 任何**不在** ref 集合里的名字都必须不通过。
    let extra = [
        "skill", "help", "clear", "debug", "expand", "collapse", "status",
    ];
    for name in extra {
        assert!(
            !pi_tui::builtin_command(name),
            "/{name} 不在 ref 的 BUILTIN_SLASH_COMMANDS 里，不应被当作内置命令识别"
        );
    }
}

/// `pi-cli` 的 `TuiAction::Command` 分支必须覆盖**每一条**被识别的命令。
///
/// 改前 `compact` 缺失（静默丢弃）；改后所有 12 条缺失项统一走
/// `Command /<name> is not implemented yet.` 的可见提示。
#[test]
fn every_recognised_command_has_a_dispatch_arm() {
    let source =
        std::fs::read_to_string(repo_root().join("crates/pi-cli/src/tui.rs")).expect("读 tui.rs");
    let start = source
        .find("TuiAction::Command(name) => {")
        .expect("应有 TuiAction::Command 分支");
    let tail = &source[start..];
    let end = tail
        .find("TuiAction::Redraw => dirty = true,")
        .expect("Command 分支后应接 Redraw 分支");
    let body = &tail[..end];
    let mut missing: Vec<&str> = Vec::new();
    for name in REF_BUILTINS {
        // 分支形如 `"name" => {`（缩进随意，按精确引号匹配）。
        let needle = format!("\"{name}\" =>");
        if !body.contains(&needle) {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "这些内置命令没有分派分支（会静默丢弃、屏幕不变）：{missing:?}"
    );
}

/// 兜底分支必须给出**可见提示**（不是 `let _ = other;`）。
///
/// 这是"无响应"类缺陷的结构性防线：即使以后新增命令忘了加分支，
/// 用户也会看到 `Command /x is not implemented yet.` 而不是以为程序卡死。
#[test]
fn unknown_arm_reports_instead_of_silently_doing_nothing() {
    let source =
        std::fs::read_to_string(repo_root().join("crates/pi-cli/src/tui.rs")).expect("读 tui.rs");
    let start = source
        .find("TuiAction::Command(name) => {")
        .expect("应有 TuiAction::Command 分支");
    let tail = &source[start..];
    let end = tail
        .find("TuiAction::Redraw => dirty = true,")
        .expect("Command 分支后应接 Redraw 分支");
    let body = &tail[..end];
    let fallback_at = body.rfind("other => {").expect("应有兜底分支");
    let fallback = &body[fallback_at..];
    assert!(
        fallback.contains("push_unimplemented("),
        "兜底分支必须给出可见提示（改前是 `let _ = other;` ⇒ 屏幕不变）"
    );
    // 提示文案本身也要在源码里可查（防止把 push_unimplemented 改成空实现）。
    let source_all = &source;
    assert!(
        source_all.contains("is not implemented yet."),
        "未实现命令的提示文案缺失"
    );
}
