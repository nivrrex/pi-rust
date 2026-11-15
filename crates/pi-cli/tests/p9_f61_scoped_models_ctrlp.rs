//! F61/19：`/scoped-models` 的改动必须**立即**影响本会话的 Ctrl+P
//! （ref `scoped-models-selector.ts:250-270` 的 `onChange` → `session.setScopedModels`）。
//!
//! 上一轮（F61/18）我把判据建在**屏幕文本**上，去掉过滤后用例仍然通过——
//! 因为 Ctrl+P 在那种夹具里没真正切换，页脚一直停在初始模型。这里改用
//! `PI_TEST_DUMP` 写出的 `step-N.json` 里的 `footer.model` 字段：它是**状态层**的
//! 事实，不依赖屏幕刷新时机。
//!
//! 场景：
//! 1. 初始 `enabledModels` 只含 m2，当前模型 m1（不在 scope 内）⇒ 按 Ctrl+P 应落到 m2；
//! 2. 面板里把 m1 也启用（空格）后按 Ctrl+P ⇒ **m1 成为候选**，循环里能回到 m1。
//!
//! 判据：`step-N.json` 的 `footer.model` 序列。

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

/// 跑一次场景，返回每步的 `footer.model`（按 step 顺序）。
fn run(keys: &str, settings: &str) -> Option<Vec<String>> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sessions = tmp.path().join("sessions");
    let steps = tmp.path().join("steps");
    for dir in [&agent, &home, &work, &sessions] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    std::fs::write(agent.join("settings.json"), settings).expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m1","name":"m1","contextWindow":200000,"maxTokens":1024},{"id":"m2","name":"m2","contextWindow":200000,"maxTokens":1024}]}}}"#,
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
        .arg("120")
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
        .env("PI_TEST_DUMP", &steps)
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
    let mut out = Vec::new();
    for index in 0..32 {
        let path = steps.join(format!("step-{index}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            break;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(model) = value
            .get("footer")
            .and_then(|footer| footer.get("model"))
            .and_then(serde_json::Value::as_str)
        {
            out.push(model.to_string());
        }
    }
    Some(out)
}

/// 只启用 m2 时，Ctrl+P 从 m1 切到 **m2**（m1 不在 scope 内）。
///
/// 这条先证明"过滤确实生效于 Ctrl+P"（上一轮缺的正是这个前提）。
#[test]
fn ctrl_p_respects_the_disabled_scope() {
    // 注意：`enabledModels` 是**启动时的 scope**，ref 会用它覆盖 `defaultModel`
    // （scope 里第一个模型成为当前模型）。所以这里 `defaultModel=m2` +
    // `enabledModels=[p/m1]` ⇒ 启动就是 m1，且 Ctrl+P 的候选**只有 m1**，
    // 按多少次都不应离开 m1。
    let Some(models) = run(
        "ctrl+p,ctrl+p",
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m2","enabledModels":["p/m1"]}"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    assert!(!models.is_empty(), "应至少有一份快照，实际 {models:?}");
    assert_eq!(
        models[0], "m1",
        "scope 里只有 p/m1 ⇒ 启动模型应是 m1（覆盖 defaultModel=m2）"
    );
    assert!(
        models.iter().all(|model| model == "m1"),
        "候选只有 m1 时 Ctrl+P 不应离开 m1。实际序列：{models:?}"
    );
}

/// 面板里把 m1 也启用后，Ctrl+P 能在 m1↔m2 之间循环（说明改动即时生效）。
#[test]
fn editing_the_scope_immediately_affects_ctrl_p() {
    // 只启用 m2 → 打开面板 → 空格启用 m1 → Esc → Ctrl+P ×2
    // 若改动未即时生效，候选仍只有 m2 ⇒ Ctrl+P 停在 m2，不会出现 m1。
    // 按键序列（`PI_TEST_KEYS` 按顺序注入，每步前都等静止）：
    // `/scoped-models` → **Esc 关补全菜单** → 回车提交 → 空格外加？
    //
    // 初始 scope 只含 m2 ⇒ 当前模型 m2、面板里 m2 已勾选、m1 未勾选。
    // 面板默认选中第一项（m2，已勾选）⇒ 先按 ↓ 移到 m1 再空格启用它。
    let Some(models) = run(
        "type:/scoped-models,escape,enter,down,space,escape,ctrl+p,ctrl+p",
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m1","enabledModels":["p/m2"]}"#,
    ) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let back_to_m1 = models.iter().any(|model| model == "m1");
    assert!(
        back_to_m1,
        "面板启用 m1 之后，Ctrl+P 应能回到 m1（改动即时生效）。实际序列：{models:?}"
    );
}
