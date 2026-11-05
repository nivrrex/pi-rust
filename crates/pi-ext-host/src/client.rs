//! 扩展宿主客户端：spawn Node 子进程、loopback 连接、握手、注册收集、工具调用。
//!
//! 传输决策见 `notes/decisions.md`（P9-1-B / S5）：**loopback TCP + 一次性令牌**，
//! stdin 只做"父进程已死"的信号（EOF ⇒ 垫片退出），stdout/stderr 留给扩展日志。

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};

use crate::handshake::{Handshake, HandshakeError, Step, ToolSpec};

/// 扩展发给宿主的 UI 帧（宿主再转给前端）。
#[derive(Debug, Clone, PartialEq)]
pub enum UiFrame {
    /// `host.ui.notify`：`kind` 为 `None` / `"info"` / `"warning"` / `"error"`。
    Notify {
        /// 文本。
        message: String,
        /// 级别。
        kind: Option<String>,
    },
    /// `host.ui.setWidget`：`lines` 为 `None` 表示清除该 key（ref `types.ts:176`）。
    SetWidget {
        /// 组件键。
        key: String,
        /// 行文本；`None` = 清除。
        lines: Option<Vec<String>>,
        /// `aboveEditor` / `belowEditor`（缺省 `aboveEditor`）。
        placement: String,
    },
    /// `host.ui.setFooter`：`lines` 为 `None` 表示恢复内置页脚（ref `types.ts:189-193`）。
    SetFooter {
        /// 替代内置状态区的行；`None` = 恢复。
        lines: Option<Vec<String>>,
    },
    /// `host.ui.setHeader`：`lines` 为 `None` 表示恢复内置启动屏（ref `types.ts:196`）。
    SetHeader {
        /// 替代启动屏的行；`None` = 恢复。
        lines: Option<Vec<String>>,
    },
    /// `host.ui.custom.open`：扩展要求打开自定义面板（ref `types.ts:201-215`）。
    CustomOpen {
        /// 初始渲染行。
        lines: Vec<String>,
        /// 是否 overlay（合成在主屏之上，ref `interactive-mode.ts:2902-2917`）。
        overlay: bool,
    },
    /// `host.ui.setTitle`：设置终端标题（ref `types.ts:199`，OSC 2）。
    SetTitle {
        /// 新标题。
        title: String,
    },
    /// `host.ui.setStatus`：`text` 为 `None` 表示清除该 key。
    SetStatus {
        /// 状态键。
        key: String,
        /// 文本。
        text: Option<String>,
    },
}
/// 宿主代为执行的动作（`host.action.call`，目前只有 `bash`）。
#[derive(Debug)]
pub struct ActionCall {
    /// 动作名（`bash`）。
    pub action: String,
    /// 参数（`bash` 是 `{command}`）。
    pub args: Value,
    /// 回填工具结果 `{content, isError}`。
    pub reply: tokio::sync::oneshot::Sender<Value>,
}

/// 扩展要求宿主写入会话的条目（`pi.appendEntry`，ref `types.ts:1499`）。
#[derive(Debug, Clone)]
pub enum SessionCmd {
    /// 退出前排空：sink 线程收到后**立即退出循环**并 join（F26/S0）。
    ///
    /// 只 drop 我们自己那份发送端是不够的：宿主状态里还留着一份 clone，
    /// 通道永远不断开 ⇒ sink 卡在 `recv()`、join 等满上限并误报"会话写入超时"。
    Shutdown,
    /// 排入一条用户消息（`pi.sendUserMessage`）。
    SendUserMessage {
        /// 文本。
        text: String,
    },
    /// 追加一条 `{type:"custom", customType, data}` 条目。
    AppendEntry {
        /// 自定义类型。
        custom_type: String,
        /// 载荷。
        data: Value,
    },
}

/// 扩展发起的阻塞式 UI 询问（`ctx.ui.select/input/confirm`）：
/// 宿主把问题转给前端，前端回答后回填 `reply`（ref `interactive-mode.ts:2530-2545`）。
#[derive(Debug)]
pub struct UiAsk {
    /// `select` / `input` / `confirm`。
    pub kind: String,
    /// 标题。
    pub title: String,
    /// `select` 的候选（其他类型为空）。
    pub options: Vec<String>,
    /// 回填答案：`Value::String` = 选中项/输入内容，`Value::Bool` = confirm，`Null` = 取消。
    pub reply: tokio::sync::oneshot::Sender<Value>,
}

use crate::protocol::{
    decode_line, encode_request, encode_response, Frame, FrameError, Request, PROTOCOL_VERSION,
};

/// 等前端回答 UI 询问的最长时间（超过按取消处理）。
const UI_ASK_TIMEOUT: Duration = Duration::from_secs(300);

/// 启动参数。
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// node 可执行文件（测试里可以传一个不存在的路径）。
    pub node: PathBuf,
    /// 垫片/runner 脚本（`tools/ext-host/runner.mjs`）。
    pub runner: PathBuf,
    /// 要加载的扩展入口（本批支持 `.mjs`/`.js`）。
    pub extensions: Vec<PathBuf>,
    /// 扩展进程的 cwd（`getAgentDir` 等按它推）。
    pub cwd: PathBuf,
    /// 额外的环境变量（例如 `PI_CODING_AGENT_DIR`）。
    pub env: Vec<(String, String)>,
}

/// 宿主错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// 找不到 node（调用方应降级为"不加载扩展"）。
    #[error("node executable not found: {0}")]
    NodeMissing(String),
    /// 子进程起不来。
    #[error("failed to spawn extension host: {0}")]
    Spawn(String),
    /// 连接/读写失败（含子进程提前退出）。
    #[error("extension host io error: {0}")]
    Io(String),
    /// 子进程退出（`None` = 被信号杀死）。
    #[error("extension host exited (code {0:?})")]
    Exited(Option<i32>),
    /// 握手失败。
    #[error("handshake failed: {0}")]
    Handshake(#[from] HandshakeError),
    /// 协议错误（非法帧 / 超大帧 / 超时）。
    #[error("protocol error: {0}")]
    Protocol(String),
    /// 扩展没有注册这个名字的工具。
    #[error("tool not registered: {0}")]
    UnknownTool(String),
}

impl From<FrameError> for HostError {
    fn from(error: FrameError) -> Self {
        HostError::Protocol(error.to_string())
    }
}

/// 宿主侧的可变状态（入站请求处理与 actor 都要访问）。
///
/// 从 `ExtHost` 里拆出来的原因：读循环要交给常驻 reader 任务，任务与 actor 都需要这些字段。
#[derive(Debug, Default)]
pub struct HostState {
    /// 发给扩展的只读 ctx 快照。
    pub ctx: Value,
    /// 最近一次 `host.ui.setStatus`。
    pub last_status: Option<(String, String)>,
    /// UI 帧上行通道。
    pub ui_sink: Option<std::sync::mpsc::Sender<UiFrame>>,
    /// 阻塞式 UI 询问通道。
    pub ui_ask: Option<std::sync::mpsc::Sender<UiAsk>>,
    /// 会话写入通道。
    pub session_sink: Option<std::sync::mpsc::Sender<SessionCmd>>,
    /// 宿主动作通道。
    pub action_sink: Option<std::sync::mpsc::Sender<ActionCall>>,
    /// 工具输出是否展开（F28/S1：`ctx.ui.getToolsExpanded()` 的宿主侧状态）。
    pub tools_expanded: bool,
}

/// 连接上可跨线程共享的部分（写端 + 应答路由表 + id 分配）。
///
/// 存在的原因：`Cmd::Invoke` 会在 actor 线程 `block_on` 等应答，UI 按键不能排进 actor 队列，
/// 需要另一条线程直接写帧、由正在读帧的 `call()` 把应答路由回来。
pub struct Wire {
    writer: std::sync::Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    pending: std::sync::Arc<
        tokio::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
    >,
    next_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// 正在执行的 `call()` 数量：>0 说明有读取循环在跑，其它线程可以直接写帧并等应答；
    /// =0 说明连接空闲（没人读帧），此时必须走 actor 的阻塞读取。
    pub in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

/// `in_flight` 的 RAII 计数器。
struct InFlightGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl std::fmt::Debug for Wire {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Wire").finish()
    }
}

/// 从连接读一帧（常驻 reader 任务与握手共用）。
async fn read_frame_from(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    child: &mut Child,
) -> Result<Frame, HostError> {
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .await
            .map_err(|error| HostError::Io(error.to_string()))?;
        if read == 0 {
            let code = child
                .try_wait()
                .ok()
                .flatten()
                .and_then(|status| status.code());
            return Err(HostError::Exited(code));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        match decode_line(trimmed) {
            Ok(frame) => return Ok(frame),
            Err(FrameError::InvalidJson(_)) => continue,
            Err(error) => return Err(HostError::Protocol(error.to_string())),
        }
    }
}

impl Wire {
    /// 写一行到连接（`handle_inbound` 回复用）。
    async fn write_line(&self, line: &str) -> Result<(), HostError> {
        let mut guard = self.writer.lock().await;
        guard
            .write_all(line.as_bytes())
            .await
            .map_err(|error| HostError::Io(error.to_string()))?;
        guard
            .flush()
            .await
            .map_err(|error| HostError::Io(error.to_string()))
    }

    /// 尚未被应答的直连请求数（测试用：`call()` 返回后应为 0）。
    pub async fn pending_len(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// 发一条请求并等响应（可由任意线程并发调用）。
    pub async fn call_direct(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            .saturating_add(1);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let line = encode_request(&Request {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        });
        {
            let mut guard = self.writer.lock().await;
            guard
                .write_all(line.as_bytes())
                .await
                .map_err(|error| HostError::Io(error.to_string()))?;
            guard
                .flush()
                .await
                .map_err(|error| HostError::Io(error.to_string()))?;
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(HostError::Protocol(format!("no response for `{method}`"))),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(HostError::Protocol(format!(
                    "timeout waiting for `{method}`"
                )))
            }
        }
    }
}

/// 一条运行中的扩展宿主。
#[derive(Debug)]
pub struct ExtHost {
    child: Child,
    /// 持有 stdin 的写端以"保活"：drop 它 ⇒ 子进程读到 EOF ⇒ 主动退出。
    _stdin: Option<tokio::process::ChildStdin>,
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    /// 连接上可跨线程共享的部分（写端 / 应答路由 / id）。
    wire: std::sync::Arc<Wire>,
    handshake: Handshake,
    tools: Vec<ToolSpec>,
    commands: Vec<String>,
    events: Vec<String>,
    /// 扩展注册的 flag（`registerFlag`）。
    flags: Vec<crate::handshake::FlagSpec>,
    /// 单个扩展的加载失败（F19）。
    load_errors: Vec<(String, String)>,
    pid: Option<u32>,
    port: u16,
    /// 宿主可变状态（reader 任务与 actor 共享）。
    state: std::sync::Arc<std::sync::Mutex<HostState>>,
}

impl ExtHost {
    /// 启动并完成握手 + 注册收集。
    pub async fn start(options: HostOptions) -> Result<Self, HostError> {
        // 1) 先监听：端口要在 spawn 之前确定。
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| HostError::Spawn(format!("bind loopback: {error}")))?;
        let port = listener
            .local_addr()
            .map_err(|error| HostError::Spawn(format!("local_addr: {error}")))?
            .port();
        // 2) 一次性令牌：只经环境变量传给子进程，不落盘、不进日志。
        let token = uuid::Uuid::new_v4().simple().to_string();
        if !options.node.exists() {
            return Err(HostError::NodeMissing(options.node.display().to_string()));
        }
        let mut command = Command::new(&options.node);
        command
            .arg(&options.runner)
            .current_dir(&options.cwd)
            .env("PI_EXT_ENDPOINT", format!("127.0.0.1:{port}"))
            .env("PI_EXT_TOKEN", &token)
            .env(
                "PI_EXT_EXTENSIONS",
                serde_json::to_string(
                    &options
                        .extensions
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_else(|_| "[]".to_string()),
            )
            // 允许外部覆盖（测试用：声明一个不匹配的版本以验证握手拒绝）。
            .env(
                "PI_EXT_PROTOCOL",
                std::env::var("PI_EXT_PROTOCOL").unwrap_or_else(|_| PROTOCOL_VERSION.to_string()),
            )
            // stdout/stderr **不接管**：扩展日志照常进宿主的终端（协议走 TCP，互不影响）。
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        for (key, value) in &options.env {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .map_err(|error| HostError::Spawn(error.to_string()))?;
        let pid = child.id();
        let stdin = child.stdin.take();

        // 3) 等子进程连上（超时后杀掉，避免留下孤儿）。
        let accept = tokio::time::timeout(Duration::from_millis(10_000), listener.accept()).await;
        let stream: TcpStream = match accept {
            Ok(Ok((stream, _addr))) => stream,
            Ok(Err(error)) => {
                let _ = child.kill().await;
                return Err(HostError::Io(format!("accept: {error}")));
            }
            Err(_elapsed) => {
                let _ = child.kill().await;
                return Err(HostError::Protocol("connect timeout".to_string()));
            }
        };
        // **只接受一个连接**：拿到连接后立刻关掉监听，后续连接会被拒（端口不再 listen）。
        drop(listener);
        stream
            .set_nodelay(true)
            .map_err(|error| HostError::Io(error.to_string()))?;
        let (read_half, writer) = stream.into_split();
        let mut host = Self {
            child,
            _stdin: stdin,
            load_errors: Vec::new(),
            reader: BufReader::new(read_half),
            wire: std::sync::Arc::new(Wire {
                writer: std::sync::Arc::new(tokio::sync::Mutex::new(writer)),
                pending: std::sync::Arc::new(tokio::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                next_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
                in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }),
            handshake: Handshake::new(token),
            tools: Vec::new(),
            commands: Vec::new(),
            events: Vec::new(),
            flags: Vec::new(),
            pid,
            port,
            state: std::sync::Arc::new(std::sync::Mutex::new(HostState::default())),
        };

        // 4) 握手 + 注册收集（两步都走 `pump`，只是期望的 Step 不同）。
        host.pump_until_ready().await?;
        Ok(host)
    }

    /// 子进程 pid（测试用于 kill -9；生产只用于日志）。
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// 调用扩展注册的命令（ref `agent-session.ts:1766-1789`）。
    pub async fn invoke_command(
        &mut self,
        name: &str,
        args: &str,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        self.call(
            "ext.command.invoke",
            json!({ "commandName": name, "args": args }),
            timeout,
        )
        .await
    }

    /// 设置 UI 上行通道（TUI 在启动时调用；`None` = 保持 stderr/记账行为）。
    pub fn set_ui_sink(&mut self, sink: Option<std::sync::mpsc::Sender<UiFrame>>) {
        if let Ok(mut state) = self.state.lock() {
            state.ui_sink = sink;
        }
    }

    /// 把一次按键转发给扩展的自定义组件（`ext.custom.input`）。
    pub async fn custom_input(&mut self, key: &str, width: usize) -> Result<Value, HostError> {
        self.call(
            "ext.custom.input",
            json!({ "key": key, "width": width }),
            Duration::from_secs(5),
        )
        .await
    }

    /// 跑扩展的渲染钩子（ref `tool-execution.ts:26-33`），返回降级帧；无渲染器时 `None`。
    pub async fn invoke_render(
        &mut self,
        tool_name: &str,
        phase: &str,
        payload: Value,
        width: usize,
    ) -> Result<Option<Value>, HostError> {
        let response = self
            .call(
                "ext.render",
                json!({
                    "toolName": tool_name,
                    "phase": phase,
                    "args": payload.get("args").cloned().unwrap_or(Value::Null),
                    "result": payload.get("result").cloned().unwrap_or(Value::Null),
                    "options": payload.get("options").cloned().unwrap_or(Value::Null),
                    "width": width,
                }),
                Duration::from_secs(2),
            )
            .await?;
        Ok(response
            .get("frames")
            .cloned()
            .filter(|value| !value.is_null()))
    }

    /// 设置宿主动作通道（`createBashToolDefinition` 等需要宿主执行的工具）。
    pub fn set_action_sink(&mut self, sink: Option<std::sync::mpsc::Sender<ActionCall>>) {
        if let Ok(mut state) = self.state.lock() {
            state.action_sink = sink;
        }
    }

    /// 设置会话写入的上行通道（`pi.appendEntry`）。
    pub fn set_session_sink(&mut self, sink: Option<std::sync::mpsc::Sender<SessionCmd>>) {
        if let Ok(mut state) = self.state.lock() {
            state.session_sink = sink;
        }
    }

    /// 设置阻塞式 UI 询问的上行通道。
    pub fn set_ui_ask(&mut self, ask: Option<std::sync::mpsc::Sender<UiAsk>>) {
        if let Ok(mut state) = self.state.lock() {
            state.ui_ask = ask;
        }
    }

    /// 设置发给扩展的 ctx 快照（`cwd`/`model`/`thinkingLevel`/`mode`/`hasUI` 等）。
    pub fn set_ctx(&mut self, ctx: Value) {
        if let Ok(mut state) = self.state.lock() {
            state.ctx = ctx;
        }
    }

    /// 最近一次 `host.ui.setStatus` 的 `(key, text)`（本批只记录）。
    #[must_use]
    pub fn last_status(&self) -> Option<(String, String)> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.last_status.clone())
    }

    /// 握手用的 loopback 端口（测试用于验证"第二个连接被拒"）。
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// 已注册的工具。
    #[must_use]
    pub fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    /// 已注册的命令名（本批只用于观测/计数，未接分发）。
    #[must_use]
    pub fn commands(&self) -> &[String] {
        &self.commands
    }

    /// 走 actor 阻塞读取的按键转发（连接空闲时用）。
    pub async fn custom_input_actor(
        &mut self,
        key: &str,
        width: usize,
    ) -> Result<Value, HostError> {
        self.call(
            "ext.custom.input",
            serde_json::json!({ "key": key, "width": width }),
            Duration::from_secs(5),
        )
        .await
    }

    /// 跨线程共享的写端与应答路由（UI 按键用）。
    #[must_use]
    pub fn wire(&self) -> std::sync::Arc<Wire> {
        std::sync::Arc::clone(&self.wire)
    }

    /// 当前 ctx 快照的克隆。
    fn ctx_clone(&self) -> Value {
        self.state
            .lock()
            .map(|state| state.ctx.clone())
            .unwrap_or(Value::Null)
    }

    /// 扩展注册的 flag。
    /// 单个扩展的加载失败（F19；`ext.ready.loadErrors`）。
    #[must_use]
    pub fn load_errors(&self) -> &[(String, String)] {
        &self.load_errors
    }

    pub fn flags(&self) -> &[crate::handshake::FlagSpec] {
        &self.flags
    }

    /// 下发命令行解析好的 flag 值（`pi.getFlag` 读它）。
    pub async fn set_flag_values(
        &mut self,
        values: Value,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        self.call("ext.flags.values", json!({ "values": values }), timeout)
            .await
    }

    /// 扩展订阅的事件名（本批只用于观测/计数）。
    #[must_use]
    pub fn events(&self) -> &[String] {
        &self.events
    }

    /// 关掉子进程：关 stdin（EOF）→ 等 2 s → 强杀。
    ///
    /// 取 `&mut self`（不是消费 self）：actor 线程需要在同一实例上关闭后退出循环。
    pub async fn shutdown(&mut self) {
        self._stdin.take();
        let _ = tokio::time::timeout(Duration::from_millis(2_000), self.child.wait()).await;
        let _ = self.child.start_kill();
    }

    /// 调用扩展工具。
    pub async fn invoke_tool(
        &mut self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        if !self.tools.iter().any(|tool| tool.name == name) {
            return Err(HostError::UnknownTool(name.to_string()));
        }
        self.call(
            "ext.tool.invoke",
            json!({"toolName": name, "args": arguments, "ctx": self.ctx_clone()}),
            timeout,
        )
        .await
    }

    /// 分发一个事件（`ext.event`，协议 §3.1）。返回扩展回的 `{handled, errors}`。
    ///
    /// 语义（本批）：**通知**——扩展的返回值被忽略；处理器抛错只记录，不影响主流程。
    /// 超时由调用方给（`protocol.md` §4 对无返回值事件没有硬性上限，默认 2 s）。
    pub async fn emit_event(
        &mut self,
        name: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        self.call(
            "ext.event",
            json!({"name": name, "payload": payload, "ctx": self.ctx_clone()}),
            timeout,
        )
        .await
    }

    /// 分发一个**有返回值**的事件（`tool_call` / `tool_result`），返回扩展给出的结果。
    ///
    /// 超时按"无处理器"处理（返回 `Ok(None)` 并已由调用方记录），与 ref 中
    /// "处理器不存在/抛错 ⇒ 不拦截、不替换"的降级一致（`runner.ts:1136-1153`、`:1080-1135`）。
    pub async fn emit_event_value(
        &mut self,
        name: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<Option<Value>, HostError> {
        let request = self
            .call(
                "ext.event",
                json!({"name": name, "payload": payload, "ctx": self.ctx_clone()}),
                timeout,
            )
            .await;
        match request {
            Ok(value) => {
                if std::env::var_os("PI_EXT_DEBUG_HOOKS").is_some() {
                    eprintln!("[hook] raw {name} -> {value}");
                }
                let errors = value.get("errors").and_then(Value::as_array);
                if let Some(errors) = errors {
                    for error in errors {
                        if let Some(text) = error.as_str() {
                            eprintln!("pi-rs: 扩展事件 {name} 处理器失败：{text}");
                        }
                    }
                }
                Ok(value.get("result").cloned().filter(|item| !item.is_null()))
            }
            Err(error @ HostError::Protocol(_)) => {
                // 超时：提示一次并降级为"无处理器"。
                eprintln!("pi-rs: 扩展事件 {name} 超时/失败（{error}），已按无处理器处理");
                Ok(None)
            }
            Err(other) => Err(other),
        }
    }

    /// 发一条请求并等响应；期间扩展发起的请求（例如 `host.log`）会被就地应答。
    pub async fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        self.wire
            .in_flight
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _guard = InFlightGuard(std::sync::Arc::clone(&self.wire.in_flight));
        let id = self
            .wire
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            .saturating_add(1);
        let line = encode_request(&Request {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        });
        self.write_line(&line).await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let frame = tokio::time::timeout_at(deadline, self.read_frame())
                .await
                .map_err(|_| HostError::Protocol(format!("timeout waiting for `{method}`")))?;
            match frame? {
                Frame::Response(response) if response.id == id => {
                    if let Some(error) = response.error {
                        return Err(HostError::Protocol(format!(
                            "{} (code {})",
                            error.message, error.code
                        )));
                    }
                    // 关闭 `in_flight` 之前把**已经登记**的直连请求排干：
                    // 否则"读到 in_flight>0 → 本循环退出 → 直连请求无人读应答"的窗口会让
                    // `custom_input` 白等 5s（见 LIMITATIONS 的 E4g 条目）。
                    self.drain_pending().await;
                    return Ok(response.result.unwrap_or(Value::Null));
                }
                // 别的请求的应答（并发场景，例如 invoke 期间 UI 按键的 `ext.custom.input`）：
                // 路由给登记它的线程；没人认领才丢弃。
                Frame::Response(response) => {
                    let sender = {
                        let mut pending = self.wire.pending.lock().await;
                        pending.remove(&response.id)
                    };
                    if let Some(sender) = sender {
                        let value = match response.error {
                            Some(error) => {
                                Value::String(format!("{} (code {})", error.message, error.code))
                            }
                            None => response.result.unwrap_or(Value::Null),
                        };
                        let _ = sender.send(value);
                    }
                    continue;
                }
                Frame::Request(incoming) => {
                    Self::handle_inbound(&self.wire, &self.state, incoming).await?;
                    continue;
                }
            }
        }
    }

    /// 读一帧；子进程关闭连接 ⇒ `Exited`。
    async fn read_frame(&mut self) -> Result<Frame, HostError> {
        read_frame_from(&mut self.reader, &mut self.child).await
    }

    /// 排干已登记的直连请求（每次读最多等 50ms，最多 20 次）。
    ///
    /// 由**即将退出**的读取循环调用：保证在 `in_flight` 归零前服务完已登记的请求。
    pub(crate) async fn drain_pending(&mut self) {
        for _ in 0..20 {
            let has_pending = {
                let pending = self.wire.pending.lock().await;
                !pending.is_empty()
            };
            if !has_pending {
                return;
            }
            let frame = tokio::time::timeout(Duration::from_millis(50), self.read_frame()).await;
            let Ok(Ok(frame)) = frame else {
                return;
            };
            self.route_frame(frame).await;
        }
    }

    /// 把一帧响应路由给登记它的线程；非响应帧忽略。
    async fn route_frame(&mut self, frame: Frame) {
        let Frame::Response(response) = frame else {
            return;
        };
        let sender = {
            let mut pending = self.wire.pending.lock().await;
            pending.remove(&response.id)
        };
        if let Some(sender) = sender {
            let value = match response.error {
                Some(error) => Value::String(format!("{} (code {})", error.message, error.code)),
                None => response.result.unwrap_or(Value::Null),
            };
            let _ = sender.send(value);
        }
    }

    /// 处理一条**入站请求**（扩展→宿主）并写回应答。
    ///
    /// 抽成独立方法是为常驻 reader 任务铺路（E4h）：任务与 actor 走同一条处理路径。
    async fn handle_inbound(
        wire: &std::sync::Arc<Wire>,
        state: &std::sync::Arc<std::sync::Mutex<HostState>>,
        incoming: crate::protocol::Incoming,
    ) -> Result<(), HostError> {
        // 本批认三类：`host.log`（调试）、`host.ui.notify`（原样打到 stderr，
        // ref 在 print/rpc 也是输出到流）、`host.ui.setStatus`（记录待 TUI 接线）。
        let response = match incoming.method.as_str() {
            "host.log" => {
                if let Some(text) = incoming.params.get("message").and_then(Value::as_str) {
                    eprintln!("[ext] {text}");
                }
                encode_response(incoming.id, Ok(Value::Null))
            }
            "host.ui.notify" => {
                let text = incoming
                    .params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let kind = incoming
                    .params
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let forwarded = state
                    .lock()
                    .ok()
                    .and_then(|s| s.ui_sink.clone())
                    .as_ref()
                    .is_some_and(|sink| {
                        sink.send(UiFrame::Notify {
                            message: text.clone(),
                            kind: kind.clone(),
                        })
                        .is_ok()
                    });
                if !forwarded {
                    eprintln!("[ext] {text}");
                }
                encode_response(incoming.id, Ok(Value::Null))
            }
            // F28/S1：`ctx.ui.setToolsExpanded(boolean)`（ref `types.ts:2586`）。
            // TUI 的实际展开目前由 `/expand` 类命令控制，这里只记录状态并回执，避免扩展抛错。
            "host.ui.setToolsExpanded" => {
                let expanded = incoming
                    .params
                    .get("expanded")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Ok(mut state) = state.lock() {
                    state.tools_expanded = expanded;
                }
                encode_response(incoming.id, Ok(Value::Null))
            }
            "host.ui.setStatus" => {
                let key = incoming
                    .params
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let text = incoming
                    .params
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let _ = state
                    .lock()
                    .ok()
                    .and_then(|s| s.ui_sink.clone())
                    .as_ref()
                    .map(|sink| {
                        sink.send(UiFrame::SetStatus {
                            key: key.clone(),
                            text: Some(text.clone()),
                        })
                    });
                if let Ok(mut state) = state.lock() {
                    state.last_status = Some((key, text));
                }
                encode_response(incoming.id, Ok(Value::Null))
            }
            "host.action.call" => {
                let action = incoming
                    .params
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let args = incoming.params.get("args").cloned().unwrap_or(Value::Null);
                let Some(sink) = state.lock().ok().and_then(|s| s.action_sink.clone()) else {
                    let response = encode_response(
                        incoming.id,
                        Err(crate::protocol::RpcError {
                            code: -32603,
                            message: "host action unavailable".to_string(),
                        }),
                    );
                    wire.write_line(&response).await?;
                    return Ok(());
                };
                let (tx, rx) = tokio::sync::oneshot::channel();
                if sink
                    .send(ActionCall {
                        action,
                        args,
                        reply: tx,
                    })
                    .is_err()
                {
                    let response = encode_response(
                        incoming.id,
                        Err(crate::protocol::RpcError {
                            code: -32603,
                            message: "host action unavailable".to_string(),
                        }),
                    );
                    wire.write_line(&response).await?;
                    return Ok(());
                }
                match tokio::time::timeout(UI_ASK_TIMEOUT, rx).await {
                    Ok(Ok(value)) => encode_response(incoming.id, Ok(value)),
                    _other => encode_response(
                        incoming.id,
                        Ok(
                            json!({"content": [{"type": "text", "text": "host action timed out"}], "isError": true}),
                        ),
                    ),
                }
            }
            "host.session.sendUserMessage" => {
                let text = incoming
                    .params
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let forwarded = state
                    .lock()
                    .ok()
                    .and_then(|s| s.session_sink.clone())
                    .as_ref()
                    .is_some_and(|sink| {
                        sink.send(SessionCmd::SendUserMessage { text: text.clone() })
                            .is_ok()
                    });
                if forwarded {
                    encode_response(incoming.id, Ok(Value::Null))
                } else {
                    encode_response(
                        incoming.id,
                        Err(crate::protocol::RpcError {
                            code: -32603,
                            message: "session writer unavailable".to_string(),
                        }),
                    )
                }
            }
            "host.session.appendEntry" => {
                let custom_type = incoming
                    .params
                    .get("customType")
                    .and_then(Value::as_str)
                    .unwrap_or("custom")
                    .to_string();
                let data = incoming.params.get("data").cloned().unwrap_or(Value::Null);
                let forwarded = state
                    .lock()
                    .ok()
                    .and_then(|s| s.session_sink.clone())
                    .as_ref()
                    .is_some_and(|sink| {
                        sink.send(SessionCmd::AppendEntry {
                            custom_type: custom_type.clone(),
                            data: data.clone(),
                        })
                        .is_ok()
                    });
                if forwarded {
                    encode_response(incoming.id, Ok(Value::Null))
                } else {
                    encode_response(
                        incoming.id,
                        Err(crate::protocol::RpcError {
                            code: -32603,
                            message: "session writer unavailable".to_string(),
                        }),
                    )
                }
            }
            "host.ui.setWidget" => {
                let key = incoming
                    .params
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let lines = incoming
                    .params
                    .get("lines")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .map(|line| line.as_str().unwrap_or_default().to_string())
                            .collect::<Vec<_>>()
                    });
                let placement = incoming
                    .params
                    .get("placement")
                    .and_then(Value::as_str)
                    .unwrap_or("aboveEditor")
                    .to_string();
                let _ = state
                    .lock()
                    .ok()
                    .and_then(|s| s.ui_sink.clone())
                    .as_ref()
                    .map(|sink| {
                        sink.send(UiFrame::SetWidget {
                            key,
                            lines,
                            placement,
                        })
                    });
                encode_response(incoming.id, Ok(Value::Null))
            }
            "host.ui.custom.open" => {
                let lines = incoming
                    .params
                    .get("lines")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .map(|line| line.as_str().unwrap_or_default().to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let _ = state
                    .lock()
                    .ok()
                    .and_then(|s| s.ui_sink.clone())
                    .as_ref()
                    .map(|sink| {
                        let overlay = incoming
                            .params
                            .get("overlay")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        sink.send(UiFrame::CustomOpen { lines, overlay })
                    });
                encode_response(incoming.id, Ok(json!({ "width": 100 })))
            }
            "host.ui.setTitle" => {
                let title = incoming
                    .params
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let _ = state
                    .lock()
                    .ok()
                    .and_then(|s| s.ui_sink.clone())
                    .map(|sink| sink.send(UiFrame::SetTitle { title }));
                encode_response(incoming.id, Ok(Value::Null))
            }
            "host.ui.setHeader" => {
                let lines = incoming
                    .params
                    .get("lines")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .map(|line| line.as_str().unwrap_or_default().to_string())
                            .collect::<Vec<_>>()
                    });
                let _ = state
                    .lock()
                    .ok()
                    .and_then(|s| s.ui_sink.clone())
                    .as_ref()
                    .map(|sink| sink.send(UiFrame::SetHeader { lines }));
                encode_response(incoming.id, Ok(Value::Null))
            }
            "host.ui.setFooter" => {
                let lines = incoming
                    .params
                    .get("lines")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .map(|line| line.as_str().unwrap_or_default().to_string())
                            .collect::<Vec<_>>()
                    });
                let _ = state
                    .lock()
                    .ok()
                    .and_then(|s| s.ui_sink.clone())
                    .as_ref()
                    .map(|sink| sink.send(UiFrame::SetFooter { lines }));
                encode_response(incoming.id, Ok(Value::Null))
            }
            "host.ui.select" | "host.ui.input" | "host.ui.confirm" => {
                // 无 UI：ref 的 `noOpUIContext`（`runner.ts:320-325`）——
                // select/input → null，confirm → false。
                let cancel_value = if incoming.method == "host.ui.confirm" {
                    Value::Bool(false)
                } else {
                    Value::Null
                };
                let Some(ask_tx) = state.lock().ok().and_then(|s| s.ui_ask.clone()) else {
                    let response = encode_response(incoming.id, Ok(cancel_value));
                    wire.write_line(&response).await?;
                    return Ok(());
                };
                let options = incoming
                    .params
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let (tx, rx) = tokio::sync::oneshot::channel();
                let kind = incoming.method.trim_start_matches("host.ui.").to_string();
                let ask = UiAsk {
                    kind,
                    title: incoming
                        .params
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    options,
                    reply: tx,
                };
                if ask_tx.send(ask).is_err() {
                    encode_response(incoming.id, Ok(cancel_value))
                } else {
                    match tokio::time::timeout(UI_ASK_TIMEOUT, rx).await {
                        Ok(Ok(value)) => encode_response(incoming.id, Ok(value)),
                        // 前端没应答/超时：按"取消"处理，不挂住扩展。
                        _other => encode_response(incoming.id, Ok(cancel_value)),
                    }
                }
            }
            _other => encode_response(
                incoming.id,
                Err(crate::protocol::RpcError {
                    code: -32601,
                    message: format!("method not implemented: {}", incoming.method),
                }),
            ),
        };
        wire.write_line(&response).await?;
        Ok(())
    }

    async fn write_line(&mut self, line: &str) -> Result<(), HostError> {
        let _ = &self.wire.in_flight;
        let mut guard = self.wire.writer.lock().await;
        guard
            .write_all(line.as_bytes())
            .await
            .map_err(|error| HostError::Io(error.to_string()))?;
        guard
            .flush()
            .await
            .map_err(|error| HostError::Io(error.to_string()))
    }

    /// 推进握手直到 `ext.ready`。
    async fn pump_until_ready(&mut self) -> Result<(), HostError> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(30_000);
        loop {
            let frame = tokio::time::timeout_at(deadline, self.read_frame())
                .await
                .map_err(|_| HostError::Protocol("timeout waiting for ext.ready".to_string()))??;
            let Frame::Request(incoming) = frame else {
                // 握手期收到响应：协议错误。
                return Err(HostError::Protocol(
                    "unexpected response during handshake".to_string(),
                ));
            };
            match self
                .handshake
                .on_request(&incoming.method, &incoming.params)?
            {
                Step::ReplyHello { fields } => {
                    let response = encode_response(
                        incoming.id,
                        Ok(json!({
                            "protocolVersion": PROTOCOL_VERSION,
                            "fields": fields,
                            "limits": {"maxFrameBytes": crate::protocol::MAX_FRAME_BYTES,
                                       "handshakeTimeoutMs": crate::protocol::HANDSHAKE_TIMEOUT_MS}
                        })),
                    );
                    self.write_line(&response).await?;
                }
                Step::Ready {
                    tools,
                    commands,
                    events,
                    flags,
                    load_errors,
                } => {
                    self.tools = tools;
                    self.commands = commands;
                    self.events = events;
                    self.flags = flags;
                    self.load_errors = load_errors;
                    let response = encode_response(incoming.id, Ok(json!({"ok": true})));
                    self.write_line(&response).await?;
                    return Ok(());
                }
                Step::None => {
                    let response = encode_response(incoming.id, Ok(Value::Null));
                    self.write_line(&response).await?;
                }
            }
        }
    }
}

impl Drop for ExtHost {
    fn drop(&mut self) {
        // 兜底：不 await（Drop 里不能异步），只发 kill 信号；正常路径走 `shutdown()`。
        let _ = self.child.start_kill();
    }
}

/// 从扩展目录里找出本批支持的入口（`.mjs` / `.js`）。
///
/// 从一个目录里发现扩展入口（ref `core/extensions/loader.ts:705-744` 的
/// `discoverExtensionsInDir`）：
/// 1. **直接文件**：`*.ts` / `*.js`（ref `:657-659` 的 `isExtensionFile`；我们还额外接受
///    `*.mjs`——`pi-rs` 自己的夹具用它，属于超集）；
/// 2. **子目录**：`index.ts` → `index.js` → `index.mjs`（ref `:689-695` 只认前两个，
///    `.mjs` 同样是超集）；
/// 3. 不递归（更深的包需要 `package.json` 清单，本批未实现，见 `LIMITATIONS.md`）。
///
/// 与 ref 的差异：ref 不排序（用 `readdirSync` 的顺序），这里按路径排序以保证确定性。
#[must_use]
pub fn discover_extensions(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if path.is_file() {
            if ["ts", "js", "mjs"]
                .iter()
                .any(|ext| file_name.ends_with(&format!(".{ext}")))
            {
                out.push(path);
            }
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        // ref `loader.ts:667-698`：`package.json` 的 `pi.extensions` 清单优先
        // （`pi-manifest.ts:24-40` 只认 `pkg.pi.extensions` 里的字符串数组）。
        if let Some(entries) = manifest_entries(&path) {
            if !entries.is_empty() {
                out.extend(entries);
                continue;
            }
        }
        for candidate in ["index.ts", "index.js", "index.mjs"] {
            let entry_file = path.join(candidate);
            if entry_file.is_file() {
                out.push(entry_file);
                break;
            }
        }
    }
    out.sort();
    out
}

/// 读 `<dir>/package.json` 的 `pi.extensions`（相对目录解析，只保留存在的项）。
fn manifest_entries(dir: &Path) -> Option<Vec<PathBuf>> {
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let list = value.get("pi")?.get("extensions")?.as_array()?;
    let mut out = Vec::new();
    for item in list {
        let rel = item.as_str()?;
        let resolved = dir.join(rel);
        if resolved.exists() {
            out.push(resolved);
        }
    }
    Some(out)
}

/// 把用户给的 `-e <path>` 展开成入口文件列表（ref `loader.ts:780-795`：
/// 目录 → 清单/`index.ts` → 再退化成"目录内发现"；文件 → 原样）。
#[must_use]
pub fn expand_extension_path(path: &Path) -> Vec<PathBuf> {
    if !path.is_dir() {
        return vec![path.to_path_buf()];
    }
    if let Some(entries) = manifest_entries(path) {
        if !entries.is_empty() {
            return entries;
        }
    }
    for candidate in ["index.ts", "index.js", "index.mjs"] {
        let entry = path.join(candidate);
        if entry.is_file() {
            return vec![entry];
        }
    }
    discover_extensions(path)
}

/// 按 ref 的顺序发现扩展：**项目级** `<cwd>/.pi/extensions` 在前，全局 `<agentDir>/extensions`
/// 在后，同一个解析后路径只保留第一次出现（ref `loader.ts:749-800` 的 `addPaths` + `seen`）。
#[must_use]
pub fn discover_extensions_in_order(cwd: &Path, agent_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for root in [cwd.join(".pi/extensions"), agent_dir.join("extensions")] {
        for path in discover_extensions(&root) {
            let key = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen.insert(key) {
                out.push(path);
            }
        }
    }
    out
}
