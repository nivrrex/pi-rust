//! 官方内置 `llama.cpp` 扩展的原生实现。
//!
//! 行为来源：`ref/pi-0.87.1/packages/coding-agent/src/extensions/llama/{client,provider,index}.ts`。
//! pi-rs 没有跨进程 provider 函数序列化，因此模型目录同步到 `models.json`，请求仍走现有
//! OpenAI-compatible provider；管理命令在后台执行，不阻塞 TUI 主循环。

use std::path::Path;
use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

/// llama provider id（ref `extensions/llama/provider.ts:12`）。
pub const PROVIDER_ID: &str = "llama.cpp";
/// 默认 llama.cpp router 地址。
pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8080";

/// llama 管理失败。
#[derive(Debug, Error)]
pub enum LlamaError {
    /// URL 不合法或协议不受支持。
    #[error("{0}")]
    Url(String),
    /// HTTP 或 JSON 失败。
    #[error("{0}")]
    Request(String),
    /// 配置读写失败。
    #[error("{0}")]
    Config(String),
    /// 命令参数不合法。
    #[error("usage: /llama [load <model> | unload <model> | download <model>]")]
    Usage,
}

/// router 报告的状态。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LlamaStatus {
    Unloaded,
    Loading,
    Loaded,
    Downloading,
    Sleeping,
}

/// 单个模型的运行状态。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LlamaModelStatus {
    pub value: LlamaStatus,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub failed: bool,
}

/// llama.cpp router 的模型目录项。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct LlamaModelInfo {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub status: LlamaModelStatus,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub architecture: Option<LlamaArchitecture>,
    #[serde(default)]
    pub meta: Option<LlamaMeta>,
}

/// 模型输入/输出模态。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LlamaArchitecture {
    #[serde(default)]
    pub input_modalities: Vec<String>,
}

/// 模型元数据。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LlamaMeta {
    #[serde(default)]
    pub n_ctx: Option<u64>,
    #[serde(default)]
    pub n_ctx_train: Option<u64>,
}

/// router `/props` 的相关字段。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LlamaProps {
    #[serde(default)]
    pub models_autoload: bool,
    #[serde(default)]
    pub chat_template: String,
}

/// 已解析的 llama 配置。
#[derive(Debug, Clone)]
pub struct LlamaConfig {
    pub server_url: String,
    pub api_key: Option<String>,
}

/// llama.cpp router HTTP 客户端。
#[derive(Debug, Clone)]
pub struct LlamaClient {
    server_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl LlamaClient {
    /// 创建客户端；URL 规范化规则与 ref `client.ts:142-151` 一致。
    pub fn new(server_url: &str, api_key: Option<String>) -> Result<Self, LlamaError> {
        let server_url = normalize_server_url(server_url)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| LlamaError::Request(error.to_string()))?;
        Ok(Self {
            server_url,
            api_key,
            http,
        })
    }

    /// 返回规范化后的 server URL。
    #[must_use]
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// 返回推理端点（ref `client.ts:154`）。
    #[must_use]
    pub fn inference_url(&self) -> String {
        format!("{}/v1", self.server_url)
    }

    async fn request(&self, path: &str, body: Option<Value>) -> Result<Value, LlamaError> {
        let mut request = self.http.post(format!("{}{path}", self.server_url));
        if let Some(body) = body {
            request = request.json(&body);
        }
        self.send(request).await
    }

    async fn get(&self, path: &str) -> Result<Value, LlamaError> {
        self.send(self.http.get(format!("{}{path}", self.server_url)))
            .await
    }

    async fn send(&self, mut request: reqwest::RequestBuilder) -> Result<Value, LlamaError> {
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| LlamaError::Request(error.to_string()))?;
        let status = response.status();
        let payload = response.json::<Value>().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let message = payload
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(LlamaError::Request(if message.is_empty() {
                format!("llama.cpp returned HTTP {status}")
            } else {
                message.to_string()
            }));
        }
        Ok(payload)
    }

    /// 读取模型目录。
    pub async fn list(&self) -> Result<Vec<LlamaModelInfo>, LlamaError> {
        let payload = self.get("/models").await?;
        let data = payload
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                LlamaError::Request("llama.cpp returned an invalid model catalog".into())
            })?;
        data.iter()
            .map(|item| {
                let id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| LlamaError::Request("model id is missing".into()))?;
                let status = item
                    .get("status")
                    .and_then(|status| status.get("value"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| LlamaError::Request(format!("model {id} has no status")))?;
                let _ = status;
                serde_json::from_value::<LlamaModelInfo>(item.clone())
                    .map_err(|error| LlamaError::Request(error.to_string()))
            })
            .collect()
    }

    /// 读取 router 属性。
    pub async fn props(&self, model: Option<&str>) -> Result<LlamaProps, LlamaError> {
        let path = model.map_or_else(
            || "/props".to_string(),
            |model| format!("/props?model={}&autoload=false", urlencoding(model)),
        );
        let payload = self.get(&path).await?;
        Ok(serde_json::from_value(payload).unwrap_or_default())
    }

    /// 加载模型。
    pub async fn load(&self, model: &str) -> Result<(), LlamaError> {
        self.request("/models/load", Some(json!({ "model": model })))
            .await
            .map(|_| ())
    }

    /// 卸载模型。
    pub async fn unload(&self, model: &str) -> Result<(), LlamaError> {
        self.request("/models/unload", Some(json!({ "model": model })))
            .await
            .map(|_| ())
    }

    /// 卸载并轮询到 `unloaded`。
    pub async fn unload_and_wait(&self, model: &str) -> Result<(), LlamaError> {
        self.unload(model).await?;
        loop {
            let models = self.list().await?;
            if models
                .iter()
                .find(|candidate| candidate.id == model)
                .is_none_or(|candidate| candidate.status.value == LlamaStatus::Unloaded)
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// 下载模型。
    pub async fn download(&self, model: &str) -> Result<(), LlamaError> {
        self.request("/models", Some(json!({ "model": model })))
            .await
            .map(|_| ())
    }
}

fn urlencoding(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// 规范化 server URL；去掉 query/hash、尾部斜杠和 `/v1`。
pub fn normalize_server_url(value: &str) -> Result<String, LlamaError> {
    let mut url = Url::parse(value.trim()).map_err(|error| LlamaError::Url(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(LlamaError::Url("Server URL must use http or https".into()));
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_string();
    let path = path.strip_suffix("/v1").unwrap_or(&path);
    let path = if path.is_empty() { "/" } else { path };
    url.set_path(path);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// 从 `auth.json` 与环境变量解析 llama 配置；没有配置时返回 `None`。
#[must_use]
pub fn resolve_config(agent_dir: &Path) -> Option<LlamaConfig> {
    let text = std::fs::read_to_string(agent_dir.join("auth.json")).ok()?;
    let parsed: Value = serde_json::from_str(&text).ok()?;
    let entry = parsed.get(PROVIDER_ID)?;
    if entry.get("type").and_then(Value::as_str) != Some("api_key") {
        return None;
    }
    let stored_url = entry
        .get("env")
        .and_then(|env| env.get("LLAMA_BASE_URL"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let env_url = std::env::var("LLAMA_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let raw_url = stored_url.or(env_url.as_deref())?;
    let server_url = normalize_server_url(raw_url).ok()?;
    let api_key = entry
        .get("key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| std::env::var("LLAMA_API_KEY").ok())
        .filter(|value| !value.trim().is_empty());
    Some(LlamaConfig {
        server_url,
        api_key,
    })
}

/// 保存 llama 凭据，保留 auth.json 未知字段。
pub fn save_config(
    agent_dir: &Path,
    server_url: &str,
    api_key: Option<&str>,
) -> Result<(), LlamaError> {
    let path = agent_dir.join("auth.json");
    let guard = crate::config_store::ConfigLockGuard::acquire(
        &path,
        Duration::from_secs(30),
        Duration::from_secs(30),
    )
    .map_err(|error| LlamaError::Config(error.to_string()))?;
    let result = (|| {
        let mut root = match std::fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => json!({}),
            Ok(text) => serde_json::from_str::<Value>(&text)
                .map_err(|error| LlamaError::Config(error.to_string()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(error) => return Err(LlamaError::Config(error.to_string())),
        };
        let object = root
            .as_object_mut()
            .ok_or_else(|| LlamaError::Config("auth.json 顶层不是对象".into()))?;
        let mut entry = object
            .remove(PROVIDER_ID)
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        entry.insert("type".into(), json!("api_key"));
        if let Some(key) = api_key.filter(|key| !key.trim().is_empty()) {
            entry.insert("key".into(), json!(key));
        } else {
            entry.remove("key");
        }
        let mut env = entry
            .get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        env.insert("LLAMA_BASE_URL".into(), json!(server_url));
        entry.insert("env".into(), Value::Object(env));
        object.insert(PROVIDER_ID.into(), Value::Object(entry));
        let text = serde_json::to_string_pretty(&root)
            .map_err(|error| LlamaError::Config(error.to_string()))?;
        crate::config_store::write_atomic(&path, &text, true)
            .map_err(|error| LlamaError::Config(error.to_string()))
    })();
    guard
        .check()
        .map_err(|error| LlamaError::Config(error.to_string()))?;
    guard
        .release()
        .map_err(|error| LlamaError::Config(error.to_string()))?;
    result
}

/// 启动时刷新已配置 llama 的模型目录；未配置时不发网络请求。
pub async fn refresh_if_configured(agent_dir: &Path) -> Result<(), LlamaError> {
    let Some(config) = resolve_config(agent_dir) else {
        return Ok(());
    };
    let client = LlamaClient::new(&config.server_url, config.api_key)?;
    let catalog = client.list().await?;
    sync_catalog(agent_dir, &client, &catalog).await
}

/// 把 router 目录同步进 models.json，保留其它 provider 与未知字段。
pub async fn sync_catalog(
    agent_dir: &Path,
    client: &LlamaClient,
    catalog: &[LlamaModelInfo],
) -> Result<(), LlamaError> {
    let router_autoload = if catalog.iter().any(|model| {
        model.status.value == LlamaStatus::Unloaded && model.source.as_deref() == Some("preset")
    }) {
        client.props(None).await?.models_autoload
    } else {
        false
    };
    let mut models = Vec::new();
    for model in catalog
        .iter()
        .filter(|model| selectable(model, router_autoload))
    {
        let props = if model.status.value == LlamaStatus::Loaded {
            Some(client.props(Some(&model.id)).await?)
        } else {
            None
        };
        models.push(to_model_value(model, client, props.as_ref()));
    }
    let path = agent_dir.join("models.json");
    let guard = crate::config_store::ConfigLockGuard::acquire(
        &path,
        Duration::from_secs(10),
        Duration::from_millis(200),
    )
    .map_err(|error| LlamaError::Config(error.to_string()))?;
    let result = (|| {
        let mut root = match std::fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => json!({}),
            Ok(text) => serde_json::from_str::<Value>(&text)
                .map_err(|error| LlamaError::Config(error.to_string()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(error) => return Err(LlamaError::Config(error.to_string())),
        };
        let object = root
            .as_object_mut()
            .ok_or_else(|| LlamaError::Config("models.json 顶层不是对象".into()))?;
        let providers = object.entry("providers").or_insert_with(|| json!({}));
        let providers = providers
            .as_object_mut()
            .ok_or_else(|| LlamaError::Config("models.json providers 不是对象".into()))?;
        let mut provider = providers
            .remove(PROVIDER_ID)
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        provider.insert("name".into(), json!(PROVIDER_ID));
        provider.insert("baseUrl".into(), json!(client.inference_url()));
        provider.insert("api".into(), json!("openai-completions"));
        provider.insert("authHeader".into(), json!(true));
        provider.insert("models".into(), Value::Array(models));
        providers.insert(PROVIDER_ID.into(), Value::Object(provider));
        let text = serde_json::to_string_pretty(&root)
            .map_err(|error| LlamaError::Config(error.to_string()))?;
        crate::config_store::write_atomic(&path, &text, false)
            .map_err(|error| LlamaError::Config(error.to_string()))
    })();
    guard
        .check()
        .map_err(|error| LlamaError::Config(error.to_string()))?;
    guard
        .release()
        .map_err(|error| LlamaError::Config(error.to_string()))?;
    result
}

fn selectable(model: &LlamaModelInfo, router_autoload: bool) -> bool {
    match model.status.value {
        LlamaStatus::Loaded | LlamaStatus::Sleeping => true,
        LlamaStatus::Unloaded => {
            router_autoload && !model.status.failed && model.source.as_deref() == Some("preset")
        }
        LlamaStatus::Loading | LlamaStatus::Downloading => false,
    }
}

fn to_model_value(
    model: &LlamaModelInfo,
    client: &LlamaClient,
    props: Option<&LlamaProps>,
) -> Value {
    let context_window = model
        .meta
        .as_ref()
        .and_then(|meta| meta.n_ctx.or(meta.n_ctx_train))
        .filter(|value| *value > 0)
        .unwrap_or(128_000);
    let reasoning = props.is_some_and(|props| props.chat_template.contains("enable_thinking"));
    let input = if model.architecture.as_ref().is_some_and(|architecture| {
        architecture
            .input_modalities
            .iter()
            .any(|value| value == "image")
    }) {
        json!(["text", "image"])
    } else {
        json!(["text"])
    };
    let mut value = json!({
        "id": model.id,
        "name": model.id,
        "api": "openai-completions",
        "baseUrl": client.inference_url(),
        "reasoning": reasoning,
        "input": input,
        "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 },
        "contextWindow": context_window,
        "maxTokens": context_window,
    });
    if reasoning {
        value["thinkingLevelMap"] = json!({
            "off": "off", "minimal": null, "low": null,
            "medium": "medium", "high": null, "xhigh": null,
        });
    }
    value
}

/// 解析 `/llama` 命令；返回参数部分。
#[must_use]
pub fn parse_command(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("/llama")?;
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix(char::is_whitespace)
}

/// 官方命令在非 TUI 模式的提示。
#[must_use]
pub fn non_tui_notice() -> &'static str {
    "/llama is available in interactive mode"
}

/// 执行 `/llama` 子命令；无参数时刷新目录。
pub async fn run_command(agent_dir: &Path, args: &str) -> Result<String, LlamaError> {
    let config = resolve_config(agent_dir).ok_or_else(|| {
        LlamaError::Config(format!("Configure llama.cpp with /login {PROVIDER_ID}"))
    })?;
    let client = LlamaClient::new(&config.server_url, config.api_key)?;
    let mut words = args.split_whitespace();
    let action = words.next().unwrap_or_default();
    let model = words.next();
    match (action, model) {
        ("", None) => {}
        ("load", Some(model)) => {
            client.load(model).await?;
            let catalog = client.list().await?;
            sync_catalog(agent_dir, &client, &catalog).await?;
            return Ok(format!("Load started for {model}"));
        }
        ("unload", Some(model)) => {
            client.unload_and_wait(model).await?;
            let catalog = client.list().await?;
            sync_catalog(agent_dir, &client, &catalog).await?;
            return Ok(format!("Unloaded {model}"));
        }
        ("download", Some(model)) => {
            client.download(model).await?;
            let catalog = client.list().await?;
            sync_catalog(agent_dir, &client, &catalog).await?;
            return Ok(format!("Downloaded {model}"));
        }
        (other, model) => {
            let _ = (other, model);
            return Err(LlamaError::Usage);
        }
    }
    let catalog = client.list().await?;
    sync_catalog(agent_dir, &client, &catalog).await?;
    Ok("Model catalogs refreshed.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_url_normalization_matches_the_reference() {
        for (input, want) in [
            ("http://127.0.0.1:8080/", "http://127.0.0.1:8080"),
            ("http://127.0.0.1:8080/v1/?x=1#h", "http://127.0.0.1:8080"),
            ("https://example.test/llama", "https://example.test/llama"),
        ] {
            assert_eq!(normalize_server_url(input).expect("url"), want);
        }
        assert!(normalize_server_url("file:///tmp/x").is_err());
    }

    #[tokio::test]
    async fn load_command_refreshes_models_and_preserves_auth() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let (requests_tx, mut requests_rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut chunk = [0u8; 2048];
                    let n = socket.read(&mut chunk).await.expect("read");
                    if n == 0 {
                        break bytes.len();
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + content_length {
                    let mut chunk = [0u8; 2048];
                    let n = socket.read(&mut chunk).await.expect("read body");
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let request = String::from_utf8_lossy(&bytes);
                let first = request.lines().next().unwrap_or_default().to_string();
                requests_tx
                    .send((first.clone(), request.to_string()))
                    .expect("record");
                let body = if first.starts_with("GET /models ") {
                    r#"{"data":[{"id":"m","status":{"value":"loaded"},"meta":{"n_ctx":4096}}]}"#
                } else if first.starts_with("GET /props") {
                    r#"{"models_autoload":true,"chat_template":"enable_thinking"}"#
                } else {
                    "{}"
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.expect("write");
            }
        });

        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(
            dir.path().join("auth.json"),
            r#"{"llama.cpp":{"type":"api_key","key":"old","extra":1},"other":{"type":"api_key","key":"keep"}}"#,
        )
        .expect("seed auth");
        std::fs::write(
            dir.path().join("models.json"),
            r#"{"providers":{"llama.cpp":{"extra":2},"other":{"models":[]}},"top":3}"#,
        )
        .expect("seed models");
        let server_url = format!("http://{addr}");
        save_config(dir.path(), &server_url, Some("secret")).expect("save config");
        let message =
            tokio::time::timeout(Duration::from_secs(10), run_command(dir.path(), "load m"))
                .await
                .expect("command timeout")
                .expect("command");
        assert_eq!(message, "Load started for m");

        let models: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("models.json")).expect("models"),
        )
        .expect("json");
        assert_eq!(models["providers"]["llama.cpp"]["models"][0]["id"], "m");
        assert_eq!(
            models["providers"]["llama.cpp"]["models"][0]["contextWindow"],
            4096
        );
        assert_eq!(
            models["providers"]["llama.cpp"]["models"][0]["reasoning"],
            true
        );
        let auth: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("auth.json")).expect("auth"),
        )
        .expect("json");
        assert_eq!(auth["llama.cpp"]["key"], "secret");
        assert_eq!(auth["llama.cpp"]["extra"], 1);
        assert_eq!(auth["other"]["key"], "keep");
        assert_eq!(auth["llama.cpp"]["env"]["LLAMA_BASE_URL"], server_url);
        assert_eq!(models["providers"]["llama.cpp"]["extra"], 2);
        assert_eq!(models["top"], 3);

        let mut requests = Vec::new();
        while let Ok(request) = requests_rx.try_recv() {
            requests.push(request);
        }
        assert_eq!(requests.len(), 3);
        assert!(requests[0].0.starts_with("POST /models/load "));
        assert!(requests[0]
            .1
            .to_lowercase()
            .contains("authorization: bearer secret"));
        assert!(requests
            .iter()
            .any(|(line, _)| line.starts_with("GET /models ")));
        assert!(requests
            .iter()
            .any(|(line, _)| line.starts_with("GET /props?model=m")));
    }

    #[test]
    fn selectable_models_follow_router_autoload_rules() {
        let model = |status: LlamaStatus, source: Option<&str>| LlamaModelInfo {
            id: "m".into(),
            aliases: Vec::new(),
            status: LlamaModelStatus {
                value: status,
                args: Vec::new(),
                failed: false,
            },
            source: source.map(str::to_string),
            architecture: None,
            meta: None,
        };
        assert!(selectable(&model(LlamaStatus::Loaded, None), false));
        assert!(selectable(&model(LlamaStatus::Sleeping, None), false));
        assert!(!selectable(
            &model(LlamaStatus::Unloaded, Some("preset")),
            false
        ));
        assert!(selectable(
            &model(LlamaStatus::Unloaded, Some("preset")),
            true
        ));
        assert!(!selectable(&model(LlamaStatus::Loading, None), true));
    }
}
