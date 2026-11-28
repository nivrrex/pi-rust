//! 把扩展注册的工具接成 `pi_core::tools::tool::Tool`（**线程安全**：内部 `Arc<Mutex<ExtHost>>`，
//! 同一扩展宿主上的调用串行化——协议本身是单连接请求/响应）。

use futures::future::BoxFuture;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::actor::HostHandle;
use crate::client::HostError;
use crate::handshake::ToolSpec;
use crate::protocol::TOOL_TIMEOUT_MS;

/// 一个由扩展实现、宿主可见的工具。
pub struct ExtTool {
    spec: ToolSpec,
    host: HostHandle,
}

impl ExtTool {
    /// 基于注册信息与宿主句柄构造。
    #[must_use]
    pub fn new(spec: ToolSpec, host: HostHandle) -> Self {
        Self { spec, host }
    }

    /// 工具名（供 `--list-tools` 等使用）。
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.spec.name
    }
}

impl pi_core::tools::tool::Tool for ExtTool {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn description(&self) -> &str {
        &self.spec.description
    }

    fn schema(&self) -> &Value {
        &self.spec.parameters
    }

    fn execute<'a>(
        &'a self,
        arguments: Value,
        _context: &'a pi_core::tools::tool::ToolContext,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, pi_core::tools::tool::ToolResult> {
        Box::pin(async move {
            use pi_core::tools::tool::{ToolResult, ToolResultBlock};
            let timeout = std::time::Duration::from_millis(TOOL_TIMEOUT_MS);
            let call = self
                .host
                .clone()
                .invoke_async(self.spec.name.clone(), arguments, timeout);
            let result = tokio::select! {
                biased;
                () = cancel.cancelled() => Err(HostError::Protocol("aborted".to_string())),
                outcome = call => outcome,
            };
            match result {
                Ok(value) => {
                    // 扩展返回 `{content:[{type:"text",text}], details?, isError?}`（ref `:487`）。
                    let mut blocks = Vec::new();
                    if let Some(items) = value.get("content").and_then(Value::as_array) {
                        for item in items {
                            match item.get("type").and_then(Value::as_str) {
                                Some("text") => blocks.push(ToolResultBlock::Text {
                                    text: item
                                        .get("text")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string(),
                                }),
                                Some("image") => blocks.push(ToolResultBlock::Image {
                                    data: item
                                        .get("data")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string(),
                                    mime_type: item
                                        .get("mimeType")
                                        .and_then(Value::as_str)
                                        .unwrap_or("image/png")
                                        .to_string(),
                                }),
                                _other => {}
                            }
                        }
                    }
                    if blocks.is_empty() {
                        blocks.push(ToolResultBlock::Text {
                            text: serde_json::to_string(&value).unwrap_or_default(),
                        });
                    }
                    ToolResult {
                        content: blocks,
                        is_error: value
                            .get("isError")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        details: value.get("details").cloned(),
                        terminate: false,
                    }
                }
                Err(error) => ToolResult::error(format!("extension tool failed: {error}")),
            }
        })
    }
}
