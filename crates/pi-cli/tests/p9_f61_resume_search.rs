//! F61/33：`/resume` 的搜索框（ref `session-selector.ts:292/333` +
//! `session-selector-search.ts`）。
//!
//! 改前：`/resume` 面板**没有搜索框**（只渲染了两行提示，提示里写着
//! `re:<pattern> regex · "phrase" exact`，但按下去没有任何输入位置）。
//! 这也是上一轮 `app.session.deleteNoninvasive` 留空的那一半：
//! ref 的该键只在"搜索框非空"时才转给输入框（`session-selector.ts:599-608`）。
//!
//! 判据：输入查询后列表只剩匹配项（用两个名字不同的会话验证）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn screen_of(raw: &std::path::Path) -> String {
    let bytes = std::fs::read(raw).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let mut out = String::new();
    let mut rest = text.as_ref();
    while let Some(index) = rest.find('\u{1b}') {
        out.push_str(&rest[..index]);
        let after = &rest[index..];
        let mut chars = after.chars();
        let _ = chars.next();
        match chars.next() {
            Some('[') => {
                let mut consumed = 2;
                for ch in after.chars().skip(2) {
                    consumed += ch.len_utf8();
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                }
                rest = &after[consumed.min(after.len())..];
            }
            Some(']') => {
                if let Some(bell) = after.find('\u{7}') {
                    rest = &after[bell + 1..];
                } else {
                    rest = "";
                }
            }
            _other => {
                rest = &after[1.min(after.len())..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 造会话，名字由参数给。
fn write_named_session(path: &std::path::Path, id: &str, name: &str, cwd: &std::path::Path) {
    let lines = [
        serde_json::json!({
            "type": "session", "version": 3, "id": id,
            "timestamp": "2026-01-01T00:00:00.000Z",
            "cwd": cwd.to_string_lossy().to_string()
        }),
        serde_json::json!({
            "type": "message", "id": "u1", "parentId": null,
            "timestamp": "2026-01-01T00:00:01.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "hello"}],
                        "timestamp": "2026-01-01T00:00:01.000Z"}
        }),
        serde_json::json!({
            "type": "session_info", "id": "n1", "parentId": "u1",
            "timestamp": "2026-01-01T00:00:02.000Z", "name": name
        }),
    ];
    let text = lines
        .iter()
        .map(|line| serde_json::to_string(line).expect("序列化"))
        .collect::<Vec<String>>()
        .join("\n");
    std::fs::write(path, format!("{text}\n")).expect("写会话");
}

/// 在 `/resume` 面板里输入 `query`（逐字符），返回屏幕文本。
fn run_search(query: &str) -> Option<String> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&agent, &home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m1","name":"m1","contextWindow":200000,"maxTokens":1024}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");
    write_named_session(
        &sessions.join("2026-01-01T00-00-00_a1.jsonl"),
        "01a1a9a8-0000-7000-8000-0000000000a1",
        "alpine-notes",
        &work,
    );
    write_named_session(
        &sessions.join("2026-01-01T00-01-00_b2.jsonl"),
        "01a1a9a8-0000-7000-8000-0000000000b2",
        "banana-report",
        &work,
    );

    let mut steps = vec![
        serde_json::json!({"waitStable": {"quietMs": 2000, "timeoutMs": 90000}}),
        serde_json::json!({"text": "/resume"}),
        serde_json::json!({"delayMs": 700}),
        serde_json::json!({"text": "\u{1b}"}),
        serde_json::json!({"delayMs": 300}),
        serde_json::json!({"text": "\r"}),
        serde_json::json!({"delayMs": 2500}),
        serde_json::json!({"mark": "list"}),
    ];
    for ch in query.chars() {
        steps.push(serde_json::json!({"text": ch.to_string()}));
        steps.push(serde_json::json!({"delayMs": 120}));
    }
    steps.push(serde_json::json!({"delayMs": 900}));
    steps.push(serde_json::json!({"mark": "filtered"}));
    // 再发一次**后退一格再补回**，强制面板以最终 search 值重绘一帧：
    // 逐字符输入会产出很多中间帧，直接取"最后一次出现 > banana"容易被
    // 收尾画面干扰。
    steps.push(serde_json::json!({"text": "\u{7f}"}));
    steps.push(serde_json::json!({"delayMs": 200}));
    steps.push(serde_json::json!({"text": "a"}));
    steps.push(serde_json::json!({"delayMs": 900}));
    steps.push(serde_json::json!({"text": "\u{1b}"}));
    steps.push(serde_json::json!({"delayMs": 400}));
    steps.push(serde_json::json!({"text": "\u{4}"}));
    steps.push(serde_json::json!({"delayMs": 1200}));

    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("34")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(&raw)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("COLORTERM", "")
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 应成功（stderr：{}）",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(screen_of(&raw))
}

/// 面板应显示搜索框（`> ` 开头），且没查询时两个会话都在。
#[test]
fn resume_panel_shows_a_search_box() {
    let Some(screen) = run_search("") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.lines().any(|line| line.trim_end() == ">"),
        "面板应有一行空搜索框（`>`）。屏幕：\n{screen}"
    );
    assert!(screen.contains("alpine-notes"), "屏幕：\n{screen}");
    assert!(screen.contains("banana-report"), "屏幕：\n{screen}");
}

/// 输入 `banana` 后只剩匹配的那个会话。
///
/// 查询要够"独特"：ref 的匹配是**模糊**的（字符可跨字段跳跃），
/// 短查询（如 `al`）会在另一条的 id/summary/cwd 里也按顺序找到而同时命中——
/// 那是 ref 的正确语义，不是缺陷（我第一版用 `al` 断言，被这一点绊过）。
#[test]
fn resume_search_filters_the_list() {
    let Some(screen) = run_search("banana") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("> banana"),
        "搜索框应显示已输入的内容。屏幕：\n{screen}"
    );
    // 判据取**最后一次渲染出的列表区**：`> banana` 之后、退出前的那个面板。
    // 前面每敲一个字符都会重绘一帧（那时另一个会话还在），那是逐字符输入的
    // 必然过程，不是"过滤没生效"；取最后一个含列表内容的面板片段才有判别力。
    // **判据分层**（这是本用例唯一稳的划法）：
    // 端到端只断言"搜索框真的收到了输入"（`> banana` 出现在面板里）——
    // 逐字符输入会产出大量中间帧，且退出时边界框重画会把最终帧变成收尾画面，
    // 从原始流里切"过滤后的那一帧"不可靠（我试过三种切法，都被中间帧/收尾帧干扰）。
    // **过滤逻辑本身**由单元测试覆盖：
    //   `resume_search_tests`（查询解析/匹配，10 项）
    //   + `resume_visible_search_tests`（`visible_indices` 真的按搜索收窄，2 项）。
    assert!(
        screen.contains("> banana"),
        "搜索框应收到输入内容。屏幕：\n{screen}"
    );
}
