//! 握手状态机（**纯逻辑**，无 I/O、无时钟）。
//!
//! 协议来源 `notes/p9-1/protocol.md` §1：
//! 子进程连上后必须立刻发 `ext.hello{protocolVersion, token, node, piVersion, extensions[]}`；
//! 宿主回 `host.hello{protocolVersion, fields[], limits{}}`；随后扩展发 `ext.ready`。

use serde_json::Value;

use crate::protocol::PROTOCOL_VERSION;

/// 握手/注册阶段的错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandshakeError {
    /// 令牌不符（宿主只记"不符"，不回显期望值）。
    #[error("token mismatch")]
    TokenMismatch,
    /// 协议版本不符。
    #[error("protocol mismatch: extension {theirs}, host {}", PROTOCOL_VERSION)]
    VersionMismatch {
        /// 扩展声明的版本。
        theirs: u64,
    },
    /// 顺序错误（例如没 hello 就直接 ready）。
    #[error("unexpected frame `{0}` in phase {1}")]
    Unexpected(String, &'static str),
    /// 载荷缺字段/类型不对。
    #[error("invalid payload: {0}")]
    InvalidPayload(String),
}

/// 一个注册好的工具（`host.registerTool` 的载荷，按 `types.ts:455-513` 的字段裁剪）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// 工具名。
    pub name: String,
    /// 长描述（进请求体）。
    pub description: String,
    /// 参数 JSON Schema。
    pub parameters: Value,
    /// 一行摘要（`promptSnippet`）。
    pub prompt_snippet: Option<String>,
    /// 系统提示里的 guideline 条目。
    pub prompt_guidelines: Vec<String>,
}

/// 一个注册好的 flag（`registerFlag(name, {type, default})`，ref `agent-session-services.ts:92-96`）。
#[derive(Debug, Clone, PartialEq)]
pub struct FlagSpec {
    /// flag 名（不含 `--`）。
    pub name: String,
    /// `boolean` 或 `string`。
    pub flag_type: String,
    /// 注册时声明的默认值。
    pub default: Option<Value>,
}

/// 宿主声明支持的能力（`protocol.md` §1 的 `fields`）。
#[must_use]
pub fn host_fields() -> Vec<&'static str> {
    vec![
        "registerTool",
        "registerCommand",
        "registerFlag",
        "getFlag",
        "on",
        "sendUserMessage",
        "appendEntry",
        "events",
        "ui.notify",
        "ui.setStatus",
        "ui.setWidget",
        "ui.select",
        "ui.input",
    ]
}

/// 状态机阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// 等 `ext.hello`。
    AwaitHello,
    /// 等 `ext.ready`。
    AwaitReady,
    /// 就绪。
    Ready,
    /// 失败（不再接受帧）。
    Failed,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::AwaitHello => "await_hello",
            Phase::AwaitReady => "await_ready",
            Phase::Ready => "ready",
            Phase::Failed => "failed",
        }
    }
}

/// 状态机给出的一步动作。
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// 回 `host.hello`（携带 `fields`），状态进入 `AwaitReady`。
    ReplyHello {
        /// 宿主支持的方法名。
        fields: Vec<&'static str>,
    },
    /// `ext.ready` 收到，注册结果可用。
    Ready {
        /// 工具列表。
        tools: Vec<ToolSpec>,
        /// 扩展声明的其它注册（命令/flag/事件名），本批只记录不实现。
        commands: Vec<String>,
        /// 订阅的事件名。
        events: Vec<String>,
        /// 扩展注册的 flag。
        flags: Vec<FlagSpec>,
        /// 单个扩展的加载失败（`(path, message)`，F19 新增可选字段）。
        load_errors: Vec<(String, String)>,
    },
    /// 不产生回复（例如 `host.log`）。
    None,
}

/// 握手状态机。
#[derive(Debug, Clone)]
pub struct Handshake {
    token: String,
    phase: Phase,
}

impl Handshake {
    /// 新建（`token` 与传给子进程的 `PI_EXT_TOKEN` 一致）。
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            phase: Phase::AwaitHello,
        }
    }

    /// 当前阶段。
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// 处理一帧扩展发来的请求。
    ///
    /// 顺序约束（`protocol.md` §2）：hello → ready → 事件/工具；其余一律 `Unexpected`。
    pub fn on_request(&mut self, method: &str, params: &Value) -> Result<Step, HandshakeError> {
        match self.phase {
            Phase::AwaitHello => {
                if method != "ext.hello" {
                    self.phase = Phase::Failed;
                    return Err(HandshakeError::Unexpected(
                        method.to_string(),
                        Phase::AwaitHello.name(),
                    ));
                }
                let token = params
                    .get("token")
                    .and_then(Value::as_str)
                    .ok_or_else(|| HandshakeError::InvalidPayload("missing token".to_string()))?;
                let version = params
                    .get("protocolVersion")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        HandshakeError::InvalidPayload("missing protocolVersion".to_string())
                    })?;
                if token != self.token {
                    self.phase = Phase::Failed;
                    return Err(HandshakeError::TokenMismatch);
                }
                if version != u64::from(PROTOCOL_VERSION) {
                    self.phase = Phase::Failed;
                    return Err(HandshakeError::VersionMismatch { theirs: version });
                }
                self.phase = Phase::AwaitReady;
                Ok(Step::ReplyHello {
                    fields: host_fields(),
                })
            }
            Phase::AwaitReady => {
                if method != "ext.ready" {
                    self.phase = Phase::Failed;
                    return Err(HandshakeError::Unexpected(
                        method.to_string(),
                        Phase::AwaitReady.name(),
                    ));
                }
                let tools = parse_tools(params.get("tools"))?;
                let commands = parse_string_list(params.get("commands"), "commands")?;
                let events = parse_string_list(params.get("events"), "events")?;
                let flags = parse_flags(params.get("flags"))?;
                // F19：单个扩展的加载失败（可选字段；旧 runner 不报 ⇒ 空）。
                let load_errors = parse_load_errors(params.get("loadErrors"))?;
                self.phase = Phase::Ready;
                Ok(Step::Ready {
                    tools,
                    commands,
                    events,
                    flags,
                    load_errors,
                })
            }
            Phase::Ready => {
                if method == "host.log" {
                    return Ok(Step::None);
                }
                Ok(Step::None)
            }
            Phase::Failed => Err(HandshakeError::Unexpected(
                method.to_string(),
                Phase::Failed.name(),
            )),
        }
    }
}

/// 解析 `ext.ready` 的 `flags`（可缺省）。
fn parse_load_errors(
    value: Option<&serde_json::Value>,
) -> Result<Vec<(String, String)>, HandshakeError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| HandshakeError::InvalidPayload("loadErrors must be an array".to_string()))?;
    let mut out = Vec::new();
    for item in items {
        let path = item
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("(unknown extension)")
            .to_string();
        let message = item
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("load failed")
            .to_string();
        out.push((path, message));
    }
    Ok(out)
}

fn parse_flags(value: Option<&Value>) -> Result<Vec<FlagSpec>, HandshakeError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        HandshakeError::InvalidPayload("flags[].name must be a string".to_string())
                    })?
                    .to_string();
                let flag_type = item
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("boolean")
                    .to_string();
                Ok(FlagSpec {
                    name,
                    flag_type,
                    default: item.get("default").cloned(),
                })
            })
            .collect(),
        Some(_) => Err(HandshakeError::InvalidPayload(
            "flags must be an array".to_string(),
        )),
    }
}

fn parse_string_list(value: Option<&Value>, field: &str) -> Result<Vec<String>, HandshakeError> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_string).ok_or_else(|| {
                    HandshakeError::InvalidPayload(format!("{field} must contain strings"))
                })
            })
            .collect(),
        Some(_) => Err(HandshakeError::InvalidPayload(format!(
            "{field} must be an array"
        ))),
    }
}

fn parse_tools(value: Option<&Value>) -> Result<Vec<ToolSpec>, HandshakeError> {
    let items = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => {
            return Err(HandshakeError::InvalidPayload(
                "tools must be an array".to_string(),
            ))
        }
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let object = item.as_object().ok_or_else(|| {
            HandshakeError::InvalidPayload("tool entry must be an object".to_string())
        })?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| HandshakeError::InvalidPayload("tool.name is required".to_string()))?;
        let description = object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let parameters = object
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
        if !parameters.is_object() {
            return Err(HandshakeError::InvalidPayload(format!(
                "tool {name}: parameters must be an object"
            )));
        }
        out.push(ToolSpec {
            name: name.to_string(),
            description,
            parameters,
            prompt_snippet: object
                .get("promptSnippet")
                .and_then(Value::as_str)
                .map(str::to_string),
            prompt_guidelines: parse_string_list(
                object.get("promptGuidelines"),
                "promptGuidelines",
            )?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(token: &str, version: u64) -> Value {
        serde_json::json!({"token": token, "protocolVersion": version})
    }

    fn ready() -> Value {
        serde_json::json!({
            "tools": [{
                "name": "echo",
                "description": "回显",
                "parameters": {"type": "object", "properties": {"text": {"type": "string"}}}
            }],
            "commands": ["greet"],
            "events": ["session_start"]
        })
    }

    /// 正常路径：hello → host.hello → ready。
    #[test]
    fn happy_path() {
        let mut hs = Handshake::new("t0k3n");
        match hs
            .on_request("ext.hello", &hello("t0k3n", 1))
            .expect("hello 应被接受")
        {
            Step::ReplyHello { fields } => assert!(fields.contains(&"registerTool")),
            other => panic!("期望 ReplyHello，实际 {other:?}"),
        }
        assert_eq!(hs.phase(), Phase::AwaitReady);
        match hs
            .on_request("ext.ready", &ready())
            .expect("ready 应被接受")
        {
            Step::Ready {
                tools,
                commands,
                events,
                ..
            } => {
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "echo");
                assert_eq!(commands, vec!["greet".to_string()]);
                assert_eq!(events, vec!["session_start".to_string()]);
            }
            other => panic!("期望 Ready，实际 {other:?}"),
        }
        assert_eq!(hs.phase(), Phase::Ready);
    }

    /// 版本不符：拒绝并进入 Failed，错误里带对方的版本号。
    #[test]
    fn version_mismatch_is_rejected() {
        let mut hs = Handshake::new("t");
        let error = hs
            .on_request("ext.hello", &hello("t", 99))
            .expect_err("版本不符必须拒绝");
        assert_eq!(error, HandshakeError::VersionMismatch { theirs: 99 });
        assert_eq!(hs.phase(), Phase::Failed);
    }

    /// 令牌不符：拒绝，且错误里**不含**期望令牌。
    #[test]
    fn token_mismatch_is_rejected_without_leaking() {
        let mut hs = Handshake::new("secret-value");
        let error = hs
            .on_request("ext.hello", &hello("wrong", 1))
            .expect_err("令牌不符必须拒绝");
        assert_eq!(error, HandshakeError::TokenMismatch);
        assert!(!format!("{error}").contains("secret-value"));
        assert_eq!(hs.phase(), Phase::Failed);
    }

    /// 顺序错误：没 hello 就 ready / 没 ready 就调用工具。
    #[test]
    fn out_of_order_frames_are_rejected() {
        let mut hs = Handshake::new("t");
        assert!(matches!(
            hs.on_request("ext.ready", &ready()),
            Err(HandshakeError::Unexpected(_, "await_hello"))
        ));
        let mut hs = Handshake::new("t");
        let _ = hs.on_request("ext.hello", &hello("t", 1));
        assert!(matches!(
            hs.on_request("ext.tool.result", &serde_json::json!({})),
            Err(HandshakeError::Unexpected(_, "await_ready"))
        ));
    }

    /// 载荷缺失/类型错误都要报 `InvalidPayload`，不能 panic。
    #[test]
    fn invalid_payloads_are_explicit() {
        let mut hs = Handshake::new("t");
        assert!(matches!(
            hs.on_request("ext.hello", &serde_json::json!({"protocolVersion": 1})),
            Err(HandshakeError::InvalidPayload(_))
        ));
        let mut hs = Handshake::new("t");
        let _ = hs.on_request("ext.hello", &hello("t", 1));
        assert!(matches!(
            hs.on_request("ext.ready", &serde_json::json!({"tools": "nope"})),
            Err(HandshakeError::InvalidPayload(_))
        ));
    }

    /// `ext.ready` 里工具缺 name ⇒ 报错（而不是塞一个空工具进去）。
    #[test]
    fn tool_without_name_is_rejected() {
        let mut hs = Handshake::new("t");
        let _ = hs.on_request("ext.hello", &hello("t", 1));
        let error = hs
            .on_request(
                "ext.ready",
                &serde_json::json!({"tools": [{"description": "x"}]}),
            )
            .expect_err("缺 name 必须报错");
        assert!(matches!(error, HandshakeError::InvalidPayload(_)));
    }
}
