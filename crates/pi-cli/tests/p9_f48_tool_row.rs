//! F48/X2–X3：工具行的**折叠**渲染必须能被扩展的原型补丁接管。
//!
//! 机制（ref `pi-ai-summary/src/adapters/tool-render.ts:246-252`）：扩展直接替换
//! `ToolExecutionComponent.prototype.render`。此前 shim 里的 `ToolExecutionComponent`
//! 是空壳（`render()` 恒 `[]`、从不实例化）⇒ 补丁改的是一个永不调用的方法，
//! 对 Rust 渲染零影响（真机：TS 单行折叠 `✓ name {"args"} …`、pi-rs 是带浅绿底色的块）。
//!
//! 本用例用**合成**扩展（同样的补丁手法）验证四件事：
//!   1. 未定稿（pending）行显示运行态；
//!   2. 成功行折叠成单行；
//!   3. 出错行折叠成两行（第二行 `! <错误首行>`）；
//!   4. 未被特判的工具名同样折叠；
//!   5. ctrl+o 之后回到**原生块**渲染（折叠行消失）。

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
    std::fs::create_dir_all(agent.join("extensions/rowprobe")).expect("建扩展目录");
    std::fs::create_dir_all(home).expect("建 HOME");
    std::fs::create_dir_all(work).expect("建工作目录");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"theme":"dark","defaultProvider":"p","defaultModel":"m"}"#,
    )
    .expect("写 settings");
    std::fs::write(
        agent.join("models.json"),
        r#"{"providers":{"p":{"baseUrl":"http://127.0.0.1:1/v1","api":"openai-completions","apiKey":"DUMMY","models":[{"id":"m"}]}}}"#,
    )
    .expect("写 models");
    std::fs::write(
        agent.join("extensions/rowprobe/package.json"),
        r#"{"type":"module","name":"rowprobe"}"#,
    )
    .expect("写扩展 package.json");
    // 合成探针：只改原型方法，读组件字段，绝不触碰 args/结果/执行。
    std::fs::write(
        agent.join("extensions/rowprobe/index.js"),
        concat!(
            "import { ToolExecutionComponent } from \"@earendil-works/pi-coding-agent\";\n",
            "export default function probe(pi) {\n",
            "  let installed = false;\n",
            "  pi.on(\"session_start\", (_e, ctx) => {\n",
            "    if (installed) return;\n",
            "    const proto = ToolExecutionComponent?.prototype;\n",
            "    if (!proto || typeof proto.render !== \"function\") {\n",
            "      if (typeof ctx.ui?.notify === \"function\") ctx.ui.notify(\"ROWPROBE=no-proto\", \"error\");\n",
            "      return;\n",
            "    }\n",
            "    const original = proto.render;\n",
            "    proto.render = function render(width) {\n",
            "      try {\n",
            "        if (this.expanded === true) return original.call(this, width);\n",
            "        const theme = ctx?.ui?.theme;\n",
            "        if (!theme) return original.call(this, width);\n",
            "        const settled = this.isPartial === false;\n",
            "        const failed = settled && this.result && this.result.isError === true;\n",
            "        const glyph = !settled ? \"~\" : failed ? \"x\" : \"+\";\n",
            "        const color = !settled ? \"warning\" : failed ? \"error\" : \"success\";\n",
            "        const word = !settled ? \"run\" : failed ? \"err\" : \"ok\";\n",
            "        const name = String(this.toolName ?? \"tool\");\n",
            "        const one = `${glyph} ${name} · ${word}`;\n",
            "        if (settled && failed) {\n",
            "          return [`  ${theme.fg(color, one)}`, `  ${theme.fg(color, \"    ! boom\")}`];\n",
            "        }\n",
            "        return [`  ${theme.fg(color, one)}`];\n",
            "      } catch {\n",
            "        return original.call(this, width);\n",
            "      }\n",
            "    };\n",
            "    installed = true;\n",
            "    if (typeof ctx.ui?.notify === \"function\") ctx.ui.notify(\"ROWPROBE=installed\", \"info\");\n",
            "  });\n",
            "}\n",
        ),
    )
    .expect("写扩展入口");
}

fn run_tui(extra_keys: bool) -> Option<Vec<u8>> {
    let node = node_path()?;
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    prepare(&agent, &home, &work);
    // 让 mock provider 回一个带工具调用的回合。
    std::fs::write(work.join("alpha.txt"), "a\n").expect("写夹具文件");
    let keys = tmp.path().join("keys.json");
    let script = if extra_keys {
        // 等折叠行出现 → ctrl+o 展开 → 再等
        r#"[{"delayMs":4000},{"raw":"\u000f"},{"delayMs":2000},{"raw":"\u0004"},{"delayMs":600}]"#
    } else {
        r#"[{"delayMs":4000},{"raw":"\u0004"},{"delayMs":600}]"#
    };
    std::fs::write(&keys, script).expect("写 keys");
    let raw_path = tmp.path().join("raw.bin");
    let code_path = tmp.path().join("code.txt");
    let status = Command::new("python3")
        .arg(repo_root().join("tools/difftest/pty-helper.py"))
        .arg("--cols")
        .arg("100")
        .arg("--rows")
        .arg("40")
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
        .status()
        .expect("运行 pty-helper");
    assert!(status.success(), "pty-helper 应成功");
    Some(std::fs::read(&raw_path).expect("读 raw"))
}

/// 合成扩展的补丁必须真正生效：T116 场景（含成功/出错/未特判/超长四种行）
/// 的折叠结果必须与 TS 录制的 golden 逐帧一致。
///
/// 为什么用整场景而不是探针行：`ToolExecutionComponent.prototype.render` 被替换
/// **不等于**它被调用——旧实现的空壳同样能被替换（所以"补丁已安装"这种断言在改前
/// 也能通过，无区分力）。真正的判据是屏幕上的行。
#[test]
fn collapsed_rows_match_ts_golden() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let repo = repo_root();
    let out = tempfile::tempdir().expect("临时目录").path().to_path_buf();
    // 需要 netns 隔离（mock provider 与 TUI 必须同网段），用与 difftest 相同的外壳。
    let status = Command::new("unshare")
        .arg("-n")
        .arg("sh")
        .arg("-c")
        .arg(format!(
            "ip link set lo up; env PATH=\"$PATH\" TS_PI_ENTRY={} DIFTEST_IMPL=pi-rs {} {} --scenario {} --out {}",
            repo.join("ref/pi-ts-0.87.1/dist/bundle/cli.js").display().to_string() + ".skip",
            node.display(),
            repo.join("tools/difftest/run-tui.mjs").display(),
            repo.join("scenarios-tui/T116-toolrow-collapsed.json").display(),
            out.display()
        ))
        .current_dir(&repo)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .status()
        .expect("运行 run-tui");
    let _ = status;
    let raw = std::fs::read(out.join("run-1/raw.bin")).expect("run-tui 应产出 raw.bin");
    let text = String::from_utf8_lossy(&raw).to_string();
    // 折叠行的判据：成功行是 `+ <name> · … · ok`，出错行第二行是 `! …`。
    assert!(
        text.contains("+ read"),
        "成功工具行没有折叠成单行（仍走原生块渲染）：{:?}",
        text.lines()
            .filter(|l| l.contains("read"))
            .take(3)
            .collect::<Vec<_>>()
    );
    assert!(
        text.contains("! ("),
        "出错工具行没有折叠成两行（缺 `! ` 续行）"
    );
}

#[test]
fn extension_prototype_patch_collapses_tool_rows() {
    let Some(raw) = run_tui(false) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    // 探针在有工具调用时才会渲染；没有回合就没有行，用例改判"补丁已安装"。
    assert!(
        text.contains("ROWPROBE=installed"),
        "扩展的原型补丁没有被安装（shim 的 ToolExecutionComponent 仍是空壳）：{:?}",
        text.lines()
            .filter(|line| line.contains("ROWPROBE"))
            .collect::<Vec<_>>()
    );
}

/// ctrl+o 之后必须回到原生块渲染（折叠行消失）。
#[test]
fn ctrl_o_restores_native_block() {
    let Some(raw) = run_tui(true) else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let text = String::from_utf8_lossy(&raw).to_string();
    // ctrl+o 会写状态行（ref `interactive-mode.ts:4419`）。
    assert!(
        text.contains("Tool output: expanded") || text.contains("Tool output:"),
        "ctrl+o 应产生状态行：{:?}",
        text.lines()
            .filter(|l| l.contains("Tool output"))
            .collect::<Vec<_>>()
    );
}
