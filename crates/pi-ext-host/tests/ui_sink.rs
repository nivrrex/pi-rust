//! UI 上行通道：`host.ui.notify` / `setStatus` 必须投递到 `set_ui_sink` 里给的 channel。

use std::path::PathBuf;
use std::time::Duration;

use pi_ext_host::{HostHandle, HostOptions};

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

#[test]
fn notify_and_status_reach_the_sink() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let host = HostHandle::start(HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: vec![repo_root().join("tools/ext-host/fixtures/ui-ext.mjs")],
        cwd: repo_root(),
        env: Vec::new(),
    })
    .expect("宿主应能启动");
    let (tx, rx) = std::sync::mpsc::channel();
    host.set_ui_sink(Some(tx));
    host.emit(
        "session_start",
        serde_json::json!({"type": "session_start", "reason": "startup"}),
        Duration::from_secs(10),
    )
    .expect("session_start 应成功");
    let mut frames = Vec::new();
    while let Ok(frame) = rx.recv_timeout(Duration::from_millis(200)) {
        frames.push(format!("{frame:?}"));
    }
    assert!(
        frames.iter().any(|f| f.contains("plugin ready")),
        "应收到 notify：{frames:?}"
    );
    assert!(
        frames.iter().any(|f| f.contains("alpha-status")),
        "应收到 setStatus：{frames:?}"
    );
    host.shutdown();
}

/// 目录内 `package.json` 的 `pi.extensions` 清单优先（ref `loader.ts:667-687`）。
#[test]
fn package_manifest_entries_win_over_index() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let root = tmp.path().join("extensions");
    let pkg_dir = root.join("with-manifest");
    std::fs::create_dir_all(pkg_dir.join("dist")).expect("建目录");
    std::fs::write(
        pkg_dir.join("package.json"),
        r#"{"pi":{"extensions":["dist/a.ts","missing.ts"]}}"#,
    )
    .expect("写 package.json");
    std::fs::write(pkg_dir.join("dist/a.ts"), "export default () => {};").expect("写入口");
    std::fs::write(pkg_dir.join("index.ts"), "export default () => {};").expect("写 index");
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).expect("建目录");
    std::fs::write(plain.join("index.js"), "export default () => {};").expect("写 index");
    let direct = root.join("direct.mjs");
    std::fs::write(&direct, "export default () => {};").expect("写直接文件");

    let found = pi_ext_host::discover_extensions(&root);
    let names: Vec<String> = found
        .iter()
        .map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string())
        .collect();
    // 排序后：`direct.mjs`、`plain/index.js`、`with-manifest/dist/a.ts`
    //（清单里不存在的 `missing.ts` 被丢掉）。
    assert_eq!(
        names,
        vec![
            "direct.mjs".to_string(),
            "plain/index.js".to_string(),
            "with-manifest/dist/a.ts".to_string()
        ],
        "清单存在时不应回落到 index.ts，且缺失项要被忽略"
    );
}
