//! 会话文件带 **UTF-8 BOM** 时仍须能读（跨平台编辑场景）。
//!
//! Windows 上的记事本等编辑器保存 UTF-8 时会加 BOM。TS 侧容忍
//! （`utils/text.ts` 有 `stripBom`，实测同一份带 BOM 的会话能正常续写），
//! pi-rs 此前报 `首行不是 session header` 且 `-c` 直接失败。

use pi_core::session_io::SessionFile;

fn write(path: &std::path::Path, bom: bool) {
    let body = concat!(
        r#"{"type":"session","version":3,"id":"b1","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/tmp"}"#,
        "\n",
        r#"{"type":"message","id":"m1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"带 BOM"}]}}"#,
        "\n"
    );
    let bytes = if bom {
        let mut v = vec![0xEF, 0xBB, 0xBF];
        v.extend_from_slice(body.as_bytes());
        v
    } else {
        body.as_bytes().to_vec()
    };
    std::fs::write(path, bytes).expect("写文件");
}

#[test]
fn session_with_bom_is_readable() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("bom.jsonl");
    write(&path, true);
    let file = SessionFile::read(&path).expect("带 BOM 的会话应能读");
    assert_eq!(file.entries.len(), 1, "应解析出 1 条消息");
    assert_eq!(file.header.id, "b1");
}

/// 不带 BOM 的照旧能读（回归）。
#[test]
fn session_without_bom_still_reads() {
    let tmp = tempfile::tempdir().expect("tmp");
    let path = tmp.path().join("plain.jsonl");
    write(&path, false);
    let file = SessionFile::read(&path).expect("普通会话应能读");
    assert_eq!(file.entries.len(), 1);
}
