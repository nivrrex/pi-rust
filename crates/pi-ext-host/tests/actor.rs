//! S3：宿主 actor 化的四类保证。
//!
//! 1) 同步/异步调用方都能用（含 current_thread 与 multi_thread 运行时）；
//! 2) 调用方 runtime 关闭后 actor 仍可用；actor 关闭后调用返回明确错误、不 panic；
//! 3) 并发调用被串行化且不死锁；
//! 4) 宿主崩溃后调用方拿到错误，并且**可以重启**一个新的 actor。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pi_ext_host::{HostHandle, HostOptions};
use serde_json::json;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/echo-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    }
}

/// 同步调用（调用线程没有 tokio 运行时）。
#[test]
fn sync_calls_work_without_a_caller_runtime() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(options(&node)).expect("actor 应能启动");
    let result = host
        .invoke("ext_echo", json!({"text": "sync"}), Duration::from_secs(10))
        .expect("同步调用应成功");
    assert_eq!(result["content"][0]["text"], "echo:sync");
    host.shutdown();
}

/// 异步调用：current_thread 与 multi_thread 两种调用方运行时都要能用。
#[tokio::test(flavor = "current_thread")]
async fn async_calls_work_in_current_thread_runtime() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(options(&node)).expect("actor 应能启动");
    let host_for_call = host.clone();
    let result = host_for_call
        .invoke_async(
            "ext_echo".to_string(),
            json!({"text": "ct"}),
            Duration::from_secs(10),
        )
        .await
        .expect("异步调用应成功");
    assert_eq!(result["content"][0]["text"], "echo:ct");
    host.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_calls_work_in_multi_thread_runtime() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(options(&node)).expect("actor 应能启动");
    let result = host
        .clone()
        .invoke_async(
            "ext_echo".to_string(),
            json!({"text": "mt"}),
            Duration::from_secs(10),
        )
        .await
        .expect("异步调用应成功");
    assert_eq!(result["content"][0]["text"], "echo:mt");
    host.shutdown();
}

/// 调用方 runtime 关闭不影响 actor（宿主不依赖调用方 runtime）。
#[test]
fn actor_survives_caller_runtime_shutdown() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(options(&node)).expect("actor 应能启动");
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            // 仅借用一个临时运行时，随后立刻 drop。
        });
    }
    let result = host
        .invoke(
            "ext_echo",
            json!({"text": "after"}),
            Duration::from_secs(10),
        )
        .expect("调用方 runtime 关闭后 actor 仍应可用");
    assert_eq!(result["content"][0]["text"], "echo:after");
    host.shutdown();
}

/// actor 关闭后调用：明确错误、不 panic。
#[test]
fn calls_after_shutdown_return_clear_error() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(options(&node)).expect("actor 应能启动");
    host.shutdown();
    let error = host
        .invoke("ext_echo", json!({"text": "x"}), Duration::from_secs(2))
        .expect_err("关闭后调用必须失败");
    assert!(error.to_string().contains("host actor stopped"), "{error}");
}

/// 并发调用被串行化（两次 200 ms 的 sleep 至少 400 ms）且不死锁。
#[test]
fn concurrent_calls_are_serialized() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(options(&node)).expect("actor 应能启动");
    let started = Instant::now();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let host = host.clone();
            std::thread::spawn(move || {
                host.invoke("ext_sleep", json!({"ms": 200}), Duration::from_secs(10))
                    .expect("调用应成功")
            })
        })
        .collect();
    for handle in handles {
        let _ = handle.join().expect("线程不应 panic");
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(380),
        "两次 200 ms 调用必须串行（实际 {elapsed:?}）"
    );
    host.shutdown();
}

/// 宿主崩溃：调用方拿到错误；随后可以重启一个新的 actor。
#[test]
fn crashed_host_reports_error_and_can_restart() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(options(&node)).expect("actor 应能启动");
    let pid = host.pid().expect("应有 pid");
    let status = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()
        .expect("kill 应可用");
    assert!(status.success());
    std::thread::sleep(Duration::from_millis(200));
    let error = host
        .invoke("ext_echo", json!({"text": "x"}), Duration::from_secs(2))
        .expect_err("宿主死后调用必须失败");
    let text = error.to_string();
    assert!(
        text.contains("exited") || text.contains("io error") || text.contains("actor stopped"),
        "{text}"
    );
    // 重启：新 actor 可正常工作。
    let restarted = HostHandle::start(options(&node)).expect("应能重启");
    let result = restarted
        .invoke(
            "ext_echo",
            json!({"text": "restart"}),
            Duration::from_secs(10),
        )
        .expect("重启后应可用");
    assert_eq!(result["content"][0]["text"], "echo:restart");
    restarted.shutdown();
}
