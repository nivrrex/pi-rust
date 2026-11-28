//! F61/21：`/trust` 命令（ref `interactive-mode.ts:5166-5180` + `trust-selector.ts`）。
//!
//! 为什么这条命令有实际意义：它决定 `<cwd>/.pi/{settings.json,extensions,skills,...}`
//! 是否生效（ref `resource-loader.ts:1029/1043`），也是扩展 `ctx.isProjectTrusted()`
//! 的取值来源——已装扩展 `pi-ai-summary` 用它决定要不要读项目级 settings。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use pi_cli::settings_selector::TrustSelector;
use pi_core::trust::{
    normalize_path, project_trust_options, TrustDecision, TrustStore, TrustUpdate,
};

fn store(dir: &Path) -> TrustStore {
    TrustStore::new(dir)
}

/// 面板默认选中"与已保存决定匹配"的项（ref `:45-48`）。
#[test]
fn selector_preselects_the_saved_decision() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let cwd = tmp.path().join("proj");
    std::fs::create_dir_all(&cwd).expect("建目录");
    let store = store(tmp.path());
    store
        .set_many(&[TrustUpdate {
            path: cwd.clone(),
            decision: Some(TrustDecision::Untrusted),
        }])
        .expect("写入");
    let saved = Some((normalize_path(&cwd), false));
    let sel = TrustSelector::new(&cwd, saved, false);
    let lines = sel.lines().join("\n");
    assert!(lines.contains("Project trust"), "{lines}");
    assert!(lines.contains("Saved decision: untrusted"), "{lines}");
    assert!(lines.contains("Current session: untrusted"), "{lines}");
    assert!(lines.contains("✓ Do not trust"), "已保存项应带 ✓：{lines}");
    assert!(
        lines.contains("→ ✓ Do not trust"),
        "默认应选中已保存的那一项：{lines}"
    );
}

/// `Saved decision: none` + 默认选第一项（Trust）。
#[test]
fn fresh_project_shows_none_and_selects_trust() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let cwd = tmp.path().join("fresh");
    std::fs::create_dir_all(&cwd).expect("建目录");
    let sel = TrustSelector::new(&cwd, None, false);
    let lines = sel.lines().join("\n");
    assert!(lines.contains("Saved decision: none"), "{lines}");
    assert!(lines.contains("→   Trust"), "应选中第一项：{lines}");
}

/// 导航：↓/`j` 下移、↑/`k` 上移，且不下溢/溢出。
#[test]
fn navigation_is_bounded() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let cwd = tmp.path().join("p");
    std::fs::create_dir_all(&cwd).expect("建目录");
    let mut sel = TrustSelector::new(&cwd, None, false);
    let len = sel.options.len();
    assert!(len >= 3, "至少 Trust / Trust parent / Do not trust：{len}");
    sel.move_up();
    assert_eq!(sel.selected, 0, "已在顶部时上移无效");
    for _ in 0..len + 3 {
        sel.move_down();
    }
    assert_eq!(sel.selected, len - 1, "下移应停在最后一项");
    sel.move_up();
    assert_eq!(sel.selected, len - 2);
}

/// 选 `Trust` 保存后，`trust.json` 里有该 cwd 的 true；`get()` 从子目录也能继承。
#[test]
fn saving_trust_writes_the_store() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let cwd = tmp.path().join("proj");
    std::fs::create_dir_all(cwd.join("sub")).expect("建目录");
    let store = store(tmp.path());
    let options = project_trust_options(&cwd, false);
    let trust = options.first().expect("有 Trust 项");
    store.set_many(&trust.updates).expect("写入");
    assert_eq!(store.get(&cwd).expect("查"), Some(TrustDecision::Trusted));
    assert_eq!(
        store.get(&cwd.join("sub")).expect("查子目录"),
        Some(TrustDecision::Trusted),
        "子目录应继承父目录的信任决定"
    );
    // 文件形状：排序 + 尾随换行（ref `writeTrustFile`）
    let text = std::fs::read_to_string(store.path()).expect("读");
    assert!(text.ends_with('\n'), "{text:?}");
    assert!(text.contains("\"true\"") || text.contains("true"), "{text}");
}

/// 选 `Trust parent folder` 保存后：父目录 true、本目录条目**清除**（ref `:79-80`）。
#[test]
fn trusting_the_parent_clears_the_local_entry() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let cwd = tmp.path().join("outer/inner");
    std::fs::create_dir_all(&cwd).expect("建目录");
    let store = store(tmp.path());
    // 先给本目录一个相反决定
    store
        .set_many(&[TrustUpdate {
            path: cwd.clone(),
            decision: Some(TrustDecision::Untrusted),
        }])
        .expect("写入");
    assert_eq!(store.get(&cwd).expect("查"), Some(TrustDecision::Untrusted));
    let options = project_trust_options(&cwd, false);
    let parent_option = options
        .iter()
        .find(|option| option.label.starts_with("Trust parent folder"))
        .expect("有父目录选项");
    store.set_many(&parent_option.updates).expect("写入");
    // 本目录条目被删除 ⇒ 回落到父目录的 true
    assert_eq!(
        store.get(&cwd).expect("查"),
        Some(TrustDecision::Trusted),
        "本目录条目应被清除、继承父目录的 true"
    );
    let text = std::fs::read_to_string(store.path()).expect("读");
    assert!(
        !text.contains(&normalize_path(&cwd)) || text.contains("null"),
        "本目录不应残留 false 条目：{text}"
    );
}

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

/// 端到端：`/trust` → 面板出现 → ↓ 选 `Do not trust` → Enter → `trust.json` 里是 false。
#[test]
fn trust_command_writes_a_decision_end_to_end() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
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

    // `/trust` → Esc 关补全 → 回车 → ↓ 到 Do not trust → 回车保存
    let steps = serde_json::json!([
        {"waitStable": {"quietMs": 2000, "timeoutMs": 90000}},
        {"text": "/trust"},
        {"delayMs": 600},
        {"text": "\u{1b}"},
        {"delayMs": 300},
        {"text": "\r"},
        {"delayMs": 2000},
        {"mark": "panel"},
        {"text": "\u{1b}[B"},
        {"delayMs": 400},
        {"text": "\u{1b}[B"},
        {"delayMs": 400},
        {"text": "\r"},
        {"delayMs": 1200},
        {"text": "\u{1b}"},
        {"delayMs": 600},
        {"text": "\u{4}"},
        {"delayMs": 1200}
    ]);
    let keys = tmp.path().join("keys.json");
    std::fs::write(&keys, serde_json::to_string(&steps).expect("序列化")).expect("写 keys");
    let raw = tmp.path().join("raw.bin");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("32")
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
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("运行 pty-helper");
    assert!(
        output.status.success(),
        "pty-helper 应成功（stderr：{}）",
        String::from_utf8_lossy(&output.stderr)
    );
    let screen = screen_of(&raw);
    assert!(
        screen.contains("Project trust"),
        "/trust 应打开面板。屏幕：\n{screen}"
    );
    let trust_file = agent.join("trust.json");
    let text = std::fs::read_to_string(&trust_file).unwrap_or_default();
    assert!(!text.is_empty(), "应写出 trust.json。屏幕：\n{screen}");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("trust.json 应是合法 JSON");
    let work_key = normalize_path(&work);
    assert_eq!(
        parsed.get(&work_key).and_then(serde_json::Value::as_bool),
        Some(false),
        "选 Do not trust 后 `{work_key}` 应为 false。trust.json：\n{text}"
    );
}

fn screen_of(raw: &Path) -> String {
    let bytes = std::fs::read(raw).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let mut out = String::new();
    let mut rest = text.as_ref();
    // 去 CSI/OSC，保留可见字符与换行（够断言用）。
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

/// `ctx.isProjectTrusted()` 必须反映 `trust.json` 的真实决定（而不是恒 `false`）。
///
/// 为什么这条重要：扩展 `pi-ai-summary` 用它决定是否读**项目级** settings
/// （`index.ts:33/52`）。此前宿主把它硬编码成 `false`，于是该扩展的项目配置
/// 永久静默失效——用户改了项目 settings 却没有效果，也没有任何提示。
#[test]
fn ctx_is_project_trusted_reflects_the_store() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let agent = tempfile::tempdir().expect("agent");
    let ext = agent.path().join("extensions/probe");
    std::fs::create_dir_all(&ext).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures/abort-ext.mjs"))
        .expect("读 fixture");
    std::fs::write(ext.join("index.mjs"), body).expect("写扩展");

    // **受信任**的项目：写入 cwd 的 true 条目。
    let cwd = std::env::current_dir().expect("cwd");
    let trusted = TrustStore::new(agent.path());
    trusted
        .set_many(&[TrustUpdate {
            path: cwd.clone(),
            decision: Some(TrustDecision::Trusted),
        }])
        .expect("写 trust.json");

    let log = agent.path().join("probe.log");
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", &node);
    std::env::set_var("PI_EXT_TEST_LOG", &log);
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    // 与 `main.rs` 同一条路径：把决定写进 ctx（宿主读 `PI_EXT_CTX`）。
    let trusted_value = TrustStore::new(agent.path())
        .get(&cwd)
        .expect("读")
        .is_some_and(|decision| decision == TrustDecision::Trusted);
    assert!(trusted_value, "前置条件：cwd 应受信任");
    pi_cli::ext_tools::update_ctx_project_trusted(trusted_value);
    pi_cli::ext_tools::set_session_id(Some("01a1a9a8-0000-7000-8000-0000000000f7".to_string()));
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::flush_pending_session_start();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&log) {
            if text.contains("value:isProjectTrusted=") {
                assert!(
                    text.contains("value:isProjectTrusted=true"),
                    "trust.json 里 cwd=true 时，ctx.isProjectTrusted() 应为 true。日志：\n{text}"
                );
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("探针日志未出现 isProjectTrusted 的值（{}）", log.display());
}
