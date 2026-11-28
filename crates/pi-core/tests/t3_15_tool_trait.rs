//! P3.1：`Tool` 抽象与"校验失败不执行"的接线。
//!
//! 判据：`golden/S26` 的校验原文（缺必填时模型收到的就是它），以及"通过校验才执行"。

use pi_core::tools::tool::{Tool, ToolContext, ToolResult, ToolResultBlock};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 记录执行次数的假工具。
struct CountingTool {
    name: String,
    schema: Value,
    calls: Arc<AtomicU32>,
}

impl Tool for CountingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "测试用工具"
    }

    fn schema(&self) -> &Value {
        &self.schema
    }

    fn execute<'a>(
        &'a self,
        arguments: Value,
        _context: &'a ToolContext,
        _cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ToolResult {
                content: vec![ToolResultBlock::Text {
                    text: format!("收到 {arguments}"),
                }],
                is_error: false,
                details: None,
                terminate: false,
            }
        })
    }
}

fn tool(schema: Value, calls: &Arc<AtomicU32>) -> CountingTool {
    CountingTool {
        name: "read".to_string(),
        schema,
        calls: Arc::clone(calls),
    }
}

fn context() -> ToolContext {
    ToolContext {
        cwd: std::path::PathBuf::from("/tmp"),
    }
}

fn read_schema() -> Value {
    json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}}})
}

#[tokio::test]
async fn missing_required_returns_the_error_without_executing() {
    let calls = Arc::new(AtomicU32::new(0));
    let tool = tool(read_schema(), &calls);
    let cancel = CancellationToken::new();

    let result = tool.invoke(json!({}), &context(), &cancel).await;

    assert!(result.is_error, "缺必填应返回错误结果");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "校验失败时**不得执行**工具"
    );
    assert!(
        result
            .text_content()
            .starts_with("Validation failed for tool \"read\":"),
        "错误文本应是 TS 的校验文案，实际：{}",
        result.text_content()
    );
    assert!(result
        .text_content()
        .contains("  - path: must have required properties path"));
}

#[tokio::test]
async fn valid_arguments_execute_the_tool() {
    let calls = Arc::new(AtomicU32::new(0));
    let tool = tool(read_schema(), &calls);
    let cancel = CancellationToken::new();

    let result = tool
        .invoke(json!({"path": "a.txt"}), &context(), &cancel)
        .await;

    assert!(!result.is_error);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "通过校验才执行");
    assert!(result.text_content().contains("a.txt"));
}

#[tokio::test]
async fn wrong_type_still_executes_because_typescript_accepts_it() {
    // golden/S26b：TS 不校验类型。我们若在 invoke 里加类型检查，就会拦下 TS 会放行的调用。
    let calls = Arc::new(AtomicU32::new(0));
    let tool = tool(read_schema(), &calls);
    let cancel = CancellationToken::new();

    let result = tool.invoke(json!({"path": 123}), &context(), &cancel).await;

    assert!(!result.is_error, "类型错不该被拦下");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn text_content_renders_image_blocks_as_placeholders() {
    let result = ToolResult {
        content: vec![
            ToolResultBlock::Text {
                text: "Read image file [image/png]".to_string(),
            },
            ToolResultBlock::Image {
                data: "AAAA".to_string(),
                mime_type: "image/png".to_string(),
            },
        ],
        is_error: false,
        details: None,
        terminate: false,
    };
    assert_eq!(
        result.text_content(),
        "Read image file [image/png]\n[image/png]"
    );
}
