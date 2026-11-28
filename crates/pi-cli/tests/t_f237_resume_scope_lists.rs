//! `/resume` 两个作用域各自持有一份列表（ref `session-selector.ts`
//! 的 `currentSessions` / `allSessions` + `setSessions`）。
//!
//! 真机症状（F237）：
//! 1. 按 Tab 切到 `All` 后屏幕一直停在 `Loading ...`，得按方向键才刷新 ——
//!    面板内容是 `open_panel` 时的快照，轮询到增量却只置了 `dirty`；
//! 2. 再切回 `Current Folder`，`All` 那些别的项目的会话**还在** ——
//!    只有一份 `sessions`，`All` 把全部并进来后没人换回去。

use pi_cli::settings_selector::{AllScanProgress, ResumeSelector, SessionRow};

fn row(dir: &std::path::Path, name: &str, id: &str, cwd: &str, mtime: u128) -> SessionRow {
    let path = dir.join(format!("{name}.jsonl"));
    std::fs::create_dir_all(dir).expect("目录");
    let text = format!(
        r#"{{"type":"session","version":3,"id":"{id}","timestamp":"2026-11-25T00:00:00.000Z","cwd":"{cwd}"}}"#
    );
    std::fs::write(&path, format!("{text}\n")).expect("写会话");
    SessionRow {
        path,
        id: id.to_string(),
        name: None,
        summary: None,
        all_messages_text: String::new(),
        cwd: cwd.to_string(),
        message_count: 0,
        modified_ms: mtime,
    }
}

fn ids(selector: &ResumeSelector) -> Vec<String> {
    selector
        .visible_indices()
        .iter()
        .map(|index| selector.sessions[*index].id.clone())
        .collect()
}

/// 切回 `Current Folder` 必须只看到本地目录的会话（不能残留 `All` 的结果）。
#[test]
fn switching_back_to_current_drops_other_projects() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mine = tmp.path().join("--mine--");
    let other = tmp.path().join("--other--");
    let local = row(&mine, "a", "local-1", "/work/mine", 100);
    let foreign = row(&other, "b", "foreign-1", "/work/other", 200);

    let mut selector = ResumeSelector::new();
    selector.current_cwd = "/work/mine".to_string();
    selector.filter_cwd = true;
    selector.set_current_rows(vec![local.clone()]);
    assert_eq!(ids(&selector), vec!["local-1"], "当前作用域只有本地会话");

    // 模拟 `All` 扫完（直接喂缓存，避免依赖线程时序）。
    selector.all_scopes = true;
    selector.all_rows = vec![foreign.clone(), local.clone()];
    selector.sessions = selector.all_rows.clone();
    assert_eq!(
        ids(&selector),
        vec!["foreign-1", "local-1"],
        "All 能看到别的项目"
    );

    selector.toggle_scope();
    assert!(!selector.all_scopes, "应切回 Current Folder");
    assert_eq!(
        ids(&selector),
        vec!["local-1"],
        "切回当前目录后不得残留别的项目的会话"
    );
}

/// 渐进扫描期间切回当前目录：显示列表立刻回本地，扫描结果只进 `All` 缓存。
#[test]
fn partial_results_do_not_leak_into_current_scope() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mine = tmp.path().join("--mine--");
    let local = row(&mine, "a", "local-1", "/work/mine", 100);

    let mut selector = ResumeSelector::new();
    selector.current_cwd = "/work/mine".to_string();
    selector.filter_cwd = true;
    selector.set_current_rows(vec![local.clone()]);

    selector.all_scopes = true;
    selector.all_rows = vec![row(
        &tmp.path().join("--other--"),
        "b",
        "foreign-1",
        "/work/o",
        300,
    )];
    selector.sessions = selector.all_rows.clone();

    selector.toggle_scope(); // 回到 Current Folder
    assert_eq!(ids(&selector), vec!["local-1"]);

    // 此时后台还可能有增量到达：不得污染当前作用域的显示列表。
    let (tx, rx) = std::sync::mpsc::channel();
    selector.all_scan = Some(rx);
    selector.all_loading = true;
    tx.send(AllScanProgress::Partial(
        vec![row(
            &tmp.path().join("--other--"),
            "c",
            "foreign-2",
            "/work/o",
            400,
        )],
        1,
        2,
    ))
    .expect("发送");
    selector.poll_all_scan();
    assert_eq!(
        ids(&selector),
        vec!["local-1"],
        "当前作用域的显示列表不得被 All 的增量改写"
    );
    assert_eq!(selector.all_rows.len(), 1, "增量应写进 All 缓存");
}

/// `All` 必须同时支持两种布局（ref `listSessionsFromDir` 直接列当前目录的
/// `*.jsonl`；`listAll()` 才遍历分区子目录）：
/// - 默认布局：根下是分区目录；
/// - 自定义会话目录（`--session-dir`）：`*.jsonl` 平铺在根下。
///
/// 此前只进子目录 ⇒ 自定义目录下按 Tab 切到 `All` 显示 `No sessions found`。
#[test]
fn all_scope_handles_flat_and_partitioned_layouts() {
    use pi_cli::settings_selector::{scan_all_sessions, scan_all_sessions_progressive};

    // 扁平布局
    let flat = tempfile::tempdir().expect("flat");
    for index in 0..3 {
        row(
            flat.path(),
            &format!("f{index}"),
            &format!("flat-{index}"),
            "/w",
            index,
        );
    }
    let sync = scan_all_sessions(flat.path());
    assert_eq!(sync.len(), 3, "扁平布局应列出根下的 3 个会话");

    let (tx, rx) = std::sync::mpsc::channel();
    scan_all_sessions_progressive(flat.path(), &tx);
    drop(tx);
    let mut done = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let AllScanProgress::Done(rows) = msg {
            done = rows;
        }
    }
    assert_eq!(done.len(), 3, "渐进扫描同样要处理扁平布局");

    // 分区布局
    let parted = tempfile::tempdir().expect("parted");
    for partition in ["--a--", "--b--"] {
        let dir = parted.path().join(partition);
        row(&dir, "x", &format!("in-{partition}"), "/w", 1);
    }
    let sync = scan_all_sessions(parted.path());
    assert_eq!(sync.len(), 2, "分区布局应列出各分区里的会话");
}
