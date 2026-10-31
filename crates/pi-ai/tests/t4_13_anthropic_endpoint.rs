//! P4-14 步骤 3：anthropic 端点路径按"baseUrl 原样交给 SDK"的约定拼。
//!
//! ref 依据：
//! - `@anthropic-ai/sdk/src/client.ts:986`
//!   `new URL(baseURL + (baseURL.endsWith('/') && path.startsWith('/') ? path.slice(1) : path))`
//!   —— 就是**字符串拼接**，只处理"baseURL 以 / 结尾 + path 以 / 开头"时的一个 `/`；
//! - `@anthropic-ai/sdk/src/client.ts:632` 默认 `baseURL: baseURL || \`https://api.anthropic.com\``；
//! - `@anthropic-ai/sdk/src/resources/beta/messages/messages.ts:122`
//!   `this._client.post('/v1/messages?beta=true', {…})`（pi 走的是 beta 端点，故有 `?beta=true`）；
//! - `packages/ai/src/api/anthropic-messages.ts:923/945/982` 把 `baseURL: model.baseUrl` **原样**交给 SDK。
//!
//! 于是最终 URL = `baseUrl` + `/v1/messages?beta=true`（含尾斜杠时不多出一条 `/`）。
//! golden/S12-anthropic 的请求路径是 `/v1/v1/messages?beta=true`（runner 把 baseUrl 写成
//! `http://127.0.0.1:<port>/v1`，TS 再拼 `/v1/messages`）——本测试直接把该事实钉住。
use pi_ai::anthropic::messages_endpoint;

#[test]
fn base_url_without_v1_gets_the_sdk_suffix() {
    // 真实 Anthropic 配置的形态（models.json 里 baseUrl 是 origin，不含 /v1）。
    assert_eq!(
        messages_endpoint("https://api.anthropic.com"),
        "https://api.anthropic.com/v1/messages?beta=true"
    );
}

#[test]
fn base_url_with_v1_keeps_it_and_appends_another_v1() {
    // 差分夹具的形态（runner 的 baseUrl 已经带 /v1）→ 与 golden 的路径一致。
    assert_eq!(
        messages_endpoint("http://127.0.0.1:1234/v1"),
        "http://127.0.0.1:1234/v1/v1/messages?beta=true"
    );
}

#[test]
fn base_url_with_trailing_slash_does_not_double_the_slash() {
    // ref `client.ts:986` 的那个分支：baseURL 以 `/` 结尾时**去掉 path 的首个 `/`**，
    // 所以尾斜杠不会拼出 `//`（两种写法结果必须相同）。
    assert_eq!(
        messages_endpoint("http://127.0.0.1:1234/v1/"),
        messages_endpoint("http://127.0.0.1:1234/v1")
    );
    assert_eq!(
        messages_endpoint("https://api.anthropic.com/"),
        "https://api.anthropic.com/v1/messages?beta=true"
    );
}

#[test]
fn matches_the_recorded_golden_path() {
    // 样本自检 + 事实钉死：golden 记录的 path 必须等于"夹具 baseUrl 拼出来的 URL"的 path 部分。
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .join("golden/S12-anthropic/requests.jsonl");
    let text = std::fs::read_to_string(&golden).expect("golden 请求存在");
    let first: serde_json::Value =
        serde_json::from_str(text.lines().next().expect("至少一条")).expect("请求行是 JSON");
    let recorded = first.get("path").and_then(|v| v.as_str()).expect("有 path");
    let mine = messages_endpoint("http://127.0.0.1:1234/v1");
    let path_of_mine = mine
        .strip_prefix("http://127.0.0.1:1234")
        .expect("前缀是夹具 host");
    assert_eq!(path_of_mine, recorded, "拼出来的 path 必须与 golden 一致");
}
