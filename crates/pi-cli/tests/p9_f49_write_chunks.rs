//! F49/Y1：一帧的写出必须**分块**，不能因为"块内没有换行"而退化成一次超大 `write_all`。
//!
//! ref `packages/tui/src/tui-main-screen.ts:9` `MAX_RENDER_WRITE_CHARS = 1024 * 1024`，
//! `BoundedTerminalWriter.append()`（`:31-58`）在缓冲满 1 MiB 时**立即 flush**，
//! 与"内容里有没有换行"无关。
//!
//! 旧实现的切点只在遇到 `\n` 时更新（`cut = index + 1`），当一段超过 64 KiB 都没有换行
//! （或整帧第一段就很长）时 `cut > start` 不成立 ⇒ 整帧一次写出。真实 95 MB 会话实测：
//! 首帧 `write=59004ms`、单次写出 50 MB（`PI_TUI_TIMING`）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;

/// 记录每次 `write` 长度的假终端。
struct Recorder {
    writes: Vec<usize>,
}

impl Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writes.push(buf.len());
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn frame_without_newlines_is_still_chunked() {
    // 8 MiB 的**单行**（无换行）——旧实现会一次写出全部。
    let payload = "x".repeat(8 * 1024 * 1024);
    let mut sink = Recorder { writes: Vec::new() };
    pi_cli::tui::write_frame_for_test(&mut sink, &payload).expect("写出应成功");
    assert!(
        sink.writes.len() > 1,
        "8 MiB 无换行的一帧应分块写出，实际 {} 次（单次 {} 字节）",
        sink.writes.len(),
        sink.writes.first().copied().unwrap_or(0)
    );
    let max = sink.writes.iter().copied().max().unwrap_or(0);
    assert!(
        max <= 1024 * 1024,
        "单次写出 {max} 字节，超过 ref 的 1 MiB 上限"
    );
}

#[test]
fn normal_frame_keeps_chunking_on_line_boundaries() {
    let payload = (0..200_000)
        .map(|i| format!("line {i}\n"))
        .collect::<String>();
    let mut sink = Recorder { writes: Vec::new() };
    pi_cli::tui::write_frame_for_test(&mut sink, &payload).expect("写出应成功");
    assert!(sink.writes.len() > 1, "多行大帧应分块");
    // 所有块都必须落在行边界上（上一块以 `\n` 结束）——这里只校验没有超限。
    let max = sink.writes.iter().copied().max().unwrap_or(0);
    assert!(max <= 1024 * 1024, "单次写出 {max} 字节，超过 1 MiB");
}
