//! F47/W2：`-c` 恢复会话时，扩展可见的 `ctx.getContextUsage()` 与 `(provider)` 前缀必须成立。
//!
//! 真机证据：`-c` 后扩展页脚显示 `0.0%/197k`（TS `63.0%/197k`）、右侧 `auto`（TS `(mw-auto) auto`）。
//! 根因：恢复分支只补了 `ctx.model`（F46），没有把**会话里的用量**写进 `ctx.usage`，
//! 于是 `ctx.getContextUsage()` 一直是启动时的占位 `{tokens:0, percent:0.0}`
//! （ref `agent-session.ts:3858-3895` 的 `getContextUsage()` 是从会话分支重算的）；
//! 同理 `availableProviderCount` 也只在新建会话分支设过，扩展页脚拿不到 ⇒ 缺 `(provider)` 前缀
//! （ref `footer.ts:193-200` + `interactive-mode.ts:5091-5099`）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

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

fn prepare(agent: &Path, home: &Path, work: &Path, sess: &Path) {
    std::fs::create_dir_all(agent.join("extensions/ctxprobe")).expect("建扩展目录");
    std::fs::create_dir_all(home).expect("建 HOME");
    std::fs::create_dir_all(work).expect("建工作目录");
    std::fs::create_dir_all(sess).expect("建会话目录");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        // 两个 provider：`(provider)` 前缀的条件是"可用 provider 数 > 1"
        // （ref `footer.ts:194`），单 provider 时无法区分"已写入 1"与"默认 1"。
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m","contextWindow":197000}]},"q":{"baseUrl":"http://127.0.0.1:2/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"n","contextWindow":128000}]}}}"#,
    )
    .expect("写 models");
    // 与会话里 `model_change` 的 (provider, modelId) 一致。
    std::fs::write(
        agent.join("extensions/ctxprobe/package.json"),
        r#"{"type":"module","name":"ctxprobe"}"#,
    )
    .expect("写扩展 package.json");
    std::fs::write(
        agent.join("extensions/ctxprobe/index.js"),
        concat!(
            "export default function probe(pi) {\n",
            "  pi.on(\"session_start\", (_e, ctx) => {\n",
            "    if (typeof ctx.ui?.setFooter !== \"function\") return;\n",
            "    ctx.ui.setFooter((_tui, theme, footerData) => ({\n",
            "      render() {\n",
            "        const m = ctx.model;\n",
            "        const cu = ctx.getContextUsage ? ctx.getContextUsage() : null;\n",
            "        const pct = cu && typeof cu.percent === \"number\" ? cu.percent.toFixed(1) : \"null\";\n",
            "        return [theme.fg(\"dim\", `CTXW2 model=${m?.id ?? \"no-model\"} win=${cu?.contextWindow ?? 0} pct=${pct} tok=${cu?.tokens ?? -1} prov=${footerData.getAvailableProviderCount()}`)];\n",
            "      },\n",
            "      invalidate() {},\n",
            "    }));\n",
            "  });\n",
            "}\n",
        ),
    )
    .expect("写扩展入口");
    let cwd = work.to_string_lossy().to_string();
    let session = format!(
        concat!(
            "{{\"type\":\"session\",\"version\":3,\"id\":\"s-f46\",\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"cwd\":{cwd:?}}}\n",
            "{{\"type\":\"model_change\",\"id\":\"e1\",\"parentId\":null,\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"provider\":\"p\",\"modelId\":\"m\"}}\n",
            "{{\"type\":\"message\",\"id\":\"m1\",\"parentId\":\"e1\",\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}],\"api\":\"openai-completions\",\"provider\":\"p\",\"model\":\"m\",\"usage\":{{\"input\":100000,\"output\":4110,\"cacheRead\":10000,\"cacheWrite\":10000,\"totalTokens\":124110,\"cost\":{{\"input\":0,\"output\":0,\"cacheRead\":0,\"cacheWrite\":0,\"total\":0}}}},\"stopReason\":\"stop\",\"timestamp\":1767225602000}}}}\n"
        ),
        cwd = cwd
    );
    std::fs::write(sess.join("2026-01-01T00-00-00-000Z_s-f46.jsonl"), session).expect("写会话");
}

fn run_tui() -> Option<Vec<u8>> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    prepare(&agent, &home, &work, &sess);
    let keys = tmp.path().join("keys.json");
    std::fs::write(
        &keys,
        r#"[{"delayMs":2500},{"delayMs":1200},{"raw":"\u0004"},{"delayMs":600}]"#,
    )
    .expect("写 keys");
    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("110")
        .arg("--rows")
        .arg("34")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("-c")
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    Some(std::fs::read(&raw_path).expect("读 raw"))
}

/// `-c` 恢复后，扩展拿到的上下文占比必须来自**会话历史**（不是启动占位 0.0），
/// 且 provider 计数必须已写入（决定右侧 `(provider)` 前缀）。
#[test]
fn resume_exposes_usage_and_provider_count_to_extensions() {
    let Some(raw) = run_tui() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    let line = text
        .lines()
        .find(|line| line.contains("CTXW2 "))
        .unwrap_or_else(|| panic!("扩展页脚没有渲染出来（探针缺失）"));
    let clean: String = line.chars().filter(|c| !c.is_control()).collect();
    assert!(
        clean.contains("pct=63.0"),
        "扩展拿到的上下文占比应来自会话历史（63.0%），实际：{clean}"
    );
    assert!(
        clean.contains("tok=124110"),
        "扩展拿到的 tokens 应是会话里最后一条 assistant 的用量，实际：{clean}"
    );
    assert!(
        clean.contains("prov=2"),
        "provider 计数应已写入（不得停在默认 1 且未同步）：{clean}"
    );
    assert!(
        clean.contains("model=m"),
        "模型名仍应是会话里的模型：{clean}"
    );
}
