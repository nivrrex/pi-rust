//! E4f：多路复用后的并发/异常路径。
//!
//! - 两个并发 `invoke` 都成功，结果不串；
//! - `invoke` 超时后**迟到**的应答被路由表丢弃，不污染后续请求；
//! - invoke 进行中宿主崩溃：调用返回明确错误（不是挂死）。

use std::path::PathBuf;
use std::time::Duration;

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
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

fn start() -> Option<HostHandle> {
    let node = node_path()?;
    Some(
        HostHandle::start(HostOptions {
            node,
            runner: repo_root().join("tools/ext-host/runner.mjs"),
            extensions: vec![repo_root().join("tools/ext-host/fixtures/echo-ext.mjs")],
            cwd: repo_root(),
            env: Vec::new(),
        })
        .expect("宿主应能启动"),
    )
}

#[test]
fn two_concurrent_invokes_do_not_mix_results() {
    let Some(host) = start() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let first = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke("ext_echo", json!({"text": "one"}), Duration::from_secs(10))
        })
    };
    let second = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke("ext_echo", json!({"text": "two"}), Duration::from_secs(10))
        })
    };
    let a = first.join().expect("线程").expect("第一次调用");
    let b = second.join().expect("线程").expect("第二次调用");
    assert_eq!(a["content"][0]["text"], "echo:one");
    assert_eq!(b["content"][0]["text"], "echo:two");
    host.shutdown();
}

#[test]
fn invoke_timeout_then_late_response_does_not_pollute() {
    let Some(host) = start() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    // `ext_sleep` 睡 300ms；超时设 50ms ⇒ 第一次必然超时。
    let timed_out = host.invoke("ext_sleep", json!({"ms": 300}), Duration::from_millis(50));
    assert!(timed_out.is_err(), "应超时");
    // 等迟到的应答真的回来，再发一条正常请求：它必须拿到自己的结果。
    std::thread::sleep(Duration::from_millis(500));
    let after = host
        .invoke(
            "ext_echo",
            json!({"text": "after"}),
            Duration::from_secs(10),
        )
        .expect("迟到的应答不应污染后续请求");
    assert_eq!(after["content"][0]["text"], "echo:after");
    host.shutdown();
}

#[test]
fn invoke_during_host_crash_returns_error() {
    let Some(host) = start() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let pid = host.pid().expect("有子进程 pid");
    let invoker = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke("ext_sleep", json!({"ms": 5000}), Duration::from_secs(20))
        })
    };
    std::thread::sleep(Duration::from_millis(200));
    // 只杀本测试启动的那个 pid。
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status();
    let result = invoker.join().expect("线程");
    assert!(result.is_err(), "宿主崩溃时调用应返回错误：{result:?}");
    host.shutdown();
}

/// S3：`call()` 返回后**不允许**残留未应答的直连请求（`pending` 必须为空）。
///
/// 做法：让工具在 invoke 期间打开自定义面板，主线程用 `custom_input` 驱动它完成；
/// 每次 invoke 结束后断言 `pending_len() == 0`（不依赖 sleep 判定，只是不变量检查）。
#[test]
fn no_pending_requests_left_after_invoke_returns() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/custom_invoke-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.set_ctx(
        json!({"cwd": repo_root().display().to_string(), "mode": "tui", "hasUI": true,
        "model": "mock-model", "thinkingLevel": "off"}),
    );

    for round in 0..5 {
        let invoke = {
            let host = host.clone();
            std::thread::spawn(move || {
                host.invoke("ext_ask_inline", json!({}), Duration::from_secs(10))
            })
        };
        let mut opened = false;
        for _ in 0..50 {
            if let Ok(pi_ext_host::UiFrame::CustomOpen { .. }) =
                rx.recv_timeout(Duration::from_millis(100))
            {
                opened = true;
                break;
            }
        }
        assert!(opened, "第 {round} 轮应打开面板");
        let _ = host.custom_input("\r", 80).expect("按键");
        let result = invoke.join().expect("线程").expect("工具应成功");
        assert_eq!(result["content"][0]["text"], "answer:yes");
        let pending = futures::executor::block_on(host.pending_len());
        assert_eq!(pending, 0, "第 {round} 轮结束后不应残留直连请求");
    }
    host.shutdown();
}

/// F30/S3'：**确定性**复现"排干检查后才登记 custom_input"的竞态。
///
/// 做法：`PI_TEST_CUSTOM_INPUT_DELAY_MS` 让 `custom_input` 在算 `busy` 之前先睡，
/// 于是调用方可以在它"还没开始检查"时就起一个 invoke —— 等到检查时 invoke 已经在跑，
/// 按键必须走直连并被读到（否则会 5s 超时失败）。
#[test]
fn custom_input_started_before_invoke_is_delivered() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/custom_invoke-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.set_ctx(
        json!({"cwd": repo_root().display().to_string(), "mode": "tui", "hasUI": true,
        "model": "mock-model", "thinkingLevel": "off"}),
    );

    std::env::set_var("PI_TEST_CUSTOM_INPUT_DELAY_MS", "300");
    let keyer = {
        let host = host.clone();
        std::thread::spawn(move || host.custom_input("\r", 80))
    };
    // 让按键线程先进入"未开始检查"的窗口，再起 invoke。
    std::thread::sleep(Duration::from_millis(100));
    let invoker = {
        let host = host.clone();
        std::thread::spawn(move || {
            host.invoke("ext_ask_inline", json!({}), Duration::from_secs(20))
        })
    };
    let mut opened = false;
    for _ in 0..50 {
        if let Ok(pi_ext_host::UiFrame::CustomOpen { .. }) =
            rx.recv_timeout(Duration::from_millis(100))
        {
            opened = true;
            break;
        }
    }
    assert!(opened, "应打开自定义面板");
    let key_result = keyer.join().expect("按键线程");
    std::env::remove_var("PI_TEST_CUSTOM_INPUT_DELAY_MS");
    assert!(key_result.is_ok(), "按键应被投递：{key_result:?}");
    let result = invoker.join().expect("线程").expect("工具应成功");
    assert_eq!(result["content"][0]["text"], "answer:yes");
    host.shutdown();
}
