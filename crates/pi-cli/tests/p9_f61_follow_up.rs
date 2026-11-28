//! F61/30：`app.message.followUp`（`alt+enter`）与 `app.message.dequeue`（`alt+up`）。
//!
//! 改前：`alt+enter` 被映射成**换行**，`alt+up` 完全没有绑定。而 ref 里
//! `core/keybindings.ts:133-139` 把这两个键分别绑定给 followUp / dequeue，
//! `interactive-mode.ts:3011-3012` 用 `defaultEditor.onAction` 注册——动作在编辑器
//! 处理链之前，所以按键的实际效果是"排队跟进消息 / 取回排队消息"，不是换行。
//! 用户按 ref 的提示按键（`/hotkeys` 里就写着）得不到任何效果。
//!
//! 判据用状态行：`dequeue` 在无排队消息时显示 ref 的原文
//! `No queued messages to restore`（`:4349-4351`），有排队时显示
//! `Restored N queued message(s) to editor`（`:4352`）。

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

/// 跑一次，`keys` 是 `PI_TEST_KEYS` 序列（不含结尾的 ctrl+d）。
fn run(keys: &str) -> Option<String> {
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

    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(tmp.path().join("code.txt"))
        .arg("--out")
        .arg(tmp.path().join("raw.bin"))
        .arg("--script")
        .arg({
            let keys_path = tmp.path().join("keys.json");
            std::fs::write(
                &keys_path,
                serde_json::to_string(&serde_json::json!([
                    {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}}
                ]))
                .expect("序列化"),
            )
            .expect("写 keys");
            keys_path
        })
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sessions)
        .env("PI_JITI", std::env::var("PI_JITI").unwrap_or_default())
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("PI_TEST_KEYS", keys)
        .env("PI_TEST_TAIL", "30")
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
    Some(screen_of(&tmp.path().join("raw.bin")))
}

/// `alt+up` 在没有排队消息时应显示 ref 的原文状态行。
#[test]
fn dequeue_without_queue_reports_nothing_to_restore() {
    // `alt+up` 的 xterm 编码是 `\u{1b}[1;3A`（modifyOtherKeys）。
    let Some(screen) = run("alt+up,ctrl+d") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(
        screen.contains("No queued messages to restore"),
        "无排队消息时 `alt+up` 应给 ref 的状态行（interactive-mode.ts:4349-4351）。屏幕：\n{screen}"
    );
}

/// `alt+enter` 在**空闲**时等同 `Enter`：输入的文字应被提交进会话区，
/// 而不是变成编辑器里的换行。
#[test]
fn follow_up_while_idle_submits_like_enter() {
    let Some(screen) = run("char:h,char:i,alt+enter,ctrl+d") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    // 判据要能区分"提交"与"换行"，且不能被退出后的收尾文字干扰：
    // - **提交** ⇒ 文字进会话区 ⇒ 回合真的跑起来了 ⇒ 屏幕上出现用量行
    //   （`x.y%/200k (auto)`，只有提交后才会更新）；
    // - **换行** ⇒ 什么都没提交 ⇒ 用量行仍是初始的 `0.0%`。
    //
    // 用"用量是否变化"判定，比找文字位置稳：文字在两种情形下都在屏幕上。
    assert!(
        screen.contains("hi"),
        "输入的文字应出现在屏幕上。屏幕：\n{screen}"
    );
    let advanced = screen
        .lines()
        .filter_map(|line| {
            let token = line.split('/').next()?;
            let value = token.trim().rsplit(' ').next()?;
            value.trim_end_matches('%').parse::<f64>().ok()
        })
        .any(|percent| percent > 0.0);
    assert!(
        advanced,
        "`alt+enter` 空闲时应**提交**（ref `:4340-4343` 的 'acts like regular Enter'）——\
         提交后回合会跑起来、用量不再是 0.0%。屏幕：\n{screen}"
    );
}
