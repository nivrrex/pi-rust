//! P9-1-E1c / S2d：宿主 actor 与 node 子进程的生命周期。
//!
//! 1) 正常路径：`HostHandle::shutdown()` 之后，actor 线程与 node 子进程都在限定时间内消失；
//! 2) panic 路径：持有句柄的线程 panic（栈展开丢弃句柄）后，node 子进程同样消失。
//!
//! 判定只看**本测试自己启动的那个 pid**（`HostHandle::pid()`），不扫系统进程。

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

/// 进程是否还在（僵尸也算"在"：`/proc/<pid>` 存在即算没退出）。
fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn wait_gone(pid: u32, limit: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !alive(pid)
}

#[test]
fn normal_shutdown_stops_actor_and_child() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(options(&node)).expect("actor 应能启动");
    let pid = host.pid().expect("宿主应有子进程 pid");
    assert!(alive(pid), "子进程应已启动");
    // 确认宿主真的可用（不是"启动了但没握手"）。
    let result = host
        .invoke("ext_echo", json!({"text": "life"}), Duration::from_secs(10))
        .expect("调用应成功");
    assert_eq!(result["content"][0]["text"], "echo:life");

    host.shutdown();
    assert!(
        wait_gone(pid, Duration::from_secs(5)),
        "shutdown 后 node 子进程应退出（pid={pid}）"
    );
    // actor 停止后调用方拿到明确错误，而不是挂死或 panic。
    let error = host
        .invoke("ext_echo", json!({"text": "x"}), Duration::from_secs(5))
        .expect_err("关闭后的调用应失败");
    assert!(
        format!("{error}").contains("stopped") || format!("{error}").contains("closed"),
        "错误应说明宿主已停止：{error}"
    );
}

#[test]
fn panic_in_owner_thread_kills_child() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let node_for_thread = node.clone();
    let pid_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<u32>));
    let slot = pid_slot.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let host = HostHandle::start(options(&node_for_thread)).expect("actor 应能启动");
        *slot.lock().expect("取锁") = host.pid();
        assert!(host.pid().is_some(), "宿主应有子进程 pid");
        panic!("模拟持有者崩溃");
    }));
    assert!(result.is_err(), "应捕获到 panic");
    let pid = pid_slot.lock().expect("取锁").expect("panic 前应记下 pid");
    assert!(
        wait_gone(pid, Duration::from_secs(10)),
        "panic 展开丢弃句柄后 node 子进程应退出（pid={pid}）"
    );
}
