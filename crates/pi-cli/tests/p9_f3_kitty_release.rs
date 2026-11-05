//! F3：kitty 抬起事件迟到时的退出残留（用户现场 `0;5:3u`）。
//!
//! 复现：按下 `\x1b[100;5u` 后 60 ms 才发抬起 `\x1b[100;5:3u`，此时进程已退出。
//! 无修复的产物（`publish-20261101y`）会让 shell 回显 `0;5:3u`；有修复则残留为空。
//!
//! 断言只看"是否出现按键残片"，不看字节数：极端时序下会把退出序列本身算进残留（无害）。

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_helper(delay_ms: u64, force_release: bool) -> String {
    let agent = tempfile::tempdir().expect("agent dir");
    std::fs::create_dir_all(agent.path().join("sess")).expect("建 sess");
    let output = Command::new("python3")
        .arg(repo_root().join("tools/difftest/kitty-release.py"))
        .arg("--bin")
        .arg(env!("CARGO_BIN_EXE_pi-rs"))
        .arg("--agent-dir")
        .arg(agent.path())
        .arg("--delay-ms")
        .arg(delay_ms.to_string())
        .args(if force_release {
            vec!["--force-release"]
        } else {
            vec![]
        })
        .output()
        .expect("运行 kitty-release.py");
    assert!(
        output.status.success(),
        "helper 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|line| line.starts_with("leak="))
        .unwrap_or_else(|| panic!("helper 没有输出 leak 行：{text}"))
        .to_string()
}

/// 断言该延迟下没有按键残片（`0;5:3u` 的十六进制是 `30 3b 35 3a 33 75`）。
fn assert_no_fragment(line: &str) {
    assert!(
        !line.contains("303b353a3375") && !line.contains("0;5:3u"),
        "退出后 shell 不应读到按键残片：{line}"
    );
}

#[test]
fn kitty_release_after_exit_does_not_leak_to_the_shell() {
    // 合规终端：应用 push 的 flags 没开位2，终端**根本不发**抬起事件 ⇒ 与延迟无关。
    assert_no_fragment(&run_helper(60, false));
    assert_no_fragment(&run_helper(600, false));
}

/// 非合规终端（不加区分地发抬起事件）：F3 的兜底 drain 仍要吃掉 60 ms 的迟到抬起。
#[test]
fn hostile_terminal_is_covered_by_the_fallback_drain() {
    assert_no_fragment(&run_helper(60, true));
}

/// 空闲窗口的下界钉在这里：200 ms 的迟到抬起必须仍被吃掉（窗口从 150 提到 250 之后）。
#[test]
fn late_release_within_the_idle_window_is_still_drained() {
    assert_no_fragment(&run_helper(120, true));
}
