//! `/resume` 的 `All` 作用域渐进扫描（ref `session-manager.ts:1976-2002`）。
//!
//! 此前切 Tab 是**同步全扫完再一次性塞进面板**：会话一多，按 Tab 整段卡住、
//! 没有任何反馈。ref 的 loader 则是边扫边 `progress(loaded,total,partialSessions)`。
//!
//! 这个测试钉住渐进语义本身（批次、进度、末批一致），UI 侧的"每帧轮询"由代码结构保证。

use pi_cli::settings_selector::{
    scan_all_sessions, scan_all_sessions_progressive, AllScanProgress,
};

fn write_session(dir: &std::path::Path, name: &str, model: &str) {
    std::fs::create_dir_all(dir).expect("分区目录");
    let path = dir.join(name);
    let text = format!(
        "{}\n{}\n",
        r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-11-25T00:00:00.000Z","cwd":"/tmp/x"}"#,
        serde_json::json!({
            "type": "model_change", "id": "m1", "parentId": null,
            "timestamp": "2026-11-25T00:00:00.000Z", "provider": model, "modelId": "m"
        })
    );
    std::fs::write(path, text).expect("写会话");
}

/// 会话多于一批的发布间隔时，必须**分批到达**，而不是一条 Done。
#[test]
fn publishes_partials_before_done() {
    let root = tempfile::tempdir().expect("tmp");
    let dir = root.path().join("proj");
    // 多于 `ALL_SESSION_PUBLISH_INTERVAL`(=100) 才能观察到中间批次：
    // 造 250 条 ⇒ 期望 1、100、200、250 四次 Partial。
    for index in 0..250 {
        write_session(&dir, &format!("s{index:03}.jsonl"), "acme");
    }
    let (tx, rx) = std::sync::mpsc::channel();
    scan_all_sessions_progressive(root.path(), &tx);
    drop(tx);
    let mut partials = Vec::new();
    let mut done = None;
    while let Ok(msg) = rx.try_recv() {
        match msg {
            AllScanProgress::Partial(rows, loaded, total) => {
                partials.push((rows.len(), loaded, total))
            }
            AllScanProgress::Done(rows) => done = Some(rows),
        }
    }
    assert!(
        partials.len() >= 3,
        "应有多批 Partial（首批/每 100/末批），实际 {partials:?}"
    );
    assert_eq!(partials[0].1, 1, "首批应是 loaded=1：{partials:?}");
    assert!(
        partials.iter().all(|(_, _, total)| *total == 250),
        "total 每批都应是文件总数：{partials:?}"
    );
    let done = done.expect("应有 Done");
    assert_eq!(done.len(), 250, "末批应是全部 250 条");
}

/// 渐进扫描的最终结果必须与同步 `scan_all_sessions` **完全一致**（行与顺序）。
#[test]
fn progressive_result_matches_sync_scan() {
    let root = tempfile::tempdir().expect("tmp");
    for (index, model) in ["acme", "other", "acme", "third"].iter().enumerate() {
        let dir = root.path().join(format!("proj{index}"));
        write_session(&dir, "a.jsonl", model);
        write_session(&dir, "b.jsonl", model);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    scan_all_sessions_progressive(root.path(), &tx);
    drop(tx);
    let mut progressive = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let AllScanProgress::Done(rows) = msg {
            progressive = rows;
        }
    }
    let sync = scan_all_sessions(root.path());
    let paths_progressive: Vec<_> = progressive.iter().map(|row| row.path.clone()).collect();
    let paths_sync: Vec<_> = sync.iter().map(|row| row.path.clone()).collect();
    assert_eq!(paths_progressive, paths_sync, "顺序与内容应与同步扫描一致");
}

/// 空会话根：立刻 Done，不得卡在 Loading。
#[test]
fn empty_root_finishes_immediately() {
    let root = tempfile::tempdir().expect("tmp");
    let (tx, rx) = std::sync::mpsc::channel();
    scan_all_sessions_progressive(root.path(), &tx);
    drop(tx);
    let first = rx.try_recv().expect("应有消息");
    assert!(
        matches!(first, AllScanProgress::Done(ref rows) if rows.is_empty()),
        "空目录应立刻 Done"
    );
}
