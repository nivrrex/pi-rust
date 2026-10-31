//! mock-provider：按场景脚本回放 SSE 的本地假 provider。
//!
//! 用法：`mock-provider <scenario.json> [--port N] [--request-log <path>]`。
//! 启动后把监听地址打到 stdout（`listening <host>:<port>`），便于 runner 抓取。
//!
//! 场景脚本格式见 `scenarios/SCHEMA.md`；响应体可以直接引用 `fixtures/sse/` 里的夹具文件。
//!
//! 隔离要求：只在回环上监听。必须由 `tools/difftest` 在 `unshare` 网络命名空间里启动，
//! 这样即使配置出错也发不出去真实请求。

#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// 场景 JSON 的顶层键：`provider`/`model`/`mode` 等由 runner 消费，
/// 这里仍显式列出并 `deny_unknown_fields`——否则拼错或大小写写错的键会被静默忽略
/// （D0：`gateAfterChunks` 一度从未生效就是这么来的）。
// 这些字段里有一大半由 runner 消费、mock 自己不读——它们存在的意义是让
// `deny_unknown_fields` 能按完整 schema 拒掉拼错的键（D0）。因此显式允许 dead_code。
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    /// P7-2：TUI 场景字段（`cols/rows/steps/snapshots`）。mock 自己不消费，
    /// 但必须显式允许，否则 `deny_unknown_fields` 会把 TUI 场景整个拒掉。
    #[serde(default)]
    tui: Option<serde_json::Value>,
    /// 按请求顺序回放；用完以后重复最后一条。
    responses: Vec<Step>,
    #[serde(default)]
    id: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    mode: String,
    /// P7-7f：TUI 场景可声明额外模型（runner 写进 `models.json`，mock 自己不消费）。
    #[serde(default)]
    // 场景可以给模型加能力（如 `{ "id": …, "reasoning": true }`）；mock 自己不用，
    // 只要求能解析（runner 会把它并进 `models.json`）。
    extra_models: Vec<serde_json::Value>,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    reasoning_model: Option<bool>,
    #[serde(default)]
    cost: Option<serde_json::Value>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    max_tokens: Option<u64>,
    #[serde(default)]
    settings: Option<serde_json::Value>,
    /// 主题名（`dark`/`light`）：runner 写进 `settings.json`，mock 只要求能解析。
    #[serde(default)]
    theme: Option<String>,
    /// 额外 provider（runner 写进 `models.json`，mock 自己不消费）。
    #[serde(default)]
    extra_providers: Option<serde_json::Value>,
    #[serde(default)]
    commands: Option<serde_json::Value>,
    #[serde(default)]
    follow_up: Option<serde_json::Value>,
    #[serde(default)]
    script: Option<serde_json::Value>,
    #[serde(default)]
    files: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    global_files: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    session_files: Option<serde_json::Value>,
    #[serde(default)]
    cli_args: Option<serde_json::Value>,
    #[serde(default)]
    enable_skills: Option<bool>,
    #[serde(default)]
    enable_prompt_templates: Option<bool>,
    /// 期望的注入次数（由 runner 读取并断言，mock 本身不使用）。
    #[serde(default)]
    expect_injections: Option<serde_json::Value>,
}

// 场景 JSON 用 camelCase（`gateAfterChunks` 等）。此前没有这个映射，
// serde 默认忽略未知字段——闸门与断连开关**一直被静默忽略**，
// S08/S09 的时序其实从未生效（D0）。
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Step {
    /// SSE 夹具路径，相对仓库根。
    file: String,
    /// HTTP 状态码，默认 200。用于注入 429 等错误。
    #[serde(default = "default_status")]
    status: u16,
    /// `Retry-After` 响应头秒数。
    #[serde(default)]
    retry_after: Option<String>,
    /// 在发出第 N 个数据块之后暂停，等待 `POST /control/release`。
    #[serde(default)]
    gate_after_chunks: Option<usize>,
    /// 在第 N 个数据块之后直接断开连接（流中途断连）。
    #[serde(default)]
    disconnect_after_chunks: Option<usize>,
    /// 按 SSE 事件把响应切成多块写出（每块一个 `\n\n` 分隔的完整事件）。
    ///
    /// 不给时整块一次写完——`gate_after_chunks` / `disconnect_after_chunks` 在整块模式下
    /// 只能落在写完之后，起不到"中途"的作用（S08 长期没覆盖到真正断连就是这个原因）。
    #[serde(default)]
    chunk_by_event: Option<bool>,
}

fn default_status() -> u16 {
    200
}

struct State {
    scenario: Scenario,
    next: usize,
    root: PathBuf,
    log_path: Option<PathBuf>,
    /// 可选的**请求头日志**（`--header-log`）：只在显式传入时写，故不影响既有 golden。
    header_log_path: Option<PathBuf>,
    /// 闸门是否已放行；每放行一次后自动重新武装，以支持一次运行里的多个闸门。
    released: bool,
    /// 已服务（开始应答）的请求数。
    served: usize,
    /// 已完整回放完的响应数。
    consumed: usize,
    /// 正在处理中的请求数。
    in_flight: i64,
    /// 闸门实际生效的次数（D1：用于断言"注入确实发生"）。
    gate_hits: usize,
    /// 实际断连的次数。
    disconnects: usize,
    /// 实际写出过 `Retry-After` 头的次数。
    retry_after_sent: usize,
}

#[derive(Debug)]
struct Request {
    path: String,
    body: String,
    /// 原始请求头（P4-15 步骤 4.1）：`(name, value)`，按出现顺序。
    headers: Vec<(String, String)>,
}

/// 读取一条 HTTP 请求（够用即可：只关心 Content-Length 与 body）。
async fn read_request(stream: &mut tokio::net::TcpStream) -> std::io::Result<Option<Request>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(header_end) = find(&buf, b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let mut content_length = 0usize;
            for line in headers.lines() {
                if let Some(value) = line
                    .strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
                {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
            let body_start = header_end + 4;
            if buf.len() >= body_start + content_length {
                let path = headers
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let body = String::from_utf8_lossy(&buf[body_start..body_start + content_length])
                    .to_string();
                // 请求头按 `name: value` 拆（第一行是请求行，跳过）。
                let parsed_headers = headers
                    .lines()
                    .skip(1)
                    .filter_map(|line| {
                        line.split_once(':').map(|(name, value)| {
                            (name.trim().to_string(), value.trim().to_string())
                        })
                    })
                    .collect();
                return Ok(Some(Request {
                    path,
                    body,
                    headers: parsed_headers,
                }));
            }
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 按 SSE 事件边界切块：每个 `\n\n` 结束的事件自成一块。
fn split_by_event(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        current.push(bytes[index]);
        if bytes[index] == b'\n' && index + 1 < bytes.len() && bytes[index + 1] == b'\n' {
            current.push(b'\n');
            index += 2;
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            continue;
        }
        index += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

async fn handle(
    mut stream: tokio::net::TcpStream,
    state: Arc<Mutex<State>>,
) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream).await? else {
        return Ok(());
    };

    if request.path == "/control/release" {
        state.lock().await.released = true;
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .await;
        return Ok(());
    }
    if request.path == "/control/status" {
        let guard = state.lock().await;
        let body = serde_json::json!({
            "served": guard.served,
            "consumed": guard.consumed,
            "inFlight": guard.in_flight,
            "gateHits": guard.gate_hits,
            "disconnects": guard.disconnects,
            "retryAfterSent": guard.retry_after_sent,
            "gateArmed": !guard.released,
        })
        .to_string();
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await;
        return Ok(());
    }
    if request.path == "/control/requests" {
        let body = std::fs::read_to_string(
            state
                .lock()
                .await
                .log_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("/dev/null")),
        )
        .unwrap_or_default();
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/x-ndjson\r\ncontent-length: {}\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await;
        return Ok(());
    }

    {
        let guard = state.lock().await;
        if let Some(path) = &guard.header_log_path {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let headers: serde_json::Map<String, serde_json::Value> = request
                    .headers
                    .iter()
                    .map(|(name, value)| (name.clone(), serde_json::json!(value)))
                    .collect();
                let entry = serde_json::json!({ "path": request.path, "headers": headers });
                let _ = writeln!(file, "{entry}");
            }
        }
    }

    let (step, root, response_index) = {
        let mut guard = state.lock().await;
        guard.served += 1;
        guard.in_flight += 1;
        let index = guard
            .next
            .min(guard.scenario.responses.len().saturating_sub(1));
        guard.next += 1;
        // 取证：每条请求**实际用到**的 spec 下标（`served` 是第几次请求）。
        // 默认关闭：S01 等钉子对 `requests.jsonl` 的**行格式**敏感（一请求一行）。
        //
        // ⚠ `MOCK_VERBOSE=1` 必须设在 **mock 这个进程**上（不是被测进程）。
        // 只给被测进程设 → mock 仍走默认格式（不含 `headers`），排查时会误判成
        // "请求没带 Authorization"（P8e-7/F1-d 实际踩过这个坑）。。
        if std::env::var_os("MOCK_VERBOSE").is_some() {
            if let Some(path) = &guard.log_path {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    // F1-d：抓 header（`authorization` 等）——默认格式不含它，
                    // 只有 MOCK_VERBOSE 下的取证行需要。
                    let headers: serde_json::Map<String, serde_json::Value> = request
                        .headers
                        .iter()
                        .map(|(name, value)| {
                            (
                                name.to_ascii_lowercase(),
                                serde_json::Value::String(value.clone()),
                            )
                        })
                        .collect();
                    let entry = serde_json::json!({
                        "path": request.path,
                        "headers": headers,
                        "body": request.body,
                        "responseIndex": index,
                        "served": guard.served,
                        "disconnectAfterChunks": guard.scenario.responses[index]
                            .disconnect_after_chunks,
                    });
                    let _ = writeln!(file, "{entry}");
                }
            }
        } else if let Some(path) = &guard.log_path {
            // 默认格式（一请求一行）——钉子测试按行读取，不能改。
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let entry = serde_json::json!({ "path": request.path, "body": request.body });
                let _ = writeln!(file, "{entry}");
            }
        }
        (
            guard.scenario.responses[index].clone(),
            guard.root.clone(),
            index,
        )
    };

    let _ = response_index;
    let bytes = std::fs::read(root.join(&step.file)).unwrap_or_default();
    // 场景回放按整块写出；跨块切分只在 oracle（B2）里需要。
    let chunks: Vec<Vec<u8>> = match step.chunk_by_event {
        Some(true) => split_by_event(&bytes),
        _ => vec![bytes],
    };

    if step.status != 200 {
        let reason = if step.status == 429 {
            "Too Many Requests"
        } else {
            "Error"
        };
        let mut head = format!(
            "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n",
            step.status, reason, 2
        );
        if let Some(retry_after) = &step.retry_after {
            head.push_str(&format!("retry-after: {retry_after}\r\n"));
            state.lock().await.retry_after_sent += 1;
        }
        head.push_str("connection: close\r\n\r\n{}");
        let _ = stream.write_all(head.as_bytes()).await;
        let mut guard = state.lock().await;
        guard.consumed += 1;
        guard.in_flight -= 1;
        return Ok(());
    }

    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n";
    let _ = stream.write_all(head.as_bytes()).await;

    let mut written = 0_usize;
    for (index, chunk) in chunks.iter().enumerate() {
        let _ = stream.write_all(chunk).await;
        let _ = stream.flush().await;
        written += chunk.len();
        if Some(index + 1) == step.gate_after_chunks {
            state.lock().await.gate_hits += 1;
            loop {
                if state.lock().await.released {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }
        if Some(index + 1) == step.disconnect_after_chunks {
            let mut guard = state.lock().await;
            guard.disconnects += 1;
            guard.consumed += 1;
            guard.in_flight -= 1;
            drop(guard);
            let _ = stream.shutdown().await;
            return Ok(());
        }
    }
    {
        let mut guard = state.lock().await;
        guard.consumed += 1;
        guard.in_flight -= 1;
        // 取证：本次响应**实际写出**的字节数（判断是"服务端没写"还是"客户端没读"）。
        if std::env::var_os("MOCK_VERBOSE").is_some() {
            if let Some(path) = &guard.log_path {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let entry = serde_json::json!({
                        "responseIndex": response_index,
                        "writtenBytes": written,
                        "completed": true,
                    });
                    let _ = writeln!(file, "{entry}");
                }
            }
        }
        // 放行过一次后重新武装，下一个闸门需要再次 release。
        guard.released = false;
    }
    let _ = stream.shutdown().await;
    Ok(())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: mock-provider <scenario.json> [--port N] [--request-log <path>]");
        std::process::exit(2);
    }
    let scenario_path = PathBuf::from(&args[1]);
    let mut port = 0u16;
    let mut log_path = None;
    let mut header_log_path = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--port" => {
                port = args
                    .get(index + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                index += 2;
            }
            "--request-log" => {
                log_path = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--header-log" => {
                header_log_path = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            _ => index += 1,
        }
    }

    let text = std::fs::read_to_string(&scenario_path)?;
    let scenario: Scenario = serde_json::from_str(&text)?;
    let root = scenario_path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let state = Arc::new(Mutex::new(State {
        scenario,
        next: 0,
        root,
        log_path,
        header_log_path,
        released: false,
        served: 0,
        consumed: 0,
        in_flight: 0,
        gate_hits: 0,
        disconnects: 0,
        retry_after_sent: 0,
    }));

    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr: SocketAddr = listener.local_addr()?;
    println!("listening {addr}");
    use std::io::Write;
    std::io::stdout().flush().ok();

    loop {
        let (stream, _) = listener.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let _ = handle(stream, state).await;
        });
    }
}
