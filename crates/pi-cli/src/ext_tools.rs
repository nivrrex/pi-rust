//! 扩展工具接入（P9-1-B 第一批）：发现扩展 → 启动宿主 → 把 `registerTool` 的结果
//! 接成 `pi_core::tools::tool::Tool`。
//!
//! 设计约束：
//! - **零影响**：没有扩展目录、没有 node、握手失败 ⇒ 返回空列表并打印一条警告，
//!   主程序其余功能照常（`--list-tools`、TUI、print 都不因它失败）；
//! - **只启动一次**：宿主是进程级单例（`OnceLock`），避免每次取工具都 spawn；
//! - **只支持 `.mjs`/`.js`**：`.ts` 需要 jiti（分发方案待定，见 `notes/questions.md`）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use pi_core::tools::tool::Tool;

/// 进程级宿主句柄（`None` = 尝试过但不可用）。
static HOST: OnceLock<Option<pi_ext_host::HostHandle>> = OnceLock::new();

/// TUI 的扩展 UI 事件上行通道（TUI 启动时注册；`None` = 非 TUI 模式）。
static UI_TX: OnceLock<std::sync::mpsc::Sender<pi_tui::TuiEvent>> = OnceLock::new();

/// 创建并注册 TUI 的 UI 上行通道：返回接收端（交给 `tui::run`），发送端留在全局。
///
/// 必须在 `default_tools`（会启动扩展宿主）**之前**调用，否则宿主拿不到发送端。
#[must_use]
pub fn register_ui_channel() -> std::sync::mpsc::Receiver<pi_tui::TuiEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = UI_TX.set(tx);
    rx
}

/// UI 询问通道（TUI 面板作答；`None` = 无前端，宿主按取消处理）。
static UI_ASK_TX: OnceLock<std::sync::mpsc::Sender<pi_ext_host::UiAsk>> = OnceLock::new();

/// 创建并注册 UI 询问通道：返回接收端（交给 `tui::run`）。
///
/// 与 [`register_ui_channel`] 一样，必须在 `default_tools`（启动宿主）之前调用。
#[must_use]
pub fn register_ui_asks() -> std::sync::mpsc::Receiver<pi_ext_host::UiAsk> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = UI_ASK_TX.set(tx);
    rx
}

/// 事件分发队列（专用线程消费，见 `emit_event`）。
/// 分发队列的载荷：`(宿主句柄, 事件名, 载荷)`。
type EmitItem = (pi_ext_host::HostHandle, String, serde_json::Value);

static EMIT_TX: OnceLock<std::sync::mpsc::Sender<EmitItem>> = OnceLock::new();

/// 把一条事件放进分发队列（专用线程逐个调用 `emit`）。
///
/// 显式传 `host`：`start_host` 里 `HOST` 还没写入，但 `session_start` 也要能入队。
fn enqueue_emit(host: pi_ext_host::HostHandle, name: &str, payload: serde_json::Value) {
    let sender = EMIT_TX.get_or_init(|| {
        let (tx, rx) =
            std::sync::mpsc::channel::<(pi_ext_host::HostHandle, String, serde_json::Value)>();
        std::thread::spawn(move || {
            while let Ok((host, name, payload)) = rx.recv() {
                if let Err(error) = host.emit(&name, payload, std::time::Duration::from_secs(2)) {
                    eprintln!("pi-rs: 扩展事件 {name} 分发失败：{error}");
                }
            }
        });
        tx
    });
    let _ = sender.send((host, name.to_string(), payload));
}

/// 扩展可见的 ctx 快照（`types.ts:313-353` 的子集），宿主启动前由调用方配置。
static CTX: OnceLock<std::sync::Mutex<serde_json::Value>> = OnceLock::new();

/// 配置 ctx（必须在扩展宿主启动前调用；只第一次生效）。
///
/// 本批只填 `cwd`/`mode`/`hasUI`；`model`/`thinkingLevel` 传 `null`
/// （它们的解析发生在装配工具之后，见 `notes/p9-1d/spotcheck.md`）。
pub fn configure_ctx(cwd: &std::path::Path, mode: &str, has_ui: bool) {
    let slot = CTX.get_or_init(|| {
        std::sync::Mutex::new(serde_json::json!({
            "cwd": cwd.display().to_string(),
            "mode": mode,
            "hasUI": has_ui,
            "model": serde_json::Value::Null,
            "thinkingLevel": serde_json::Value::Null,
        }))
    });
    // 二次调用（测试/多入口）只更新可变字段，不覆盖 model/thinkingLevel。
    if let Ok(mut guard) = slot.lock() {
        guard["cwd"] = serde_json::Value::String(cwd.display().to_string());
        guard["mode"] = serde_json::Value::String(mode.to_string());
        guard["hasUI"] = serde_json::Value::Bool(has_ui);
    }
    push_ctx();
}

/// 更新 ctx 里的终端宽度（列数）。扩展的页脚/组件按它渲染（ref 的 `render(width)`）。
pub fn update_ctx_width(cols: usize) {
    let Some(slot) = CTX.get() else {
        return;
    };
    if let Ok(mut guard) = slot.lock() {
        guard["width"] = serde_json::json!(cols);
    }
    push_ctx();
}

/// 把 `ctx.model` 设成 ref 的 **Model 对象**（`types.ts:327` 是 `Model<any> | undefined`，
/// 不是字符串）。扩展普遍读 `ctx.model?.id` / `.provider` / `.reasoning`
/// （如实测的 pi-ai-summary 页脚），此前我们存的是字符串 ⇒ `model=undefined`、页脚丢模型名。
pub fn set_ctx_model_object(id: &str, provider: &str, reasoning: bool, context_window: u64) {
    let Some(slot) = CTX.get() else {
        return;
    };
    if let Ok(mut guard) = slot.lock() {
        guard["model"] = serde_json::json!({
            "id": id,
            "provider": provider,
            "reasoning": reasoning,
            "contextWindow": context_window,
        });
    }
    push_ctx();
}

/// 更新 ctx 里的当前模型与思考档位（切换模型 / Ctrl+P / `/thinking` 后调用）。
///
/// ref `types.ts:325-334`：`ctx.model` 与 `ctx.thinkingLevel` 是**当前**值，扩展在
/// 下一次事件/工具调用里应看到新值。
pub fn update_ctx_model(model: Option<&str>, thinking_level: Option<&str>) {
    let Some(slot) = CTX.get() else {
        return;
    };
    if let Ok(mut guard) = slot.lock() {
        if let Some(model) = model {
            guard["model"] = serde_json::Value::String(model.to_string());
        }
        if let Some(level) = thinking_level {
            guard["thinkingLevel"] = serde_json::Value::String(level.to_string());
        }
    }
    push_ctx();
}

/// 更新 ctx 里的上下文用量（`ctx.getContextUsage()`，ref `types.ts:294-300`）。
///
/// `tokens` 未知时传 `None`（ref 里刚压缩完就是 `null`）。
pub fn update_ctx_usage(tokens: Option<u64>, context_window: u64, percent: Option<f64>) {
    let Some(slot) = CTX.get() else {
        return;
    };
    if let Ok(mut guard) = slot.lock() {
        guard["usage"] = serde_json::json!({
            "tokens": tokens,
            "contextWindow": context_window,
            "percent": percent,
        });
    }
    push_ctx();
}

fn ctx_snapshot() -> serde_json::Value {
    let mut value = CTX
        .get()
        .and_then(|slot| slot.lock().ok().map(|guard| guard.clone()))
        .unwrap_or(serde_json::Value::Null);
    if let Some(id) = session_id() {
        if let Some(object) = value.as_object_mut() {
            object.insert("sessionId".to_string(), serde_json::Value::String(id));
        }
    }
    value
}

/// 把最新 ctx 推给正在运行的宿主（`HostHandle::set_ctx`）。
fn push_ctx() {
    if let Some(host) = HOST.get().and_then(|slot| slot.as_ref()) {
        host.set_ctx(ctx_snapshot());
    }
}

/// 握手时收集到的工具声明（**不可变快照**）。
///
/// 为什么要单独存：`ExtHost` 在工具执行期间被 `Mutex` 占着，
/// 列工具（`--list-tools`、TUI 重建 parts）不能因为锁被占就少列——手工注册信息不会变。
static SPECS: OnceLock<Vec<pi_ext_host::ToolSpec>> = OnceLock::new();

/// 扩展排入的用户消息（`pi.sendUserMessage`），由主循环在下一轮注入。
static PENDING_USER_MESSAGES: OnceLock<std::sync::Mutex<Vec<String>>> = OnceLock::new();

/// 是否有扩展订阅了某事件（`agent_end` 之类需要稍等异步派发）。
#[must_use]
pub fn has_event(name: &str) -> bool {
    HOST.get()
        .and_then(|slot| slot.as_ref())
        .map(|host| host.events().iter().any(|event| event == name))
        .unwrap_or(false)
}

/// 取走所有待注入的用户消息（主循环每轮结束时调用）。
#[must_use]
pub fn take_pending_user_messages() -> Vec<String> {
    PENDING_USER_MESSAGES
        .get()
        .and_then(|slot| {
            slot.lock()
                .ok()
                .map(|mut guard| std::mem::take(&mut *guard))
        })
        .unwrap_or_default()
}

/// 当前会话文件（`pi.appendEntry` 写它；由 print/TUI 在建立 writer 后设置）。
static SESSION_FILE: OnceLock<std::sync::Mutex<Option<PathBuf>>> = OnceLock::new();

/// 当前会话 id（F28/S1）：扩展 `ctx.sessionManager.getSessionId()` 用。
static SESSION_ID: OnceLock<std::sync::Mutex<Option<String>>> = OnceLock::new();

/// 宿主已启动但会话尚未登记时，`session_start` 延后到 `set_session_id` 之后派发（F28/S1）。
static PENDING_START: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 记录当前会话 id（须在 `dispatch_session_start` 之前调用）。
pub fn set_session_id(id: Option<String>) {
    let slot = SESSION_ID.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = id;
    }
    // 宿主可能已在解析 flag 时启动：把带 id 的 ctx 推给宿主，事件里才能看到。
    push_ctx();
}

/// 当前会话 id。
#[must_use]
pub fn session_id() -> Option<String> {
    SESSION_ID
        .get()
        .and_then(|slot| slot.lock().ok().and_then(|guard| guard.clone()))
}

/// 记录当前会话文件路径（扩展 `pi.appendEntry` 用）。
pub fn set_session_file(path: &Path) {
    let slot = SESSION_FILE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(path.to_path_buf());
    }
    // 扩展的 `ctx.sessionManager.getSessionFile()/getEntries()` 读它（ref `types.ts:1854`）。
    if let Some(ctx_slot) = CTX.get() {
        if let Ok(mut guard) = ctx_slot.lock() {
            guard["sessionFile"] = serde_json::Value::String(path.display().to_string());
        }
    }
    push_ctx();
}

/// 宿主动作：`createBashToolDefinition` 等需要宿主执行的工具（目前只有 `bash`）。
fn spawn_action_sink() -> Option<std::sync::mpsc::Sender<pi_ext_host::ActionCall>> {
    let (tx, rx) = std::sync::mpsc::channel::<pi_ext_host::ActionCall>();
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        while let Ok(call) = rx.recv() {
            let result = match call.action.as_str() {
                "bash" => {
                    let command = call
                        .args
                        .get("command")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let tool = pi_core::tools::bash::BashTool;
                    let context = pi_core::tools::tool::ToolContext {
                        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                    };
                    let cancel = tokio_util::sync::CancellationToken::new();
                    let outcome = runtime.block_on(tool.execute(
                        serde_json::json!({ "command": command }),
                        &context,
                        &cancel,
                    ));
                    serde_json::json!({
                        "content": [{ "type": "text", "text": outcome.text_content() }],
                        "isError": outcome.is_error,
                    })
                }
                other => serde_json::json!({
                    "content": [{ "type": "text", "text": format!("unknown host action: {other}") }],
                    "isError": true,
                }),
            };
            let _ = call.reply.send(result);
        }
    });
    Some(tx)
}

/// sink 线程的发送端与句柄（`shutdown_session_sink` 用）。
type SessionSinkSlot = std::sync::Mutex<
    Option<(
        std::sync::mpsc::Sender<pi_ext_host::SessionCmd>,
        std::thread::JoinHandle<()>,
    )>,
>;

static SESSION_SINK: OnceLock<SessionSinkSlot> = OnceLock::new();

/// 退出前收尾：drop 发送端（让 sink 的 `recv` 返回 Err 并退出循环），再 join 线程。
///
/// **必须有上限**：sink 卡在 `PI_TEST_DELAY_SINK_MS` 或慢磁盘时不能把进程拖死；
/// 超时就打日志放过（条目已丢，但用户能看到原因）。返回是否在时限内 join 成功。
pub fn shutdown_session_sink(timeout: std::time::Duration) -> bool {
    let Some(slot) = SESSION_SINK.get() else {
        return true;
    };
    let Some((tx, handle)) = slot.lock().ok().and_then(|mut guard| guard.take()) else {
        return true;
    };
    // 发一条 Shutdown 让 sink 退出循环（宿主 state 里还有一份 sender clone，光 drop 不够）。
    let _ = tx.send(pi_ext_host::SessionCmd::Shutdown);
    drop(tx);
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let _ = handle.join();
        let _ = done_tx.send(());
    });
    match done_rx.recv_timeout(timeout) {
        Ok(()) => true,
        Err(_) => {
            eprintln!(
                "pi-rs: 退出前等待会话写入超时（{} ms），可能有 appendEntry 未落盘",
                timeout.as_millis()
            );
            false
        }
    }
}

/// 启动会话写入的消费者线程（只做"追加一行 JSON"）。
fn spawn_session_sink() -> Option<std::sync::mpsc::Sender<pi_ext_host::SessionCmd>> {
    let (tx, rx) = std::sync::mpsc::channel::<pi_ext_host::SessionCmd>();
    let handle = std::thread::spawn(move || {
        while let Ok(command) = rx.recv() {
            match command {
                pi_ext_host::SessionCmd::Shutdown => break,
                pi_ext_host::SessionCmd::SendUserMessage { text } => {
                    let slot =
                        PENDING_USER_MESSAGES.get_or_init(|| std::sync::Mutex::new(Vec::new()));
                    if let Ok(mut guard) = slot.lock() {
                        guard.push(text);
                    }
                }
                pi_ext_host::SessionCmd::AppendEntry { custom_type, data } => {
                    // **仅测试用**（F23/S3）：人为推迟 sink 的写入，让"进程退出早于 sink 写完"
                    // 必然发生，用于确定性复现 R2。生产路径默认不设置该变量。
                    if let Ok(ms) = std::env::var("PI_TEST_DELAY_SINK_MS") {
                        if let Ok(ms) = ms.trim().parse::<u64>() {
                            if ms > 0 {
                                std::thread::sleep(std::time::Duration::from_millis(ms));
                            }
                        }
                    }
                    // 会话文件可能比宿主晚就绪（`default_tools` 也可能在 writer 之前被调用），
                    // 所以这里短暂等待，而不是把条目丢掉。
                    let mut path = None;
                    // 并行跑全量时宿主可能比 writer 早很多就绪：给足 10s。
                    for _ in 0..200 {
                        path = SESSION_FILE
                            .get()
                            .and_then(|slot| slot.lock().ok().and_then(|guard| guard.clone()));
                        if path.is_some() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    let Some(path) = path else {
                        // F11：不再静默丢弃——告警并保留内容，便于用户/我们定位。
                        eprintln!(
                            "pi-rs: appendEntry 未写入（等待会话路径超时 10s）：\n  customType={custom_type} data={data}"
                        );
                        continue;
                    };
                    // 与 ref `appendCustomEntry` 对齐：补 `id` / `parentId` / 真实时间戳，
                    // 并推进共享叶子（否则后续条目会挂到旧父节点上，链分叉）。
                    let parent = SESSION_CHAIN
                        .get()
                        .and_then(|chain| chain.lock().ok().and_then(|guard| guard.clone()));
                    let id: String = uuid::Uuid::new_v4().to_string().chars().take(8).collect();
                    let line = serde_json::json!({
                        "type": "custom",
                        "customType": custom_type,
                        "data": data,
                        "id": id,
                        "parentId": parent.map_or(serde_json::Value::Null, serde_json::Value::String),
                        "timestamp": pi_core::timefmt::unix_millis_to_iso8601(
                            pi_core::timefmt::now_unix_millis()
                        ),
                    });
                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                    {
                        use std::io::Write;
                        let _ = writeln!(file, "{line}");
                    }
                    if let Some(chain) = SESSION_CHAIN.get() {
                        if let Ok(mut guard) = chain.lock() {
                            *guard = Some(id);
                        }
                    }
                }
            }
        }
    });
    // 句柄存起来：退出前要 drop 发送端并 join，否则进程先退、sink 还没写完（F23/S3 的 R2）。
    let slot = SESSION_SINK.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = Some((tx.clone(), handle));
    }
    Some(tx)
}

/// 命令行里未被 pi-rs 消费的 `--flag`（值 = 显式值，`None` = 裸开关）。
static UNKNOWN_FLAGS: OnceLock<std::collections::BTreeMap<String, Option<String>>> =
    OnceLock::new();

/// 记录未知 flag（两阶段解析的第一阶段，ref `cli/args.ts:236-250`）。
pub fn set_unknown_flags(flags: std::collections::BTreeMap<String, Option<String>>) {
    let _ = UNKNOWN_FLAGS.set(flags);
}

/// 两阶段解析的第二阶段：扩展加载（`registerFlag`）之后校验未知 flag 并下发
/// （ref `agent-session-services.ts:92-125`）。
///
/// 失败时按 ref 的措辞打印 `Error: …` 并以退出码 1 结束（与 `main` 里其它诊断一致）。
fn resolve_flags(host: &pi_ext_host::HostHandle) {
    let Some(unknown) = UNKNOWN_FLAGS.get() else {
        return;
    };
    if unknown.is_empty() {
        return;
    }
    let registered = host.flags().to_vec();
    let mut values = serde_json::Map::new();
    let mut unknown_names: Vec<String> = Vec::new();
    for (name, value) in unknown {
        let Some(spec) = registered.iter().find(|spec| &spec.name == name) else {
            unknown_names.push(name.clone());
            continue;
        };
        if spec.flag_type == "boolean" {
            values.insert(name.clone(), serde_json::Value::Bool(true));
            continue;
        }
        match value {
            Some(text) => {
                values.insert(name.clone(), serde_json::Value::String(text.clone()));
            }
            None => {
                eprintln!("Error: Extension flag \"--{name}\" requires a value");
                std::process::exit(1);
            }
        }
    }
    if !unknown_names.is_empty() {
        let list = unknown_names
            .iter()
            .map(|name| format!("--{name}"))
            .collect::<Vec<_>>()
            .join(", ");
        let plural = if unknown_names.len() == 1 { "" } else { "s" };
        eprintln!("Error: Unknown option{plural}: {list}");
        std::process::exit(1);
    }
    if let Err(error) = host.set_flag_values(
        serde_json::Value::Object(values),
        std::time::Duration::from_secs(5),
    ) {
        eprintln!("pi-rs: 扩展 flag 下发失败：{error}");
    }
}

/// 没有可用扩展时的 flag 校验（ref 里 flag 只可能由扩展认领）：
/// 发现不到任何扩展 ⇒ 任何未知 flag 都是 `Unknown option(s)`。
#[must_use]
pub fn flags_without_extensions(agent_dir: &Path, cwd: &Path) -> Option<String> {
    let unknown = UNKNOWN_FLAGS.get()?;
    if unknown.is_empty() {
        return None;
    }
    let mut discovered = if discovery_disabled() {
        Vec::new()
    } else {
        pi_ext_host::discover_extensions_in_order(cwd, agent_dir)
    };
    if let Some(extra) = EXTRA_EXTENSIONS.get() {
        for path in extra {
            discovered.extend(pi_ext_host::expand_extension_path(path));
        }
    }
    if !discovered.is_empty() {
        return None;
    }
    let list = unknown
        .keys()
        .map(|name| format!("--{name}"))
        .collect::<Vec<_>>()
        .join(", ");
    let plural = if unknown.len() == 1 { "" } else { "s" };
    Some(format!("Unknown option{plural}: {list}"))
}

/// `-e/--extension` 给的显式路径（`set_extra_extensions` 在启动宿主前写入）。
static EXTRA_EXTENSIONS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// `--no-extensions / -ne`：跳过**发现**（项目级 `.pi/extensions` 与 `<agentDir>/extensions`），
/// 显式 `-e` 路径照常加载（ref `resource-loader.ts:452-454` / `:556-559`）。
static NO_EXTENSIONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 记录 `--no-extensions / -ne`（`main` 在启动宿主前调用）。
pub fn set_no_extensions(enabled: bool) {
    NO_EXTENSIONS.store(enabled, std::sync::atomic::Ordering::SeqCst);
}

fn discovery_disabled() -> bool {
    NO_EXTENSIONS.load(std::sync::atomic::Ordering::SeqCst)
}

/// 记录 `-e/--extension` 的路径（相对路径按当前目录解析，ref `loader.ts:781-783`）。
pub fn set_extra_extensions(paths: &[String]) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resolved: Vec<PathBuf> = paths
        .iter()
        .map(|raw| {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .collect();
    let _ = EXTRA_EXTENSIONS.set(resolved);
}

/// 与 `SessionWriter` 共享的"当前叶子 id"（F25/S3）：扩展的 appendEntry 裸追加时也要
/// 拿到正确的 `parentId`，否则会话链断掉（ref `session-manager.ts:1288-1299`）。
static SESSION_CHAIN: OnceLock<std::sync::Arc<std::sync::Mutex<Option<String>>>> = OnceLock::new();

/// 取共享叶子句柄（惰性创建；所有入口共用同一份）。
pub fn session_chain() -> std::sync::Arc<std::sync::Mutex<Option<String>>> {
    SESSION_CHAIN
        .get_or_init(|| std::sync::Arc::new(std::sync::Mutex::new(None)))
        .clone()
}

/// 登记共享叶子（各入口在建好 writer 后调用一次）。
pub fn set_session_chain(chain: std::sync::Arc<std::sync::Mutex<Option<String>>>) {
    let _ = SESSION_CHAIN.set(chain);
}

/// 扩展加载问题（ref `interactive-mode.ts:1874-1894` 的 `[Extension issues]` 段用）。
static EXT_ISSUES: OnceLock<std::sync::Mutex<Vec<String>>> = OnceLock::new();

fn record_issue(message: impl Into<String>) {
    let slot = EXT_ISSUES.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    if let Ok(mut guard) = slot.lock() {
        guard.push(message.into());
    }
}

/// 向 TUI 投递一条事件（没有 TUI 时静默忽略）。供非扩展来源（如 skills 标签）使用。
pub fn emit_ui_event(event: pi_tui::TuiEvent) {
    if let Some(tx) = UI_TX.get() {
        let _ = tx.send(event);
    }
}

/// 启动屏要展示的扩展问题（没有则为空）。
#[must_use]
pub fn extension_issues() -> Vec<String> {
    EXT_ISSUES
        .get()
        .and_then(|slot| slot.lock().ok().map(|guard| guard.clone()))
        .unwrap_or_default()
}

/// 已加载扩展的入口路径（启动屏 `[Extensions]` 段用，ref `interactive-mode.ts:1821-1832`）。
static EXT_ENTRIES: OnceLock<Vec<std::path::PathBuf>> = OnceLock::new();

/// 启动屏的扩展标签：`dir/index.ts` → `dir`，直接文件保持文件名（ref
/// `getCompactExtensionLabels` 的 `:1475-1491`：去掉末尾的 `index.ts`/`index.js`）。
#[must_use]
pub fn extension_labels() -> Vec<String> {
    let mut out: Vec<String> = EXT_ENTRIES
        .get()
        .map(|entries| {
            entries
                .iter()
                .map(|path| {
                    let is_index = path
                        .file_name()
                        .map(|name| {
                            // ref `getCompactExtensionLabels`：去掉末尾的 `index.*`（含 `.mjs`/`.mts`）。
                            matches!(
                                name.to_string_lossy().as_ref(),
                                "index.ts" | "index.js" | "index.mjs" | "index.mts"
                            )
                        })
                        .unwrap_or(false);
                    if is_index {
                        path.parent()
                            .and_then(|parent| parent.file_name())
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string())
                    } else {
                        path.file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string())
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// 找 node：`PI_NODE` 优先，其次在 `PATH` 里找可执行文件。
#[must_use]
pub fn find_node() -> Option<std::path::PathBuf> {
    if let Some(explicit) = std::env::var_os("PI_NODE") {
        let path = std::path::PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    find_node_in_path(&std::env::var_os("PATH")?)
}

/// 在给定的 `PATH` 值里找 node（`node` 与 Windows 的 `node.exe` 都试）。
///
/// 抽成纯函数是为了能单测：改 `PATH` 环境变量在并行测试里会互相踩。
#[must_use]
pub fn find_node_in_path(paths: &std::ffi::OsStr) -> Option<std::path::PathBuf> {
    // Windows 上可执行文件带扩展名（`node.exe`）：只试 `node` 会漏掉所有 Windows 安装，
    // 真机表现是"扩展未加载：找不到 node"，而 `node -v` 明明能用。两者都试，不区分平台
    // （Linux 目录里不会有 `node.exe`，多试一次无副作用）。
    for dir in std::env::split_paths(paths) {
        for name in ["node", "node.exe"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 扩展根目录：`<agentDir>/extensions`。
#[must_use]
pub fn extensions_root(agent_dir: &Path) -> std::path::PathBuf {
    agent_dir.join("extensions")
}

/// runner 脚本位置：与可执行文件同目录的 `ext-host/runner.mjs`，或开发树里的 `tools/ext-host`。
fn runner_path() -> Option<std::path::PathBuf> {
    if let Some(explicit) = std::env::var_os("PI_EXT_RUNNER") {
        let path = std::path::PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    // 1) 随发行包放置：<exe 目录>/ext-host/runner.mjs
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("ext-host/runner.mjs");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    // 2) 源码树（cargo run / 测试）：从 CARGO_MANIFEST_DIR 往上找
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest.join("../../tools/ext-host/runner.mjs");
    if candidate.is_file() {
        return Some(candidate);
    }
    // 3) **单文件发行包**（用户从 release 只拿一个二进制时）：runner 内嵌在可执行文件里，
    //    运行时落到临时目录。没有这一步，"扩展未加载：找不到 ext-host/runner.mjs" 会让
    //    所有扩展能力在发行版上不可用（Armbian 真机实测）。
    materialize_embedded_runner()
}

/// 内嵌的扩展宿主脚本（编译期由 `include_str!` 打入）。
///
/// **必须包含 shim 与 resolve.mjs**：`runner.mjs` 会 `import "./shim/pi-tui.mjs"` 等
/// （`runner.mjs:13-20`）。只写 runner.mjs 会让 node 报
/// `ERR_MODULE_NOT_FOUND ... /shim/pi-tui.mjs`（Armbian 真机实测）。
const EMBEDDED_EXT_HOST: &[(&str, &str)] = &[
    (
        "runner.mjs",
        include_str!("../../../tools/ext-host/runner.mjs"),
    ),
    ("jiti.mjs", include_str!("../../../tools/ext-host/jiti.mjs")),
    (
        "resolve.mjs",
        include_str!("../../../tools/ext-host/resolve.mjs"),
    ),
    (
        "shim/pi-ai.mjs",
        include_str!("../../../tools/ext-host/shim/pi-ai.mjs"),
    ),
    (
        "shim/pi-coding-agent.mjs",
        include_str!("../../../tools/ext-host/shim/pi-coding-agent.mjs"),
    ),
    (
        "shim/pi-tui.mjs",
        include_str!("../../../tools/ext-host/shim/pi-tui.mjs"),
    ),
    (
        "shim/theme-colors.mjs",
        include_str!("../../../tools/ext-host/shim/theme-colors.mjs"),
    ),
    (
        "shim/typebox.mjs",
        include_str!("../../../tools/ext-host/shim/typebox.mjs"),
    ),
];

/// 派发 `session_start`（F28/S1）。
///
/// **必须**在"会话文件已建 + `set_session_file`/`set_session_id` 已登记"之后调用：
/// 扩展在 `session_start` 里读 `ctx.sessionManager.getSessionId()`/`appendEntry`，
/// 宿主启动本身要早于此（flag 两阶段解析需要扩展声明），故派发与启动解耦。
pub fn dispatch_session_start(reason: &str, previous: Option<&std::path::Path>) {
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref().cloned()) else {
        return;
    };
    emit_session_start_with(host, reason, previous);
}

/// 会话已登记、system 条目已写之后调用：把延后的 `session_start` 派发出去。
pub fn flush_pending_session_start() {
    if PENDING_START.swap(false, std::sync::atomic::Ordering::SeqCst) {
        dispatch_session_start("startup", None);
    }
}

/// 无会话的 CLI 模式（`--list-tools` 等）在宿主启动后显式派发一次 `session_start`。
pub fn dispatch_startup_without_session() {
    PENDING_START.store(false, std::sync::atomic::Ordering::SeqCst);
    dispatch_session_start("startup", None);
}

/// 用**指定**宿主派发（`start_host` 里 HOST 还没写进全局槽位时用）。
fn emit_session_start_with(
    host: pi_ext_host::HostHandle,
    reason: &str,
    previous: Option<&std::path::Path>,
) {
    let payload = session_start_payload(reason, previous);
    if UI_ASK_TX.get().is_some() {
        // 有 UI 前端（TUI）：`session_start` 里的 `ctx.ui.select` 要等面板作答，
        // 同步派发会把启动流程阻塞到用户操作之前 ⇒ 放到独立线程里派发，
        // **派发完成之后**再让启动屏显示 `[Extensions]` 清单（ref 的清单是在
        // `session_start` 之后渲染的，见 `notes/decisions.md`）。

        std::thread::spawn(move || {
            if let Err(error) = host.emit(
                "session_start",
                payload,
                std::time::Duration::from_secs(3600),
            ) {
                eprintln!("pi-rs: 扩展事件 session_start 分发失败：{error}");
            }
            let labels = extension_labels();
            let paths: Vec<String> = EXT_ENTRIES
                .get()
                .map(|entries| {
                    entries
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(tx) = UI_TX.get() {
                let _ = tx.send(pi_tui::TuiEvent::ExtensionLabels(labels));
                let _ = tx.send(pi_tui::TuiEvent::ExtensionPaths(paths));
                // 加载问题同步进启动屏（`[Extension issues]`），否则 TUI 里完全看不到。
                let issues = extension_issues();
                if !issues.is_empty() {
                    let _ = tx.send(pi_tui::TuiEvent::ExtensionIssues(issues));
                }
            }
        });
    } else if let Err(error) =
        host.emit("session_start", payload, std::time::Duration::from_secs(2))
    {
        eprintln!("pi-rs: 扩展事件 session_start 分发失败: {error}");
    }
}

/// 把内嵌的宿主脚本写到临时目录（同进程内只写一次），返回 `runner.mjs` 的路径。
fn materialize_embedded_runner() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("pi-rs-ext-host-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("shim")).ok()?;
        for (relative, contents) in EMBEDDED_EXT_HOST {
            let path = dir.join(relative);
            // 内容随二进制固定：字节数一致就复用，避免每次启动重写。
            let same = std::fs::metadata(&path)
                .map(|meta| meta.len() as usize == contents.len())
                .unwrap_or(false);
            if !same {
                std::fs::write(&path, contents).ok()?;
            }
        }
        Some(dir.join("runner.mjs"))
    })
    .clone()
}

/// 返回扩展工具（首次调用时启动宿主）。失败时返回空列表并打印警告。
#[must_use]
pub fn extension_tools(agent_dir: &Path, cwd: &Path) -> Vec<Arc<dyn Tool>> {
    let host = HOST.get_or_init(|| start_host(agent_dir, cwd));
    let Some(host) = host.as_ref() else {
        return Vec::new();
    };
    // 用不可变快照，避免"工具正在执行 ⇒ 锁被占 ⇒ 少列工具"。
    let tools = SPECS.get().cloned().unwrap_or_default();
    tools
        .into_iter()
        .map(|spec| Arc::new(pi_ext_host::ExtTool::new(spec, host.clone())) as Arc<dyn Tool>)
        .collect()
}

fn start_host(agent_dir: &Path, cwd: &Path) -> Option<pi_ext_host::HostHandle> {
    let root = extensions_root(agent_dir);
    // ref 的顺序：项目级 `.pi/extensions` 在前、全局目录在后（`loader.ts:770-776`）。
    let mut extensions = if discovery_disabled() {
        Vec::new()
    } else {
        pi_ext_host::discover_extensions_in_order(cwd, agent_dir)
    };
    // `-e/--extension`（ref `loader.ts:780-795`）：按给定顺序追加，已出现的路径不重复。
    if let Some(extra) = EXTRA_EXTENSIONS.get() {
        for path in extra {
            for entry in pi_ext_host::expand_extension_path(path) {
                let key = entry.canonicalize().unwrap_or_else(|_| entry.clone());
                let seen = extensions.iter().any(|existing| {
                    existing.canonicalize().unwrap_or_else(|_| existing.clone()) == key
                });
                if !seen {
                    extensions.push(entry);
                }
            }
        }
    }
    if extensions.is_empty() {
        return None;
    }
    let Some(node) = find_node() else {
        eprintln!(
            "pi-rs: 扩展未加载：找不到 node（{} 下有 {} 个扩展）。安装 node 或设置 PI_NODE。",
            root.display(),
            extensions.len()
        );
        return None;
    };
    let Some(runner) = runner_path() else {
        eprintln!("pi-rs: 扩展未加载：找不到 ext-host/runner.mjs");
        record_issue("找不到 ext-host/runner.mjs（扩展宿主无法启动）");
        return None;
    };
    let mut env = vec![(
        "PI_CODING_AGENT_DIR".to_string(),
        agent_dir.display().to_string(),
    )];
    if let Some(home) = std::env::var_os("HOME") {
        env.push(("HOME".to_string(), home.to_string_lossy().to_string()));
    }
    let options = pi_ext_host::HostOptions {
        node,
        runner,
        extensions: extensions.clone(),
        cwd: cwd.to_path_buf(),
        env,
    };
    // actor：宿主由专用线程独占（`notes/decisions.md` 的 "扩展宿主 actor 化"）。
    let host = match pi_ext_host::HostHandle::start(options) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("pi-rs: 扩展未加载：{error}");
            record_issue(format!("扩展宿主启动失败：{error}"));
            return None;
        }
    };
    // F19：单个扩展的加载失败要进 `[Extension issues]`（ref `interactive-mode.ts:1874-1894`）。
    for (path, message) in host.load_errors() {
        eprintln!("pi-rs: 扩展加载失败：{path}: {message}");
        record_issue(format!("{path}: {message}"));
    }
    // 两阶段解析的第二步：扩展已 registerFlag，现在才能校验命令行里的未知 flag。
    resolve_flags(&host);
    let _ = SPECS.set(host.tools().to_vec());
    let _ = EXT_ENTRIES.set(extensions.clone());
    host.set_ctx(ctx_snapshot());
    // UI 上行：宿主 actor 把 `host.ui.*` 帧送进这个 channel，再转成 TuiEvent
    // （ref `interactive-mode.ts:2541/2543`）。
    if let Some(sink) = spawn_action_sink() {
        host.set_action_sink(Some(sink));
    }
    if let Some(sink) = spawn_session_sink() {
        host.set_session_sink(Some(sink));
    }
    if let Some(ask_tx) = UI_ASK_TX.get().cloned() {
        host.set_ui_ask(Some(ask_tx));
    }
    if let Some(ui_tx) = UI_TX.get().cloned() {
        let (frame_tx, frame_rx) = std::sync::mpsc::channel::<pi_ext_host::client::UiFrame>();
        host.set_ui_sink(Some(frame_tx));
        std::thread::spawn(move || {
            while let Ok(frame) = frame_rx.recv() {
                let event = match frame {
                    pi_ext_host::client::UiFrame::Notify { message, kind } => {
                        pi_tui::TuiEvent::ExtensionNotice { message, kind }
                    }
                    pi_ext_host::client::UiFrame::SetStatus { key, text } => {
                        pi_tui::TuiEvent::ExtensionStatus { key, text }
                    }
                    pi_ext_host::client::UiFrame::CustomOpen { lines, overlay } => {
                        if overlay {
                            pi_tui::TuiEvent::OverlayOpen(Some(lines))
                        } else {
                            pi_tui::TuiEvent::CustomOpen(lines)
                        }
                    }
                    pi_ext_host::client::UiFrame::SetTitle { title } => {
                        pi_tui::TuiEvent::Title(title)
                    }
                    pi_ext_host::client::UiFrame::SetHeader { lines } => {
                        pi_tui::TuiEvent::HeaderOverride(lines)
                    }
                    pi_ext_host::client::UiFrame::SetFooter { lines } => {
                        pi_tui::TuiEvent::FooterOverride(lines)
                    }
                    pi_ext_host::client::UiFrame::SetWidget {
                        key,
                        lines,
                        placement,
                    } => pi_tui::TuiEvent::ExtensionWidget {
                        key,
                        lines,
                        placement,
                    },
                };
                if ui_tx.send(event).is_err() {
                    break;
                }
            }
        });
    }
    // F28/S1：会话还没建（扩展在 flag 解析阶段就加载）时，`session_start` 延后到
    // `set_session_id` 之后派发——扩展在事件里要读 `getSessionId()`。
    // F28/S1：**一律**延后到会话落盘之后再派发（调用方 `flush_pending_session_start`），
    // 扩展在 `session_start` 里 `appendEntry` 时链叶子才是 system 条目的 id。
    PENDING_START.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = &host;
    Some(host)
}

pub fn emit_event(name: &str, payload: serde_json::Value) {
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref()).cloned() else {
        return;
    };
    enqueue_emit(host, name, payload);
}

/// 把一次按键转给扩展的自定义组件；返回 `{lines, closed, result?}`。
#[must_use]
pub fn custom_input(key: &str, width: usize) -> Option<serde_json::Value> {
    let host = HOST.get().and_then(|slot| slot.as_ref()).cloned()?;
    host.custom_input(key, width).ok()
}

/// 把渲染钩子的降级帧展开成行（宿主侧宽度裁剪）。
///
/// `None` = 扩展没有渲染器、或渲染器用了不支持的组件（调用方应回落到内置渲染）。
#[must_use]
pub fn flatten_render_frame(frame: &serde_json::Value, width: usize) -> Option<Vec<String>> {
    match frame.get("kind").and_then(|kind| kind.as_str()) {
        Some("text") => {
            // 照 ref `components/text.ts:53-100`：空/纯空白 ⇒ `[""]`；制表符变 3 空格；
            // `paddingX` 在窄终端压到 `floor((width-1)/2)`；内容按 `width-2*paddingX` **折行**；
            // 上下各 `paddingY` 空行。
            let raw = frame
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or_default();
            let padding_y = frame.get("paddingY").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if raw.trim().is_empty() {
                // ref `components/text.ts:53-58`：空/纯空白**直接返回空数组**（连 paddingY 也不画）。
                return Some(Vec::new());
            }
            let requested_x = frame.get("paddingX").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let padding_x = requested_x.min(width.saturating_sub(1) / 2);
            let content_width = width.saturating_sub(padding_x * 2).max(1);
            let normalized = raw.replace('\t', "   ");
            let mut lines: Vec<String> = vec![String::new(); padding_y];
            for input_line in normalized.split('\n') {
                for wrapped in wrap_ansi(input_line, content_width) {
                    lines.push(format!("{}{wrapped}", " ".repeat(padding_x)));
                }
            }
            lines.extend(vec![String::new(); padding_y]);
            Some(lines)
        }
        Some("spacer") => {
            let count = frame.get("lines").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            Some(vec![String::new(); count])
        }
        Some("stack") => {
            let mut lines = Vec::new();
            for child in frame
                .get("children")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
            {
                lines.extend(flatten_render_frame(&child, width)?);
            }
            Some(lines)
        }
        // `unsupported`（或未知帧）：回落内置渲染。
        _other => None,
    }
}

/// 折行时把 ANSI 转义序列当**零宽**（ref `utils.ts:851-900` 的 `wrapTextWithAnsi` 同语义）。
///
/// 只用于渲染钩子的文本帧；不改变 pi-tui 既有折行路径。
fn wrap_ansi(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        // CSI：`ESC [` 到 `@`–`~` 的终止字节，宽度 0。
        if ch == '\u{1b}' && index + 1 < chars.len() && chars[index + 1] == '[' {
            let mut end = index + 2;
            while end < chars.len() && !('@'..='~').contains(&chars[end]) {
                end += 1;
            }
            let end = (end + 1).min(chars.len());
            current.extend(&chars[index..end]);
            index = end;
            continue;
        }
        // 单字符宽度用 pi-tui 的可见宽度（CJK 记 2）。
        let char_width = pi_tui::display_width(&ch.to_string());
        if used + char_width > width && used > 0 {
            // 恰好填满后遇到**空格**：直接断行并丢掉它（pi-tui 的 `wrap_to_width` 同策略，
            // 否则会白白少放一个词）。
            if ch == ' ' {
                out.push(std::mem::take(&mut current));
                used = 0;
                index += 1;
                continue;
            }
            // 否则回退到最近的空格。
            if let Some(space_at) = current.rfind(' ') {
                if space_at > 0 && !current[..space_at].trim().is_empty() {
                    let rest = current[space_at + 1..].to_string();
                    out.push(current[..space_at].trim_end().to_string());
                    current = rest;
                    used = pi_tui::display_width(&current);
                    continue;
                }
            }
            out.push(std::mem::take(&mut current));
            used = 0;
            continue;
        }
        current.push(ch);
        used += char_width;
        index += 1;
    }
    out.push(current);
    out
}

/// 取扩展对某个工具调用的渲染结果（`phase` = `call`/`result`）；`None` = 回落内置渲染。
#[must_use]
pub fn render_tool_block(
    tool_name: &str,
    phase: &str,
    payload: serde_json::Value,
    width: usize,
) -> Option<Vec<String>> {
    let host = HOST.get().and_then(|slot| slot.as_ref()).cloned()?;
    let frame = host.render(tool_name, phase, payload, width).ok()??;
    flatten_render_frame(&frame, width)
}

/// `input` 钩子的结果（ref `types.ts:952-975` 的 `InputEventResult`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputOutcome {
    /// 原样提交。
    Continue,
    /// 用替换后的文本提交（ref 的 `action:"transform"`）。
    Transform(String),
    /// 扩展已接管，**不跑 agent**（ref 的 `action:"handled"`）。
    Handled,
}

/// 已注册的扩展命令名（宿主握手时收集）。
#[must_use]
pub fn command_names() -> Vec<String> {
    HOST.get()
        .and_then(|slot| slot.as_ref())
        .map(|host| host.commands().to_vec())
        .unwrap_or_default()
}

/// 尝试把 `/name args` 交给扩展命令（ref `agent-session.ts:1766-1789`）。
///
/// 返回 `true` 表示已处理（**不**再把这段文本当 prompt 发送）。命令不存在 ⇒ `false`。
pub fn try_run_command(text: &str) -> bool {
    if !text.starts_with('/') {
        return false;
    }
    let body = &text[1..];
    let (name, args) = match body.find(' ') {
        Some(index) => (&body[..index], &body[index + 1..]),
        None => (body, ""),
    };
    if !command_names().iter().any(|known| known == name) {
        return false;
    }
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref()).cloned() else {
        return false;
    };
    match host.invoke_command(name, args, std::time::Duration::from_secs(30)) {
        Ok(_value) => true,
        Err(error) => {
            eprintln!("pi-rs: 扩展命令 {name} 执行失败：{error}");
            true
        }
    }
}

/// 派发 `input` 事件（ref `agent-session.ts:1553-1572` + `runner.ts:1412-1460`）。
///
/// 无宿主 / 无处理器 / 处理器出错时一律 `Continue`（与 ref 的"异常只上报不阻断"一致）。
#[must_use]
pub fn run_input_hooks(text: &str, source: &str) -> InputOutcome {
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref()).cloned() else {
        return InputOutcome::Continue;
    };
    let payload = serde_json::json!({
        "type": "input",
        "text": text,
        "source": source,
    });
    let Ok(Some(value)) = host.emit_value("input", payload, std::time::Duration::from_secs(10))
    else {
        return InputOutcome::Continue;
    };
    let Some(action) = value.get("action").and_then(|a| a.as_str()) else {
        return InputOutcome::Continue;
    };
    match action {
        "handled" => InputOutcome::Handled,
        "transform" => value
            .get("text")
            .and_then(|t| t.as_str())
            .map(|t| InputOutcome::Transform(t.to_string()))
            .unwrap_or(InputOutcome::Continue),
        _other => InputOutcome::Continue,
    }
}

/// `before_agent_start` 的结果：注入的自定义消息 + 系统提示覆盖。
#[derive(Debug, Clone, Default)]
pub struct BeforeAgentStart {
    /// `{customType, content}` 形式的自定义消息（ref `types.ts:735-748`）。
    pub messages: Vec<serde_json::Value>,
    /// 系统提示覆盖（后者胜）。
    pub system_prompt: Option<String>,
}

/// 派发 `before_agent_start`（ref `agent-session.ts:1703`、`runner.ts:1312-1365`）。
pub fn emit_before_agent_start(prompt: &str, system_prompt: &str) -> BeforeAgentStart {
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref()).cloned() else {
        return BeforeAgentStart::default();
    };
    let payload = serde_json::json!({
        "type": "before_agent_start",
        "prompt": prompt,
        "systemPrompt": system_prompt,
    });
    let Some(value) = host
        .emit_value(
            "before_agent_start",
            payload,
            std::time::Duration::from_secs(10),
        )
        .ok()
        .flatten()
    else {
        return BeforeAgentStart::default();
    };
    BeforeAgentStart {
        messages: value
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        system_prompt: value
            .get("systemPrompt")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

/// 派发 `session_shutdown`（ref `agent-session-runtime.ts:167-176` 的 `teardownCurrent`
/// 与 `:404-411` 的 `dispose`）：`{type, reason, targetSessionFile?}`，
/// reason 取值 `new` / `resume` / `quit`。
pub fn emit_session_shutdown(reason: &str, target: Option<&std::path::Path>) {
    let payload = serde_json::json!({
        "type": "session_shutdown",
        "reason": reason,
        "targetSessionFile": target.map(|path| path.display().to_string()),
    });
    emit_event("session_shutdown", payload);
}

/// `user_bash` 的三种返回（ref `runner.ts:1154-1170`）。
pub enum UserBash {
    /// 没有扩展处理（或处理器返回 `undefined`）⇒ 本地执行。
    None,
    /// 扩展直接给了结果，本地不再执行（ref `interactive-mode.ts:6747-6773`）。
    Result(serde_json::Value),
    /// 扩展提供了执行实现（ref `:6781-6805`）。
    Operations(serde_json::Value),
    /// 处理器抛错 ⇒ **fail-closed**，不执行（ref `:6741-6744`）。
    Failed,
}

/// 派发 `user_bash`（ref `interactive-mode.ts:6732-6744`）。
#[must_use]
pub fn emit_user_bash(
    command: &str,
    exclude_from_context: bool,
    cwd: &std::path::Path,
) -> UserBash {
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref()).cloned() else {
        return UserBash::None;
    };
    let payload = serde_json::json!({
        "type": "user_bash",
        "command": command,
        "excludeFromContext": exclude_from_context,
        "cwd": cwd.display().to_string(),
    });
    match host.emit_value("user_bash", payload, std::time::Duration::from_secs(30)) {
        Ok(Some(value)) => {
            if value.get("__failed").is_some() {
                return UserBash::Failed;
            }
            if let Some(result) = value.get("result") {
                UserBash::Result(result.clone())
            } else if let Some(operations) = value.get("operations") {
                UserBash::Operations(operations.clone())
            } else {
                UserBash::None
            }
        }
        Ok(None) => UserBash::None,
        Err(_) => UserBash::Failed,
    }
}

/// 派发 `session_before_switch`（ref `agent-session-runtime.ts:231` 的 `emitBeforeSwitch`）。
///
/// 返回 `true` 表示扩展要求**取消**本次切换（`{cancelled:true}`）。没有扩展/超时按"不取消"处理
/// （ref 里没有 handler 时结果为空，等同放行）。
#[must_use]
pub fn emit_session_before_switch(reason: &str, previous: Option<&std::path::Path>) -> bool {
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref()) else {
        return false;
    };
    let payload = serde_json::json!({
        "type": "session_before_switch",
        "reason": reason,
        "previousSessionFile": previous.map(|path| path.display().to_string()),
    });
    host.emit_value(
        "session_before_switch",
        payload,
        std::time::Duration::from_secs(10),
    )
    .ok()
    .flatten()
    .and_then(|value| value.get("cancelled").and_then(serde_json::Value::as_bool))
    .unwrap_or(false)
}

/// 派发 `agent_before_settle`（ref `agent-session.ts:1532-1550`、`runner.ts:928-975`）。
///
/// 返回扩展要求的 `continue` 与要落盘的 `entries`（print 路径据此决定是否再跑一轮）。
#[must_use]
pub fn emit_agent_before_settle(outcome: &str) -> SettleOutcome {
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref()).cloned() else {
        return SettleOutcome::default();
    };
    let payload = serde_json::json!({
        "type": "agent_before_settle",
        "outcome": outcome,
    });
    host.emit_value(
        "agent_before_settle",
        payload,
        std::time::Duration::from_secs(10),
    )
    .ok()
    .flatten()
    .map(|value| SettleOutcome {
        should_continue: value
            .get("continue")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        entries: value
            .get("entries")
            .and_then(|items| items.as_array())
            .cloned()
            .unwrap_or_default(),
    })
    .unwrap_or_default()
}

/// `agent_before_settle` 的返回值（ref `types.ts:817-822`）。
#[derive(Debug, Clone, Default)]
pub struct SettleOutcome {
    /// 是否再跑一轮。
    pub should_continue: bool,
    /// 要写进会话的条目（本实现以 `role:"custom"` 消息落盘）。
    pub entries: Vec<serde_json::Value>,
}

/// `session_start` 的载荷（ref `types.ts:568-575`：`{type, reason, previousSessionFile?}`）。
#[must_use]
pub fn session_start_payload(
    reason: &str,
    previous: Option<&std::path::Path>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({"type": "session_start", "reason": reason});
    if let Some(path) = previous {
        payload["previousSessionFile"] = serde_json::Value::String(path.display().to_string());
    }
    payload
}

/// `--list-tools` 的一行（内置 + 扩展）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRow {
    /// 工具名。
    pub name: String,
    /// 来源：`builtin` / `extension`。
    pub source: &'static str,
    /// 描述首行。
    pub description: String,
}

/// 组装 `--list-tools` 的行（内置在前，扩展按名字排序）。
#[must_use]
pub fn collect_rows(builtin: &[Arc<dyn Tool>], extensions: &[Arc<dyn Tool>]) -> Vec<ToolRow> {
    let mut rows: Vec<ToolRow> = builtin
        .iter()
        .map(|tool| ToolRow {
            name: tool.name().to_string(),
            source: "builtin",
            description: first_line(tool.description()),
        })
        .collect();
    let mut ext_rows: Vec<ToolRow> = extensions
        .iter()
        .map(|tool| ToolRow {
            name: tool.name().to_string(),
            source: "extension",
            description: first_line(tool.description()),
        })
        .collect();
    ext_rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows.extend(ext_rows);
    rows
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}

/// 带扩展钩子的工具装饰器（`tool_call` / `tool_result`）。
///
/// ref 语义与行号：`tool_call` 在执行前、**首个 block 即止**（`runner.ts:1136-1153`、
/// `agent-loop.ts:722-740`）；`tool_result` 在执行后链式替换（`runner.ts:1080-1135`）。
///
/// 线程模型：宿主由 actor 线程独占（`notes/decisions.md`）；这里只通过句柄的
/// `*_async` 方法调用（内部 `spawn_blocking`，不阻塞 async worker）。
pub struct HookedTool {
    inner: Arc<dyn Tool>,
    host: pi_ext_host::HostHandle,
}

impl HookedTool {
    /// 装饰一个工具。
    #[must_use]
    pub fn new(inner: Arc<dyn Tool>, host: pi_ext_host::HostHandle) -> Self {
        Self { inner, host }
    }

    async fn emit(
        host: pi_ext_host::HostHandle,
        name: &'static str,
        payload: serde_json::Value,
    ) -> Option<serde_json::Value> {
        let timeout = std::time::Duration::from_secs(2);
        match host
            .emit_value_async(name.to_string(), payload, timeout)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                eprintln!("pi-rs: 扩展事件 {name} 失败：{error}");
                None
            }
        }
    }
}

impl Tool for HookedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> &serde_json::Value {
        self.inner.schema()
    }

    fn execute<'a>(
        &'a self,
        arguments: serde_json::Value,
        context: &'a pi_core::tools::tool::ToolContext,
        cancel: &'a tokio_util::sync::CancellationToken,
    ) -> futures::future::BoxFuture<'a, pi_core::tools::tool::ToolResult> {
        Box::pin(async move {
            use pi_core::tools::tool::{ToolResult, ToolResultBlock};
            let tool_call_id = next_tool_call_id();
            let tool_name = self.inner.name().to_string();
            let before = Self::emit(
                self.host.clone(),
                "tool_call",
                serde_json::json!({
                    "type": "tool_call",
                    "toolName": tool_name,
                    "toolCallId": tool_call_id,
                    "input": arguments.clone(),
                }),
            )
            .await;
            // `tool_call` 的**就地修改**（ref `agent-loop.ts:727`/`:770-775`：扩展拿到的是同一个
            // `args` 对象，改它即改执行参数）。宿主只认"原对象被就地改过"——返回 `{ input }`
            // 在 ref 里无效（`runner.ts:1134-1152` 只用 `block/reason/terminate`），这里同样忽略
            // 一个**替换**语义：runner 回传的永远是它收到的那个 `input` 对象。
            let mut arguments = arguments;
            if let Some(result) = before.as_ref() {
                if let Some(mutated) = result.get("input") {
                    if mutated.is_object() {
                        arguments = mutated.clone();
                    }
                }
            }
            if let Some(result) = before {
                if result.get("block").and_then(serde_json::Value::as_bool) == Some(true) {
                    let reason = result
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .filter(|text| !text.is_empty())
                        .unwrap_or("Tool execution was blocked")
                        .to_string();
                    return ToolResult {
                        content: vec![ToolResultBlock::Text { text: reason }],
                        is_error: true,
                        details: None,
                        terminate: result
                            .get("terminate")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    };
                }
            }
            let mut result = self.inner.execute(arguments.clone(), context, cancel).await;
            let after = Self::emit(
                self.host.clone(),
                "tool_result",
                serde_json::json!({
                    "type": "tool_result",
                    "toolName": tool_name,
                    "toolCallId": tool_call_id,
                    "input": arguments,
                    "content": blocks_to_json(&result.content),
                    "details": result.details,
                    "isError": result.is_error,
                }),
            )
            .await;
            if let Some(replacement) = after {
                if let Some(content) = replacement.get("content") {
                    let blocks = json_to_blocks(content);
                    if !blocks.is_empty() {
                        result.content = blocks;
                    }
                }
                if let Some(details) = replacement.get("details") {
                    if !details.is_null() {
                        result.details = Some(details.clone());
                    }
                }
                if let Some(is_error) = replacement
                    .get("isError")
                    .and_then(serde_json::Value::as_bool)
                {
                    result.is_error = is_error;
                }
            }
            result
        })
    }
}

fn next_tool_call_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    format!("pi-rs-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn blocks_to_json(blocks: &[pi_core::tools::tool::ToolResultBlock]) -> serde_json::Value {
    use pi_core::tools::tool::ToolResultBlock;
    serde_json::Value::Array(
        blocks
            .iter()
            .map(|block| match block {
                ToolResultBlock::Text { text } => serde_json::json!({"type": "text", "text": text}),
                ToolResultBlock::Image { data, mime_type } => {
                    serde_json::json!({"type": "image", "data": data, "mimeType": mime_type})
                }
            })
            .collect(),
    )
}

fn json_to_blocks(value: &serde_json::Value) -> Vec<pi_core::tools::tool::ToolResultBlock> {
    use pi_core::tools::tool::ToolResultBlock;
    let mut out = Vec::new();
    if let Some(items) = value.as_array() {
        for item in items {
            match item.get("type").and_then(serde_json::Value::as_str) {
                Some("text") => out.push(ToolResultBlock::Text {
                    text: item
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                }),
                Some("image") => out.push(ToolResultBlock::Image {
                    data: item
                        .get("data")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    mime_type: item
                        .get("mimeType")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("image/png")
                        .to_string(),
                }),
                _other => {}
            }
        }
    }
    out
}

/// 当扩展宿主可用时把工具包一层钩子；否则原样返回。
#[must_use]
pub fn wrap_with_hooks(tools: Vec<Arc<dyn Tool>>) -> Vec<Arc<dyn Tool>> {
    let Some(host) = HOST.get().and_then(|slot| slot.as_ref()).cloned() else {
        return tools;
    };
    tools
        .into_iter()
        .map(|tool| Arc::new(HookedTool::new(tool, host.clone())) as Arc<dyn Tool>)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static str, &'static str);

    impl Tool for Fake {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            self.1
        }
        fn schema(&self) -> &serde_json::Value {
            static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
            SCHEMA.get_or_init(|| serde_json::json!({"type": "object"}))
        }
        fn execute<'a>(
            &'a self,
            _arguments: serde_json::Value,
            _context: &'a pi_core::tools::tool::ToolContext,
            _cancel: &'a tokio_util::sync::CancellationToken,
        ) -> futures::future::BoxFuture<'a, pi_core::tools::tool::ToolResult> {
            Box::pin(async { pi_core::tools::tool::ToolResult::text("ok") })
        }
    }

    /// 内置在前、扩展按名字排序；描述取首行。
    #[test]
    fn rows_are_grouped_and_sorted() {
        let builtin: Vec<Arc<dyn Tool>> = vec![Arc::new(Fake("read", "读文件\n第二行"))];
        let extensions: Vec<Arc<dyn Tool>> = vec![
            Arc::new(Fake("web_search", "网页搜索")),
            Arc::new(Fake("ext_echo", "回显")),
        ];
        let rows = collect_rows(&builtin, &extensions);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, vec!["read", "ext_echo", "web_search"]);
        assert_eq!(rows[0].description, "读文件");
        assert_eq!(rows[1].source, "extension");
    }

    /// 没有扩展目录 ⇒ `extension_tools` 返回空（不 spawn、不报错）。
    #[test]
    fn missing_extensions_dir_is_silent() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(extension_tools(dir.path(), Path::new("/tmp")).is_empty());
    }

    /// Windows 的 node 带扩展名：`PATH` 里只有 `node.exe` 时也必须找到。
    /// 真机曾报"扩展未加载：找不到 node"，而 `node -v` 正常。
    #[test]
    fn find_node_accepts_exe_suffix_on_path() {
        let dir = tempfile::tempdir().expect("临时目录");
        let exe = dir.path().join("node.exe");
        std::fs::write(&exe, b"").expect("写假 node.exe");
        // 纯函数：只给定 PATH 值，不碰进程环境（并行测试下安全）。
        assert_eq!(
            find_node_in_path(dir.path().as_os_str()).as_deref(),
            Some(exe.as_path())
        );
    }

    /// `PI_NODE` 指向不存在的文件 ⇒ `find_node` 返回 None（不 panic）。
    #[test]
    fn find_node_honours_explicit_override() {
        // 该测试读进程环境；用不存在的路径验证覆盖分支。
        let previous = std::env::var_os("PI_NODE");
        // SAFETY（测试内）：单线程测试进程，设置/恢复环境变量不与其他线程竞争。
        std::env::set_var("PI_NODE", "/nonexistent/node-pi-test");
        assert!(find_node().is_none());
        match previous {
            Some(value) => std::env::set_var("PI_NODE", value),
            None => std::env::remove_var("PI_NODE"),
        }
    }
}
