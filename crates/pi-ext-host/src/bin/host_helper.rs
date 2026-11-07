//! 测试用宿主 helper（P9-1-C / S3）：启动扩展宿主后打印子进程 pid，然后阻塞等待。
//!
//! 用途：集成测试 spawn 本 helper → 读 pid → `kill -9` helper → 断言 node 子进程在 5 s 内退出
//! （验证"父进程被强杀时子进程不会变孤儿"）。
//!
//! 用法：
//! - `host_helper <runner.mjs> <extension.mjs>`：打印 `child_pid=` 后常驻（父死子退测试用）；
//! - `host_helper --probe <runner.mjs> <ext...>`：加载并打印注册计数（JSON），随后退出。

use std::path::PathBuf;

/// `PI_NODE` 优先（必须是真实文件），否则在 `PATH` 里找 `node`（与 `pi-cli` 的 `find_node` 同口径，
/// 但本 helper 只服务测试，不引入额外依赖）。
fn node_from_env() -> PathBuf {
    if let Some(explicit) = std::env::var_os("PI_NODE") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return path;
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("node");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("node")
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next().unwrap_or_default();
    let probe = first == "--probe";
    let runner_arg = if probe { args.next() } else { Some(first) };
    let extensions: Vec<PathBuf> = args.map(PathBuf::from).collect();
    let Some(runner) = runner_arg else {
        eprintln!("usage: host_helper [--probe] <runner.mjs> <extension…>");
        std::process::exit(2);
    };
    if extensions.is_empty() {
        eprintln!("host_helper: 至少需要一个扩展入口");
        std::process::exit(2);
    }
    let options = pi_ext_host::HostOptions {
        node: node_from_env(),
        runner: PathBuf::from(runner),
        extensions,
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        env: Vec::new(),
    };
    let mut host = match pi_ext_host::ExtHost::start(options).await {
        Ok(host) => host,
        Err(error) => {
            eprintln!("host_helper: {error}");
            std::process::exit(1);
        }
    };
    if probe {
        // 计数口径与 notes/p9-0/usage-matrix.md 对齐：工具数 / 命令名 / 事件名。
        let report = serde_json::json!({
            "tools": host.tools().iter().map(|tool| tool.name.clone()).collect::<Vec<_>>(),
            "commands": host
                .commands()
                .iter()
                .map(|command| command.name.clone())
                .collect::<Vec<_>>(),
            "events": host.events(),
            "pid": host.pid(),
        });
        println!("PROBE {report}");
        host.shutdown().await;
        return;
    }
    println!("child_pid={}", host.pid().unwrap_or(0));
    // 保持 alive 直到被杀（父进程死 ⇒ 本进程死 ⇒ node 的 stdin EOF ⇒ 自行退出）。
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
