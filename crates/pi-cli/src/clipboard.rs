//! 剪贴板写入（`ctrl+x` 复制最后一条助手消息用）。
//!
//! 规格来源 ref `utils/clipboard.ts:74-140`：
//! 1. 先试**平台命令**（Linux 上按环境选择）：`termux-clipboard-set`（`TERMUX_VERSION`）、
//!    `wl-copy`（`WAYLAND_DISPLAY`）、`xclip -selection clipboard` / `xsel --clipboard --input`（`DISPLAY`）；
//! 2. 命令都不可用或无显示环境时退化为 **OSC 52**（`\x1b]52;c;<base64>\x07`，`:15-24`，
//!    载荷上限 100_000 个 base64 字符）；
//! 3. 桌面环境（有 `DISPLAY`/`WAYLAND_DISPLAY`）且都失败时**报错**而不是假装成功（`:127-140`）。
//!
//! 本模块把"选哪些命令"和"该不该发 OSC 52"做成**纯函数**，便于单测；I/O 在 [`write_to_clipboard`]。

/// 剪贴板写入方式（纯决策结果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardPlan {
    /// 依次尝试这些命令（每个命令的参数已展开），把文本写到 stdin。
    Commands(Vec<Vec<String>>),
    /// 直接发 OSC 52（无显示环境 / 远程会话）。
    Osc52,
    /// 命令与 OSC 52 都不可用 ⇒ 报错（桌面环境的失败分支）。
    Unavailable(&'static str),
}

/// 决定用哪种方式写剪贴板（**纯函数**，只看环境变量）。
#[must_use]
pub fn plan(env: &dyn Fn(&str) -> Option<String>, is_remote: bool) -> ClipboardPlan {
    let mut commands: Vec<Vec<String>> = Vec::new();
    if env("TERMUX_VERSION").is_some() {
        commands.push(vec!["termux-clipboard-set".to_string()]);
    }
    if env("WAYLAND_DISPLAY").is_some() {
        commands.push(vec!["wl-copy".to_string()]);
    }
    if env("DISPLAY").is_some() {
        commands.push(vec![
            "xclip".to_string(),
            "-selection".to_string(),
            "clipboard".to_string(),
        ]);
        commands.push(vec![
            "xsel".to_string(),
            "--clipboard".to_string(),
            "--input".to_string(),
        ]);
    }
    let headless = env("DISPLAY").is_none()
        && env("WAYLAND_DISPLAY").is_none()
        && env("TERMUX_VERSION").is_none();
    if !commands.is_empty() {
        // 有命令可试：先试命令；万一都失败，远程会话仍可退化到 OSC 52。
        return ClipboardPlan::Commands(commands);
    }
    if headless || is_remote {
        return ClipboardPlan::Osc52;
    }
    ClipboardPlan::Unavailable("Clipboard unavailable")
}

/// 是否远程会话（ref `:11-13`：`SSH_CONNECTION`/`SSH_CLIENT`/`MOSH_CONNECTION`）。
#[must_use]
pub fn is_remote(env: &dyn Fn(&str) -> Option<String>) -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "MOSH_CONNECTION"]
        .iter()
        .any(|name| env(name).is_some())
}

/// 把文本写到系统剪贴板：先按 [`plan`] 试命令，再退化到 OSC 52。
///
/// # Errors
/// 桌面环境且命令不可用、或 OSC 52 载荷过大时返回错误描述（调用方显示为状态行）。
pub fn write_to_clipboard(text: &str) -> Result<(), String> {
    let env = |name: &str| std::env::var(name).ok();
    let plan = plan(&env, is_remote(&env));
    match plan {
        ClipboardPlan::Commands(commands) => {
            for command in &commands {
                if run_command(command, text) {
                    return Ok(());
                }
            }
            if is_remote(&env) {
                return emit_osc52(text);
            }
            Err(
                "Clipboard unavailable: install `clipboard tools` (wl-copy / xclip / xsel)"
                    .to_string(),
            )
        }
        ClipboardPlan::Osc52 => emit_osc52(text),
        ClipboardPlan::Unavailable(reason) => Err(reason.to_string()),
    }
}

/// 跑一条剪贴板命令（把文本写 stdin）；找不到命令或非零退出都算失败。
fn run_command(argv: &[String], text: &str) -> bool {
    use std::io::Write as _;
    let Some((program, args)) = argv.split_first() else {
        return false;
    };
    let child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    if let Some(stdin) = child.stdin.as_mut() {
        if stdin.write_all(text.as_bytes()).is_err() {
            return false;
        }
    }
    matches!(child.wait(), Ok(status) if status.success())
}

/// 发 OSC 52（ref `:15-24`：载荷超过 100_000 个 base64 字符时放弃）。
fn emit_osc52(text: &str) -> Result<(), String> {
    use std::io::Write as _;
    let encoded = crate::tui::base64_encode(text.as_bytes());
    if encoded.len() > 100_000 {
        return Err("Clipboard unavailable: text exceeds the OSC 52 size limit".to_string());
    }
    let mut out = std::io::stdout();
    out.write_all(format!("\x1b]52;c;{encoded}\x07").as_bytes())
        .map_err(|error| format!("Clipboard unavailable: {error}"))?;
    out.flush()
        .map_err(|error| format!("Clipboard unavailable: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{is_remote, plan, ClipboardPlan};

    fn env_of(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    /// 无显示环境 ⇒ OSC 52（容器/无头场景；ref `:127-136` 的 headless 分支）。
    #[test]
    fn headless_uses_osc52() {
        let env = env_of(&[]);
        assert_eq!(plan(&env, false), ClipboardPlan::Osc52);
    }

    /// Wayland/X11 环境 ⇒ 先试平台命令（顺序照 ref `:92-103`）。
    #[test]
    fn desktop_prefers_commands() {
        let wayland = env_of(&[("WAYLAND_DISPLAY", "wayland-0")]);
        assert_eq!(
            plan(&wayland, false),
            ClipboardPlan::Commands(vec![vec!["wl-copy".to_string()]])
        );
        let x11 = env_of(&[("DISPLAY", ":0")]);
        assert_eq!(
            plan(&x11, false),
            ClipboardPlan::Commands(vec![
                vec![
                    "xclip".to_string(),
                    "-selection".to_string(),
                    "clipboard".to_string()
                ],
                vec![
                    "xsel".to_string(),
                    "--clipboard".to_string(),
                    "--input".to_string()
                ],
            ])
        );
        let termux = env_of(&[("TERMUX_VERSION", "0.118")]);
        assert_eq!(
            plan(&termux, false),
            ClipboardPlan::Commands(vec![vec!["termux-clipboard-set".to_string()]])
        );
    }

    /// 桌面环境但没有任何可用命令 ⇒ 报错而不是假装成功（ref `:137-140`）。
    #[test]
    fn desktop_without_tools_reports_unavailable() {
        let env = env_of(&[]);
        assert_eq!(plan(&env, false), ClipboardPlan::Osc52);
        // 有 SSH 会话时即便无显示也走 OSC 52（ref `:132-134`）
        assert_eq!(plan(&env, true), ClipboardPlan::Osc52);
        let remote = env_of(&[("SSH_CONNECTION", "1.2.3.4")]);
        assert!(is_remote(&remote));
        assert_eq!(plan(&remote, true), ClipboardPlan::Osc52);
    }
}
