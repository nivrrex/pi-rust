//! F46/U2：`-c` 恢复会话时，扩展可见的 `ctx.model` 必须有值。
//!
//! 真机证据：底栏（扩展页脚 `pi-ai-summary`）显示 `no-model → auto`、上下文占比 `0.0%/197k`。
//! 根因：恢复分支从未调用 `set_ctx_model_object` ⇒ 扩展读到的 `ctx.model` 是 null
//! （ref `pi-ai-summary/src/footer-cny.ts:274/342`：`(ctx.model)?.contextWindow ?? 0`、
//! `model?.id || "no-model"`）。原生页脚走 `state.model`，所以只有扩展页脚表现为 no-model。

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
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m","contextWindow":197000}]}}}"#,
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
            "    ctx.ui.setFooter((_tui, theme) => ({\n",
            "      render() {\n",
            "        const m = ctx.model;\n",
            "        return [theme.fg(\"dim\", `CTXMODEL=${m?.id ?? \"no-model\"}/${m?.provider ?? \"-\"}`)];\n",
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
            "{{\"type\":\"message\",\"id\":\"m1\",\"parentId\":\"e1\",\"timestamp\":\"2026-01-01T00:00:01.000Z\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}],\"api\":\"openai-completions\",\"provider\":\"p\",\"model\":\"m\",\"usage\":{{\"input\":100,\"output\":20,\"cacheRead\":40,\"cacheWrite\":10,\"totalTokens\":170,\"cost\":{{\"input\":0,\"output\":0,\"cacheRead\":0,\"cacheWrite\":0,\"total\":0}}}},\"stopReason\":\"stop\",\"timestamp\":1767225602000}}}}\n"
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

/// `-c` 恢复后，扩展页脚拿到的 `ctx.model` 不得是 `no-model`。
#[test]
fn resume_exposes_model_to_extensions() {
    let Some(raw) = run_tui() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    assert!(
        !text.contains("CTXMODEL=no-model"),
        "恢复会话时扩展的 ctx.model 为空（真机表现为页脚 no-model / 0.0%）"
    );
    assert!(
        text.contains("CTXMODEL=m/p") || text.contains("CTXMODEL=m/"),
        "扩展应看到会话里的模型：{:?}",
        text.lines()
            .filter(|l| l.contains("CTXMODEL="))
            .take(2)
            .collect::<Vec<_>>()
    );
}
