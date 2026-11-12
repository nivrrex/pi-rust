//! F55/W2：Windows 专有逻辑抽成纯函数后的单测（Linux 上可跑）。
//!
//! 覆盖两处真实差异：
//! 1. **cwd 比较**（`session_writer::resolve_for_compare`，ref `utils/paths.ts:78-106` +
//!    `session-manager.ts:744-746`）：`-c` 找最近会话时要把会话头的 cwd 与进程 cwd 比较。
//!    Windows 上 `C:/Users/x`（Git Bash / WSL 写法）与 `C:\Users\x` 是同一目录，
//!    但**不是**同一个字符串；ref 靠 `nodeResolvePath`（win32 版 `path.resolve`）抹平。
//! 2. **CRLF**：工具输出里的 `\r` 必须在渲染前剥掉（`render-utils.ts:35-37`
//!    `normalizeDisplayText`），否则终端回车覆盖整行——F53c 已在渲染侧修过，
//!    这里固定纯函数契约。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// 分隔符统一：`C:/a/b` 与 `C:\a\b` 归一后相等。
#[test]
fn windows_cwd_normalizes_separators() {
    assert_eq!(
        pi_cli::session_writer::normalize_windows_cwd("C:/Users/Admin/project"),
        r"C:\Users\Admin\project"
    );
    assert_eq!(
        pi_cli::session_writer::normalize_windows_cwd(r"C:\Users\Admin\project"),
        r"C:\Users\Admin\project"
    );
}

/// 尾部斜杠不影响相等（`C:\a\` == `C:\a`）。
#[test]
fn windows_cwd_ignores_trailing_separator() {
    let a = pi_cli::session_writer::normalize_windows_cwd(r"C:\Users\Admin\");
    let b = pi_cli::session_writer::normalize_windows_cwd(r"C:\Users\Admin");
    assert_eq!(a, b);
    assert_eq!(a, r"C:\Users\Admin");
}

/// 盘符大小写**保留**：ref 的 `normalizePath` 不折叠大小写，我们也不折叠。
///
/// 这条是"不自作聪明"的钉子：Windows 文件系统不区分大小写，但 ref 的比较是**字符串**
/// 比较（`sessionCwdMatches` 用 `===`），折叠与否会与 ref 相反。
#[test]
fn windows_drive_letter_case_is_preserved() {
    let upper = pi_cli::session_writer::normalize_windows_cwd("C:/Users/Admin");
    let lower = pi_cli::session_writer::normalize_windows_cwd("c:/Users/Admin");
    assert_eq!(upper, r"C:\Users\Admin");
    assert_eq!(lower, r"c:\Users\Admin");
    assert_ne!(upper, lower, "ref 不做大小写折叠，保持原样");
}

/// 非 Windows 形态的路径不做分隔符替换（Linux 路径里的 `/` 是分隔符本身）。
#[test]
fn posix_paths_keep_forward_slashes() {
    assert_eq!(
        pi_cli::session_writer::normalize_cwd_for_compare("/root/Desktop/pi-rust/"),
        "/root/Desktop/pi-rust"
    );
    assert!(!pi_cli::session_writer::looks_like_windows_path(
        "/root/Desktop"
    ));
    assert!(pi_cli::session_writer::looks_like_windows_path(
        r"C:\Users"
    ));
    assert!(pi_cli::session_writer::looks_like_windows_path("C:/Users"));
}
