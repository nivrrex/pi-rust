//! shell 解析（纯函数）——逐条对齐 ref `utils/shell.ts:63-118` 的 `getShellConfig`。
//!
//! 解析顺序（ref）：
//! 1. 用户配置的 `shellPath`（settings.json）——存在则用，不存在**直接报错**（不回退）；
//! 2. Windows：`%ProgramFiles%\Git\bin\bash.exe` → `%ProgramFiles(x86)%\Git\bin\bash.exe`
//!    → PATH 里的 `bash.exe`（ref 用 `where` 查、并再校验存在）→ 都没有则报错并列出已尝试路径；
//! 3. Unix：`/bin/bash` → PATH 里的 `bash` → 兜底 `sh -c`。
//!
//! I/O 全部由调用方注入（`exists`、`path_entries`），便于在 Linux 上单测 Windows 布局。

/// 平台（显式传入，避免测试依赖编译目标）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Windows（`win32`）。
    Windows,
    /// 其它（Unix）。
    Unix,
}

/// 解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellChoice {
    /// 选中的 shell。
    Found {
        /// 可执行文件路径（可能是裸名 `bash`/`sh`，由 PATH 解析）。
        shell: String,
        /// 传给它的参数（通常是 `-c`）。
        args: Vec<String>,
    },
    /// 没找到：带着"已尝试的路径"返回，便于上游把清单拼进报错（ref 的报错就列了这些）。
    NotFound {
        /// 已尝试的候选（按解析顺序）。
        tried: Vec<String>,
    },
    /// `shellPath` 配了但文件不存在（ref `:70-73` 抛 `Custom shell path not found: <path>`）。
    CustomPathMissing {
        /// 用户配置的路径原文。
        path: String,
    },
}

/// ref `:24-28` 的 legacy WSL bash 判定：`X:\Windows\System32\bash.exe` / `Sysnative`。
///
/// 这类 bash 走 `-s` + stdin 传输命令（ref `:29-32`）。
#[must_use]
pub fn is_legacy_wsl_bash(path: &str) -> bool {
    let normalized = path.replace('/', "\\").to_ascii_lowercase();
    let bytes = normalized.as_bytes();
    if bytes.len() < 10 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    let drive = &normalized[..1];
    normalized.starts_with(&format!("{drive}:\\windows\\"))
        && (normalized.ends_with("\\system32\\bash.exe")
            || normalized.ends_with("\\sysnative\\bash.exe"))
}

/// 解析 shell（ref `utils/shell.ts:67-118`）。
///
/// `exists` 由调用方提供（真实实现用 `Path::exists`，测试注入目录布局）。
#[must_use]
pub fn resolve_shell(
    platform: Platform,
    custom_shell_path: Option<&str>,
    program_files: Option<&str>,
    program_files_x86: Option<&str>,
    path_entries: &[String],
    exists: &dyn Fn(&str) -> bool,
) -> ShellChoice {
    // 1) 用户配置优先；配了但不存在 ⇒ 直接报错（ref 不回退）。
    if let Some(custom) = custom_shell_path.filter(|value| !value.trim().is_empty()) {
        if exists(custom) {
            return found(custom);
        }
        return ShellChoice::CustomPathMissing {
            path: custom.to_string(),
        };
    }

    if platform == Platform::Windows {
        let mut tried = Vec::new();
        for base in [program_files, program_files_x86].into_iter().flatten() {
            let candidate = format!("{base}\\Git\\bin\\bash.exe");
            if exists(&candidate) {
                return found(&candidate);
            }
            tried.push(candidate);
        }
        // PATH 里的 bash.exe（ref 用 `where` 查；这里等价为按 PATH 顺序找文件）。
        if let Some(hit) = find_on_path("bash.exe", path_entries, exists) {
            return found(&hit);
        }
        tried.push("bash.exe (PATH)".to_string());
        return ShellChoice::NotFound { tried };
    }

    if exists("/bin/bash") {
        return found("/bin/bash");
    }
    if let Some(hit) = find_on_path("bash", path_entries, exists) {
        return found(&hit);
    }
    ShellChoice::Found {
        shell: "sh".to_string(),
        args: vec!["-c".to_string()],
    }
}

fn found(shell: &str) -> ShellChoice {
    // ref `getBashShellConfig`：legacy WSL bash 用 `-s`（命令走 stdin），其余 `-c`。
    let args = if is_legacy_wsl_bash(shell) {
        vec!["-s".to_string()]
    } else {
        vec!["-c".to_string()]
    };
    ShellChoice::Found {
        shell: shell.to_string(),
        args,
    }
}

fn find_on_path(
    name: &str,
    path_entries: &[String],
    exists: &dyn Fn(&str) -> bool,
) -> Option<String> {
    for entry in path_entries {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // 两种分隔符都试：Windows 的 PATH 有时混用 `/` 与 `\`。
        for separator in ['\\', '/'] {
            let candidate = format!("{}{separator}{name}", entry.trim_end_matches(['\\', '/']));
            if exists(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// `shell -c` 之外的传输方式（ref 的 `commandTransport`）。
#[must_use]
pub fn uses_stdin_transport(shell: &str) -> bool {
    is_legacy_wsl_bash(shell)
}

/// 全局 `shellPath`（settings.json）：由 pi-cli 在启动时登记，工具执行时读取。
static SHELL_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// 登记 settings.json 的 `shellPath`（ref `settings-manager.ts:1010` 的 `getShellPath()`）。
pub fn set_shell_path(path: Option<&str>) {
    if let Some(path) = path.filter(|value| !value.trim().is_empty()) {
        let _ = SHELL_PATH.set(path.to_string());
    }
}

/// 当前进程的解析入口：读取环境变量（`ProgramFiles`/`ProgramFiles(x86)`/`PATH`）与真实文件系统。
///
/// 与 [`resolve_shell`] 同语义，只是把 I/O 换成了真实实现。
#[must_use]
pub fn resolve_current() -> ShellChoice {
    let platform = if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Unix
    };
    let program_files = std::env::var("ProgramFiles").ok();
    let program_files_x86 = std::env::var("ProgramFiles(x86)").ok();
    let path_entries: Vec<String> = std::env::var("PATH")
        .unwrap_or_default()
        .split(if cfg!(windows) { ';' } else { ':' })
        .map(str::to_string)
        .collect();
    let exists = |candidate: &str| std::path::Path::new(candidate).exists();
    resolve_shell(
        platform,
        SHELL_PATH.get().map(String::as_str),
        program_files.as_deref(),
        program_files_x86.as_deref(),
        &path_entries,
        &exists,
    )
}
