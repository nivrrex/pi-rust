//! F46/U3：light 主题下扩展的 `theme.fg("success", …)` 必须用**浅色**调色板。
//!
//! 真机证据：`✓ 全部完成` 在 light 主题下 TS 是深绿 `#588458`，pi-rs 是深色主题的
//! `#b5bd68`。根因：扩展宿主的 shim 色表**只由 `theme/dark.json` 生成**、且从不随
//! `settings.json` 的 `theme` 切换（ref `theme/theme.ts:529` 按主题名取对应主题对象）。
//!
//! 本用例在 light 主题下跑一个只渲染 `success` 色块的合成扩展，断言用的是浅色值
//! （`88;132;88`，256 色模式下为 65），不是深色值（`181;189;104` / 143）。

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

fn prepare(agent: &Path, home: &Path, work: &Path) {
    std::fs::create_dir_all(agent.join("extensions/colorprobe")).expect("建扩展目录");
    std::fs::create_dir_all(home).expect("建 HOME");
    std::fs::create_dir_all(work).expect("建工作目录");
    // 关键：**浅色主题**。
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"light","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("extensions/colorprobe/package.json"),
        r#"{"type":"module","name":"colorprobe"}"#,
    )
    .expect("写扩展 package.json");
    std::fs::write(
        agent.join("extensions/colorprobe/index.js"),
        concat!(
            "export default function probe(pi) {\n",
            "  pi.on(\"session_start\", (_e, ctx) => {\n",
            "    if (typeof ctx.ui?.setFooter !== \"function\") return;\n",
            "    ctx.ui.setFooter((_tui, theme) => ({\n",
            "      render() { return [theme.fg(\"success\", \"SUCCESS-COLOR-PROBE\")]; },\n",
            "      invalidate() {},\n",
            "    }));\n",
            "  });\n",
            "}\n",
        ),
    )
    .expect("写扩展入口");
}

fn run_tui() -> Option<Vec<u8>> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    prepare(&agent, &home, &work);
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
        .arg("100")
        .arg("--rows")
        .arg("30")
        .arg("--exit-code-file")
        .arg(&code_path)
        .arg("--out")
        .arg(&raw_path)
        .arg("--script")
        .arg(&keys)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", tmp.path().join("sess"))
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .env("COLORTERM", "") // 强制 256 色，使断言与终端能力无关
        .env("TERM", "xterm-256color")
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    Some(std::fs::read(&raw_path).expect("读 raw"))
}

/// 浅色主题下 `success` 必须是浅色调色板的值。
#[test]
fn light_theme_uses_light_success_color() {
    let Some(raw) = run_tui() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    let Some(index) = text.find("SUCCESS-COLOR-PROBE") else {
        panic!("扩展页脚没有渲染出来（探针缺失）");
    };
    // 取探针前面最近的一段 SGR。
    let window = &text[index.saturating_sub(120)..index];
    let dark = "38;2;181;189;104";
    let light = "38;2;88;132;88";
    assert!(
        !window.contains(dark),
        "light 主题下仍在使用深色调色板（{dark}）：{window:?}"
    );
    assert!(
        window.contains(light) || window.contains("38;5;65"),
        "light 主题下 success 应为浅色值（{light} 或 256 色 65）：{window:?}"
    );
}
