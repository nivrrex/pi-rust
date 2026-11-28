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

// ---------------------------------------------------------------------------
// 剪贴板图片（`app.clipboard.pasteImage`，ref `utils/clipboard-image.ts`）
// ---------------------------------------------------------------------------

/// 支持的图片 MIME（ref `:17`）。顺序即"取值尝试顺序"。
pub const SUPPORTED_IMAGE_MIME_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// 去掉 `;charset=…` 之类参数并小写（ref `baseMimeType`，`:29-31`）。
#[must_use]
pub fn base_mime_type(mime_type: &str) -> String {
    mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_lowercase()
}

/// MIME → 文件扩展名（ref `extensionForImageMimeType`，`:33-44`）；不支持 ⇒ `None`。
#[must_use]
pub fn extension_for_image_mime_type(mime_type: &str) -> Option<&'static str> {
    match base_mime_type(mime_type).as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        _other => None,
    }
}

/// 从 `TARGETS` 列表里挑一个最优先的支持类型（ref `selectPreferredImageMimeType`）。
///
/// 保持 `SUPPORTED_IMAGE_MIME_TYPES` 的优先级（png 最优先），
/// 而不是剪贴板给出的顺序：截图工具常同时提供 `image/png` 与 `image/bmp`，
/// 按剪贴板顺序可能优先拿到不支持的格式。
#[must_use]
pub fn select_preferred_image_mime_type(targets: &[String]) -> Option<&'static str> {
    let normalized: Vec<String> = targets.iter().map(|t| base_mime_type(t)).collect();
    SUPPORTED_IMAGE_MIME_TYPES
        .iter()
        .find(|supported| normalized.iter().any(|target| target == *supported))
        .copied()
}

/// 读到的剪贴板图片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

/// 跑一条命令、返回 stdout 字节（失败/超时/非零退出 ⇒ `None`）。
fn run_command_capture(argv: &[String]) -> Option<Vec<u8>> {
    let (program, args) = argv.split_first()?;
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// 读剪贴板图片（ref `readClipboardImage` 的 Linux 分支，`:220-247`）。
///
/// 顺序：Wayland ⇒ `wl-paste`；X11 ⇒ `xclip`。取不到图片返回 `None`
/// （调用方据此退化到"插入剪贴板文本"，ref `:3058-3066`）。
///
/// **不 panic**：剪贴板命令缺失、无显示环境、权限被拒都只是 `None`。
#[must_use]
pub fn read_clipboard_image(env: &dyn Fn(&str) -> Option<String>) -> Option<ClipboardImage> {
    if env("TERMUX_VERSION").is_some() {
        // ref `:213-215`：Termux 无图片剪贴板。
        return None;
    }
    let wayland = env("WAYLAND_DISPLAY").is_some()
        || env("XDG_SESSION_TYPE").is_some_and(|value| value == "wayland");
    if wayland {
        let types = run_command_capture(&["wl-paste".to_string(), "--list-types".to_string()])?;
        let targets: Vec<String> = String::from_utf8_lossy(&types)
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        let preferred = select_preferred_image_mime_type(&targets)?;
        let data = run_command_capture(&[
            "wl-paste".to_string(),
            "--type".to_string(),
            preferred.to_string(),
            "--no-newline".to_string(),
        ])?;
        return (!data.is_empty()).then(|| ClipboardImage {
            bytes: data,
            mime_type: preferred.to_string(),
        });
    }
    // 没有 DISPLAY 就没有 X11 剪贴板（`?` 直接短路成 `None`）。
    env("DISPLAY")?;
    let types = run_command_capture(&[
        "xclip".to_string(),
        "-selection".to_string(),
        "clipboard".to_string(),
        "-t".to_string(),
        "TARGETS".to_string(),
        "-o".to_string(),
    ]);
    let targets: Vec<String> = types
        .as_deref()
        .map(|bytes| {
            String::from_utf8_lossy(bytes)
                .split(['\r', '\n'])
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let preferred = select_preferred_image_mime_type(&targets);
    // ref `:186-192`：优先试选中的类型，再按支持列表逐个试。
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(preferred) = preferred {
        candidates.push(preferred);
    }
    for supported in SUPPORTED_IMAGE_MIME_TYPES {
        if !candidates.contains(&supported) {
            candidates.push(supported);
        }
    }
    for mime_type in candidates {
        let data = run_command_capture(&[
            "xclip".to_string(),
            "-selection".to_string(),
            "clipboard".to_string(),
            "-t".to_string(),
            mime_type.to_string(),
            "-o".to_string(),
        ]);
        if let Some(data) = data {
            if !data.is_empty() {
                return Some(ClipboardImage {
                    bytes: data,
                    mime_type: mime_type.to_string(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod clipboard_image_tests {
    use super::*;

    #[test]
    fn base_mime_type_strips_parameters_and_lowercases() {
        assert_eq!(base_mime_type("image/PNG;charset=utf-8"), "image/png");
        assert_eq!(base_mime_type("  image/jpeg "), "image/jpeg");
    }

    #[test]
    fn extensions_cover_the_supported_set() {
        assert_eq!(extension_for_image_mime_type("image/png"), Some("png"));
        assert_eq!(extension_for_image_mime_type("image/jpeg"), Some("jpg"));
        assert_eq!(extension_for_image_mime_type("image/webp"), Some("webp"));
        assert_eq!(extension_for_image_mime_type("image/gif"), Some("gif"));
        assert_eq!(extension_for_image_mime_type("image/bmp"), None);
        assert_eq!(extension_for_image_mime_type("text/plain"), None);
    }

    #[test]
    fn preferred_type_follows_the_supported_order_not_clipboard_order() {
        let targets = vec!["image/gif".to_string(), "image/png".to_string()];
        assert_eq!(
            select_preferred_image_mime_type(&targets),
            Some("image/png"),
            "应按支持列表的优先级（png 先于 gif），而不是剪贴板给出的顺序"
        );
    }

    #[test]
    fn no_supported_type_means_none() {
        let targets = vec!["text/plain".to_string(), "image/bmp".to_string()];
        assert_eq!(select_preferred_image_mime_type(&targets), None);
        assert_eq!(select_preferred_image_mime_type(&[]), None);
    }

    #[test]
    fn termux_never_has_an_image() {
        let env = |name: &str| (name == "TERMUX_VERSION").then(|| "1".to_string());
        assert_eq!(read_clipboard_image(&env), None);
    }

    #[test]
    fn no_display_means_no_image() {
        let env = |_name: &str| None;
        assert_eq!(read_clipboard_image(&env), None);
    }
}

/// 读剪贴板**文本**（`app.clipboard.pasteImage` 的 fallback，ref
/// `interactive-mode.ts:3058-3066`：没有图片就插入文本）。
///
/// 与 [`read_clipboard_image`] 同样的后端顺序；取不到返回 `None`。
#[must_use]
pub fn read_clipboard_text(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if env("TERMUX_VERSION").is_some() {
        let bytes = run_command_capture(&["termux-clipboard-get".to_string()])?;
        return String::from_utf8(bytes)
            .ok()
            .filter(|text| !text.is_empty());
    }
    let wayland = env("WAYLAND_DISPLAY").is_some()
        || env("XDG_SESSION_TYPE").is_some_and(|value| value == "wayland");
    if wayland {
        let bytes = run_command_capture(&["wl-paste".to_string()])?;
        return String::from_utf8(bytes)
            .ok()
            .filter(|text| !text.is_empty());
    }
    // 没有 DISPLAY 就没有 X11 剪贴板（`?` 直接短路成 `None`）。
    env("DISPLAY")?;
    let bytes = run_command_capture(&[
        "xclip".to_string(),
        "-selection".to_string(),
        "clipboard".to_string(),
        "-o".to_string(),
    ])?;
    String::from_utf8(bytes)
        .ok()
        .filter(|text| !text.is_empty())
}

/// 把剪贴板内容插入编辑器（ref `handleClipboardPaste`，`interactive-mode.ts:3047-3068`）。
///
/// 顺序：**图片优先**（写临时文件、插入文件路径）→ 否则插入文本 → 都没有则 `None`。
/// 图片走临时文件是 ref 的既有设计：编辑器只接受文本，图片由模型按路径读取。
///
/// 返回要插入的文本；`None` = 剪贴板里既没图片也没文本。
#[must_use]
pub fn clipboard_paste_text(env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(image) = read_clipboard_image(env) {
        let extension = extension_for_image_mime_type(&image.mime_type).unwrap_or("png");
        // 文件名带随机后缀，避免同一次会话里多次粘贴互相覆盖（ref `:3053`）。
        let file_name = format!("pi-clipboard-{}.{extension}", uuid::Uuid::now_v7());
        let path = std::env::temp_dir().join(file_name);
        if std::fs::write(&path, &image.bytes).is_ok() {
            return Some(path.to_string_lossy().to_string());
        }
        // 写临时文件失败（磁盘/权限）⇒ 退化到文本分支，不静默丢内容。
    }
    read_clipboard_text(env)
}
