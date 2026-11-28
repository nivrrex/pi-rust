//! 运行期告警通道（TUI 与 print/json/rpc 共用的**唯一**出口）。
//!
//! 为什么需要它：TUI 是 inline 渲染，stderr 与画面挂在**同一个 PTY** 上。
//! 在主循环画完一帧之后写 stderr，那段字节会落在**当前光标位置**——
//! 也就是编辑器/输入框那一行，于是"错误信息出现在用户输入框里"（真机反馈）。
//! 首帧之前写的还能被后续帧覆盖，**会话中途**写的就留在屏幕上了。
//!
//! 规则：TUI 运行期间的告警一律 [`warn`] 入队，由主循环取走渲染成转录区的
//! 提示行；非 TUI（`-p`/`--mode json`/rpc）照旧写 stderr，那里 stderr 是正当输出。
//!
//! 与 `settings::pending_diagnostics()` 的区别：那个是**启动期**配置诊断，
//! 由 `tui::run` 一次性插进会话区；这里是**运行期**随时可能产生的告警
//! （扩展事件分发失败、会话文件写入失败、bash 运行时创建失败…）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

static PENDING: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// 当前是否 TUI 模式。由 `main` 在选定模式时设置（与 `main::tui_mode()` 同源）。
static TUI: AtomicBool = AtomicBool::new(false);

fn queue() -> &'static Mutex<Vec<String>> {
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

/// 由 `main` 在解析出运行模式后调用。
pub fn set_tui_mode(active: bool) {
    TUI.store(active, Ordering::Relaxed);
}

/// 是否处于 TUI 模式。
#[must_use]
pub fn is_tui() -> bool {
    TUI.load(Ordering::Relaxed)
}

/// 报一条运行期告警。
///
/// TUI：入队，等主循环取走渲染；重复消息与空消息直接忽略（避免刷屏）。
/// 其它模式：写 stderr。
pub fn warn(message: impl AsRef<str>) {
    let message = message.as_ref();
    if message.trim().is_empty() {
        return;
    }
    if is_tui() {
        if let Ok(mut list) = queue().lock() {
            // 同一条消息不重复堆叠（一次失败常常每帧都会重试）。
            if list.last().map(String::as_str) != Some(message) {
                list.push(message.to_string());
            }
        }
    } else {
        eprintln!("{message}");
    }
}

/// 取走已入队的告警（主循环每帧调用一次）。
#[must_use]
pub fn drain() -> Vec<String> {
    queue()
        .lock()
        .map(|mut list| list.drain(..).collect())
        .unwrap_or_default()
}

/// 清空队列（测试用）。
pub fn clear() {
    if let Ok(mut list) = queue().lock() {
        list.clear();
    }
}
