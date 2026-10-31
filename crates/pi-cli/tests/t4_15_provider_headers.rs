//! P4-15 步骤 4.1：`apiKey` / `headers` / `authHeader` 必须由 models.json 驱动。
//!
//! ref 语义：
//! - `core/model-config.ts:232` `apiKey`（provider 级）、`:235` provider 级 `headers`、
//!   `:202` 模型级 `headers`、`:237` `authHeader`；
//! - `provider-composer.ts:311-316` `configuredApiKey`：`extension.apiKey ?? config.apiKey`；
//! - `provider-composer.ts:431-445` `rawModelHeaders`：**模型级 headers 覆盖 provider 级**；
//! - `provider-composer.ts:297-309` `withConfiguredAuth`：配置头覆盖 auth 头；`authHeader` 为真时
//!   追加 `Authorization: Bearer <apiKey>`（没有 key 就报错）；
//! - `ai/src/api/openai-completions.ts:760` `headers = { "User-Agent": …, ...model.headers }`
//!   —— **模型头就是客户端默认头**（不是每次请求单独传）。
//! - 优先级（`model-runtime.ts:604`）：`providerOptions.apiKey ?? resolution.auth.apiKey`。
//!
//! 本测试走**生产构造路径**（models.json 夹具 → `ModelsConfig::resolve` → `build_provider` → 真实 client），
//! 由 mock 的 `--header-log` 记录**实际发出的请求头**。夹具里只放占位值，不含任何真实密钥。
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pi_ai::config::ModelsConfig;
use pi_ai::{Context, Message, ProviderRequest};

struct MockProvider {
    child: Child,
    port: u16,
    header_log: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl MockProvider {
    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("crate 目录在工作区根下")
            .to_path_buf()
    }

    fn start() -> Self {
        let repo = Self::repo_root();
        let binary = repo.join("target/debug/mock-provider");
        assert!(
            binary.exists(),
            "先构建 mock-provider：cargo build -p mock-provider（缺失：{}）",
            binary.display()
        );
        let dir = tempfile::tempdir().expect("建临时目录");
        let scenario = dir.path().join("scenario.json");
        let fixture = repo.join("fixtures/sse/openai/text-basic.sse");
        std::fs::write(
            &scenario,
            format!(
                "{{\"responses\":[{{\"file\":{}}}]}}\n",
                serde_json::to_string(&fixture.to_string_lossy().to_string()).expect("可序列化")
            ),
        )
        .expect("写临时场景");
        let header_log = dir.path().join("headers.jsonl");
        let mut child = Command::new(&binary)
            .current_dir(&repo)
            .arg(&scenario)
            .arg("--port")
            .arg("0")
            .arg("--header-log")
            .arg(&header_log)
            .stdout(Stdio::piped())
            .spawn()
            .expect("mock-provider 可执行文件存在");
        let stdout = child.stdout.take().expect("已取走 stdout");
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).expect("mock 会打印监听地址");
        let port: u16 = line
            .trim()
            .strip_prefix("listening 127.0.0.1:")
            .unwrap_or_else(|| panic!("地址格式为 listening 127.0.0.1:PORT，实际：{line:?}"))
            .parse()
            .expect("端口是数字");
        Self {
            child,
            port,
            header_log,
            _dir: dir,
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// 第一条请求实际带上的头（小写名 → 值），没发过请求时 panic。
    fn first_request_headers(&self) -> std::collections::BTreeMap<String, String> {
        let text = std::fs::read_to_string(&self.header_log).expect("mock 应已写下请求头日志");
        let line = text
            .lines()
            .find(|line| !line.trim().is_empty())
            .expect("至少有一条请求记录");
        let entry: serde_json::Value = serde_json::from_str(line).expect("日志行是 JSON");
        let map = entry
            .get("headers")
            .and_then(serde_json::Value::as_object)
            .expect("日志有 headers");
        map.iter()
            .map(|(name, value)| {
                (
                    name.to_ascii_lowercase(),
                    value.as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request() -> ProviderRequest {
    ProviderRequest {
        model: "mock-model".to_string(),
        context: Context::from_messages(
            Some("sys".to_string()),
            &[Message::User {
                content: pi_ai::UserContent::Text("hi".to_string()),
                timestamp: 0,
                extra: serde_json::Map::new(),
            }],
        ),
        max_completion_tokens: 64,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    }
}

/// 夹具：provider 级 `apiKey`/`headers` + 模型级 `headers`（值全是占位）。
fn fixture(base_url: &str, api: &str, extra_provider: &str, extra_model: &str) -> String {
    format!(
        "{{\"providers\":{{\"fixture\":{{\"api\":\"{api}\",\"baseUrl\":\"{base_url}\",\
         \"apiKey\":\"placeholder-key-abc\",\"headers\":{{\"x-fixture\":\"yes\"}}{extra_provider},\
         \"models\":[{{\"id\":\"mock-model\",\"headers\":{{\"x-model\":\"m\"}}{extra_model},\
         \"cost\":{{\"input\":1,\"output\":2,\"cacheRead\":0,\"cacheWrite\":0}}}}]}}}}}}"
    )
}

async fn drive(provider: &dyn pi_ai::Provider) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stream = loop {
        match provider.stream(request()).await {
            Ok(stream) => break stream,
            Err(error) => {
                assert!(Instant::now() < deadline, "等待 mock 就绪超时：{error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    while futures::StreamExt::next(&mut stream).await.is_some() {}
}

#[tokio::test]
async fn models_json_api_key_and_headers_reach_the_real_request() {
    let mock = MockProvider::start();
    let text = fixture(&mock.base_url(), "openai-completions", "", "");
    let config: ModelsConfig = serde_json::from_str(&text).expect("夹具可解析");
    let resolved = config
        .resolve("fixture", "mock-model")
        .expect("可解析出模型");
    // 样本自检：夹具里确实有 apiKey 与两级 headers（防止"配置根本没读到"式的空转）。
    assert_eq!(resolved.api_key.as_deref(), Some("placeholder-key-abc"));
    assert_eq!(
        resolved.headers,
        vec![
            ("x-fixture".to_string(), "yes".to_string()),
            ("x-model".to_string(), "m".to_string()),
        ],
        "provider + 模型级 headers 合并（模型级在后即覆盖）"
    );

    let provider = pi_cli::runtime::build_provider(&resolved, None).expect("构造成功");
    drive(provider.as_ref()).await;

    let headers = mock.first_request_headers();
    assert_eq!(
        headers.get("x-fixture").map(String::as_str),
        Some("yes"),
        "{headers:?}"
    );
    assert_eq!(
        headers.get("x-model").map(String::as_str),
        Some("m"),
        "{headers:?}"
    );
    // models.json 的 apiKey → 鉴权头（openai 侧默认 Bearer）。
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer placeholder-key-abc"),
        "{headers:?}"
    );
}

#[tokio::test]
async fn provider_without_configured_headers_sends_none_of_them() {
    // 负向对照：不配 `headers`/`apiKey` → 这些头一个都不该出现。
    let mock = MockProvider::start();
    let text = format!(
        "{{\"providers\":{{\"plain\":{{\"api\":\"openai-completions\",\"baseUrl\":\"{}\",\
         \"models\":[{{\"id\":\"mock-model\",\"cost\":{{\"input\":1,\"output\":2,\"cacheRead\":0,\"cacheWrite\":0}}}}]}}}}}}",
        mock.base_url()
    );
    let config: ModelsConfig = serde_json::from_str(&text).expect("夹具可解析");
    let resolved = config.resolve("plain", "mock-model").expect("可解析出模型");
    assert!(resolved.headers.is_empty(), "样本自检：没有配置头");
    assert!(resolved.api_key.is_none(), "样本自检：没有 apiKey");

    // F1-e 之后"无 key"会在构造阶段被拒绝（ref 行为），本用例测的是
    // "没有配置 `headers`"——所以给一个 key，并只断言**没有额外自定义头**
    // （authorization 是凭据机制必然产生的一行，不属于"配置头"）。
    let provider =
        pi_cli::runtime::build_provider(&resolved, Some("sk-test".to_string())).expect("构造成功");
    drive(provider.as_ref()).await;

    let headers = mock.first_request_headers();
    assert!(!headers.contains_key("x-fixture"), "{headers:?}");
    assert!(!headers.contains_key("x-model"), "{headers:?}");
}

#[tokio::test]
async fn auth_header_flag_switches_anthropic_to_bearer() {
    // ref `provider-composer.ts:304-307`：`authHeader: true` → `Authorization: Bearer <apiKey>`。
    // anthropic 默认是 `x-api-key`，所以这条能区分出该开关真的生效。
    let mock = MockProvider::start();
    let text = fixture(
        &mock.base_url(),
        "anthropic-messages",
        ",\"authHeader\":true",
        "",
    );
    let config: ModelsConfig = serde_json::from_str(&text).expect("夹具可解析");
    let resolved = config
        .resolve("fixture", "mock-model")
        .expect("可解析出模型");
    assert!(resolved.auth_header, "样本自检：夹具开了 authHeader");

    let provider = pi_cli::runtime::build_provider(&resolved, None).expect("构造成功");
    drive(provider.as_ref()).await;

    let headers = mock.first_request_headers();
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some("Bearer placeholder-key-abc"),
        "authHeader 必须发 Bearer：{headers:?}"
    );
}
