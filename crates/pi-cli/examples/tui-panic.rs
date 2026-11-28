//! W4 自测用：进入 TUI 终端态后**主动 panic**，用于验证 panic hook 能否恢复终端。
//!
//! 只是测试/自测用的独立二进制（`cargo build --example tui-panic`），**不在生产代码里**留任何开关。
use std::io::Write;

fn main() {
    // 先安装 hook（与 `pi_cli::tui::run` 内部一致），再进入 raw mode 并 panic。
    pi_cli::tui::install_panic_hook();
    let _guard = match pi_cli::tui::TerminalGuard::new() {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("无法进入 raw mode: {error}");
            std::process::exit(2);
        }
    };
    let mut out = std::io::stdout();
    let _ = out.write_all(b"about to panic\n");
    let _ = out.flush();
    panic!("tui-panic example：故意 panic 以验证终端恢复");
}
