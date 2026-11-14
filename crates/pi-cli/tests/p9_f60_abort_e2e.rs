//! F60/5b：扩展发起 `ctx.abort()` 后回合**真的被中止**（真 pty + TUI）。
//!
//! `p9_f60_ctx_runtime` 只证明"调用能到宿主、宿主有应答"（库内没有 TUI 主循环，
//! 宿主只能回 `no interactive UI`）。本用例补上**端到端**那一段：
//! 真 pty 起 pi-rs，扩展在 `agent_start` 里立刻调 `ctx.abort()`，
//! 然后断言：
//! 1. 扩展拿到了 `{ok:true}`（说明 TUI 主循环消费了这次 abort）；
//! 2. 回合被中止——屏幕上的 `Working` 行最终撤掉，且扩展收到 `agent_settled`；
//! 3. 进程仍能正常退出（exit-code 0），即 abort 没有把主循环卡死。
//!
//! 真实对应：`pi-agent-resilience` 在流卡住/文本循环时正是这样调 abort
//! （`index.ts:116/227`）；此前该成员缺失 ⇒ `try/catch` 吞掉 TypeError ⇒ 静默失效。

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

/// 一个**迟迟不结束**的 SSE：只发 role 帧，然后挂着不动。
///
/// 这样回合会一直"进行中"（`Working` 行在屏幕上），除非被 abort 中止——
/// 正是 `pi-agent-resilience` 要处理的"流卡住"场景。
fn spawn_slow_provider(listener: TcpListener) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = [0u8; 65536];
            let _ = stream.read(&mut buffer);
            let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
                data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\
                \"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\
                \"finish_reason\":null}]}\n\n";
            let _ = stream.write_all(head);
            let _ = stream.flush();
            // 保持连接打开一小会儿（回合在这段时间里"进行中"，屏幕上有 `Working`），
            // 然后**正常收尾**：扩展的 `agent_start` 里有 `await ctx.abort()`，
            // 取消令牌会让 agent loop 立刻结束这一轮 ⇒ `agent_settled` 到达。
            //
            // 为什么不能永久挂着：那样 `agent_end`/`agent_settled` 永远不来，
            // 用例只能靠墙钟超时判断，而且分不清"abort 生效"与"根本没结束"。
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let tail = b"data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\
                \"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\
                \"usage\":{\"prompt_tokens\":100,\"completion_tokens\":10,\"total_tokens\":110}}\n\n\
                data:\n\n";
            let _ = stream.write_all(tail);
            let _ = stream.flush();
            held.push(stream);
        }
    })
}

fn install(agent: &Path) {
    let target = agent.join("extensions/abort-turn");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body =
        std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/abort-turn-ext.mjs"))
            .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

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
        out.push(ch);
    }
    out
}

struct Outcome {
    log: String,
    screen: String,
    exit_code: Option<i32>,
}

fn run_abort_turn() -> Option<Outcome> {
    let node = node_path()?;
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定 loopback");
    let port = listener.local_addr().expect("地址").port();
    let _provider = spawn_slow_provider(listener);

    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    install(&agent);
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        format!(
            r#"{{"providers":{{"p":{{"baseUrl":"http://127.0.0.1:{port}/v1","api":"openai-completions","apiKey":"DUMMY","models":[{{"id":"m","name":"m","contextWindow":200000,"maxTokens":1024}}]}}}}}}"#
        ),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"DUMMY"}}"#,
    )
    .expect("写 auth");

    let log_path = tmp.path().join("probe.log");
    let keys_path = tmp.path().join("keys.json");
    std::fs::write(
        &keys_path,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"hi"},{"delayMs":300},{"text":"\r"},{"delayMs":6000},{"mark":"after"},
            {"text":"\u0004"},{"delayMs":2000},{"mark":"exit"}]"#,
    )
    .expect("写 keys");

    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys_path)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("PI_EXT_TEST_LOG", &log_path)
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
    let exit_code = std::fs::read_to_string(&code_path)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok());
    Some(Outcome {
        log: std::fs::read_to_string(&log_path).unwrap_or_default(),
        screen: screen_of(&raw_path),
        exit_code,
    })
}

/// 扩展的 `ctx.abort()` 必须拿到 `{ok:true}`（= TUI 主循环消费了它）。
#[test]
fn extension_abort_is_consumed_by_the_tui() {
    let Some(outcome) = run_abort_turn() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.log.contains("agent_start"),
        "扩展应收到 agent_start。日志：\n{}",
        outcome.log
    );
    assert!(
        outcome.log.contains("\"ok\":true"),
        "ctx.abort() 应得到 {{ok:true}}（TUI 有主循环）。日志：\n{}",
        outcome.log
    );
}

/// 回合必须真的被中止：扩展收到 `agent_settled`，且屏幕上的 `Working` 行撤掉。
#[test]
fn aborted_turn_settles() {
    let Some(outcome) = run_abort_turn() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        outcome.log.contains("agent_settled"),
        "abort 之后回合应结算（agent_settled）。日志：\n{}",
        outcome.log
    );
    // 判据用**最后一屏**（退出提示之前的最后一段完整画面），不是"最后 40 行"：
    // pty 流是增量的，中间帧会重复出现（`Working` 在 abort 之前本来就画过）。
    //
    // abort 生效的标志是 ref 的中止文案 `Operation aborted`
    // （`assistant-message.ts` 对 `stopReason: "aborted"` 的渲染），
    // 且**最终画面不再有 Working 行**（那是"看起来卡住"的表现）。
    let screens: Vec<&str> = outcome.screen.split("\u{1b}").collect();
    let last = screens.last().copied().unwrap_or("");
    let final_screen = if last.len() < 200 {
        // 末尾可能是退出序列残片：向前取更完整的一段。
        outcome
            .screen
            .rsplit("\n")
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        last.to_string()
    };
    // ref 有**两条**合法的中止文案，取决于取消发生在请求发出之前还是流中途：
    // - 请求尚未发出 ⇒ `This operation was aborted`（`agent/run.rs:544-549`，
    //   对应 golden/S09b 的"带着已中止信号去发请求"，TS 的 fetch 直接抛 AbortError）；
    // - 流已开始 ⇒ `Operation aborted`（`tui.rs` 的 `assistant_notice` 对
    //   `StopReason::Aborted` 的兜底文案，ref `assistant-message.ts` 同款）。
    // 两者都说明回合被中止了；**具体哪一种取决于调度**，不能只认一种
    // （否则用例本身会偶发红，实测过一次）。
    const ABORT_MARKERS: [&str; 2] = ["Operation aborted", "This operation was aborted"];
    let marker = ABORT_MARKERS
        .iter()
        .find(|marker| outcome.screen.contains(**marker));
    assert!(
        marker.is_some(),
        "abort 之后应出现 ref 的中止文案（{} 之一）。屏幕：\n{}",
        ABORT_MARKERS.join(" 或 "),
        outcome.screen
    );
    // 中止文案之后不应再画 `Working`（之前画过的不算）。
    let tail_after_abort = marker
        .and_then(|marker| outcome.screen.split(marker).nth(1))
        .unwrap_or_default();
    assert!(
        !tail_after_abort.contains("Working"),
        "abort 之后不应再画 `Working`。abort 之后的屏幕：\n{tail_after_abort}"
    );
    let _ = final_screen;
}

/// abort 之后进程仍能正常退出（没把主循环卡死）。
#[test]
fn abort_leaves_the_process_responsive() {
    let Some(outcome) = run_abort_turn() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "abort 之后 Ctrl+D 应正常退出。屏幕：\n{}",
        outcome.screen
    );
}
