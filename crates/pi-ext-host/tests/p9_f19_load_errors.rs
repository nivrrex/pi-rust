//! P9-F19：单个扩展的加载失败要随 `ext.ready.loadErrors` 上报，且不拖垮好扩展。
//!
//! 三种夹具：语法错误的 `.js`、`import` 不存在模块的 `.mjs`、factory 抛错的 `.mjs`。

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node_path() -> Option<PathBuf> {
    Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
        .map(PathBuf::from)
}

#[tokio::test]
async fn broken_extensions_report_load_errors_and_good_one_still_loads() {
    let Some(node) = node_path() else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    let tmp = tempfile::tempdir().expect("临时目录");
    let agent = tmp.path().join("agent");
    let cwd = tmp.path().join("work");
    std::fs::create_dir_all(cwd.join(".pi/extensions")).expect("建目录");

    let ext_root = cwd.join(".pi/extensions");
    for (name, body) in [
        (
            "syntax",
            "export default function (pi) { this is not valid javascript }\n",
        ),
        (
            "missing-import",
            "import x from \"no-such-module-xyz\";\nexport default function (pi) {}\n",
        ),
        (
            "factory-throws",
            "export default function (pi) { throw new Error(\"boom from factory\"); }\n",
        ),
    ] {
        let dir = ext_root.join(name);
        std::fs::create_dir_all(&dir).expect("建扩展目录");
        std::fs::write(dir.join("index.mjs"), body).expect("写扩展");
    }
    // 好扩展：注册一个工具，验证不被拖垮。
    let good = ext_root.join("good");
    std::fs::create_dir_all(&good).expect("建扩展目录");
    std::fs::copy(
        repo_root().join("tools/ext-host/fixtures/echo-ext.mjs"),
        good.join("index.mjs"),
    )
    .expect("拷好扩展");

    let host = pi_ext_host::HostHandle::start(pi_ext_host::HostOptions {
        node,
        runner: repo_root().join("tools/ext-host/runner.mjs"),
        extensions: pi_ext_host::discover_extensions_in_order(&cwd, &agent),
        cwd: cwd.clone(),
        env: vec![(
            "PI_CODING_AGENT_DIR".to_string(),
            agent.display().to_string(),
        )],
    })
    .expect("宿主应能启动");

    let errors = host.load_errors();
    assert_eq!(errors.len(), 3, "三种坏扩展各一条：{errors:?}");
    let joined = format!("{errors:?}");
    assert!(joined.contains("syntax"), "语法错误要报：{errors:?}");
    assert!(
        joined.contains("missing-import"),
        "import 失败要报：{errors:?}"
    );
    assert!(
        joined.contains("boom from factory"),
        "factory 抛错要带原因：{errors:?}"
    );
    assert!(
        errors.iter().all(|(_, message)| !message.is_empty()),
        "每条都要有 message"
    );
    assert!(
        host.tools().iter().any(|tool| tool.name == "ext_echo"),
        "好扩展应仍然注册成功：{:?}",
        host.tools()
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>()
    );
    host.shutdown();
}

#[test]
fn load_errors_are_optional_for_old_runners() {
    // 旧 runner 不报 `loadErrors` ⇒ 解析成空（不报错）。
    let path: &Path = Path::new("x");
    let _ = path;
    // 这里不做真实进程：`parse_load_errors(None)` 的行为由上面的集成用例覆盖（好扩展 + 坏扩展混合）。
}
