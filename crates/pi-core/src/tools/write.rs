//! `write` 工具：把内容写入文件（不存在则创建、存在则覆盖）。
//!
//! 对应 `packages/coding-agent/src/core/tools/write.ts` 与 `spec/e-tools.md` 第 3 节。
//!
//! **未实现**：TS 把执行体放在"文件变更队列"里（同一次会话的写操作串行），本项目还没有
//! 那个队列；目前每次写入独立进行。这一点在 `notes/p2-reconcile.md` 里记着。

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::tool::{Tool, ToolContext, ToolResult};

/// `write` 工具。
#[derive(Debug, Default)]
pub struct WriteTool;

/// 参数 schema：两个必填字符串。
pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["path", "content"],
        "properties": {
            "path": {"type": "string", "description": "Path to the file to write (relative or absolute)"},
            "content": {"type": "string", "description": "Content to write to the file"}
        }
    })
}

impl WriteTool {
    /// 新建。
    pub fn new() -> Self {
        Self
    }
}

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories."
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
        Box::pin(async move {
            // 参数名来自 schema；校验已在 invoke 里做过，这里缺失只可能是调用方绕过 invoke。
            let path = crate::tools::tool::arg_string(arguments.get("path"));
            let path = path.as_str();
            let content = crate::tools::tool::arg_string(arguments.get("content"));

            // 取消检查点：每个 await 之前都检查，与 TS 的 `throwIfAborted()` 位置对应。
            if cancel.is_cancelled() {
                return ToolResult::error("Operation aborted");
            }

            let absolute = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                context.cwd.join(path)
            };

            if let Some(parent) = absolute.parent() {
                if let Err(error) = tokio::fs::create_dir_all(parent).await {
                    return ToolResult::error(format!(
                        "Could not create directory {}: {error}",
                        parent.display()
                    ));
                }
            }
            if cancel.is_cancelled() {
                return ToolResult::error("Operation aborted");
            }

            match tokio::fs::write(&absolute, content).await {
                // 成功文本用**原始入参 path**，不是绝对路径（`write.ts` 的实测）。
                Ok(()) => ToolResult::text(format!("Successfully wrote to {path}")),
                Err(error) => ToolResult::error(format!("Could not write file {path}: {error}")),
            }
        })
    }
}
