//! P3.5：**流式期间**必须能取到累积快照。
//!
//! 回归背景：json 模式要逐增量发 `message_update`（`ai/src/types.ts:653-668` 每个增量事件都带
//! `partial`）。我这边一度在**整个流式过程**里 `final_message()` 都是 `None`——解码有三条分支，
//! 只有两条写了共享槽位，真实走的 `decoder.push(&chunk)` 那条没写。用真实 mock 逐个事件断言。

use std::process::{Child, Command, Stdio};

/// mock 守卫：离开作用域时 kill + wait（clippy 要求子进程被 wait）。
struct MockGuard(Child);

impl Drop for MockGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn every_streamed_event_has_a_current_snapshot() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("工作区根")
        .to_path_buf();
    let mut child = Command::new(root.join("target/debug/mock-provider"))
        .current_dir(&root)
        .arg("scenarios/S01.json")
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .spawn()
        .expect("mock 可启动");
    let stdout = child.stdout.take().expect("已取走 stdout");
    let guard = MockGuard(child);
    let port = {
        use std::io::BufRead;
        let mut line = String::new();
        std::io::BufReader::new(stdout)
            .read_line(&mut line)
            .expect("mock 会打印监听地址");
        line.trim()
            .strip_prefix("listening 127.0.0.1:")
            .expect("地址格式")
            .to_string()
    };

    let client = pi_ai::openai::ChatCompletionsClient::new(
        format!("http://127.0.0.1:{port}/v1"),
        pi_ai::decoder::Rates::zero(),
    );
    use pi_ai::Provider;
    let request = pi_ai::ProviderRequest {
        model: "mock-model".to_string(),
        context: pi_ai::Context::from_messages(
            None,
            &[pi_ai::Message::User {
                content: pi_ai::UserContent::Text("hi".to_string()),
                timestamp: 0,
                extra: serde_json::Map::new(),
            }],
        ),
        max_completion_tokens: 4096,
        tools: Vec::new(),
        thinking: None,
        headers: Vec::new(),
    };
    let mut stream = client.stream(request).await.expect("建流");

    let mut count = 0usize;
    while let Some(event) = stream.next_event().await {
        assert!(
            stream.final_message().expect("未中毒").is_some(),
            "第 {} 个事件（{event:?}）之后应能取到累积快照",
            count + 1
        );
        count += 1;
        if count > 12 {
            break;
        }
    }
    drop(guard);
    assert!(count >= 2, "至少要走几个事件（实际 {count}）");
}
