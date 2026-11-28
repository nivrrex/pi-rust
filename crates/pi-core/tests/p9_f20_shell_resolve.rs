//! P9-F20/S1：shell 解析纯函数（ref `utils/shell.ts:63-118`）。
//!
//! 用注入的 `exists` 在 Linux 上模拟 Windows 目录布局，覆盖：shellPath 正斜杠/含空格、
//! shellPath 不存在（ref 直接报错不回退）、默认 Git 路径命中、仅 PATH 命中、全部缺失的已尝试清单。

use pi_core::tools::shell_resolve::{resolve_shell, uses_stdin_transport, Platform, ShellChoice};

fn layout(paths: &'static [&'static str]) -> impl Fn(&str) -> bool {
    move |candidate: &str| paths.contains(&candidate)
}

#[test]
fn custom_shell_path_with_forward_slashes_and_spaces_wins() {
    let exists = layout(&["C:/Program Files/Git/bin/bash.exe"]);
    let choice = resolve_shell(
        Platform::Windows,
        Some("C:/Program Files/Git/bin/bash.exe"),
        None,
        None,
        &[],
        &exists,
    );
    assert_eq!(
        choice,
        ShellChoice::Found {
            shell: "C:/Program Files/Git/bin/bash.exe".to_string(),
            args: vec!["-c".to_string()],
        }
    );
}

#[test]
fn missing_custom_shell_path_is_an_error_not_a_fallback() {
    // ref `:70-73`：配了 shellPath 但文件不存在 ⇒ 抛错，**不回退**到 Git Bash/PATH。
    let exists = layout(&["C:\\Program Files\\Git\\bin\\bash.exe"]);
    let choice = resolve_shell(
        Platform::Windows,
        Some("C:\\nope\\bash.exe"),
        Some("C:\\Program Files"),
        None,
        &["C:\\Program Files\\Git\\bin".to_string()],
        &exists,
    );
    assert_eq!(
        choice,
        ShellChoice::CustomPathMissing {
            path: "C:\\nope\\bash.exe".to_string()
        }
    );
}

#[test]
fn windows_falls_back_to_git_bash_in_program_files() {
    let exists = layout(&["C:\\Program Files\\Git\\bin\\bash.exe"]);
    let choice = resolve_shell(
        Platform::Windows,
        None,
        Some("C:\\Program Files"),
        None,
        &[],
        &exists,
    );
    assert_eq!(
        choice,
        ShellChoice::Found {
            shell: "C:\\Program Files\\Git\\bin\\bash.exe".to_string(),
            args: vec!["-c".to_string()],
        }
    );
}

#[test]
fn windows_path_hit_reports_the_full_path() {
    // 只有 PATH 命中（Git 若装在非默认位置，或不设 ProgramFiles）。
    let exists = layout(&["D:/tools/msys64/usr/bin/bash.exe"]);
    let choice = resolve_shell(
        Platform::Windows,
        None,
        None,
        None,
        &["D:/tools/msys64/usr/bin".to_string()],
        &exists,
    );
    assert_eq!(
        choice,
        ShellChoice::Found {
            shell: "D:/tools/msys64/usr/bin/bash.exe".to_string(),
            args: vec!["-c".to_string()],
        }
    );
}

#[test]
fn windows_all_missing_lists_what_was_tried() {
    let exists = layout(&[]);
    let choice = resolve_shell(
        Platform::Windows,
        None,
        Some("C:\\Program Files"),
        Some("C:\\Program Files (x86)"),
        &["C:\\Windows\\System32".to_string()],
        &exists,
    );
    let ShellChoice::NotFound { tried } = choice else {
        panic!("应返回 NotFound：{choice:?}");
    };
    assert_eq!(tried.len(), 3, "两条 Git Bash + PATH 一条：{tried:?}");
    assert!(tried[0].contains("Git\\bin\\bash.exe"), "{tried:?}");
    assert!(tried[1].contains("Program Files (x86)"), "{tried:?}");
    assert!(tried[2].contains("PATH"), "{tried:?}");
}

#[test]
fn unix_prefers_bin_bash_then_path_then_sh() {
    let exists = layout(&["/bin/bash"]);
    assert_eq!(
        resolve_shell(Platform::Unix, None, None, None, &[], &exists),
        ShellChoice::Found {
            shell: "/bin/bash".to_string(),
            args: vec!["-c".to_string()],
        }
    );

    let exists = layout(&["/opt/homebrew/bin/bash"]);
    assert_eq!(
        resolve_shell(
            Platform::Unix,
            None,
            None,
            None,
            &["/opt/homebrew/bin".to_string()],
            &exists
        ),
        ShellChoice::Found {
            shell: "/opt/homebrew/bin/bash".to_string(),
            args: vec!["-c".to_string()],
        }
    );

    let exists = layout(&[]);
    assert_eq!(
        resolve_shell(Platform::Unix, None, None, None, &[], &exists),
        ShellChoice::Found {
            shell: "sh".to_string(),
            args: vec!["-c".to_string()],
        }
    );
}

#[test]
fn legacy_wsl_bash_uses_stdin_transport() {
    let exists = layout(&["C:\\Windows\\System32\\bash.exe"]);
    let choice = resolve_shell(
        Platform::Windows,
        None,
        None,
        None,
        &["C:\\Windows\\System32".to_string()],
        &exists,
    );
    assert_eq!(
        choice,
        ShellChoice::Found {
            shell: "C:\\Windows\\System32\\bash.exe".to_string(),
            args: vec!["-s".to_string()],
        }
    );
    assert!(uses_stdin_transport("C:\\Windows\\System32\\bash.exe"));
    assert!(!uses_stdin_transport(
        "C:\\Program Files\\Git\\bin\\bash.exe"
    ));
}

#[test]
fn windows_path_hit_on_system32_uses_stdin_transport() {
    // ref `utils/shell.ts:20-32`：`X:\Windows\System32\bash.exe`（legacy WSL）走 `-s` + stdin。
    let exists = layout(&["C:\\Windows\\System32\\bash.exe"]);
    let choice = resolve_shell(
        Platform::Windows,
        None,
        None,
        None,
        &["C:\\Windows\\System32".to_string()],
        &exists,
    );
    assert_eq!(
        choice,
        ShellChoice::Found {
            shell: "C:\\Windows\\System32\\bash.exe".to_string(),
            args: vec!["-s".to_string()],
        }
    );
}
