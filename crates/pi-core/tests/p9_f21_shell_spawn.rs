//! P9-F21/S3b：`shellPath` 指向**路径含空格**的伪 bash 时，bash 工具的真实 spawn 行为。
//!
//! 断言：① 命令作为**独立参数**传入（不拼接字符串）；② cwd 正确；③ 超时能终止该进程与其子进程。
//!
//! 用测试里的可执行脚本冒充 bash：它把 `$@` 与 `$PWD` 记录到文件，并按第一个参数决定行为。

use std::path::Path;

use pi_core::tools::bash::BashTool;
use pi_core::tools::tool::{Tool, ToolContext};
use tokio_util::sync::CancellationToken;

/// 两个测试共用同一个伪 bash 目录：`set_shell_path` 是进程级 OnceLock，先到先得；
/// 若各用各的临时目录，后一个测试的命令会打到前一个目录里的脚本上（实测会 flake）。
fn shared_fake() -> &'static std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("p9f21-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建共享目录");
        let path = write_fake_bash(&dir);
        pi_core::tools::shell_resolve::set_shell_path(Some(&path.display().to_string()));
        path
    })
}

fn write_fake_bash(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("fake bash/bin/my bash");
    std::fs::create_dir_all(path.parent().expect("父目录")).expect("建目录");
    // `-c <command>`：把参数与 cwd 记录到 log，再执行命令（用系统 bash 解释，保证 sleep 之类可用）。
    let script = "#!/bin/bash\n\
        echo \"ARGC=$#\" >> \"$(dirname \"$0\")/log\"\n\
        for a in \"$@\"; do echo \"ARG=[$a]\" >> \"$(dirname \"$0\")/log\"; done\n\
        echo \"PWD=$PWD\" >> \"$(dirname \"$0\")/log\"\n\
        exec /bin/bash \"$@\"\n";
    std::fs::write(&path, script).expect("写脚本");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("元数据").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    path
}

#[tokio::test]
async fn fake_bash_with_spaces_receives_command_as_single_argv() {
    let fake = shared_fake();
    let tmp = tempfile::tempdir().expect("临时目录");
    assert!(fake.exists(), "伪 bash 应存在");

    let work = tmp.path().join("work dir");
    std::fs::create_dir_all(&work).expect("建工作目录");
    let context = ToolContext { cwd: work.clone() };
    let cancel = CancellationToken::new();
    let result = BashTool
        .execute(
            serde_json::json!({"command": "echo hello-from-fake"}),
            &context,
            &cancel,
        )
        .await;
    assert!(!result.is_error, "应执行成功：{}", result.text_content());
    assert!(
        result.text_content().contains("hello-from-fake"),
        "输出应含命令结果：{}",
        result.text_content()
    );

    let log =
        std::fs::read_to_string(fake.parent().expect("父目录").join("log")).unwrap_or_default();
    assert!(log.contains("ARGC=2"), "应传两个参数（-c 与命令）：{log}");
    assert!(
        log.contains("ARG=[-c]") && log.contains("ARG=[echo hello-from-fake]"),
        "命令必须作为**独立参数**传入（不拼接）：{log}"
    );
    assert!(
        log.contains(&format!("PWD={}", work.display())),
        "cwd 应为工具上下文的目录：{log}"
    );
}

#[tokio::test]
async fn fake_bash_timeout_kills_the_process_tree() {
    let _fake = shared_fake();
    let tmp = tempfile::tempdir().expect("临时目录");
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).expect("建工作目录");
    let context = ToolContext { cwd: work };
    let cancel = CancellationToken::new();
    let started = std::time::Instant::now();
    let result = BashTool
        .execute(
            serde_json::json!({"command": "sleep 30", "timeout": 1}),
            &context,
            &cancel,
        )
        .await;
    assert!(result.is_error, "超时应标记错误");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(15),
        "应在超时后很快返回，实际 {:?}",
        started.elapsed()
    );
    let text = result.text_content().to_lowercase();
    assert!(
        text.contains("timed out") || text.contains("timeout"),
        "应提示超时：{text}"
    );
}
