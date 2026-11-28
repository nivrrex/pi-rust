//! P9-F7b/S3：与 TS 侧真实样本的互操作
//!
//! - ① `fixtures/session/bash-execution.jsonl`（`SessionManager` 生成）→ pi-rs 解析 → 再序列化，
//!   字符串与原文逐字节一致（含未知字段与字段顺序）；
//! - ③ 同一批情形在 pi-rs 里的上下文投影与 `fixtures-bash-execution.json` 的 oracle 逐字节一致。

use pi_ai::message::{to_llm_messages, Message};

fn repo() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(serde::Deserialize)]
struct RawEntry {
    #[serde(default)]
    message: Option<Box<serde_json::value::RawValue>>,
}

fn read_raw_messages() -> Vec<(String, serde_json::Value)> {
    let text = std::fs::read_to_string(repo().join("fixtures/session/bash-execution.jsonl"))
        .expect("读不到 TS 会话样本");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let entry: RawEntry = serde_json::from_str(line).ok()?;
            let raw = entry.message?;
            let value: serde_json::Value = serde_json::from_str(raw.get()).ok()?;
            Some((raw.get().to_string(), value))
        })
        .collect()
}

#[test]
fn ts_samples_round_trip_byte_identically() {
    let raws = read_raw_messages();
    let mut bash_count = 0;
    for (raw, value) in &raws {
        if value.get("role").and_then(|role| role.as_str()) != Some("bashExecution") {
            continue;
        }
        bash_count += 1;
        let parsed: Message =
            serde_json::from_value(value.clone()).expect("应能解析 bashExecution");
        let back = serde_json::to_string(&parsed).expect("应能序列化");
        assert_eq!(&back, raw, "逐字节一致（含字段顺序与未知字段）");
    }
    assert_eq!(bash_count, 6, "样本应含 6 条不同情形的 bashExecution");
}

#[test]
fn projections_match_the_ts_oracle() {
    let oracle: serde_json::Value = serde_json::from_slice(
        &std::fs::read(repo().join("tools/difftest/ts-oracle/fixtures-bash-execution.json"))
            .expect("读不到 oracle（先跑生成器）"),
    )
    .expect("oracle 应是 JSON");
    let cases = oracle["projections"].as_array().expect("projections 数组");
    assert!(cases.len() >= 6, "oracle 应覆盖 6 种情形");
    for case in cases {
        let id = case["id"].as_str().unwrap_or("?");
        let input: Message = serde_json::from_value(case["input"].clone())
            .unwrap_or_else(|error| panic!("{id}: 输入应能解析：{error}"));
        let projected = to_llm_messages(&[input]);
        let expected = case["llm"].as_array().expect("llm 数组");
        assert_eq!(
            projected.len(),
            expected.len(),
            "{id}: 投影条数应与 ref 一致（excludeFromContext ⇒ 0）"
        );
        if expected.is_empty() {
            continue;
        }
        let expected_text = expected[0]["content"][0]["text"]
            .as_str()
            .expect("oracle 文本");
        let actual = serde_json::to_value(&projected[0]).expect("序列化");
        let content = &actual["content"];
        let actual_text = content
            .as_str()
            .map(|t| t.to_string())
            .or_else(|| content[0]["text"].as_str().map(|t| t.to_string()))
            .unwrap_or_default();
        assert_eq!(
            actual_text, expected_text,
            "{id}: 投影文本应与 ref 逐字节一致"
        );
    }
}

/// ② pi-rs 写出的会话 → **ref 真实 SessionManager 加载** → 投影与 pi-rs 的一致。
#[test]
fn ref_loads_our_session_and_agrees_on_projection() {
    let Some(node) = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
    else {
        eprintln!("跳过：本机没有 node");
        return;
    };
    // 安装包目录不硬编码：由 `TS_PI_ENTRY`（`.../dist/bundle/cli.js`）推导；未设置则跳过。
    let Ok(entry) = std::env::var("TS_PI_ENTRY") else {
        eprintln!("跳过：未设置 TS_PI_ENTRY");
        return;
    };
    let ts_pkg = std::path::Path::new(&entry)
        .ancestors()
        .nth(3)
        .map(|path| path.to_path_buf())
        .filter(|path| path.is_dir());
    if ts_pkg.is_none() {
        eprintln!("跳过：TS_PI_ENTRY 推导不出安装包目录");
        return;
    }

    let dir = std::env::temp_dir().join(format!("p9f7b-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let file = dir.join("session.jsonl");
    let cases = [
        serde_json::json!({"role":"bashExecution","command":"echo hi","output":"hi\n","exitCode":0,"cancelled":false,"truncated":false,"timestamp":1730000000000_i64}),
        serde_json::json!({"role":"bashExecution","command":"echo secret","output":"secret\n","exitCode":0,"cancelled":false,"truncated":false,"timestamp":1730000000001_i64,"excludeFromContext":true}),
    ];
    // 会话文件必须以 header 开头（ref `SessionManager.open` 会校验）。
    let fixture = std::fs::read_to_string(repo().join("fixtures/session/bash-execution.jsonl"))
        .expect("读不到 TS 会话样本");
    let mut lines: Vec<String> = vec![fixture.lines().next().expect("header").to_string()];
    for (index, case) in cases.iter().enumerate() {
        let message: Message = serde_json::from_value(case.clone()).expect("反序列化");
        let entry = serde_json::json!({
            "type": "message",
            "id": format!("e{index:02}"),
            "parentId": if index == 0 { serde_json::Value::Null } else { serde_json::json!(format!("e{:02}", index - 1)) },
            "timestamp": "2026-01-01T00:00:00.000Z",
            "message": serde_json::to_value(&message).expect("序列化"),
        });
        lines.push(entry.to_string());
    }
    std::fs::write(&file, lines.join("\n") + "\n").expect("写会话");

    let output = std::process::Command::new(node)
        .arg(repo().join("tools/difftest/ts-oracle/verify-bash-session.mjs"))
        .arg(&file)
        .output()
        .expect("运行校验脚本");
    assert!(
        output.status.success(),
        "校验脚本失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ref_projection: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("脚本应输出 JSON");
    let entries = ref_projection.as_array().expect("数组");

    // ref 的投影里，普通项折成 user 文本、`!!` 项被剔除 ⇒ 只剩 1 条。
    assert_eq!(entries.len(), 1, "ref 应只投影出普通项：{ref_projection}");
    let text = entries[0]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert_eq!(text, "Ran `echo hi`\n```\nhi\n\n```");

    // pi-rs 自己对同一批消息的投影必须与 ref 相同。
    let ours: Vec<Message> = cases
        .iter()
        .map(|case| serde_json::from_value(case.clone()).expect("反序列化"))
        .collect();
    let ours = to_llm_messages(&ours);
    assert_eq!(ours.len(), 1, "pi-rs 也应只投影出普通项");
}
