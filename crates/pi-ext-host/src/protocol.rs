//! 帧编解码（纯函数，无 I/O）。
//!
//! 协议来源：`notes/p9-1/protocol.md`（S5 决策后：控制通道 = loopback TCP + 令牌）。
//! 每帧是**一行** JSON（`\n` 结尾），JSON-RPC 2.0 形状。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 协议版本（与 `protocol.md` §1 的 `protocolVersion` 对应）。
pub const PROTOCOL_VERSION: u32 = 1;

/// 单帧上限：超过即视为协议错误（防止扩展把整个文件塞进一帧）。
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// 握手超时（`protocol.md` §1：10 s；本机实测冷加载 7–54 ms）。
pub const HANDSHAKE_TIMEOUT_MS: u64 = 10_000;

/// 注册收集超时（`protocol.md` §3.1：`ext.ready` 30 s）。
pub const READY_TIMEOUT_MS: u64 = 30_000;

/// 工具调用超时（协议里"无统一上限"，宿主这里给 30 分钟兜底，防止永久挂起）。
pub const TOOL_TIMEOUT_MS: u64 = 30 * 60 * 1000;

/// 宿主 → 扩展的请求。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// 固定 `"2.0"`。
    pub jsonrpc: String,
    /// 请求 id（宿主自增）。
    pub id: u64,
    /// 方法名（`ext.event` / `ext.tool.invoke` / `host.shutdown` …）。
    pub method: String,
    /// 参数。
    pub params: Value,
}

/// 扩展 → 宿主的响应。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// 固定 `"2.0"`。
    pub jsonrpc: String,
    /// 对应请求 id。
    pub id: u64,
    /// 成功结果。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// 失败信息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// JSON-RPC 错误体。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    /// 错误码（见 `protocol.md` §5）。
    pub code: i64,
    /// 人类可读信息。
    pub message: String,
}

/// 扩展 → 宿主的请求（动作/UI 帧，本批只用 `host.registerTool` 与 `host.log`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Incoming {
    /// 固定 `"2.0"`。
    pub jsonrpc: String,
    /// 请求 id。
    pub id: u64,
    /// 方法名。
    pub method: String,
    /// 参数。
    pub params: Value,
}

/// 解码结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// 扩展回的响应。
    Response(Response),
    /// 扩展发起的请求。
    Request(Incoming),
}

/// 帧错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// 超过 `MAX_FRAME_BYTES`。
    #[error("frame too large: {0} bytes (limit {MAX_FRAME_BYTES})")]
    TooLarge(usize),
    /// 不是合法 JSON（扩展把日志写进了通道时最常见）。
    #[error("invalid json: {0}")]
    InvalidJson(String),
    /// 形状不对（缺 `method`/`id`，或 `jsonrpc` 不是 2.0）。
    #[error("invalid frame: {0}")]
    InvalidShape(String),
}

/// 编码一行（追加 `\n`）。
#[must_use]
pub fn encode_request(request: &Request) -> String {
    let mut line = serde_json::to_string(request).unwrap_or_default();
    line.push('\n');
    line
}

/// 编码响应（宿主回给扩展的请求）。
#[must_use]
pub fn encode_response(id: u64, result: Result<Value, RpcError>) -> String {
    let response = match result {
        Ok(value) => Response {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(value),
            error: None,
        },
        Err(error) => Response {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        },
    };
    let mut line = serde_json::to_string(&response).unwrap_or_default();
    line.push('\n');
    line
}

/// 解码一行。
///
/// 判定顺序（**有意这样排**）：`id` + `result`/`error` 且**无** `method` ⇒ 响应；
/// 有 `method` ⇒ 请求；其余 ⇒ `InvalidShape`。这样扩展的日志行（纯文本）会先被
/// `InvalidJson` 挡掉，而不会被误判成帧。
pub fn decode_line(line: &str) -> Result<Frame, FrameError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(line.len()));
    }
    let value: Value =
        serde_json::from_str(line).map_err(|error| FrameError::InvalidJson(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| FrameError::InvalidShape("top-level is not an object".to_string()))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(FrameError::InvalidShape(
            "jsonrpc must be \"2.0\"".to_string(),
        ));
    }
    let id = object
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| FrameError::InvalidShape("id must be an unsigned integer".to_string()))?;
    if let Some(method) = object.get("method").and_then(Value::as_str) {
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        return Ok(Frame::Request(Incoming {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        }));
    }
    let result = object.get("result").cloned();
    let error = object
        .get("error")
        .and_then(|value| serde_json::from_value::<RpcError>(value.clone()).ok());
    if result.is_none() && error.is_none() {
        return Err(FrameError::InvalidShape(
            "neither method nor result/error".to_string(),
        ));
    }
    Ok(Frame::Response(Response {
        jsonrpc: "2.0".to_string(),
        id,
        result,
        error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u64, method: &str) -> Request {
        Request {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params: serde_json::json!({"a": 1}),
        }
    }

    /// 编码 → 解码 往返。
    #[test]
    fn request_roundtrip() {
        let line = encode_request(&request(7, "ext.tool.invoke"));
        assert!(line.ends_with('\n'));
        match decode_line(line.trim_end()) {
            Ok(Frame::Request(incoming)) => {
                assert_eq!(incoming.id, 7);
                assert_eq!(incoming.method, "ext.tool.invoke");
                assert_eq!(incoming.params["a"], 1);
            }
            other => panic!("应解出请求，实际 {other:?}"),
        }
    }

    /// 响应（带 result / 带 error）都能解。
    #[test]
    fn response_roundtrip() {
        let ok = encode_response(3, Ok(serde_json::json!({"tools": []})));
        match decode_line(ok.trim_end()) {
            Ok(Frame::Response(response)) => {
                assert_eq!(response.id, 3);
                assert!(response.result.is_some());
                assert!(response.error.is_none());
            }
            other => panic!("应解出响应，实际 {other:?}"),
        }
        let err = encode_response(
            4,
            Err(RpcError {
                code: -32013,
                message: "boom".into(),
            }),
        );
        match decode_line(err.trim_end()) {
            Ok(Frame::Response(response)) => {
                assert_eq!(response.error.as_ref().map(|e| e.code), Some(-32013));
            }
            other => panic!("应解出错误响应，实际 {other:?}"),
        }
    }

    /// 扩展把日志写进通道：先 `InvalidJson`，不会 panic、不会被当成帧。
    #[test]
    fn polluted_line_is_rejected_as_json_error() {
        assert!(matches!(
            decode_line("hello from console.log"),
            Err(FrameError::InvalidJson(_))
        ));
        // 半行 JSON
        assert!(matches!(
            decode_line("{\"jsonrpc\":\"2.0\",\"id\":1,\"met"),
            Err(FrameError::InvalidJson(_))
        ));
    }

    /// 形状错误：`jsonrpc` 版本、缺 id、既无 method 也无 result。
    #[test]
    fn shape_errors_are_explicit() {
        assert!(matches!(
            decode_line(r#"{"jsonrpc":"1.0","id":1,"method":"x","params":{}}"#),
            Err(FrameError::InvalidShape(_))
        ));
        assert!(matches!(
            decode_line(r#"{"jsonrpc":"2.0","method":"x","params":{}}"#),
            Err(FrameError::InvalidShape(_))
        ));
        assert!(matches!(
            decode_line(r#"{"jsonrpc":"2.0","id":2}"#),
            Err(FrameError::InvalidShape(_))
        ));
    }

    /// 超大帧被拒（不做分配以外的处理）。
    #[test]
    fn oversized_frame_is_rejected() {
        let payload = "x".repeat(MAX_FRAME_BYTES + 1);
        assert!(matches!(
            decode_line(&payload),
            Err(FrameError::TooLarge(_))
        ));
    }
}
