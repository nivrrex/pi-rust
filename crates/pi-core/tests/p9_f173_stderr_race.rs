//! bash 工具的 stdout/stderr 合并**不得丢流**（ref `core/tools/bash.ts:122-124`：
//! 两流挂在同一个 `onData` 上、按到达顺序追加）。
//!
//! 现状用 `tokio::select!` 在同一循环里轮询两路 `read`，被取消的那一路
//! （`AsyncRead` 的 `poll_read` 不保证取消安全）可能**吃掉已到内核缓冲的字节** ⇒
//! 偶发整段 stderr 消失。`F52g-ctrl-o-w80` 的 3 runs 里 1 个就是这样：
//! `cd` 到不存在的目录时 `cd: ... 没有那个文件或目录` 整段不见，只剩 `seq` 的 stdout。
//!
//! 本用例反复执行同一命令，要求每一次都拿到 stderr。

use pi_core::tools::bash::BashTool;
use pi_core::tools::tool::{Tool, ToolContext};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn stderr_is_never_dropped_across_repeated_runs() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let context = ToolContext {
        cwd: tmp.path().to_path_buf(),
    };
    let cancel = CancellationToken::new();
    // stderr 先出、stdout 随后大量输出：最容易踩到"取消吞字节"。
    // 与 `F52g-ctrl-o-w80` 的夹具同形态：`cd` 到不存在的目录（bash 报 stderr）
    // 紧接 `seq 1 60`（stdout 大量输出），两流几乎同时可读。
    let command = "cd /tmp/pi-rs-no-such-dir-for-stderr-race-abcdefghijklmnop\nseq 1 60";
    for round in 0..12 {
        let result = BashTool
            .execute(serde_json::json!({ "command": command }), &context, &cancel)
            .await;
        let text = result.text_content();
        assert!(
            text.contains("没有那个文件或目录") || text.contains("No such file or directory"),
            "第 {round} 轮丢了 cd 的 stderr；实际输出：{text}"
        );
        assert!(
            text.contains("60"),
            "第 {round} 轮丢了 stdout 尾部；实际输出：{text}"
        );
    }
}
