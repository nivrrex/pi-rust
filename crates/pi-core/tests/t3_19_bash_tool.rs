//! P3.1：`bash` 工具——成功、非零退出、超时、取消、工作目录、超时参数校验。

use pi_core::tools::bash::BashTool;
use pi_core::tools::tool::{Tool, ToolContext};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn ctx(dir: &std::path::Path) -> ToolContext {
    ToolContext {
        cwd: dir.to_path_buf(),
    }
}

#[tokio::test]
async fn runs_a_command_and_returns_its_stdout() {
    let dir = tempfile::tempdir().expect("临时目录");
    let result = BashTool
        .invoke(
            json!({"command": "echo hello"}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error, "{}", result.text_content());
    // **尾换行保留**：`golden/S05` 的 toolResult 原文是 `"hi\n"`（echo 的输出带换行），
    // 所以这里也应当是 `"hello\n"`，不做 trim。
    assert_eq!(result.text_content(), "hello\n");
}

#[tokio::test]
async fn silent_command_reports_no_output() {
    let dir = tempfile::tempdir().expect("临时目录");
    let result = BashTool
        .invoke(
            json!({"command": "true"}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error);
    assert_eq!(result.text_content(), "(no output)");
}

#[tokio::test]
async fn non_zero_exit_appends_the_status_and_is_an_error() {
    let dir = tempfile::tempdir().expect("临时目录");
    let result = BashTool
        .invoke(
            json!({"command": "echo out; exit 3"}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    // `appendStatus` 在**有输出**时插两个换行（`bash.ts:341`），输出自身还带一个尾换行。
    assert_eq!(result.text_content(), "out\n\n\nCommand exited with code 3");
}

#[tokio::test]
async fn stderr_is_captured() {
    let dir = tempfile::tempdir().expect("临时目录");
    let result = BashTool
        .invoke(
            json!({"command": "echo err >&2"}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error);
    assert_eq!(result.text_content(), "err\n");
}

#[tokio::test]
async fn timeout_kills_the_process_group_and_reports_seconds() {
    let dir = tempfile::tempdir().expect("临时目录");
    // 派生一个后代进程，若只杀直接子进程它会活下来并继续持有管道。
    let result = BashTool
        .invoke(
            json!({"command": "sleep 30", "timeout": 0.3}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error, "超时应是错误结果");
    assert!(
        result
            .text_content()
            .contains("Command timed out after 0.3 seconds"),
        "{}",
        result.text_content()
    );
}

/// 进程组清理的**行为**断言：超时后整个进程组（含 `sh -c` 派生出的孙子）都必须消失，
/// 且从发起到返回的总耗时不超过 `KILL_GRACE` 加一个宽裕余量。
///
/// 判据用"孙子写下的标记文件"：孙子先 `sleep 2` 再 `touch late.txt`。若只杀组长，孙子
/// 会活下来并在 2 秒后创建该文件；若整组被杀，文件永不出现。**这比断言文案强得多**——
/// 文案对、进程没死，正是之前 30 秒才跑完的那种情况。
#[tokio::test]
async fn timeout_kills_the_whole_process_group_within_the_bound() {
    let dir = tempfile::tempdir().expect("临时目录");
    let late = dir.path().join("late.txt");
    let started = std::time::Instant::now();
    let result = BashTool
        .invoke(
            json!({
                "command": "( sleep 2; touch late.txt ) & sleep 30",
                "timeout": 0.3,
            }),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    let elapsed = started.elapsed();

    assert!(result.is_error, "超时应是错误结果");
    // 上限：0.3 秒超时 + 0.5 秒 KILL_GRACE，给到 5 秒余量已非常宽裕；
    // 修复前只杀组长时，这里会等到孙子退出，耗时 ~2 秒以上。
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "整组清理应在 5 秒内完成，实际 {elapsed:?}"
    );
    // 等到孙子本该写文件的时刻之后，确认它没能活下来。
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert!(
        !late.exists(),
        "孙子进程活了下来（它写了 {}），说明只杀了组长、没有杀整组",
        late.display()
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "含清理的总耗时应在上限内"
    );
}

#[tokio::test]
async fn cancellation_reports_aborted() {
    let dir = tempfile::tempdir().expect("临时目录");
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        token.cancel();
    });

    let result = BashTool
        .invoke(json!({"command": "sleep 30"}), &ctx(dir.path()), &cancel)
        .await;
    assert!(result.is_error);
    assert!(
        result.text_content().contains("Command aborted"),
        "{}",
        result.text_content()
    );
}

#[tokio::test]
async fn missing_working_directory_is_reported() {
    let result = BashTool
        .invoke(
            json!({"command": "echo hi"}),
            &ctx(std::path::Path::new("/definitely/missing/dir")),
            &CancellationToken::new(),
        )
        .await;
    assert!(result.is_error);
    assert!(result
        .text_content()
        .starts_with("Working directory does not exist: "));
}

#[tokio::test]
async fn invalid_timeout_values_use_the_typescript_messages() {
    for bad in [json!(0), json!(-1)] {
        let dir = tempfile::tempdir().expect("临时目录");
        let result = BashTool
            .invoke(
                json!({"command": "true", "timeout": bad}),
                &ctx(dir.path()),
                &CancellationToken::new(),
            )
            .await;
        assert!(result.is_error);
        assert_eq!(
            result.text_content(),
            "Invalid timeout: must be a finite number of seconds"
        );
    }
}

#[tokio::test]
async fn cancelled_before_start_does_not_run_the_command() {
    let dir = tempfile::tempdir().expect("临时目录");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = BashTool
        .invoke(
            json!({"command": "touch should-not-exist"}),
            &ctx(dir.path()),
            &cancel,
        )
        .await;
    assert!(result.is_error);
    assert_eq!(result.text_content(), "Command aborted");
    assert!(
        !dir.path().join("should-not-exist").exists(),
        "取消后不得执行"
    );
}

#[tokio::test]
async fn truncated_output_is_fully_persisted_with_the_official_shape() {
    let dir = tempfile::tempdir().expect("临时目录");
    let result = BashTool
        .invoke(
            json!({"command": "python3 -c \"import sys; sys.stdout.write('x' * 100000)\""}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error, "{}", result.text_content());
    let details = result.details.expect("截断必须有 details");
    let path = details["fullOutputPath"].as_str().expect("完整输出路径");
    assert_eq!(std::fs::read(path).expect("读取完整输出").len(), 100000);
    assert_eq!(details["truncation"]["truncated"], true);
    assert_eq!(details["truncation"]["totalBytes"], 100000);
    assert_eq!(details["truncation"]["maxBytes"], 50 * 1024);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn concurrent_truncated_outputs_get_distinct_files() {
    let dir = tempfile::tempdir().expect("临时目录");
    let context = ctx(dir.path());
    let cancel = CancellationToken::new();
    let (first, second) = tokio::join!(
        BashTool.invoke(
            json!({"command": "python3 -c \"import sys; sys.stdout.write('a' * 60000)\""}),
            &context,
            &cancel,
        ),
        BashTool.invoke(
            json!({"command": "python3 -c \"import sys; sys.stdout.write('b' * 60000)\""}),
            &context,
            &cancel,
        ),
    );
    assert!(!first.is_error, "{}", first.text_content());
    assert!(!second.is_error, "{}", second.text_content());
    let first_path = first.details.expect("first details")["fullOutputPath"]
        .as_str()
        .expect("first path")
        .to_string();
    let second_path = second.details.expect("second details")["fullOutputPath"]
        .as_str()
        .expect("second path")
        .to_string();
    assert_ne!(first_path, second_path);
    assert_eq!(
        std::fs::read(&first_path).expect("first output"),
        vec![b'a'; 60000]
    );
    assert_eq!(
        std::fs::read(&second_path).expect("second output"),
        vec![b'b'; 60000]
    );
    let _ = std::fs::remove_file(first_path);
    let _ = std::fs::remove_file(second_path);
}

#[test]
fn schema_requires_command_and_allows_timeout() {
    assert_eq!(BashTool.schema()["required"], json!(["command"]));
    assert_eq!(BashTool.schema()["properties"]["timeout"]["type"], "number");
}

/// F52 债务②（F53c 补）：stdout / stderr **按到达顺序合并**，不是"先读完 stdout 再读 stderr"。
///
/// ref `core/tools/bash.ts:122-124`：两个流挂同一个 `onData` 回调，谁先到谁先写。
/// 改前实现先读完 stdout 再读 stderr ⇒ 输出被重排成 `out1 out2 err1 err2`。
#[tokio::test]
async fn stdout_and_stderr_merge_in_arrival_order() {
    let dir = tempfile::tempdir().expect("临时目录");
    // 交替写：out1 → err1 → out2 → err2，每步之间留足时间让另一端读完。
    let command = "echo out1; sleep 0.15; echo err1 1>&2; sleep 0.15; \
                   echo out2; sleep 0.15; echo err2 1>&2";
    let result = BashTool
        .invoke(
            json!({"command": command}),
            &ctx(dir.path()),
            &CancellationToken::new(),
        )
        .await;
    assert!(!result.is_error, "{}", result.text_content());
    let text = result.text_content();
    let order: Vec<&str> = text
        .lines()
        .filter(|line| ["out1", "err1", "out2", "err2"].contains(line))
        .collect();
    assert_eq!(
        order,
        vec!["out1", "err1", "out2", "err2"],
        "stdout/stderr 必须按到达顺序合并，实际：{text:?}"
    );
}
