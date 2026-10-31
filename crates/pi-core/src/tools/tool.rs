//! `Tool` 抽象：名称、描述、JSON schema 参数、执行入口。
//!
//! 对应 `packages/coding-agent/src/core/tools/*.ts` 的工具定义 + RUNBOOK-4 的 P3.1。
//!
//! **线程安全**：`Tool` 要求 `Send + Sync`；`execute` 接受 `&self`，实现内部若有共享可变
//! 状态必须自行同步。`ToolContext` 与取消令牌以只读引用传入，实现不得长期持有。

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::schema::validate_required;

/// 工具结果里的内容块。
///
/// 形状与会话里 `toolResult` 的内容一致（图片用 `data` + `mime_type`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolResultBlock {
    /// 文本。
    Text {
        /// 文本内容。
        text: String,
    },
    /// 图片（base64 负载与 MIME 类型）。
    Image {
        /// base64 负载，不含前缀。
        data: String,
        /// MIME 类型。
        mime_type: String,
    },
}

/// 工具参数转字符串，语义同 JS 的 `String(value)`。
///
/// **有 golden 证据**：`golden/S26b` 里 `{"path": 123}` 被 TS **接受**，解析出的路径是
/// `<TMP>/work/123`——数字被转成了十进制字符串。我原先统一用
/// `value.as_str().unwrap_or_default()`，非字符串一律变空串，于是 `123` 解析到 cwd，
/// 产出 `EIO: i/o error, access '<TMP>/'`，与 TS 不一致（由 `t3_26_loop_scenarios` 抓出）。
///
/// `String()` 的语义按 JS 规范：`String([1,2]) == "1,2"`、`String([]) == ""`、
/// `String(null) == "null"`、普通对象是 `"[object Object]"`、数字取十进制表示。
pub fn arg_string(value: Option<&Value>) -> String {
    match value {
        None => String::new(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| arg_string(Some(item)))
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}

/// 工具执行结果。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    /// 内容块（文本/图片）。
    pub content: Vec<ToolResultBlock>,
    /// 是否为错误结果。
    pub is_error: bool,
    /// 工具自定义的附加数据（如截断信息）。
    pub details: Option<Value>,
    /// 该工具要求**提前终止本次工具批次**（`types.ts:73` 的 `terminate?: boolean`）。
    ///
    /// 整批的终止条件是"批次非空**且**每个已 finalize 的结果都为 true"，
    /// 见 `agent::should_terminate_batch`（`agent-loop.ts:685-687`）。
    pub terminate: bool,
}

impl ToolResult {
    /// 纯文本的成功结果。
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolResultBlock::Text { text: text.into() }],
            is_error: false,
            details: None,
            terminate: false,
        }
    }

    /// 纯文本的错误结果。
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolResultBlock::Text { text: text.into() }],
            is_error: true,
            details: None,
            terminate: false,
        }
    }

    /// 把内容块拼成一段文本（文本块之间用换行，图片以占位符表示）。
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .map(|block| match block {
                ToolResultBlock::Text { text } => text.clone(),
                // mime 本身已含 `image/` 前缀（如 `image/png`）⇒ 此前会拼成 `[image/image/png]`。
                ToolResultBlock::Image { mime_type, .. } => format!("[{mime_type}]"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 工具执行时的只读上下文。
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// 工作目录；工具里的相对路径以它为基准。
    pub cwd: std::path::PathBuf,
}

/// 工具抽象。
pub trait Tool: Send + Sync {
    /// 工具名（与系统提示、请求体里的 `name` 一致）。
    fn name(&self) -> &str;

    /// 工具描述（进请求体的 `description`）。
    fn description(&self) -> &str;

    /// 参数 JSON schema。
    fn schema(&self) -> &Value;

    /// 执行。入参已通过 [`Tool::invoke`] 的必填校验。
    fn execute<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, ToolResult>;

    /// 校验 + 执行。
    ///
    /// **校验失败时不执行**，直接把错误文本当作工具结果返回——这与 TS 一致
    /// （`golden/S26`：缺必填时模型收到的是 `Validation failed for tool "read": …`）。
    fn invoke<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        match validate_required(self.name(), self.schema(), &arguments) {
            Ok(()) => self.execute(arguments, context, cancel),
            Err(error) => Box::pin(async move { ToolResult::error(error.message) }),
        }
    }

    /// 带**进度上报**的执行入口。
    ///
    /// 默认实现直接转给 [`Tool::invoke`] 并忽略回调——多数工具没有流式输出，
    /// 这样加它不需要改动那三个工具与既有测试。有流式输出的工具（bash）覆盖它。
    ///
    /// 语义对应 TS 的 `onUpdate` 回调：每次上报产出一条 `tool_execution_update` 事件
    /// （golden/S05 实测：一次调用会有**多条** update，第一条是空 `content`）。
    fn invoke_with_progress<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        cancel: &'a CancellationToken,
        on_update: &'a (dyn Fn(&ToolResult) + Send + Sync),
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        let _ = on_update;
        self.invoke(arguments, context, cancel)
    }
}
