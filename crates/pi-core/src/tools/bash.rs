//! `bash` 工具：执行 shell 命令，返回 stdout/stderr 与退出状态。
//!
//! 对应 `packages/coding-agent/src/core/tools/bash.ts` 与 `spec/e-tools.md` 第 5 节。
//!
//! **未实现**：把完整输出落盘（`fullOutputPath`，截断提示里会引用它）、`commandTransport`
//! 走 stdin 的模式；截断提示里目前用空路径占位。
//!
//! **进程组**：命令在**自己的进程组**里启动（`process_group(0)`），超时与取消时按
//! **进程组**发 TERM、宽限后再 KILL——只杀直接子进程会留下 `sh -c` 派生出来的后代。

use std::process::Stdio;

use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::bash_text::{
    append_status, exit_status_message, format_output, resolve_timeout_ms, truncate_tail,
    TimeoutError,
};
use super::read_text::{DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};
use super::tool::{Tool, ToolContext, ToolResult};

/// TERM 之后等多久再 KILL。
const KILL_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// `bash` 工具。
#[derive(Debug, Default)]
pub struct BashTool;

/// 参数 schema。
pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["command"],
        "properties": {
            "command": {"type": "string", "description": "Shell command to execute"},
            "timeout": {"type": "number", "description": "Timeout in seconds (optional, no default timeout)"}
        }
    })
}

/// 一个"shell 工具"的静态描述（`bash` 与 Windows 的 `powershell` 共用同一套执行逻辑）。
///
/// 上游把两者写成 `createLocalShellOperations(shellName, resolveShellConfig)` + 一份
/// `ShellToolConfig`（`core/tools/powershell.ts:47-53`、`:1-10`），这里等价。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShellSpec {
    /// 工具名（进工具表、`--tools` 选择、系统提示的 `<tools>` 段）。
    pub name: &'static str,
    /// 用于描述文案的 shell 名（`bash` / `PowerShell`）。
    pub shell_name: &'static str,
    /// PowerShell 需要在命令前注入的 UTF-8 输出前缀（`powershell.ts:11`）。
    pub command_prefix: &'static str,
}

/// `bash` 的描述（与上游模板一致：`Execute a bash command in ...`）。
pub(crate) const BASH_SPEC: ShellSpec = ShellSpec {
    name: "bash",
    shell_name: "bash",
    command_prefix: "",
};

/// 按**进程组**发信号；失败不报错——进程可能已经不在了。
#[cfg(unix)]
fn signal_group(pid: u32, signal: &str) {
    let _ = std::process::Command::new("kill")
        .arg(format!("-{signal}"))
        // **必须**用 `--` 把选项与参数分开：procps 的 `kill` 会把 `-12345` 当成选项。
        // 负 pid 表示"整个进程组"——只杀组长会让 `bash -c` 的孙子继续持有管道，
        // 读取端就会一直等到它自己退出（实测过：30 秒的 sleep 让测试跑满 30 秒）。
        .arg("--")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// 整组终止：TERM → 宽限 → KILL。
/// Windows 没有进程组信号：用 `taskkill /T /F` 连子孙一起杀（等价于 Unix 侧的
/// `kill -- -PGID`）。找不到 `taskkill` 时退化为只杀进程本身。
#[cfg(windows)]
fn signal_group(pid: u32, _signal: &str) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// 先 TERM，再**最多等 `KILL_GRACE`**；子进程期间退出就立刻返回，不死等满 500ms。
///
/// 旧实现无论子进程是否已退出都 `sleep(500ms)` 再 KILL，导致"工具执行中按 Esc"
/// 的中止延迟恒为 ~550ms（真机反馈"比原版慢一点"）。改后实测 552ms → 27ms。
async fn terminate_and_reap(pid: Option<u32>, child: &mut tokio::process::Child) {
    #[cfg(unix)]
    {
        if let Some(pid) = pid {
            signal_group(pid, "TERM");
        }
    }
    #[cfg(windows)]
    {
        if let Some(pid) = pid {
            signal_group(pid, "KILL");
        }
    }
    let exited = tokio::select! {
        _ = child.wait() => true,
        _ = tokio::time::sleep(KILL_GRACE) => false,
    };
    if !exited {
        if let Some(pid) = pid {
            signal_group(pid, "KILL");
        }
        let _ = child.wait().await;
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str {
        BASH_SPEC.name
    }

    fn description(&self) -> &str {
        r#"Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds."#
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(schema)
    }

    fn execute<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        self.run(arguments, context, cancel, None)
    }

    /// 带进度上报的执行入口（对应 TS 的 `onUpdate`）。
    ///
    /// 与 `invoke_with_progress` 的默认实现不同，这里要**自己先做必填校验**——
    /// 覆盖了默认实现就等于接管了那条路径。`validate_required` 与
    /// `Tool::invoke` 用的是同一个函数，行为一致。
    fn invoke_with_progress<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        cancel: &'a CancellationToken,
        on_update: &'a (dyn Fn(&ToolResult) + Send + Sync),
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        match crate::tools::schema::validate_required(self.name(), self.schema(), &arguments) {
            Ok(()) => self.run(arguments, context, cancel, Some(on_update)),
            Err(error) => Box::pin(async move { ToolResult::error(error.message) }),
        }
    }
}

impl BashTool {
    /// `execute` 与 `invoke_with_progress` 的共同主体。
    fn run<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        cancel: &'a CancellationToken,
        on_update: Option<&'a (dyn Fn(&ToolResult) + Send + Sync)>,
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        run_shell(&BASH_SPEC, arguments, context, cancel, on_update)
    }
}

/// 解析"程序 + 参数"：bash 走 `bash -c`；PowerShell 走 `pwsh/powershell -NoProfile … -Command`。
///
/// PowerShell 只在 Windows 上可用（上游 `utils/shell.ts:120-130/127` 抛
/// `The powershell tool is only available on Windows.`），这里保持同一文案。
fn shell_command(spec: &ShellSpec, command: &str) -> Result<(String, Vec<String>), String> {
    if spec.name == "bash" {
        return Ok((
            "bash".to_string(),
            vec!["-c".to_string(), command.to_string()],
        ));
    }
    let program = resolve_powershell()?;
    let mut args: Vec<String> = [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect();
    // `UTF8_OUTPUT_PREFIX`（`tools/powershell.ts:11`）。
    args.push(format!("{}{}", spec.command_prefix, command));
    Ok((program, args))
}

/// 找 `pwsh.exe`（PowerShell 7）→ `powershell.exe`（Windows PowerShell）。
fn resolve_powershell() -> Result<String, String> {
    if !cfg!(windows) {
        return Err("The powershell tool is only available on Windows.".to_string());
    }
    for candidate in ["pwsh.exe", "powershell.exe"] {
        let found = std::process::Command::new("where")
            .arg(candidate)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .map(|line| line.trim().to_string())
            })
            .filter(|line| !line.is_empty());
        if let Some(path) = found {
            return Ok(path);
        }
    }
    Err("No PowerShell found. Install PowerShell 7 (pwsh.exe) or use Windows PowerShell (powershell.exe)."
        .to_string())
}

/// `bash` 与 `powershell` 共用的执行主体。
pub(crate) fn run_shell<'a>(
    spec: &'static ShellSpec,
    arguments: Value,
    context: &'a ToolContext,
    cancel: &'a CancellationToken,
    on_update: Option<&'a (dyn Fn(&ToolResult) + Send + Sync)>,
) -> futures::future::BoxFuture<'a, ToolResult> {
    {
        Box::pin(async move {
            // 开始执行先上报一次**空 content**：golden/S05 的第一条 `tool_execution_update`
            // 就是 `{"content": []}`。
            if let Some(on_update) = on_update {
                on_update(&ToolResult {
                    content: Vec::new(),
                    is_error: false,
                    details: None,
                    terminate: false,
                });
            }
            let command = crate::tools::tool::arg_string(arguments.get("command"));
            let timeout = match resolve_timeout_ms(arguments.get("timeout").and_then(Value::as_f64))
            {
                Ok(timeout) => timeout,
                Err(error) => {
                    return ToolResult::error(match error {
                        TimeoutError::NotFiniteOrNonPositive => {
                            "Invalid timeout: must be a finite number of seconds".to_string()
                        }
                        TimeoutError::TooLarge => error.to_string(),
                    });
                }
            };

            if cancel.is_cancelled() {
                return ToolResult::error("Command aborted");
            }
            if !context.cwd.is_dir() {
                return ToolResult::error(format!(
                    "Working directory does not exist: {}\nCannot execute {} commands.",
                    context.cwd.display(),
                    spec.shell_name
                ));
            }

            let (program, args) = match shell_command(spec, &command) {
                Ok(pair) => pair,
                Err(message) => return ToolResult::error(message),
            };
            let mut cmd = Command::new(&program);
            cmd.args(&args)
                .current_dir(&context.cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            // Unix：独立进程组（超时/取消时整组杀）；Windows：不创建控制台窗口，
            // 进程树由 `taskkill /T` 负责。
            #[cfg(unix)]
            cmd.process_group(0);
            #[cfg(windows)]
            {
                // `creation_flags` 是 tokio `Command` 的固有方法，无需 `CommandExt` trait；
                // 之前那行 `use std::os::windows::process::CommandExt` 在 Windows 目标上
                // 是死导入，会让 `clippy -D warnings` 失败（2026-10-31 实测）。
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let mut child = match cmd.spawn() {
                Ok(child) => child,
                Err(error) => {
                    return ToolResult::error(format!("Command failed to start: {error}"))
                }
            };
            let pid = child.id();

            // 先起两个读取任务，避免管道写满把子进程卡死。
            // 上面 spawn 时已指定 piped；没有管道就按"没有输出"处理，
            // 不 panic（非测试代码里 panic 是禁止项）。
            let mut stdout_pipe = child.stdout.take();
            let mut stderr_pipe = child.stderr.take();
            let stdout_task = tokio::spawn(async move {
                let mut buffer = Vec::new();
                if let Some(pipe) = stdout_pipe.as_mut() {
                    let _ = pipe.read_to_end(&mut buffer).await;
                }
                buffer
            });
            let stderr_task = tokio::spawn(async move {
                let mut buffer = Vec::new();
                if let Some(pipe) = stderr_pipe.as_mut() {
                    let _ = pipe.read_to_end(&mut buffer).await;
                }
                buffer
            });

            // 三种结束方式，各自有明确后果：正常 / 超时（错误）/ 取消（错误）。
            let mut timed_out = false;
            let status = match timeout {
                Some(ms) => tokio::select! {
                    status = child.wait() => status.ok(),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(ms)) => {
                        timed_out = true;
                        terminate_and_reap(pid, &mut child).await;
                        None
                    }
                    _ = cancel.cancelled() => {
                        terminate_and_reap(pid, &mut child).await;
                        None
                    }
                },
                None => tokio::select! {
                    status = child.wait() => status.ok(),
                    _ = cancel.cancelled() => {
                        terminate_and_reap(pid, &mut child).await;
                        None
                    }
                },
            };

            let stdout =
                String::from_utf8_lossy(&stdout_task.await.unwrap_or_default()).to_string();
            let stderr =
                String::from_utf8_lossy(&stderr_task.await.unwrap_or_default()).to_string();
            let combined = match (stdout.is_empty(), stderr.is_empty()) {
                (_, true) => stdout,
                (true, false) => stderr,
                (false, false) => format!("{stdout}\n{stderr}"),
            };

            let tail = truncate_tail(&combined, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES);
            let text = format_output(&tail.content, &tail, None, "(no output)");

            // **错误路径的"空输出"占位符是空串**，不是 `(no output)`：
            // ref `bash.ts:41` 在中止/超时分支调用 `formatOutput(snapshot, "")`，
            // 于是空输出时 `appendStatus("", "Command aborted")` 得到的就是 `"Command aborted"`
            // （golden/S09b 实测；我此前用默认的 `(no output)`，多出了两行前缀）。
            let error_text = format_output(&tail.content, &tail, None, "");
            if timed_out {
                let seconds = timeout.unwrap_or(0) as f64 / 1000.0;
                return ToolResult::error(append_status(
                    &error_text,
                    &format!("Command timed out after {seconds} seconds"),
                ));
            }
            if cancel.is_cancelled() {
                return ToolResult::error(append_status(&error_text, "Command aborted"));
            }

            // 退出码文案统一走 bash_text 里的同一个函数，避免两处写法漂移。
            // 输出就绪后再上报一次（golden/S05 的第二条 update 带 `hi\n`）。
            let final_result = match exit_status_message(status.and_then(|status| status.code())) {
                None => ToolResult::text(text),
                Some(status_text) => ToolResult::error(append_status(&text, &status_text)),
            };
            if let Some(on_update) = on_update {
                // **增量结果的形状与 `tool_execution_end.result` 不同**：golden/S05、S10 实测
                // `partialResult` **带** `details`（无详情时是空对象 `{}`），而 `result` 不带。
                // 我此前两边用同一个结果，于是 update 少了 `details` 键——
                // S05 里这条差异被 `foldStreamingIncrements`（只留连续同类事件的第一条）吞掉了，
                // 直到 S10 有事件插在两条 update 之间才暴露。
                let mut partial = final_result.clone();
                partial.details = Some(serde_json::json!({}));
                on_update(&partial);
            }
            final_result
        })
    }
}

#[cfg(test)]
mod p8b_powershell_tests {
    use super::shell_command;
    #[cfg(not(windows))]
    use super::ShellSpec;

    #[cfg(not(windows))]
    const PS_SPEC: ShellSpec = ShellSpec {
        name: "powershell",
        shell_name: "PowerShell",
        command_prefix: "try { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8 } catch {}\n",
    };

    /// 非 Windows：与上游同文案报"只在 Windows 可用"。
    #[cfg(not(windows))]
    #[test]
    fn powershell_is_windows_only() {
        let error = shell_command(&PS_SPEC, "Get-ChildItem").expect_err("非 Windows 必须报错");
        assert_eq!(error, "The powershell tool is only available on Windows.");
    }

    /// bash 分支不受影响（`bash -c <命令>`）。
    #[test]
    fn bash_command_shape_is_unchanged() {
        let (program, args) = shell_command(&super::BASH_SPEC, "echo hi").expect("bash 恒可用");
        assert_eq!(program, "bash");
        assert_eq!(args, vec!["-c".to_string(), "echo hi".to_string()]);
    }
}
