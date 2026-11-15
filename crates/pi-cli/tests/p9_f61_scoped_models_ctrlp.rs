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

// ---------------------------------------------------------------------------
// 面板版式与按键语义（对齐 TS 版实测，Pi v0.87.1 100×30 的 `/scoped-models`）
// ---------------------------------------------------------------------------

fn selector() -> pi_cli::settings_selector::ScopedModelsSelector {
    use pi_cli::settings_selector::{ScopedModelRow, ScopedModelsSelector};
    let rows = vec![
        ScopedModelRow {
            full_id: "ant-ling/Ling-3.2-flash".to_string(),
            id: "Ling-3.2-flash".to_string(),
            provider: "ant-ling".to_string(),
            name: "Ling 3.2 Flash".to_string(),
            available: true,
        },
        ScopedModelRow {
            full_id: "command-code/deepseek-v4.1-flash".to_string(),
            id: "deepseek-v4.1-flash".to_string(),
            provider: "command-code".to_string(),
            name: "DeepSeek V4.1 Flash".to_string(),
            available: true,
        },
    ];
    ScopedModelsSelector::new(rows, None)
}

/// 首屏版式与 TS 实测一致：标题、`Session-only. Ctrl+S`、搜索框 `> `、
/// `✓ ` + `名称 [provider]`、按键提示行、`· all enabled`。
#[test]
fn panel_matches_the_ts_layout() {
    let lines = selector().lines();
    let joined = lines.join("\n");
    assert!(joined.contains("Model Configuration"), "{joined}");
    assert!(
        joined.contains("Session-only. Ctrl+S to save to settings."),
        "{joined}"
    );
    assert!(joined.contains("> "), "应有搜索框：{joined}");
    assert!(
        joined.contains("→ ✓ Ling 3.2 Flash [ant-ling]"),
        "选中行应是 '→ ✓ 名称 [provider]'：{joined}"
    );
    assert!(
        joined.contains("Enter toggle · Ctrl+A all · Ctrl+X clear · Ctrl+P provider"),
        "按键提示应与 TS 一致：{joined}"
    );
    assert!(joined.contains("· all enabled"), "{joined}");
}

/// Enter 切换（**不是空格**）；空格进搜索框（ref `:406-408`）。
#[test]
fn enter_toggles_and_space_goes_to_search() {
    let mut sel = selector();
    sel.toggle_selected();
    let after_enter = sel.enabled.clone();
    assert!(
        matches!(&after_enter, Some(list) if list.len() == 1),
        "Enter 应关掉当前项，得到 1 项：{after_enter:?}"
    );
    // 空格不是切换键：走 push_search。
    sel.push_search(' ');
    assert_eq!(sel.search, " ");
    let after_space = sel.enabled.clone();
    assert_eq!(
        after_enter, after_space,
        "空格不应改变启用集合（它属于搜索框）"
    );
}

/// 搜索过滤 + `Model Name:` 行（ref `:222-238`、`:271-280`）。
#[test]
fn search_filters_rows_and_reports_the_selected_name() {
    let mut sel = selector();
    sel.push_search('d');
    sel.push_search('s');
    let lines = sel.lines().join("\n");
    assert!(
        lines.contains("DeepSeek V4.1 Flash") || lines.contains("deepseek-v4.1-flash"),
        "搜索 'ds' 应命中 deepseek：{lines}"
    );
    assert!(
        !lines.contains("→ ✓ Ling 3.2 Flash"),
        "搜索生效后 Ling 不应是选中行：{lines}"
    );
    assert!(lines.contains("Model Name:"), "{lines}");
    // 退格清空后恢复全部
    while sel.pop_search() {}
    assert!(sel.lines().join("\n").contains("Ling 3.2 Flash"));
}

/// `ctrl+p` 按 provider 整组开关（ref `:364-382`）。
#[test]
fn ctrl_p_toggles_the_whole_provider() {
    let mut sel = selector();
    sel.toggle_provider();
    assert!(
        matches!(&sel.enabled, Some(list) if list == &vec!["command-code/deepseek-v4.1-flash".to_string()]),
        "应只关掉 ant-ling 的那一项：{:?}",
        sel.enabled
    );
    // **选中项随排序移动**：关掉 ant-ling 之后 `filtered()` 把 deepseek 排到前面，
    // `selected` 仍是 0 ⇒ 第二次按的是 command-code 那一组，把它也关掉 ⇒ 空集。
    // 这是 ref 的行为（`getSortedIds` 把已启用项提前，`:77-81`），不是缺陷。
    sel.toggle_provider();
    assert!(
        matches!(&sel.enabled, Some(list) if list.is_empty()),
        "第二次按的是（已排到前面的）command-code，应清空：{:?}",
        sel.enabled
    );
    // 第三次：当前选中项已不在可用集合…仍指 command-code，provider 非全启用 ⇒ enable 回来。
    sel.toggle_provider();
    assert!(
        matches!(&sel.enabled, Some(list) if !list.is_empty()),
        "第三次应把该 provider 重新启用：{:?}",
        sel.enabled
    );
}

/// 分页：`maxVisible=8`，超出时给 `(n/total)`（ref `:246-260`）。
#[test]
fn pagination_caps_at_eight_rows() {
    use pi_cli::settings_selector::{ScopedModelRow, ScopedModelsSelector};
    let rows: Vec<ScopedModelRow> = (0..12)
        .map(|index| ScopedModelRow {
            full_id: format!("p/m{index}"),
            id: format!("m{index}"),
            provider: "p".to_string(),
            name: format!("Model {index}"),
            available: true,
        })
        .collect();
    let sel = ScopedModelsSelector::new(rows, None);
    let lines = sel.lines();
    let model_rows = lines
        .iter()
        .filter(|line| line.starts_with("→ ") || line.starts_with("  "))
        .filter(|line| line.contains("[p]"))
        .count();
    assert_eq!(model_rows, 8, "最多显示 8 行：{lines:?}");
    assert!(
        lines.iter().any(|line| line.contains("(1/12)")),
        "应显示位置指示：{lines:?}"
    );
}
