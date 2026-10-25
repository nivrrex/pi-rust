//! `powershell` 工具（**仅 Windows**）。
//!
//! 对应上游 `packages/coding-agent/src/core/tools/powershell.ts`：它复用 bash 的
//! `createShellToolDefinition`，只是换了一套 `ShellToolConfig`：
//! - 名字/标签 `powershell`、`shellName = "PowerShell"`、UI 提示符 `PS>`；
//! - 系统提示片段 `Execute PowerShell commands`；
//! - 命令前注入 `try { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8 } catch {}`（`:11`）；
//! - shell 解析 `pwsh.exe` → `powershell.exe`，参数
//!   `-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command`（`utils/shell.ts:120-130`）。
//!
//! 非 Windows 上执行会返回 `The powershell tool is only available on Windows.`（与上游同文案），
//! 但工具本身仍可出现在工具表里（`--tools powershell` 可选中），与上游一致。

use super::bash::{run_shell, ShellSpec};

/// PowerShell 的 UTF-8 输出前缀（`tools/powershell.ts:11`）。
pub const UTF8_OUTPUT_PREFIX: &str =
    "try { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8 } catch {}\n";

/// `powershell` 工具。
#[derive(Debug, Default)]
pub struct PowerShellTool;

const SPEC: ShellSpec = ShellSpec {
    name: "powershell",
    shell_name: "PowerShell",
    command_prefix: UTF8_OUTPUT_PREFIX,
};

impl super::tool::Tool for PowerShellTool {
    fn name(&self) -> &str {
        SPEC.name
    }

    fn description(&self) -> &str {
        r#"Execute a PowerShell command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds."#
    }

    fn schema(&self) -> &serde_json::Value {
        static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(super::bash::schema)
    }

    fn execute<'a>(
        &'a self,
        arguments: serde_json::Value,
        context: &'a super::tool::ToolContext,
        cancel: &'a tokio_util::sync::CancellationToken,
    ) -> futures::future::BoxFuture<'a, super::tool::ToolResult> {
        run_shell(&SPEC, arguments, context, cancel, None)
    }

    fn invoke_with_progress<'a>(
        &'a self,
        arguments: serde_json::Value,
        context: &'a super::tool::ToolContext,
        cancel: &'a tokio_util::sync::CancellationToken,
        on_update: &'a (dyn Fn(&super::tool::ToolResult) + Send + Sync),
    ) -> futures::future::BoxFuture<'a, super::tool::ToolResult> {
        match super::schema::validate_required(self.name(), self.schema(), &arguments) {
            Ok(()) => run_shell(&SPEC, arguments, context, cancel, Some(on_update)),
            Err(error) => Box::pin(async move { super::tool::ToolResult::error(error.message) }),
        }
    }
}
