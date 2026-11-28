//! `OutputAccumulator` 与官方 npm 包的批量快照对拍。

use pi_core::tools::output_accumulator::OutputAccumulator;
use serde_json::Value;

fn workspace() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate 在工作区根下")
        .to_path_buf()
}

fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .expect("chunks 是数组")
        .iter()
        .map(|byte| u8::try_from(byte.as_u64().expect("byte")).expect("byte <= 255"))
        .collect()
}

#[test]
fn snapshots_match_the_official_accumulator() {
    let text = std::fs::read_to_string(workspace().join("fixtures/output-accumulator/cases.json"))
        .expect("oracle 夹具存在（先跑 output-accumulator-oracle.mjs）");
    let cases = serde_json::from_str::<Value>(&text)
        .expect("夹具是 JSON")
        .as_object()
        .expect("顶层是对象")
        .clone();

    for (name, case) in cases {
        let max_lines = case["maxLines"].as_u64().expect("maxLines") as usize;
        let max_bytes = case["maxBytes"].as_u64().expect("maxBytes") as usize;
        let mut output = OutputAccumulator::new(max_lines, max_bytes, "pi-output-oracle-rs");
        for chunk in case["chunks"].as_array().expect("chunks") {
            output.append(&bytes(chunk)).expect("append");
        }
        output.finish().expect("finish");
        let snapshot = output.snapshot(true).expect("snapshot");

        assert_eq!(
            snapshot.content,
            case["content"].as_str().expect("content"),
            "{name}: content"
        );
        assert_eq!(
            snapshot.last_line_bytes as u64,
            case["lastLineBytes"].as_u64().expect("lastLineBytes"),
            "{name}: lastLineBytes"
        );
        let mut actual_truncation = serde_json::to_value(&snapshot.truncation).expect("serialize");
        if actual_truncation.get("truncatedBy").is_none() {
            actual_truncation["truncatedBy"] = Value::Null;
        }
        assert_eq!(actual_truncation, case["truncation"], "{name}: truncation");

        match (&snapshot.full_output_path, case.get("fullOutput")) {
            (Some(path), Some(expected)) => assert_eq!(
                std::fs::read(path).expect("read full output"),
                bytes(expected),
                "{name}: fullOutput"
            ),
            (None, None) => {}
            (path, expected) => {
                panic!("{name}: fullOutput presence mismatch: {path:?}, {expected:?}")
            }
        }
        if let Some(path) = snapshot.full_output_path {
            let _ = std::fs::remove_file(path);
        }
    }
}
