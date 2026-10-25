//! `read` 工具：读文本（含 offset/limit 与截断）与图片。
//!
//! 文本分支复用 [`super::read_text`] 的纯逻辑；这里只负责 I/O 与图片分支。
//! 对应 `packages/coding-agent/src/core/tools/read.ts` 与 `spec/e-tools.md` 第 2 节。
//!
//! **未实现**：图片的缩放/裁剪（`processImage` + `model.inputLimits`）、非视觉模型的
//! 省略提示（`[Current model does not support images. …]`）——那两项要等 agent loop 把
//! 模型信息传进来。图片目前只做"按扩展名识别 + 原样 base64 返回"。

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::read_text::{render_text_read, ReadTextError, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};
use super::tool::{Tool, ToolContext, ToolResult, ToolResultBlock};

/// `read` 工具。
#[derive(Debug, Default)]
pub struct ReadTool;

/// 参数 schema。
pub fn schema() -> Value {
    json!({
        "type": "object",
        "required": ["path"],
        "properties": {
            "path": {"type": "string", "description": "Path to the file to read (relative or absolute)"},
            "offset": {"type": "number", "description": "Line number to start reading from (1-indexed)"},
            "limit": {"type": "number", "description": "Maximum number of lines to read"}
        }
    })
}

/// 按扩展名识别受支持的图片类型。
///
/// 支持 jpg/jpeg、png、gif、webp、bmp（与工具 description 列出的格式一致）。
pub fn detect_image_mime_type(path: &std::path::Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        // 其余扩展名（含没有扩展名）都不是受支持的图片。
        other => {
            let _ = other;
            None
        }
    }
}

/// 把字节编码成 base64（标准字母表 + 填充），手写以免引入依赖。
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(triple >> 18) as usize & 0x3f] as char);
        out.push(TABLE[(triple >> 12) as usize & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[triple as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

/// Node 风格的 access 失败文案（`golden/S26b` 实测）。
fn access_error(absolute: &std::path::Path, source: &std::io::Error) -> String {
    let kind = if source.kind() == std::io::ErrorKind::NotFound {
        "ENOENT: no such file or directory"
    } else if source.kind() == std::io::ErrorKind::PermissionDenied {
        "EACCES: permission denied"
    } else {
        "EIO: i/o error"
    };
    format!("{kind}, access '{}'", absolute.display())
}

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete."
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
            let path = crate::tools::tool::arg_string(arguments.get("path"));
            let path = path.as_str();
            let offset = arguments
                .get("offset")
                .and_then(Value::as_u64)
                .map(|value| value as usize);
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map(|value| value as usize);

            if cancel.is_cancelled() {
                return ToolResult::error("Operation aborted");
            }

            let absolute = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                context.cwd.join(path)
            };

            // 顺序与 TS 一致：先 access（存在且可读），再判断是不是图片。
            if let Err(error) = tokio::fs::metadata(&absolute).await {
                return ToolResult::error(access_error(&absolute, &error));
            }
            if cancel.is_cancelled() {
                return ToolResult::error("Operation aborted");
            }

            let bytes = match tokio::fs::read(&absolute).await {
                Ok(bytes) => bytes,
                Err(error) => return ToolResult::error(access_error(&absolute, &error)),
            };
            if cancel.is_cancelled() {
                return ToolResult::error("Operation aborted");
            }

            if let Some(mime_type) = detect_image_mime_type(&absolute) {
                return ToolResult {
                    content: vec![
                        ToolResultBlock::Text {
                            text: format!("Read image file [{mime_type}]"),
                        },
                        ToolResultBlock::Image {
                            data: base64_encode(&bytes),
                            mime_type: mime_type.to_string(),
                        },
                    ],
                    is_error: false,
                    details: None,
                    terminate: false,
                };
            }

            // 非 UTF-8 的文本按有损转换；TS 侧同样按 utf-8 读，这里不额外报错。
            let text = String::from_utf8_lossy(&bytes);
            match render_text_read(&text, offset, limit, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES) {
                Ok((rendered, truncation)) => {
                    // ref `tools/read.ts:155-173`：命中截断（首行超限 或 内容被截断）时
                    // 带 `details = { truncation }`（`TruncationResult` 全字段，见
                    // `read_text.rs` 的 `Truncation`）；未截断时不带该键。
                    let mut result = ToolResult::text(rendered);
                    if let Some(truncation) = truncation {
                        result.details = Some(serde_json::json!({ "truncation": truncation }));
                    }
                    result
                }
                Err(error) => ToolResult::error(match error {
                    ReadTextError::OffsetBeyondEnd {
                        offset,
                        total_lines,
                    } => {
                        format!("Offset {offset} is beyond end of file ({total_lines} lines total)")
                    }
                }),
            }
        })
    }
}
