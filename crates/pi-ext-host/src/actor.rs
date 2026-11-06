//! 宿主 actor：单线程持有 `ExtHost`，所有调用经 channel 串行化。
//!
//! 设计见 `notes/decisions.md` 的 "扩展宿主 actor 化"：宿主的 TCP/子进程句柄绑在创建它的
//! runtime 上，因此**必须**由一个长期存活的线程独占；调用方只持句柄，不 `block_on` 任何
//! 别人的 runtime。

use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use serde_json::Value;

use crate::client::{ExtHost, HostError, HostOptions};

/// 一次调用的种类。
enum Cmd {
    /// 排干"直连请求"的应答（F29/S5）：`custom_input` 走直连后，若此刻 actor 恰好空闲
    /// （没有 invoke 在读帧），应答就没人读 ⇒ 5s 超时。这条指令让 actor 主动读一轮。
    Drain,
    Emit {
        name: String,
        payload: Value,
        timeout: Duration,
        reply: Sender<Result<Value, HostError>>,
    },
    EmitValue {
        name: String,
        payload: Value,
        timeout: Duration,
        reply: Sender<Result<Option<Value>, HostError>>,
    },
    Invoke {
        name: String,
        args: Value,
        timeout: Duration,
        reply: Sender<Result<Value, HostError>>,
    },
    SetCtx {
        ctx: Value,
        reply: Sender<()>,
    },
    /// 转发按键（连接空闲时走 actor 的阻塞读取）。
    CustomInput {
        key: String,
        width: usize,
        reply: Sender<Result<Value, HostError>>,
    },
    /// 跑扩展的渲染钩子。
    Render {
        tool_name: String,
        phase: String,
        payload: Value,
        width: usize,
        reply: Sender<Result<Option<Value>, HostError>>,
    },
    /// 设置宿主动作通道（`host.action.call`）。
    SetActionSink {
        sink: Option<std::sync::mpsc::Sender<crate::client::ActionCall>>,
    },
    /// 设置会话写入通道（`pi.appendEntry`）。
    SetSessionSink {
        sink: Option<std::sync::mpsc::Sender<crate::client::SessionCmd>>,
    },
    /// 下发命令行解析好的 flag 值。
    SetFlagValues {
        values: Value,
        timeout: Duration,
        reply: Sender<Result<Value, HostError>>,
    },
    /// 调用扩展命令。
    InvokeCommand {
        name: String,
        args: String,
        timeout: Duration,
        reply: Sender<Result<Value, HostError>>,
    },
    /// 设置 UI 上行通道。
    SetUiSink {
        sink: Option<std::sync::mpsc::Sender<crate::client::UiFrame>>,
    },
    /// 设置阻塞式 UI 询问通道（带应答，保证后续事件能看到它）。
    SetUiAsk {
        ask: Option<std::sync::mpsc::Sender<crate::client::UiAsk>>,
        reply: Sender<()>,
    },
    Shutdown {
        reply: Sender<()>,
    },
}

/// actor 句柄（可克隆，`Send + Sync`）。
#[derive(Clone)]
/// `active_invokes` 的 RAII 计数（F29/S5）。
struct ActiveInvokeGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl ActiveInvokeGuard {
    fn new(counter: std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Self {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for ActiveInvokeGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub struct HostHandle {
    tx: Sender<Cmd>,
    /// 排队/执行中的 `invoke*` 数量（F29/S5）。
    ///
    /// `custom_input` 用它判断"actor 是否会被长任务占住"：actor 是单线程 + FIFO，
    /// 一旦某个 `Cmd::Invoke` 先入队，后续入队的 `Cmd::CustomInput` 要等它完成才会被处理，
    /// 而 invoke 又在等扩展应答、扩展在等这次按键 ⇒ 5s 超时（全量并行时偶发）。
    active_invokes: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// 宿主暴露的 loopback 端口（测试用）。
    port: u16,
    /// node 子进程 pid（测试用）。
    pid: Option<u32>,
    /// 已注册工具（不可变快照，避免"调用中锁住 ⇒ 列不出工具"）。
    tools: Vec<crate::handshake::ToolSpec>,
    commands: Vec<String>,
    events: Vec<String>,
    /// 扩展注册的 flag（`registerFlag`）。
    flags: Vec<crate::handshake::FlagSpec>,
    /// 单个扩展的加载失败（F19：`ext.ready` 的可选 `loadErrors`）。
    load_errors: Vec<(String, String)>,
    /// 跨线程写端与应答路由（UI 按键不经过 actor）。
    wire: std::sync::Arc<crate::client::Wire>,
    runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for HostHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostHandle")
            .field("port", &self.port)
            .field("pid", &self.pid)
            .field("tools", &self.tools.len())
            .finish()
    }
}

/// actor 启动失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActorError {
    /// 宿主本身起不来（无 node / 握手失败…）。
    #[error("{0}")]
    Host(#[from] HostError),
    /// actor 线程没起来。
    #[error("host actor failed to start")]
    SpawnFailed,
    /// actor 已停止（宿主崩溃 / 显式 shutdown）。
    #[error("host actor stopped")]
    Stopped,
}

/// actor 启动后回传给调用方的元数据。
struct HostMeta {
    wire: std::sync::Arc<crate::client::Wire>,
    runtime: tokio::runtime::Handle,
    port: u16,
    pid: Option<u32>,
    tools: Vec<crate::handshake::ToolSpec>,
    commands: Vec<String>,
    events: Vec<String>,
    flags: Vec<crate::handshake::FlagSpec>,
    load_errors: Vec<(String, String)>,
}

impl HostHandle {
    /// 启动 actor（阻塞直到握手完成或失败）。
    pub fn start(options: HostOptions) -> Result<Self, ActorError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("pi-ext-host".to_string())
            .spawn(move || actor_main(options, rx, ready_tx))
            .map_err(|_| ActorError::SpawnFailed)?;
        match ready_rx.recv() {
            Ok(Ok(meta)) => Ok(Self {
                tx,
                active_invokes: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                port: meta.port,
                pid: meta.pid,
                tools: meta.tools,
                commands: meta.commands,
                events: meta.events,
                flags: meta.flags,
                load_errors: meta.load_errors,
                wire: meta.wire,
                runtime: meta.runtime,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(ActorError::SpawnFailed),
        }
    }

    /// 已注册工具（不可变快照）。
    #[must_use]
    pub fn tools(&self) -> &[crate::handshake::ToolSpec] {
        &self.tools
    }

    /// 已注册命令名（仅观测）。
    #[must_use]
    pub fn commands(&self) -> &[String] {
        &self.commands
    }

    /// 已订阅事件名（仅观测）。
    #[must_use]
    pub fn events(&self) -> &[String] {
        &self.events
    }

    /// 宿主 loopback 端口。
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// node 子进程 pid。
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn call<R>(
        &self,
        build: impl FnOnce(Sender<Result<R, HostError>>) -> Cmd,
    ) -> Result<R, HostError> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.tx
            .send(build(reply_tx))
            .map_err(|_| HostError::Protocol("host actor stopped".to_string()))?;
        reply_rx
            .recv()
            .map_err(|_| HostError::Protocol("host actor stopped".to_string()))?
    }

    /// 分发事件（通知语义）。
    pub fn emit(&self, name: &str, payload: Value, timeout: Duration) -> Result<Value, HostError> {
        self.call(|reply| Cmd::Emit {
            name: name.to_string(),
            payload,
            timeout,
            reply,
        })
    }

    /// 分发有返回值的事件（`tool_call` / `tool_result`）。
    pub fn emit_value(
        &self,
        name: &str,
        payload: Value,
        timeout: Duration,
    ) -> Result<Option<Value>, HostError> {
        self.call(|reply| Cmd::EmitValue {
            name: name.to_string(),
            payload,
            timeout,
            reply,
        })
    }

    /// 调用扩展注册的命令（`registerCommand`）。
    pub fn invoke_command(
        &self,
        name: &str,
        args: &str,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        let _guard = ActiveInvokeGuard::new(std::sync::Arc::clone(&self.active_invokes));
        self.call(|reply| Cmd::InvokeCommand {
            name: name.to_string(),
            args: args.to_string(),
            timeout,
            reply,
        })
    }

    /// 调用扩展工具。
    pub fn invoke(&self, name: &str, args: Value, timeout: Duration) -> Result<Value, HostError> {
        let _guard = ActiveInvokeGuard::new(std::sync::Arc::clone(&self.active_invokes));
        self.call(|reply| Cmd::Invoke {
            name: name.to_string(),
            args,
            timeout,
            reply,
        })
    }

    /// 设置 UI 上行通道（TUI 用；传 `None` 恢复 stderr）。
    pub fn set_ui_sink(&self, sink: Option<std::sync::mpsc::Sender<crate::client::UiFrame>>) {
        let _ = self.tx.send(Cmd::SetUiSink { sink });
    }

    /// 扩展注册的 flag（握手时收集，`set_flag_values` 之前就可用）。
    #[must_use]
    /// 单个扩展的加载失败（`(path, message)`）。
    pub fn load_errors(&self) -> &[(String, String)] {
        &self.load_errors
    }

    pub fn flags(&self) -> &[crate::handshake::FlagSpec] {
        &self.flags
    }

    /// 下发命令行解析好的 flag 值（`pi.getFlag` 读它）。
    pub fn set_flag_values(&self, values: Value, timeout: Duration) -> Result<Value, HostError> {
        self.call(|reply| Cmd::SetFlagValues {
            values: values.clone(),
            timeout,
            reply,
        })
    }

    /// 尚未被应答的直连请求数（测试与诊断用）。
    pub async fn pending_len(&self) -> usize {
        self.wire.pending_len().await
    }

    /// 转发一次按键给扩展的自定义组件。
    ///
    /// **不经过 actor**：工具调用期间 actor 会在 `block_on` 里等应答，按键必须能并发发出
    /// （见 `notes/decisions.md` 的 E4f 设计）。
    pub fn custom_input(&self, key: &str, width: usize) -> Result<Value, HostError> {
        // **仅测试用**：把"检查 busy"推迟到调用方起 invoke 之后，使"排干检查后才登记"
        // 的那条竞态必然发生（生产路径不设该变量 ⇒ 行为不变）。
        if let Ok(ms) = std::env::var("PI_TEST_CUSTOM_INPUT_DELAY_MS") {
            if let Ok(ms) = ms.trim().parse::<u64>() {
                std::thread::sleep(std::time::Duration::from_millis(ms));
            }
        }
        // 有 invoke 在排队或执行 ⇒ actor 会被占住，走直连（读取由 invoke 的读帧循环服务）；
        // 否则 actor 空闲，按键交给它阻塞读取（此时直连会没人读应答）。
        let busy = self
            .active_invokes
            .load(std::sync::atomic::Ordering::SeqCst)
            > 0
            || self
                .wire
                .in_flight
                .load(std::sync::atomic::Ordering::SeqCst)
                > 0;
        if !busy {
            return self.call(|reply| Cmd::CustomInput {
                key: key.to_string(),
                width,
                reply,
            });
        }
        let wire = std::sync::Arc::clone(&self.wire);
        let key = key.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        // 让 actor 读一轮：直连应答的读取依赖"有人在读帧"，actor 空闲时没人读。
        let _ = self.tx.send(Cmd::Drain);
        self.runtime.spawn(async move {
            let result = wire
                .call_direct(
                    "ext.custom.input",
                    serde_json::json!({ "key": key, "width": width }),
                    Duration::from_secs(5),
                )
                .await;
            let _ = tx.send(result);
        });
        match rx.recv_timeout(Duration::from_secs(6)) {
            Ok(result) => result,
            Err(_) => Err(HostError::Protocol(
                "timeout waiting for `ext.custom.input`".to_string(),
            )),
        }
    }

    /// 跑扩展的渲染钩子（返回降级帧）。
    pub fn render(
        &self,
        tool_name: &str,
        phase: &str,
        payload: Value,
        width: usize,
    ) -> Result<Option<Value>, HostError> {
        self.call(|reply| Cmd::Render {
            tool_name: tool_name.to_string(),
            phase: phase.to_string(),
            payload: payload.clone(),
            width,
            reply,
        })
    }

    /// 设置宿主动作通道（`createBashToolDefinition` 的执行）。
    pub fn set_action_sink(
        &self,
        sink: Option<std::sync::mpsc::Sender<crate::client::ActionCall>>,
    ) {
        let _ = self.tx.send(Cmd::SetActionSink { sink });
    }

    /// 设置会话写入通道（`pi.appendEntry`）。
    pub fn set_session_sink(
        &self,
        sink: Option<std::sync::mpsc::Sender<crate::client::SessionCmd>>,
    ) {
        let _ = self.tx.send(Cmd::SetSessionSink { sink });
    }

    /// 设置阻塞式 UI 询问通道（`ctx.ui.select/input/confirm`）。
    pub fn set_ui_ask(&self, ask: Option<std::sync::mpsc::Sender<crate::client::UiAsk>>) {
        let (reply, rx) = std::sync::mpsc::channel();
        if self.tx.send(Cmd::SetUiAsk { ask, reply }).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }

    /// 更新 ctx 快照。
    pub fn set_ctx(&self, ctx: Value) {
        let (reply, _rx) = std::sync::mpsc::channel();
        let _ = self.tx.send(Cmd::SetCtx { ctx, reply });
    }

    /// 异步包装（供 async 调用方使用；内部转到阻塞池，不阻塞 async worker）。
    pub async fn emit_async(
        self,
        name: String,
        payload: Value,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        tokio::task::spawn_blocking(move || self.emit(&name, payload, timeout))
            .await
            .map_err(|error| HostError::Protocol(format!("blocking task failed: {error}")))?
    }

    /// 异步包装（有返回值事件）。
    pub async fn emit_value_async(
        self,
        name: String,
        payload: Value,
        timeout: Duration,
    ) -> Result<Option<Value>, HostError> {
        tokio::task::spawn_blocking(move || self.emit_value(&name, payload, timeout))
            .await
            .map_err(|error| HostError::Protocol(format!("blocking task failed: {error}")))?
    }

    /// 异步包装（工具调用）。
    pub async fn invoke_async(
        self,
        name: String,
        args: Value,
        timeout: Duration,
    ) -> Result<Value, HostError> {
        tokio::task::spawn_blocking(move || self.invoke(&name, args, timeout))
            .await
            .map_err(|error| HostError::Protocol(format!("blocking task failed: {error}")))?
    }

    /// 关闭宿主（等 actor 线程退出）。
    pub fn shutdown(&self) {
        let (reply, rx) = std::sync::mpsc::channel();
        if self.tx.send(Cmd::Shutdown { reply }).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }
}

fn actor_main(
    options: HostOptions,
    rx: Receiver<Cmd>,
    ready: Sender<Result<HostMeta, ActorError>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            let _ = ready.send(Err(ActorError::SpawnFailed));
            return;
        }
    };
    let mut host = match runtime.block_on(ExtHost::start(options)) {
        Ok(host) => host,
        Err(error) => {
            let _ = ready.send(Err(ActorError::Host(error)));
            return;
        }
    };
    let meta = HostMeta {
        wire: host.wire(),
        runtime: runtime.handle().clone(),
        port: host.port(),
        pid: host.pid(),
        tools: host.tools().to_vec(),
        commands: host.commands().to_vec(),
        events: host.events().to_vec(),
        flags: host.flags().to_vec(),
        load_errors: host.load_errors().to_vec(),
    };
    if ready.send(Ok(meta)).is_err() {
        runtime.block_on(host.shutdown());
        return;
    }
    while let Ok(command) = rx.recv() {
        match command {
            Cmd::Emit {
                name,
                payload,
                timeout,
                reply,
            } => {
                let result = runtime.block_on(host.emit_event(&name, payload, timeout));
                let _ = reply.send(result);
            }
            Cmd::EmitValue {
                name,
                payload,
                timeout,
                reply,
            } => {
                let result = runtime.block_on(host.emit_event_value(&name, payload, timeout));
                let _ = reply.send(result);
            }
            Cmd::Drain => {
                runtime.block_on(host.drain_pending());
            }
            Cmd::Invoke {
                name,
                args,
                timeout,
                reply,
            } => {
                let result = runtime.block_on(host.invoke_tool(&name, args, timeout));
                let _ = reply.send(result);
            }
            Cmd::InvokeCommand {
                name,
                args,
                timeout,
                reply,
            } => {
                let result = runtime.block_on(host.invoke_command(&name, &args, timeout));
                let _ = reply.send(result);
            }
            Cmd::SetFlagValues {
                values,
                timeout,
                reply,
            } => {
                let result = runtime.block_on(host.set_flag_values(values, timeout));
                let _ = reply.send(result);
            }
            Cmd::SetCtx { ctx, reply } => {
                host.set_ctx(ctx);
                let _ = reply.send(());
            }
            Cmd::SetUiSink { sink } => {
                host.set_ui_sink(sink);
            }
            Cmd::CustomInput { key, width, reply } => {
                let result = runtime.block_on(host.custom_input_actor(&key, width));
                let _ = reply.send(result);
            }
            Cmd::Render {
                tool_name,
                phase,
                payload,
                width,
                reply,
            } => {
                let result =
                    runtime.block_on(host.invoke_render(&tool_name, &phase, payload, width));
                let _ = reply.send(result);
            }
            Cmd::SetActionSink { sink } => {
                host.set_action_sink(sink);
            }
            Cmd::SetSessionSink { sink } => {
                host.set_session_sink(sink);
            }
            Cmd::SetUiAsk { ask, reply } => {
                host.set_ui_ask(ask);
                let _ = reply.send(());
            }
            Cmd::Shutdown { reply } => {
                runtime.block_on(host.shutdown());
                let _ = reply.send(());
                break;
            }
        }
    }
    // 调用方把句柄全部丢弃（含 panic 展开）时也要真正收尾：只 `start_kill` 不 `wait`
    // 会留下僵尸进程，`/proc/<pid>` 一直存在。这里走与 `Shutdown` 相同的收尾路径。
    runtime.block_on(host.shutdown());
}
