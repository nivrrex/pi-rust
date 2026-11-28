//! F79：扩展对 `tool_call` 的 `event.input` **就地改写**必须真正影响执行参数。
//!
//! ref：`agent-loop.ts:727` 把同一个 `args` 对象交给扩展，改它就是改执行参数；
//! 返回 `{ input }` 反而**无效**（`runner.ts:1134-1152` 只认 `block/reason/terminate`）。
//!
//! 真实使用方是 `pi-bash-guard`（用户已装）：它给没写 `timeout` 的 bash 调用注入
//! 默认超时、并把超大值夹到上限（`policy.ts:33-40`），写法正是
//! `(event.input as Record<string, unknown>).timeout = patch.timeout`。
//! 这条链路断掉的话，该扩展**静默失效**——bash 仍然可以无限期挂住。

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
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// 起 mock，返回（子进程, base_url）。响应按 `responses` 顺序取用。
fn start_mock(scenario: &Path) -> (std::process::Child, String) {
    let mut child = Command::new(repo_root().join("target/debug/mock-provider"))
        .current_dir(repo_root())
        .arg(scenario)
        .arg("--port")
        .arg("0")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("启动 mock-provider");
    let base_url = {
        use std::io::BufRead;
        let stdout = child.stdout.take().expect("stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        let mut found = None;
        for line in lines.by_ref() {
            let line = line.expect("读 stdout");
            if let Some(rest) = line.strip_prefix("listening ") {
                found = Some(
                    rest.rsplit(':')
                        .next()
                        .expect("端口")
                        .parse::<u16>()
                        .expect("数字"),
                );
                break;
            }
        }
        format!(
            "http://127.0.0.1:{}/v1",
            found.expect("mock 应打印 listening 行")
        )
    };
    // stdout 已被 take，后续读会 panic；用 `Child::kill` 时不需要它。
    (child, base_url)
}

/// 装一个把 bash `timeout` 改成 1 秒的扩展（与 `pi-bash-guard` 同一写法）。
fn install_mutating_extension(agent: &Path) {
    let dir = agent.join("extensions/mutate");
    std::fs::create_dir_all(&dir).expect("建扩展目录");
    std::fs::write(
        dir.join("index.mjs"),
        concat!(
            "export default function mutate(pi) {\n",
            "  pi.on(\"tool_call\", (event) => {\n",
            "    if (event?.toolName !== \"bash\") return undefined;\n",
            "    const input = event.input;\n",
            "    if (input && typeof input === \"object\") input.timeout = 1;\n",
            "    return undefined;\n",
            "  });\n",
            "}\n",
        ),
    )
    .expect("写扩展");
}

/// 把 `tool-bash.sse` 的 `echo hi` 换成 `sleep 3`，写进临时目录。
fn slow_bash_sse(dir: &Path) -> PathBuf {
    let source = repo_root().join("fixtures/sse/openai/tool-bash.sse");
    let text = std::fs::read_to_string(&source).expect("读 sse 夹具");
    let patched = text.replace(
        "\\\"command\\\":\\\"echo hi\\\"",
        "\\\"command\\\":\\\"sleep 3\\\"",
    );
    assert_ne!(patched, text, "夹具里应有 `echo hi` 可替换");
    let path = dir.join("slow-bash.sse");
    std::fs::write(&path, patched).expect("写 sse");
    path
}

/// `tool_call` 的就地改写必须让 bash 按 1 秒超时（而不是跑满 3 秒）。
#[test]
fn in_place_input_mutation_changes_execution() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    let sess = tmp.path().join("sess");
    for dir in [&agent, &home, &work, &sess] {
        std::fs::create_dir_all(dir).expect("建目录");
    }
    install_mutating_extension(&agent);
    let sse = slow_bash_sse(tmp.path());
    // 场景写到临时目录，`file` 用绝对路径（mock 按仓库根解析相对路径）。
    let scenario = tmp.path().join("slow.json");
    std::fs::write(
        &scenario,
        format!(
            r#"{{"id":"F79","description":"就地改写","provider":"openai","model":"mock-model","mode":"print","prompt":"hi","responses":[{{"file":{}}},{{"file":"fixtures/sse/openai/text-basic.sse"}}]}}"#,
            serde_json::to_string(&sse.to_string_lossy().to_string()).expect("路径")
        ),
    )
    .expect("写场景");
    let (mut mock, base_url) = start_mock(&scenario);
    std::fs::write(
        agent.join("models.json"),
        format!(
            r#"{{"providers":{{"p":{{"baseUrl":"{base_url}","api":"openai-completions","apiKey":"k","models":[{{"id":"m1","reasoning":false}}]}}}}}}"#
        ),
    )
    .expect("写 models");
    std::fs::write(
        agent.join("auth.json"),
        r#"{"p":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth");
    std::fs::write(
        agent.join("settings.json"),
        r#"{"defaultProvider":"p","defaultModel":"m1"}"#,
    )
    .expect("写 settings");
    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_pi-rs"))
        .current_dir(&work)
        .arg("--mode")
        .arg("json")
        .arg("hi")
        .env("HOME", &home)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", &sess)
        .env("PI_NODE", &node)
        .output()
        .expect("跑 pi-rs");
    let elapsed = started.elapsed();
    let _ = mock.kill();
    let _ = mock.wait();
    let events = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        events.contains("Command timed out after 1 seconds"),
        "扩展把 timeout 改成 1 秒后 bash 应按 1 秒超时；实际事件：{}",
        &events[events.len().saturating_sub(600)..]
    );
    // 特意**不**断言总耗时：启动期（读 docs、加载扩展、jiti 求值）本身就要几秒，
    // 与 bash 是否超时无关，用它做判据会引入不稳定。`Command timed out after
    // 1 seconds` 这条文本已经是决定性证据——`sleep 3` 未被 1 秒超时打断是不可能出现它的。
    let _ = elapsed;
}
