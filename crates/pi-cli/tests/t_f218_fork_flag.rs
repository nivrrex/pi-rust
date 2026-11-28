//! `--fork <session.jsonl>`（ref `main.ts:367-390` + `session-manager.ts:1812-1867`）。
//!
//! 与 rpc 的 `fork` **不同**：那个只复制目标条目的祖先链；`--fork` 复制源会话的
//! **全部非 header 条目**（含其它分支），新 header：
//! `parentSession` = 源文件路径、`cwd` = 当前目录、`id` = 新生成的 id。

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pi-rs")
}

fn source(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("src.jsonl");
    let body = concat!(
        r#"{"type":"session","version":3,"id":"src-1","timestamp":"2026-01-01T00:00:00.000Z","cwd":"/old/place"}"#,
        "\n",
        r#"{"type":"message","id":"m1","parentId":null,"timestamp":"2026-01-01T00:00:01.000Z","message":{"role":"user","content":[{"type":"text","text":"主干"}]}}"#,
        "\n",
        r#"{"type":"message","id":"m2a","parentId":"m1","timestamp":"2026-01-01T00:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"分支A"}]}}"#,
        "\n",
        r#"{"type":"message","id":"m2b","parentId":"m1","timestamp":"2026-01-01T00:00:03.000Z","message":{"role":"assistant","content":[{"type":"text","text":"分支B"}]}}"#,
        "\n"
    );
    std::fs::write(&path, body).expect("写源会话");
    path
}

#[test]
fn fork_copies_every_entry_into_a_new_session() {
    let tmp = tempfile::tempdir().expect("tmp");
    let src = source(tmp.path());
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).expect("建工作目录");
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).expect("建会话目录");

    let result = Command::new(bin())
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", tmp.path())
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .arg("--fork")
        .arg(&src)
        .arg("-p")
        .arg("hi")
        .output()
        .expect("应能执行");
    // 不校验退出码（mock 不可达会失败）；只校验**新会话文件被正确写出**。
    let _ = &result;

    let mut produced: Vec<std::path::PathBuf> = std::fs::read_dir(&sessions)
        .expect("读会话目录")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .collect();
    produced.sort();
    assert_eq!(produced.len(), 1, "应生成恰好一个新会话文件：{produced:?}");
    let text = std::fs::read_to_string(&produced[0]).expect("读新会话");
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    // header + 三条条目（含两条分支）全部复制。
    assert_eq!(lines.len(), 4, "应复制全部非 header 条目：{text}");
    assert!(lines[1].contains("主干"), "应含主干");
    assert!(lines[2].contains("分支A"), "应含分支 A");
    assert!(
        lines[3].contains("分支B"),
        "应含分支 B（--fork 复制全部分支）"
    );
    // header：parentSession 指向源文件、cwd 换成当前目录。
    assert!(
        lines[0].contains("parentSession"),
        "header 应带 parentSession"
    );
    assert!(
        lines[0].contains(&src.display().to_string()),
        "parentSession 应指向源文件：{}",
        lines[0]
    );
    assert!(
        !lines[0].contains("/old/place"),
        "cwd 不应沿用源会话的：{}",
        lines[0]
    );
}

/// 源文件不存在 ⇒ 报错、退出 1（ref `No session found matching '…'`）。
#[test]
fn missing_source_fails() {
    let tmp = tempfile::tempdir().expect("tmp");
    let result = Command::new(bin())
        .current_dir(tmp.path())
        .env("PI_CODING_AGENT_DIR", tmp.path())
        .arg("--fork")
        .arg("no-such-session")
        .output()
        .expect("应能执行");
    assert!(!result.status.success(), "应退出非 0");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("No session found")
            || stderr.contains("not found")
            || stderr.contains("不存在"),
        "应说明找不到会话：{stderr}"
    );
}
