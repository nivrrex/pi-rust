//! F60/6：`pi.registerMarkdownTransformer` 必须真的生效。
//!
//! 用户已装扩展 `pi-secrets` 用它在**渲染前**把命中的密钥替换成掩码
//! （`src/index.ts:326`：`pi.registerMarkdownTransformer((md) => redactText("markdown", md))`），
//! 目的是让密钥**不进终端 scrollback**——它自己的注释写明「message_end/tool_result
//! 之前就已经流式渲染，必须有自己的钩子」。
//!
//! 宿主此前只 `void transformer;`（记录但不应用）⇒ 脱敏**完全失效**：密钥会原样
//! 画到屏幕上。这不是功能缺失，是**信息泄露**，因此本批实现并加了用例。
//!
//! 覆盖两层：
//! 1. 库内：注册计数上报 + `ext.markdown.transform` 往返（变换真的发生）；
//! 2. 真 pty：助手回复里的密钥在**屏幕上**被替换（`pi-secrets` 的实际用途）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SECRET: &str = "SECRET-abc123";
const MASK: &str = "[REDACTED]";

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

fn install_probe(agent: &Path) {
    let target = agent.join("extensions/md-transform");
    std::fs::create_dir_all(&target).expect("建扩展目录");
    let body =
        std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/md-transform-ext.mjs"))
            .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 库内验证：扩展注册后，宿主侧的计数 > 0，且变换往返把密钥换掉。
#[test]
fn markdown_transformer_is_registered_and_applied() {
    let Some(_node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path());
    let log = agent.path().join("probe.log");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node_path().expect("node"));
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    pi_cli::ext_tools::configure_ctx(agent.path(), "tui", true);
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000c6".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();

    // 握手是异步完成的，等计数变成 1（最多 6 s）。
    let mut count = 0usize;
    for _ in 0..60 {
        count = pi_cli::ext_tools::markdown_transformer_count();
        if count > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        count > 0,
        "扩展注册的 Markdown 变换器没有被宿主记录（改前恒为 0）"
    );

    let input = format!("key is {SECRET} here");
    let output = pi_cli::ext_tools::transform_markdown(&input, 80);
    assert!(
        !output.contains(SECRET),
        "变换后仍含密钥（脱敏没生效）：{output:?}"
    );
    assert!(output.contains(MASK), "变换后应出现掩码：{output:?}");
    // 探针也应记录到 width 参数（ref `types.ts:1304` 的 context）。
    let body = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        body.contains("transform:in="),
        "扩展的变换器应被真正调用过。日志：\n{body}"
    );
}

/// 真 pty：助手回复里的密钥在**屏幕上**必须被替换。
///
/// 这是 `pi-secrets` 的实际用途——密钥不能进终端 scrollback。
#[test]
fn assistant_reply_is_redacted_on_screen() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定 loopback");
    let port = listener.local_addr().expect("地址").port();
    let body = format!(
        concat!(
            "data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
            "\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"\"}},",
            "\"finish_reason\":null}}]}}\n\n",
            "data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
            "\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"the key is {secret}\"}},",
            "\"finish_reason\":null}}]}}\n\n",
            "data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,",
            "\"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],",
            "\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}}}\n\n",
            "data:\n\n"
        ),
        secret = SECRET
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = [0u8; 65536];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    for dir in [&home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    install_probe(&agent);
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

    let keys_path = tmp.path().join("keys.json");
    std::fs::write(
        &keys_path,
        r#"[{"waitStable":{"quietMs":1500,"timeoutMs":60000}},{"mark":"s0"},
            {"text":"hi"},{"delayMs":300},{"text":"\r"},{"delayMs":6000},{"mark":"after"},
            {"text":"\u0004"},{"delayMs":1500},{"mark":"exit"}]"#,
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
        .env("PI_EXT_TEST_LOG", tmp.path().join("probe.log"))
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

    let screen = screen_of(&raw_path);
    assert!(
        !screen.contains(SECRET),
        "屏幕上出现了未脱敏的密钥（信息泄露）。屏幕：\n{screen}"
    );
    assert!(
        screen.contains(MASK),
        "屏幕上应出现掩码 {MASK}（证明变换器在真实渲染路径上生效）。屏幕：\n{screen}"
    );
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
