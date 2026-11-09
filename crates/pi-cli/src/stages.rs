//! 只读的阶段计时（`PI_TUI_STAGES=<文件>`）。
//!
//! 为什么需要：`PI_TUI_TIMING` 只覆盖"一帧内的 build/diff/write"，而首帧到进程输出之间
//! 还有会话查找、读文件、JSON 解析、构建上下文、宿主 spawn、扩展加载等段落 ——
//! 没有这些分段就只能看到"首帧 4.1 s"这么一个总数（F50 的实测差值就是这么来的）。
//!
//! 关闭时**零开销**：不建文件、不格式化。

use std::io::Write as _;
use std::sync::OnceLock;
use std::time::Instant;

fn origin() -> &'static Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now)
}

/// 记一个阶段点（`<name>\t<毫秒>`，追加）。
pub fn mark(name: &str) {
    let Some(path) = std::env::var_os("PI_TUI_STAGES") else {
        return;
    };
    let elapsed = origin().elapsed().as_millis();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{name}\t{elapsed}");
    }
}
