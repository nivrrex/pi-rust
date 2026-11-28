//! F41/P0：扩展宿主进程**不得持有 tty**。
//!
//! 真机 strace（用户提供）：pi-rs 恢复 cooked → 5.8 ms 后 pi-rs 退出 → 12.5 ms 后宿主 node
//! 对 fd 1、fd 2 各执行一次 `TCSETS`（写回 `ICANON|ECHO`）→ 20.3 ms 后 node 退出。
//! 结果是 shell 已把 tty 设为 raw 之后，node 又把它改回 cooked ⇒ 用户首个按键被内核字面回显
//! （`^[[A`）。根因是宿主 node 的 stdout/stderr **继承了 tty**，Node 在退出时会对 tty 做
//! `uv_tty_reset_mode`（写回启动时保存的 cooked 模式）。
//!
//! 本用例直接断言机制：pi-rs 运行期间，`runner.mjs` 的 fd 0/1/2 **都不指向字符终端**。
//! 改前：fd 1/2 = `/dev/pts/N`（tty）⇒ 失败；改后：fd 0 = `/dev/null`、fd 1/2 = `pipe:[…]`。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
    std::fs::create_dir_all(agent.join("extensions/probe")).expect("建扩展目录");
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
        agent.join("extensions/probe/package.json"),
        r#"{"type":"module","name":"probe"}"#,
    )
    .expect("写扩展 package.json");
    std::fs::write(
        agent.join("extensions/probe/index.js"),
        "export default function probe(pi) {}\n",
    )
    .expect("写扩展入口");
}

/// 找出 `runner.mjs` 的 pid（无扩展宿主时返回 `None`）。
fn find_runner() -> Option<u32> {
    for entry in std::fs::read_dir("/proc").ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let pid: u32 = match name.to_str().and_then(|text| text.parse().ok()) {
            Some(pid) => pid,
            None => continue,
        };
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        // **只认 node 进程**：父 shell/测试二进制的命令行里也会出现 `runner.mjs` 字样
        // （`--list-tools` 的路径参数、`grep` 之类），按 argv[0] 收窄。
        let mut parts = cmdline.split(|byte| *byte == 0);
        let argv0 = parts.next().unwrap_or_default();
        if !String::from_utf8_lossy(argv0).contains("node") {
            continue;
        }
        if String::from_utf8_lossy(&cmdline).contains("runner.mjs") {
            return Some(pid);
        }
    }
    None
}

fn fd_target(pid: u32, fd: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/fd/{fd}"))
        .ok()
        .map(|path| path.to_string_lossy().to_string())
}

/// 宿主进程的 fd 0/1/2 不得指向 tty。
#[test]
fn extension_host_never_holds_a_tty() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let home = tmp.path().join("home");
    let work = tmp.path().join("work");
    prepare(&agent, &home, &work);
    // **必须让 pi-rs 自己挂在 pty 上**：只有 stdout/stderr 是 tty 时，宿主才可能继承 tty
    // （这正是真机 strace 观测到的前提）。测试进程自身没有 tty，用 `script(1)` 分配一个。
    // `--list-tools` 会启动宿主、加载扩展、打印清单后退出，足够观察到宿主存活期。
    let mut child = Command::new(repo_root().join("tools/difftest/tty-run.sh"))
        .arg(tmp.path().join("pseudo-tty.log"))
        .arg(format!("{} --list-tools", env!("CARGO_BIN_EXE_pi-rs")))
        .current_dir(&work)
        .env("PI_CODING_AGENT_DIR", &agent)
        .env("PI_CODING_AGENT_SESSION_DIR", tmp.path().join("sess"))
        .env("PI_NODE", &node)
        .env("HOME", &home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动 pi-rs");

    // 轮询等宿主起来（最多 20s），期间抓它的 fd 快照。
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut snapshot: Option<Vec<(u32, String)>> = None;
    while Instant::now() < deadline {
        if let Some(pid) = find_runner() {
            let fds: Vec<(u32, String)> = (0..3)
                .filter_map(|fd| fd_target(pid, fd).map(|target| (fd, target)))
                .collect();
            if fds.len() == 3 {
                snapshot = Some(fds);
                break;
            }
        }
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();

    let Some(fds) = snapshot else {
        panic!("未能观察到扩展宿主进程（runner.mjs）的 fd 0/1/2");
    };
    let tty: Vec<&(u32, String)> = fds
        .iter()
        .filter(|(_, target)| target.starts_with("/dev/pts") || target.starts_with("/dev/tty"))
        .collect();
    assert!(
        tty.is_empty(),
        "扩展宿主不得持有 tty（Node 退出时会把 termios 写回 cooked，导致 shell 里首个按键字面回显）：{:?}",
        fds
    );
}
