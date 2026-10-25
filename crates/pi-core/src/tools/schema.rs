//! 工具参数的 JSON schema 校验——**手写的最小实现**，不引第三方库。
//!
//! 行为**以 golden 为准**（`golden/S26`、`S26b`、`S26c` 的实测）：
//!
//! | 传入 | TS 的行为 |
//! |---|---|
//! | 缺必填（`{}` 对 `{required:["path"]}`） | **报校验错误** |
//! | 类型错（`{"path":123}`） | **放行**（数字被当字符串用，随后 `access("123")` 失败） |
//! | 多余字段（`{"path":"a.txt","whatever":true}`） | **放行**（多余字段被忽略） |
//!
//! 因此本实现**只检查必填**：加了类型检查或多余字段拒绝，我们就会产出 TS 不会产出的错误，
//! 差分反而对不上。"更严格"在这里是缺陷，不是优点。

use serde_json::Value;

/// 工具参数校验失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SchemaError {
    /// 交给模型的完整文案（与 TS 逐字一致）。
    pub message: String,
    /// 缺失的必填字段。
    pub missing: Vec<String>,
}

/// 校验 `arguments` 是否满足 `schema` 的必填要求。
///
/// `schema` 里只看 `required`；`properties`/`items`/`enum` 一律不看（见模块文档的实测表）。
pub fn validate_required(tool: &str, schema: &Value, arguments: &Value) -> Result<(), SchemaError> {
    let Some(required) = schema.get("required").and_then(Value::as_array) else {
        return Ok(());
    };
    let object = arguments.as_object();
    let missing: Vec<String> = required
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| !object.is_some_and(|map| map.contains_key(*name)))
        .map(str::to_string)
        .collect();
    if missing.is_empty() {
        return Ok(());
    }

    // 文案逐字照 `golden/S26`：
    //   Validation failed for tool "read":
    //     - path: must have required properties path
    //
    //   Received arguments:
    //   {}
    let mut message = format!("Validation failed for tool \"{tool}\":\n");
    for name in &missing {
        message.push_str(&format!(
            "  - {name}: must have required properties {name}\n"
        ));
    }
    message.push('\n');
    message.push_str("Received arguments:\n");
    message.push_str(&render_arguments(arguments));
    Err(SchemaError { message, missing })
}

/// 参数原文的渲染：**缩进两格的 pretty JSON**。
///
/// 由 `golden/S26d`、`S26e` 实测确认（非空参数）：
/// ```text
/// Received arguments:
/// {
///   "path": "a.txt"
/// }
/// ```
/// 我原先按紧凑 JSON 写（`{"path":"a.txt"}`）是错的，当时把"非空形式未验证"记在心里，
/// 现在有 golden 了，改成逐字一致。`serde_json::to_string_pretty` 的缩进就是两格。
fn render_arguments(arguments: &Value) -> String {
    serde_json::to_string_pretty(arguments).unwrap_or_else(|_| String::from("{}"))
}
