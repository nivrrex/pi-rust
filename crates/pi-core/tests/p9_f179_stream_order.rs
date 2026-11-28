//! bash 工具必须**按到达顺序**合并 stdout/stderr（ref `core/tools/bash.ts:122-124`：
//! 两流挂在同一个 `onData` 上、追加进同一缓冲区）。
//!
//! 现状用 `tokio::select!` 在同一循环里轮询两路 `read`：stdout 持续就绪时，
//! 随机选择可能反复命中它直到 EOF，stderr 的数据被推到后面 ⇒ 顺序反转。
//! 真机复现率约 2/12（`F52g-ctrl-o-w80`：`cd` 的报错从 `seq` 之前跑到之后）。
//!
//! 本用例在同一 runtime 上并发跑多份，逼近 TUI 场景的竞争条件。

use pi_core::tools::bash::BashTool;
use pi_core::tools::tool::{Tool, ToolContext};
use tokio_util::sync::CancellationToken;

const CMD: &str = "cd /tmp/pi-rs-f179-no-such-dir-abcdefghijklmnop\nseq 1 60";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stderr_stays_before_later_stdout() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut handles = Vec::new();
    for _ in 0..16 {
        let cwd = tmp.path().to_path_buf();
        handles.push(tokio::spawn(async move {
            let context = ToolContext { cwd };
            let cancel = CancellationToken::new();
            let result = BashTool
                .execute(serde_json::json!({ "command": CMD }), &context, &cancel)
                .await;
            result.text_content()
        }));
    }
    let mut bad = Vec::new();
    for (index, handle) in handles.into_iter().enumerate() {
        let text = handle.await.expect("任务");
        let err = text.find("cd:").unwrap_or(usize::MAX);
        let first = text.find("\n1\n").unwrap_or(usize::MAX);
        if err == usize::MAX || err > first {
            bad.push(index);
        }
    }
    assert!(
        bad.is_empty(),
        "这些轮次的 stderr 缺失或跑到 stdout 之后：{bad:?}（共 16 轮）"
    );
}
