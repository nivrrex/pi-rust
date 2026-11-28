//! F49/Y1：恢复会话时参与渲染/上下文的条目必须是 **`buildContextEntries` 的结果**，
//! 而不是"文件里全部条目"。
//!
//! ref `packages/coding-agent/src/core/session-manager.ts:476-509`：
//! 从最后一条 `compaction` 起取（含该 compaction 自身、从 `firstKeptEntryId` 开始保留的
//! 条目、以及 compaction 之后的全部条目）。压缩前的历史**不再进入上下文**，所以
//! `renderInitialMessages()`（`interactive-mode.ts:4056-4062`）也不会把它渲染出来。
//!
//! 旧实现直接把 `file.entries` 里所有 `message` 交给 TUI ⇒ 一个压缩过 23 次、95 MB 的
//! 会话会把 50 万行全部渲染并写到终端（真机实测首帧 50 MB、47 秒；TS 同会话 1.8 MB、3 秒）。
use pi_core::session::SessionEntry;
use pi_core::session_io::SessionFile;

fn entry_line(json: &str) -> SessionEntry {
    match pi_core::session::parse_line(json).expect("条目应可解析") {
        pi_core::session::SessionLine::Entry(entry) => entry,
        pi_core::session::SessionLine::Header(_) => panic!("夹具不应含 header"),
    }
}

fn sample() -> Vec<SessionEntry> {
    // 结构：c1 → m1 → m2 → compaction(cp1, firstKept=m2) → m3，只有 m2、m3 与 cp1 应保留。
    vec![
        entry_line(
            r#"{"type":"message","id":"c1","parentId":null,"timestamp":"t1","message":{"role":"user","content":[{"type":"text","text":"旧一"}],"timestamp":1}}"#,
        ),
        entry_line(
            r#"{"type":"message","id":"m1","parentId":"c1","timestamp":"t2","message":{"role":"assistant","api":"a","provider":"p","model":"m","content":[{"type":"text","text":"旧二"}],"stopReason":"stop","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"timestamp":2}}"#,
        ),
        entry_line(
            r#"{"type":"message","id":"m2","parentId":"m1","timestamp":"t3","message":{"role":"assistant","api":"a","provider":"p","model":"m","content":[{"type":"text","text":"保留"}],"stopReason":"stop","usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"timestamp":3}}"#,
        ),
        entry_line(
            r#"{"type":"compaction","id":"cp1","parentId":"m2","timestamp":"t4","summary":"摘要","firstKeptEntryId":"m2","tokensBefore":1000}"#,
        ),
        entry_line(
            r#"{"type":"message","id":"m3","parentId":"cp1","timestamp":"t5","message":{"role":"user","content":[{"type":"text","text":"压缩后"}],"timestamp":5}}"#,
        ),
    ]
}

#[test]
fn resume_messages_skip_history_before_last_compaction() {
    let kept = SessionFile::messages_of(&sample());
    let texts: Vec<String> = kept.iter().map(|message| format!("{message:?}")).collect();
    let joined = texts.join("\n");
    assert!(
        !joined.contains("旧一") && !joined.contains("旧二"),
        "压缩前的历史不应进入恢复渲染：{joined}"
    );
    assert!(
        joined.contains("保留") && joined.contains("压缩后"),
        "压缩点之后（及 firstKeptEntryId 起）的条目必须保留：{joined}"
    );
}

#[test]
fn session_without_compaction_keeps_everything() {
    let entries: Vec<SessionEntry> = sample()
        .into_iter()
        .filter(|entry| !matches!(entry, SessionEntry::Compaction { .. }))
        .collect();
    let kept = SessionFile::messages_of(&entries);
    // 4 条 message（c1/m1/m2/m3）全在。
    assert_eq!(kept.len(), 4, "无压缩时应保留全部 message");
}

#[test]
fn session_file_helper_is_used_by_the_resume_path() {
    // 该测试只保证 API 存在并可被调用（真正的接线在 pi-cli，由
    // `p9_f49_resume_compaction` 的 pty 用例断言屏幕内容）。
    let file = SessionFile {
        header: serde_json::from_str(
            r#"{"type":"session","version":3,"id":"s","timestamp":"t","cwd":"/tmp"}"#,
        )
        .expect("header 应可解析"),
        entries: sample(),
    };
    let kept = file.context_messages();
    assert!(!kept.is_empty(), "SessionFile::context_messages 不应为空");
}
