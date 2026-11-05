//! S3 加固验收：令牌只走环境变量、单连接、父死子退、无 node 文案。
//!
//! 全部只操作**本测试自己启动**的进程；`pgrep`/`kill` 只作用于拿到的 pid。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pi_ext_host::{ExtHost, HostError, HostOptions};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runner() -> PathBuf {
    repo_root().join("tools/ext-host/runner.mjs")
}

fn fixture() -> PathBuf {
    repo_root().join("tools/ext-host/fixtures/echo-ext.mjs")
}

fn node_path() -> Option<PathBuf> {
    let output = std::process::Command::new("sh")
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

fn options(node: &std::path::Path) -> HostOptions {
    HostOptions {
        node: node.to_path_buf(),
        runner: runner(),
        extensions: vec![fixture()],
        cwd: repo_root(),
        env: Vec::new(),
    }
}

/// 令牌必须经环境变量传给子进程，**不得**出现在 argv（`/proc/<pid>/cmdline`）。
/// 同时验证 `/proc/<pid>/environ`（exec 时快照）里确实有 `PI_EXT_TOKEN=`。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn token_travels_in_env_not_argv() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let mut host = ExtHost::start(options(&node)).await.expect("宿主应能启动");
    let pid = host.pid().expect("应有 pid");
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).expect("读 cmdline");
    let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");
    assert!(
        !cmdline.contains("PI_EXT_TOKEN="),
        "令牌不得出现在 argv：{cmdline}"
    );
    assert!(
        !cmdline.contains("token="),
        "令牌不得出现在 argv：{cmdline}"
    );
    let environ = std::fs::read(format!("/proc/{pid}/environ")).expect("读 environ");
    let environ = String::from_utf8_lossy(&environ);
    let token = environ
        .split('\0')
        .find_map(|entry| entry.strip_prefix("PI_EXT_TOKEN="))
        .expect("令牌应经环境变量传递（environ 里应有 PI_EXT_TOKEN）");
    assert!(token.len() >= 32, "令牌长度异常：{}", token.len());
    // 令牌的**值本身**不得出现在 argv 里（上一条只查了变量名）。
    assert!(!cmdline.contains(token), "令牌的明文值不得出现在 argv");
    host.shutdown().await;
}

/// 只接受一个连接：握手完成后，同名端口再连应当被拒（监听已关闭）。
#[tokio::test]
async fn second_connection_is_refused() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let mut host = ExtHost::start(options(&node)).await.expect("宿主应能启动");
    let port = host.port();
    let attempt = tokio::net::TcpStream::connect(("127.0.0.1", port)).await;
    assert!(
        attempt.is_err(),
        "第二个连接应被拒（监听在 accept 之后立即关闭）"
    );
    host.shutdown().await;
}

/// 父进程被 SIGKILL：node 子进程必须在 5 s 内退出（stdin 管道 EOF 触发垫片退出）。
#[test]
fn child_exits_when_parent_is_killed() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let helper = env!("CARGO_BIN_EXE_host_helper");
    let mut child = std::process::Command::new(helper)
        .arg(runner())
        .arg(fixture())
        .env("PI_NODE", &node)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("启动 helper");
    // 读第一行拿 node 子进程 pid。
    let stdout = child.stdout.take().expect("有 stdout");
    let mut reader = std::io::BufReader::new(stdout);
    // 扩展的 stdout 噪音与 helper 的输出共用 stdout（协议走 TCP）⇒ 逐行找到 `child_pid=`。
    let mut pid: u32 = 0;
    for _ in 0..50 {
        let mut line = String::new();
        {
            use std::io::BufRead as _;
            if reader.read_line(&mut line).expect("读 helper 输出") == 0 {
                break;
            }
        }
        if let Some(value) = line.trim().strip_prefix("child_pid=") {
            if let Ok(parsed) = value.parse::<u32>() {
                pid = parsed;
                break;
            }
        }
    }
    assert!(pid > 1, "应有 child_pid=（实际输出里没找到）");
    // 杀 helper（父），然后等子进程消失。
    let status = std::process::Command::new("kill")
        .arg("-9")
        .arg(child.id().to_string())
        .status()
        .expect("kill helper");
    assert!(status.success());
    let _ = child.wait();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut gone = false;
    while Instant::now() < deadline {
        // `kill -0` 探测该 pid 是否还存在（进程已回收时返回非 0）。
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !alive {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(gone, "父被 SIGKILL 后 node 子进程 {pid} 未在 5 s 内退出");
}

/// 无 node：`NodeMissing` 且错误信息里带路径（供 --list-tools 的提示文案使用）。
#[tokio::test]
async fn missing_node_message_mentions_path() {
    let error = ExtHost::start(options(std::path::Path::new(
        "/nonexistent/node-s3-hardening",
    )))
    .await
    .expect_err("必须报错");
    match error {
        HostError::NodeMissing(path) => assert!(path.contains("/nonexistent/node-s3-hardening")),
        other => panic!("期望 NodeMissing，实际 {other:?}"),
    }
}
