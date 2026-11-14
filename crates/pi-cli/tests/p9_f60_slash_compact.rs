//! F60/4：TUI 的 `/compact` 必须**真的压缩**并给出可见结果。
//!
//! 真机现象（用户报告）：TUI 里输入 `/compact` 回车**完全没反应**——屏幕一个字节都不变。
//! 取证结论：`TuiAction::Command` 的 `match command_word` 里没有 `"compact"` 分支，
//! 落到 `other => {}`（注释「其余命令尚未实现 ⇒ 不改变屏幕」），于是命令被**静默丢弃**。
//!
//! ref 的行为链（`interactive-mode.ts:3187-3192` → `:6822-6830` → `agent-session.ts:2406-2550`）：
//! ① 清编辑器；② `compaction_start{reason:"manual"}` → 状态行 `Compacting context...`；
//! ③ 摘要请求 → 写 `compaction` 条目；④ `compaction_end` → 清会话区、按
//! `buildContextEntries` 重渲染、末尾追加压缩块（`:3577-3596`）；⑤ 失败 → `showError`。
//!
//! 本用例走**真实 provider 路径**（本地假 provider 回放 SSE，不是桩），
//! 断言的是"发出命令后屏幕在限定步数内出现变化"，不用墙钟阈值。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
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

fn mock_provider_bin() -> Option<PathBuf> {
    // 优先用已经构建好的二进制；没有就跳过（用例本身不负责编译）。
    for profile in ["debug", "release"] {
        let candidate = repo_root()
            .join("target")
            .join(profile)
            .join("mock-provider");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 一个最小的假 provider：对每个请求回放同一段 SSE（含 usage，压缩需要它算 token）。
///
/// 端口由内核分配（`127.0.0.1:0`），避免与真实端口或其他用例冲突；
/// 它与被测进程在同一台机器上，但**不联外网**：`baseUrl` 指向这个 loopback 端口。
fn spawn_mock(listener: TcpListener, body: &'static str) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            // 读完请求头（不解析：所有请求都回同一段响应）。
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    })
}

/// 回放的 SSE：一条带 usage 的 assistant 回复（`after-compact.sse` 同款形状，
/// 再加一个可被当作摘要正文的文本）。
const SSE_BODY: &str = concat!(
    "data: {\"id\":\"chatcmpl-mock\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
    "\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},",
    "\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"chatcmpl-mock\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
    "\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"压缩摘要正文\"},",
    "\"finish_reason\":null}]}\n\n",
    "data: {\"id\":\"chatcmpl-mock\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
    "\"model\":\"mock-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],",
    "\"usage\":{\"prompt_tokens\":12000,\"completion_tokens\":60,\"total_tokens\":12060}}\n\n",
    "data:",
);

/// 合成会话：一问一答（带 usage），够 `prepare_compaction` 给出方案。
fn write_session(path: &Path, cwd: &Path) {
    let header = serde_json::json!({
        "type": "session",
        "id": "s0",
        "parentId": serde_json::Value::Null,
        "timestamp": "2026-01-01T00:00:00Z",
        "cwd": cwd.display().to_string(),
        "version": 3,
    });
    // 必须**多轮**：`keepRecentTokens = 1` 时 cut 点会落在最后一条 user 上，
    // 只有一轮问答时"要摘要的历史"为空 ⇒ `prepare_compaction` 返回
    // `Nothing to compact (session too small)`（实测踩过，压缩测不成）。
    let mut lines = vec![header.to_string()];
    let mut parent = "s0".to_string();
    for turn in 1..=3 {
        let user_id = format!("u{turn}");
        lines.push(
            serde_json::json!({
                "type": "message",
                "id": user_id,
                "parentId": parent,
                "timestamp": "2026-01-01T00:00:00Z",
                "message": { "role": "user", "content": [{ "type": "text", "text": format!("第{turn}问") }] },
            })
            .to_string(),
        );
        let assistant_id = format!("a{turn}");
        lines.push(
            serde_json::json!({
                "type": "message",
                "id": assistant_id,
                "parentId": user_id,
                "timestamp": "2026-01-01T00:00:00Z",
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": format!("第{turn}答") }],
                    "api": "openai-completions",
                    "provider": "p",
                    "model": "m",
                    "usage": {
                        "input": 12000, "output": 60, "cacheRead": 0, "cacheWrite": 0,
                        "totalTokens": 12060,
                        "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 }
                    },
                    "stopReason": "stop",
                    "timestamp": 0,
                },
            })
            .to_string(),
        );
        parent = assistant_id;
    }
    let _ = parent;
    std::fs::write(path, lines.join("\n") + "\n").expect("写会话");
}

fn write_agent(agent: &Path, port: u16) {
    std::fs::create_dir_all(agent).expect("建 agent 目录");
    std::fs::write(
        agent.join("settings.json"),
        // `keepRecentTokens` 取 1：一问一答必然"可压"（ref 的默认 20000 会让小会话
        // 报 `Nothing to compact (session too small)`，那就测不到成功路径了）。
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m",
            "compaction":{"enabled":true,"reserveTokens":1000,"keepRecentTokens":1}}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        format!(
            r#"{{"providers":{{"p":{{"baseUrl":"http://127.0.0.1:{port}/v1","api":"openai-completions",
                "apiKey":"DUMMY","models":[{{"id":"m","name":"m","contextWindow":200000,"maxTokens":1024}}]}}}}}}"#
        ),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");
}

struct Run {
    screen: String,
    /// 会话文件里 `compaction` 条目的条数（=1 表示真的写进去了）。
    compaction_entries: usize,
    exit_code: Option<i32>,
}

/// 在真 pty 里跑 `pi-rs -c`，注入 `/compact` + 回车，然后 Ctrl+D 退出。
fn run_compact() -> Option<Run> {
    let node = node_path()?;
    let _ = mock_provider_bin();
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定 loopback 端口");
    let port = listener.local_addr().expect("本地地址").port();
    let _mock = spawn_mock(listener, SSE_BODY);

    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sessions");
    std::fs::create_dir_all(&home).expect("建 HOME");
    std::fs::create_dir_all(&work).expect("建 work");
    std::fs::create_dir_all(&sess).expect("建 sessions");
    write_agent(&agent, port);
    let session_path = sess.join("2026-01-01T00-00-00-000Z_comp.jsonl");
    write_session(&session_path, &work);

    let keys_path = tmp.path().join("keys.json");
    std::fs::write(
        &keys_path,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"/compact"},{"delayMs":600},{"mark":"typed"},
            {"text":"\r"},{"delayMs":6000},{"mark":"after"},
            {"text":"\u0004"},{"delayMs":1200},{"mark":"exit"}]"#,
    )
    .expect("写 keys");
    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let dump = tmp.path().join("steps");
    std::fs::create_dir_all(&dump).expect("建 dump");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("40")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys_path)
        .arg("--marks")
        .arg(dump.join("marks.jsonl"))
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("-c")
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        // 只驱动真实按键（不用 `PI_TEST_KEYS`：那条路径的逐步 `step_json` 会重渲染整帧，
        // 与本用例要测的"命令是否产生可见变化"无关）。
        .env("COLORTERM", "")
        .env("LANG", "zh_CN.UTF-8")
        .env("LC_ALL", "zh_CN.UTF-8")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    let screen = screen_of(&raw_path);
    let compaction_entries = std::fs::read_to_string(&session_path)
        .map(|text| {
            text.lines()
                .filter(|line| line.contains("\"type\":\"compaction\""))
                .count()
        })
        .unwrap_or(0);
    let exit_code = std::fs::read_to_string(&code_path)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok());
    Some(Run {
        screen,
        compaction_entries,
        exit_code,
    })
}

/// 去 ANSI 的可见屏幕（足够断言文案；不追求与 golden 逐格一致）。
fn screen_of(raw: &Path) -> String {
    let bytes = std::fs::read(raw).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if next == '\u{7}' {
                        break;
                    }
                }
            }
            continue;
        }
        if ch == '\r' || ch == '\n' {
            out.push('\n');
            continue;
        }
        out.push(ch);
    }
    out
}

/// **核心回归**：`/compact` 必须在屏幕上产生可见变化，并把 `compaction` 条目写进会话。
///
/// 改前：命令落到 `other => {}`，屏幕逐字节不变、会话文件里也没有 `compaction` 条目
/// （断言失败文本：`/compact 没有产生任何可见输出`）。
#[test]
fn slash_compact_produces_visible_output_and_writes_the_entry() {
    let Some(run) = run_compact() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        run.screen.contains("Compacting context") || run.screen.contains("[compaction]"),
        "/compact 没有产生任何可见输出（改前即静默丢弃）。屏幕：\n{}",
        run.screen
    );
    assert!(
        run.compaction_entries >= 1,
        "会话文件里没有 compaction 条目（说明命令根本没执行）。屏幕：\n{}",
        run.screen
    );
}

/// 压缩块必须带上 token 数（ref 折叠行 `Compacted from N tokens (ctrl+o to expand)`）。
#[test]
fn slash_compact_shows_the_compaction_block() {
    let Some(run) = run_compact() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        run.screen.contains("[compaction]") && run.screen.contains("Compacted from"),
        "压缩块未渲染。屏幕：\n{}",
        run.screen
    );
    assert!(
        run.screen.contains("tokens"),
        "压缩块应显示 token 数。屏幕：\n{}",
        run.screen
    );
}

/// **与 TS 逐字一致的格式**：ref `compaction-summary-message.ts:36` 用
/// `tokensBefore.toLocaleString()`（千分位分组，`12,060`），不是 footer 的 `12k` 缩写。
///
/// 改前用 `format_tokens` ⇒ `Compacted from 12k tokens`，与 TS 的
/// `Compacted from 12,060 tokens` 不一致（实测 ts.bin 对比得出）。
#[test]
fn slash_compact_formats_tokens_like_to_locale_string() {
    let Some(run) = run_compact() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        run.screen.contains("Compacted from 12,060 tokens"),
        "压缩块的 token 数应按千分位分组（ref 的 toLocaleString）。屏幕：\n{}",
        run.screen
    );
}

/// 压缩完成后进程仍能正常退出（不能因为压缩把主循环卡住）。
#[test]
fn slash_compact_leaves_the_process_responsive() {
    let Some(run) = run_compact() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_eq!(
        run.exit_code,
        Some(0),
        "压缩后 Ctrl+D 应正常退出（进程卡死会让 exit code 缺失）。屏幕：\n{}",
        run.screen
    );
}
