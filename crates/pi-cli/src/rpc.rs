//! rpc 模式的帧格式与命令派发（`spec/h-rpc.md`）。
//!
//! **本版本只实现** `prompt`、`steer`、`follow_up`、`abort`、`get_state` 五个命令；
//! 其余**已识别但未实现**的命令一律返回明确错误，不静默忽略。
//!
//! 帧格式（`jsonl.ts:4-30`）：
//! - **严格 JSONL，只按 LF 切分**——载荷里可以含 U+2028/U+2029 等 Unicode 分隔符，
//!   所以不能用 Node 的 `readline`、也不能用 Rust 的 `str::lines()`（它同样会切这些分隔符）；
//! - 每行末尾的 `\r` 剥掉；
//! - 流结束时残留内容作为最后一行发出。

use serde_json::{json, Value};

/// `get_state` 的 `data`（`rpc-types.ts:96-109` 的子集：本项目能提供的字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcSessionState {
    /// **完整模型对象**（golden/S16 的 `get_state.data.model` 与 `set_model` 的 `data` 同形）。
    pub model: Option<Value>,
    /// 当前思考等级。
    pub thinking_level: String,
    /// 自动压缩是否开启（本项目尚未实现压缩，固定 `true` 与 TS 默认一致）。
    pub auto_compaction_enabled: bool,
    pub is_streaming: bool,
    pub steering_mode: String,
    pub follow_up_mode: String,
    pub session_file: Option<String>,
    pub session_id: String,
    pub message_count: usize,
    pub pending_message_count: usize,
}

impl RpcSessionState {
    /// 转成线上形状。未实现的字段**不出现**（而不是填 `null`，避免客户端把"不支持"
    /// 误读成"值为空"）。
    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "thinkingLevel": self.thinking_level,
            "autoCompactionEnabled": self.auto_compaction_enabled,
            "isStreaming": self.is_streaming,
            "isCompacting": false,
            "steeringMode": self.steering_mode,
            "followUpMode": self.follow_up_mode,
            "sessionId": self.session_id,
            "messageCount": self.message_count,
            "pendingMessageCount": self.pending_message_count,
        });
        if let Some(model) = &self.model {
            value["model"] = model.clone();
        }
        if let Some(file) = &self.session_file {
            value["sessionFile"] = json!(file);
        }
        value
    }
}

/// rpc 运行时要做的副作用。测试用假实现，生产用 `main` 里的实现。
pub trait RpcRuntime {
    /// `get_commands` 的载荷（ref `modes/rpc/rpc-mode.ts:682-712`：extensions → prompt → skills）。
    ///
    /// 默认实现返回空清单（测试替身无需关心该命令）。
    fn commands(&self) -> serde_json::Value {
        serde_json::json!({ "commands": [] })
    }

    fn prompt(&mut self, message: &str) -> Result<(), String>;
    fn steer(&mut self, message: &str) -> Result<(), String>;
    fn follow_up(&mut self, message: &str) -> Result<(), String>;
    fn abort(&mut self) -> Result<(), String>;
    /// 切换思考等级（并写会话条目）。
    fn set_thinking_level(&mut self, level: &str) -> Result<(), String>;
    /// 切换模型（并写会话条目）。`数据`为空表示不返回模型信息。
    fn set_model(&mut self, provider: &str, model_id: &str) -> Result<Option<Value>, String>;
    fn state(&self) -> RpcSessionState;
    /// 会话树（`get_tree`）。形状 `{tree, leafId}`，节点 `{entry, children}`。
    /// 〔`session-manager.ts:1522` `getTree()`；`rpc-mode.ts:651-654`〕
    fn tree(&self) -> Result<Value, String>;
    /// fork 到某条条目（`fork` 默认 `position:"before"`；`clone` 用 `"at"`）。
    /// 形状 `{text?, cancelled}`。〔`agent-session-runtime.ts:262-334`；`rpc-mode.ts:613-631`〕
    fn fork(&self, entry_id: &str, position: &str) -> Result<Value, String>;
    /// `compact`：手动压缩（ref `agent-session.ts:2406-2550`）。S20 的 golden 覆盖的是**失败**路径
    /// （前一轮自动压缩已把最后一条变成 `compaction` ⇒ `Already compacted`）。
    fn compact(&mut self, custom_instructions: Option<&str>) -> Result<Value, String>;
    /// `clear_queue`：清空 steering/follow-up 队列，返回被清掉的文本。
    /// 形状 `{steering, followUp}`。〔`rpc-mode.ts:433-435`；`agent-session.ts:2043-2051`〕
    fn clear_queue(&self) -> Result<Value, String>;
    /// `switch_session`：打开指定会话文件并切过去；形状 `{cancelled}`。〔`rpc-mode.ts:605-611`；`agent-session-runtime.ts:203-217`〕
    fn switch_session(&self, session_path: &str) -> Result<Value, String>;
    /// `new_session`：开一份**新会话**并切过去；形状 `{cancelled}`。〔`rpc-mode.ts:437-444`〕
    fn new_session(&self) -> Result<Value, String>;
    /// `get_messages`：已物化的消息列表。形状 `{messages}`。〔`rpc-mode.ts:674-676`〕
    fn messages(&self) -> Result<Value, String>;
    /// `get_last_assistant_text`：最后一条 assistant 的文本（跳过 aborted 且内容为空的）。
    /// 形状 `{text}`；`text` 为 `None` 时该键**消失**（TS 的 `undefined` 在 JSON 里就是如此）。
    /// 〔`rpc-mode.ts:656-658`；`agent-session.ts:3972-3984`〕
    fn last_assistant_text(&self) -> Result<Value, String>;
    /// `get_entries`：全部条目（**不含** `type=="session"` 的文件头）+ 可选 `since` 之后的部分。
    /// 形状 `{entries, leafId}`。〔`rpc-mode.ts:638-648`；`session-manager.ts:1513-1515` 的 `getEntries`〕
    fn entries(&self, since: Option<&str>) -> Result<Value, String>;
    /// `clone`：以**当前叶子**为界 fork（`position:"at"`）；没有叶子时报
    /// `Cannot clone session: no current entry selected`。〔`rpc-mode.ts:621-631`〕
    fn clone_session(&self) -> Result<Value, String>;

    /// 可 fork 的用户消息（`get_fork_messages`）。形状 `{messages:[{entryId,text}]}`。
    /// 〔`agent-session.ts:3782` `getUserMessagesForForking()`；`rpc-mode.ts:633-635`〕
    fn fork_messages(&self) -> Result<Value, String>;
}

/// `data` 需要**按给定键序**输出时的内部标记：`{"__orderedRaw": "<JSON 片段>"}`。
///
/// `serde_json` 的对象按键名排序，而有些响应的键序是判据（TS 会读回逐字节比较，例如
/// `get_tree.data` 是 `{tree, leafId}`）。这个标记只在 rpc 输出的最后一步被消费
/// （`rpc_session::handle` 自己拼字符串），**绝不落盘**。
pub const ORDERED_RAW_KEY: &str = "__orderedRaw";

/// 本版本实现的命令。
const SUPPORTED: [&str; 19] = [
    "prompt",
    "steer",
    "follow_up",
    "abort",
    "get_state",
    "set_thinking_level",
    "set_model",
    "get_tree",
    "get_fork_messages",
    "fork",
    "clone",
    "get_entries",
    "get_last_assistant_text",
    "get_messages",
    "new_session",
    "switch_session",
    "clear_queue",
    "compact",
    "get_commands",
];

/// TS 认识、但本项目**未实现**的命令（`rpc-types.ts:20-74` 的其余成员）。
const RECOGNIZED_UNIMPLEMENTED: [&str; 13] = [
    "cycle_model",
    "get_available_models",
    "cycle_thinking_level",
    "get_available_thinking_levels",
    "set_steering_mode",
    "set_follow_up_mode",
    "set_auto_compaction",
    "set_auto_retry",
    "abort_retry",
    "bash",
    "abort_bash",
    "get_session_stats",
    "export_html",
];

/// 成功响应形状：`{ id?, type: "response", command, success: true }`，有数据才带 `data`。
/// 〔rpc-mode.ts:64-73〕
pub fn response_ok(id: Option<&Value>, command: &str, data: Option<Value>) -> Value {
    let mut value = json!({
        "type": "response",
        "command": command,
        "success": true,
    });
    if let Some(id) = id {
        value["id"] = id.clone();
    }
    if let Some(data) = data {
        value["data"] = data;
    }
    value
}

/// 错误响应：`{ id?, type: "response", command, success: false, error }`。〔rpc-mode.ts:75-76〕
pub fn response_error(id: Option<&Value>, command: &str, error: &str) -> Value {
    let mut value = json!({
        "type": "response",
        "command": command,
        "success": false,
        "error": error,
    });
    if let Some(id) = id {
        value["id"] = id.clone();
    }
    value
}

/// 处理一行输入，返回要写回 stdout 的响应。
///
/// 错误分支照 TS：解析失败时 `id` 缺省、`command` 是字面量 `"parse"`；
/// 未知命令是 `Unknown command: <type>`（`command` 填该未知类型名）。
pub fn handle_line(line: &str, runtime: &mut dyn RpcRuntime) -> Value {
    let parsed: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return response_error(None, "parse", &format!("Failed to parse command: {error}"))
        }
    };
    handle_command(&parsed, runtime)
}

/// 处理一条已解析的命令。
pub fn handle_command(command: &Value, runtime: &mut dyn RpcRuntime) -> Value {
    let id = command.get("id").cloned();
    let kind = command
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = || {
        command
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
    };

    if !SUPPORTED.contains(&kind) {
        return if RECOGNIZED_UNIMPLEMENTED.contains(&kind) {
            // **与 TS 的差异**：TS 实现了这些命令；本项目未实现，因此报"未实现"而不是
            // `Unknown command`——两种情况的处理办法不同（一个等实现，一个可能是协议错）。
            response_error(
                id.as_ref(),
                kind,
                &format!("Command not implemented in this version: {kind}"),
            )
        } else {
            response_error(id.as_ref(), kind, &format!("Unknown command: {kind}"))
        };
    }

    let result = match kind {
        "prompt" => runtime.prompt(message()),
        "steer" => runtime.steer(message()),
        "follow_up" => runtime.follow_up(message()),
        "abort" => runtime.abort(),
        "set_thinking_level" => {
            let level = command
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or_default();
            runtime.set_thinking_level(level)
        }
        "set_model" => {
            let provider = command
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let model_id = command
                .get("modelId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return match runtime.set_model(provider, model_id) {
                Ok(data) => response_ok(id.as_ref(), kind, data),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "get_commands" => {
            return response_ok(id.as_ref(), kind, Some(runtime.commands()));
        }
        "get_state" => {
            return response_ok(id.as_ref(), kind, Some(runtime.state().to_json()));
        }
        "get_tree" => {
            return match runtime.tree() {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "compact" => {
            let custom = command.get("customInstructions").and_then(Value::as_str);
            return match runtime.compact(custom) {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "clear_queue" => {
            return match runtime.clear_queue() {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "switch_session" => {
            let session_path = command
                .get("sessionPath")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return match runtime.switch_session(session_path) {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "new_session" => {
            return match runtime.new_session() {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "get_messages" => {
            return match runtime.messages() {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "get_last_assistant_text" => {
            return match runtime.last_assistant_text() {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "get_entries" => {
            let since = command.get("since").and_then(Value::as_str);
            return match runtime.entries(since) {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "clone" => {
            return match runtime.clone_session() {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "get_fork_messages" => {
            return match runtime.fork_messages() {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        "fork" => {
            let entry_id = command
                .get("entryId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let position = command
                .get("position")
                .and_then(Value::as_str)
                .unwrap_or("before");
            return match runtime.fork(entry_id, position) {
                Ok(data) => response_ok(id.as_ref(), kind, Some(data)),
                Err(error) => response_error(id.as_ref(), kind, &error),
            };
        }
        // 走到这里的只可能是"已识别但未实现"（未知命令在上面就被拦下了）。
        other => {
            return response_error(
                id.as_ref(),
                kind,
                &format!("Command not implemented in this version: {other}"),
            )
        }
    };
    match result {
        // `prompt`/`steer`/`follow_up`/`abort` 的成功响应**不带 data**——
        // 它们是异步的，进展由事件流给出。〔rpc-types.ts:116-121〕
        Ok(()) => response_ok(id.as_ref(), kind, None),
        Err(error) => response_error(id.as_ref(), kind, &error),
    }
}

/// 严格 JSONL 的行切分：**只按 `\n`**，剥掉行尾的 `\r`。
///
/// 不能用 `str::lines()`：它会在 U+2028/U+2029 等处切分，而这些都是合法 JSON 字符串
/// 内容（`jsonl.ts:4-21` 记录了同样的理由）。
pub fn split_lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}
