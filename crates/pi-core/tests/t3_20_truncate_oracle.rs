//! P3.2：截断与格式化的**纯函数**与 TS 逐项一致。
//!
//! 期望由 `tools/oracle/truncate-oracle.mjs` 直接调用 npm 包里的
//! `truncateHead` / `truncateTail` / `formatSize` 生成（35 组用例）。

use pi_core::tools::bash_text::{format_size, truncate_tail};
use pi_core::tools::read_text::{truncate_head, TruncatedBy};
use serde_json::Value;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn cases() -> serde_json::Map<String, Value> {
    let text = std::fs::read_to_string(workspace().join("fixtures/truncate/cases.json"))
        .expect("oracle 夹具存在（先跑 tools/oracle/run-truncate.sh）");
    serde_json::from_str::<Value>(&text)
        .expect("夹具是 JSON")
        .as_object()
        .expect("顶层是对象")
        .clone()
}

fn truncated_by_name(value: Option<TruncatedBy>) -> Option<String> {
    value.map(|by| match by {
        TruncatedBy::Lines => "lines".to_string(),
        TruncatedBy::Bytes => "bytes".to_string(),
    })
}

#[test]
fn truncate_head_matches_typescript_on_every_case() {
    for (name, case) in cases() {
        let content = case["content"].as_str().expect("content 是字符串");
        let max_lines = case["maxLines"].as_u64().expect("maxLines") as usize;
        let max_bytes = case["maxBytes"].as_u64().expect("maxBytes") as usize;
        let expected = &case["head"];

        let actual = truncate_head(content, max_lines, max_bytes);
        assert_eq!(
            actual.content,
            expected["content"].as_str().expect("content"),
            "{name}: content"
        );
        assert_eq!(
            actual.truncated,
            expected["truncated"].as_bool().expect("truncated"),
            "{name}: truncated"
        );
        assert_eq!(
            truncated_by_name(actual.truncated_by).as_deref(),
            expected["truncatedBy"].as_str(),
            "{name}: truncatedBy"
        );
        assert_eq!(
            actual.output_lines as u64,
            expected["outputLines"].as_u64().expect("outputLines"),
            "{name}: outputLines"
        );
        assert_eq!(
            actual.first_line_exceeds_limit,
            expected["firstLineExceedsLimit"].as_bool().unwrap_or(false),
            "{name}: firstLineExceedsLimit"
        );
    }
}

#[test]
fn truncate_tail_matches_typescript_on_every_case() {
    for (name, case) in cases() {
        let content = case["content"].as_str().expect("content 是字符串");
        let max_lines = case["maxLines"].as_u64().expect("maxLines") as usize;
        let max_bytes = case["maxBytes"].as_u64().expect("maxBytes") as usize;
        let expected = &case["tail"];

        let actual = truncate_tail(content, max_lines, max_bytes);
        assert_eq!(
            actual.content,
            expected["content"].as_str().expect("content"),
            "{name}: content"
        );
        assert_eq!(
            actual.truncated,
            expected["truncated"].as_bool().expect("truncated"),
            "{name}: truncated"
        );
        assert_eq!(
            truncated_by_name(actual.truncated_by).as_deref(),
            expected["truncatedBy"].as_str(),
            "{name}: truncatedBy"
        );
        assert_eq!(
            actual.total_lines as u64,
            expected["totalLines"].as_u64().expect("totalLines"),
            "{name}: totalLines"
        );
        assert_eq!(
            actual.output_lines as u64,
            expected["outputLines"].as_u64().expect("outputLines"),
            "{name}: outputLines"
        );
        assert_eq!(
            actual.output_bytes as u64,
            expected["outputBytes"].as_u64().expect("outputBytes"),
            "{name}: outputBytes"
        );
        if let Some(want) = expected["lastLinePartial"].as_bool() {
            assert_eq!(actual.last_line_partial, want, "{name}: lastLinePartial");
        }
    }
}

#[test]
fn format_size_matches_the_oracle_samples() {
    // 由 oracle 的第 36 行输出实测：999=>999B、1024=>1.0KB、1536=>1.5KB、51200=>50.0KB
    assert_eq!(format_size(999), "999B");
    assert_eq!(format_size(1024), "1.0KB");
    assert_eq!(format_size(1536), "1.5KB");
    assert_eq!(format_size(51200), "50.0KB");
}
