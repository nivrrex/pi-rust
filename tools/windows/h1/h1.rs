// H1 最小实验（**不属于 pi-rs**，放在仓库外 C:\Users\Admin\pi-rs-wintest\h1）。
//
// 用法：h1.exe <shape> <mode> [prefix]
//   shape:
//     csi   —— 前缀 'A'*prefix + "\x1b[0m" + TAIL   （验证 CSI 跨 4096 边界）
//     osc   —— 前缀 'A'*prefix + "\x1b]8;;\x07" + TAIL（验证 OSC 8 关闭序列跨边界）
//     real  —— 现状形态：一行文本 + 行尾复位 `\x1b[0m \x1b]8;;\x07` + CRLF（一次写完）
//     real-long —— 同 real，但行加长到 3000 字节（仍 < 4096）
//   mode:
//     std   —— std::io::stdout().write_all(&bytes)（一次 write_all，不分片）
//     wcw   —— 一次 WriteConsoleW（对照）
//     split —— 按 4096 字节硬切后逐块 write_all（模拟"被 4096 切断"的旧版形态）
//
// 观察点：屏幕上是否出现 "0m" / ";;" 残片（TAIL/END 之前）。
#![allow(unsafe_code)]

use std::io::Write;

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    pub type Handle = *mut c_void;
    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetStdHandle(which: u32) -> Handle;
        pub fn GetConsoleMode(h: Handle, mode: *mut u32) -> i32;
        pub fn SetConsoleMode(h: Handle, mode: u32) -> i32;
    }
    /// 与 pi-rs 一样打开输出句柄的 VT 处理（`ENABLE_VIRTUAL_TERMINAL_PROCESSING`）。
    /// 经典 conhost 默认关闭，不开的话屏幕上看到的是字面量 `[0m`，实验无意义。
    pub fn enable_vt() -> i32 {
        unsafe {
            let handle = GetStdHandle(0xFFFF_FFF5u32);
            let mut mode = 0u32;
            if GetConsoleMode(handle, &mut mode) == 0 {
                return -1; // 不是控制台（ConPTY 管道）：不需要
            }
            SetConsoleMode(handle, mode | 0x0004)
        }
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn WriteConsoleW(
            h: Handle,
            buf: *const u16,
            len: u32,
            written: *mut u32,
            reserved: *mut c_void,
        ) -> i32;
    }
}

fn build(shape: &str, prefix: usize) -> String {
    let reset = "\u{1b}[0m";
    let osc_close = "\u{1b}]8;;\u{7}";
    match shape {
        "csi" => format!("{}{}TAIL\r\n<<END>>\r\n", "A".repeat(prefix), reset),
        "osc" => format!("{}{}TAIL\r\n<<END>>\r\n", "A".repeat(prefix), osc_close),
        "real" => format!(
            "\u{1b}[38;2;118;118;118m{}\u{1b}[0m {}\r\n<<END>>\r\n",
            "pi v0.87.1 placeholder line with normal length", osc_close
        ),
        "real-long" => format!(
            "\u{1b}[38;2;118;118;118m{}\u{1b}[0m {}\r\n<<END>>\r\n",
            "L".repeat(prefix.max(1)),
            osc_close
        ),
        _ => format!("{}\r\n<<END>>\r\n", shape),
    }
}

fn write_std(text: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

fn write_split(text: &str, chunk: usize) {
    let bytes = text.as_bytes();
    let mut out = std::io::stdout();
    for piece in bytes.chunks(chunk) {
        let _ = out.write_all(piece);
        let _ = out.flush();
    }
}

fn write_wcw(text: &str) {
    #[cfg(windows)]
    unsafe {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let handle = win::GetStdHandle(0xFFFF_FFF5u32); // STD_OUTPUT_HANDLE
        let mut written = 0u32;
        let ok = win::WriteConsoleW(
            handle,
            wide.as_ptr(),
            wide.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        );
        eprintln!("wcw ok={ok} written={written}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shape = args.get(1).cloned().unwrap_or_else(|| "real".to_string());
    let mode = args.get(2).cloned().unwrap_or_else(|| "std".to_string());
    let prefix: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(4094);

    #[cfg(windows)]
    {
        let vt = win::enable_vt();
        eprintln!("vt_mode={vt}");
    }
    let text = build(&shape, prefix);
    eprintln!(
        "shape={shape} mode={mode} prefix={prefix} total_bytes={}",
        text.len()
    );
    match mode.as_str() {
        "wcw" => write_wcw(&text),
        "split" => write_split(&text, 4096),
        _ => write_std(&text),
    }
}
