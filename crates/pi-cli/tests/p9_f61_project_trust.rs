//! F61/39：`project_trust` 事件（ref `project-trust.ts:46-96`、`runner.ts:292-322`）。
//!
//! 此前 pi-rs 完全不派发它，且信任裁决只读 `trust.json`——缺少 ref 的完整顺序：
//! ① 显式覆盖 ② 无受控资源即可信 ③ **扩展裁决** ④ `trust.json`
//! ⑤ `defaultProjectTrust` ⑥ ask（无 UI ⇒ 不可信）。
//!
//! 后果：扩展**无法参与信任决策**，`defaultProjectTrust` 设置被完全忽略。
//!
//! 三个场景必须**串行**（一个 `#[test]` 内依次跑）：扩展宿主是进程级单例，
//! 多个 `#[test]` 并行会互相抢句柄（实测 `first_definitive_verdict_stops_the_chain`
//! 在全量并行时失败）。这与 `p9_f1_tool_call_input`、`p9_f61_before_tree` 同一教训。

use std::path::Path;

fn node_path() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| std::path::PathBuf::from(text))
}

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn install_probe(agent: &Path, fixture: &str) {
    let target = agent.join("extensions/project-trust");
    std::fs::create_dir_all(&target).expect("建目录");
    let body = std::fs::read_to_string(repo_root().join("tools/ext-host/fixtures").join(fixture))
        .expect("读 fixture");
    std::fs::write(target.join("index.mjs"), body).expect("写扩展");
}

/// 装 fixture、重启宿主、派发一次 `project_trust`。
fn probe(fixture: &str) -> Option<Option<pi_core::trust::TrustVerdict>> {
    let node = node_path()?;
    let agent = tempfile::tempdir().expect("agent");
    install_probe(agent.path(), fixture);
    std::env::set_var("PI_CODING_AGENT_DIR", agent.path());
    std::env::set_var("PI_NODE", node);
    let cwd = std::env::current_dir().expect("cwd");
    pi_cli::ext_tools::configure_ctx(&cwd, "tui", true);
    let _tools = pi_cli::print_mode::default_tools(&pi_cli::args::Args::default());
    pi_cli::ext_tools::reload_host(agent.path(), &cwd).expect("重启宿主");
    Some(pi_cli::ext_tools::emit_project_trust(&cwd))
}

/// 三个场景串行验证（扩展宿主是进程级单例，不能并行）。
#[test]
fn project_trust_semantics() {
    // ① 扩展裁决 `yes` ⇒ 带回 `trusted` 与 `remember`。
    let Some(yes) = probe("project-trust-yes-ext.mjs") else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let yes = yes.expect("扩展给出 yes 时应返回裁决");
    assert!(yes.trusted, "应裁决为信任：{yes:?}");
    assert!(yes.remember, "应带回 remember=true：{yes:?}");

    // ② 只给 `undecided` ⇒ 不裁决（返回 `None`，调用方继续走后续规则）。
    let undecided = probe("project-trust-undecided-ext.mjs").expect("node 在");
    assert!(
        undecided.is_none(),
        "undecided 不应被当成裁决：{undecided:?}"
    );

    // ③ 第一个 `no`、第二个 `undecided` ⇒ 采纳 **`no`**（遇到 yes/no 就停）。
    //
    // 这是专用分支的真正价值：ref `runner.ts:292-322` 遇到第一个 `yes`/`no`
    // **立即返回**。若改用通用的"合并所有返回值"，后一个 `undecided` 会覆盖前一个
    // `no` ⇒ 宿主按"不裁决"处理，一个明确表达"不信任"的扩展被静默忽略。
    let stopped = probe("project-trust-two-ext.mjs")
        .expect("node 在")
        .expect("第一个 handler 应给出裁决");
    assert!(!stopped.trusted, "应采纳第一个 handler 的 no：{stopped:?}");
    assert!(
        stopped.remember,
        "应带回第一个 handler 的 remember：{stopped:?}"
    );
}
