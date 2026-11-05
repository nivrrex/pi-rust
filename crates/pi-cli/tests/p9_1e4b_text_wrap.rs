//! S3：`Text` 帧的折行/内边距必须与 ref 真实 `Text` 类一致
//! （oracle 夹具由 `tools/difftest/tui-oracle/gen-text-wrap.mjs` 从 ref 生成，禁止手改）。

use serde_json::Value;

fn fixture() -> Value {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/difftest/tui-oracle/fixtures-text-wrap.json");
    serde_json::from_str(&std::fs::read_to_string(&path).expect("读 oracle 夹具")).expect("JSON")
}

#[test]
fn text_frames_match_the_ref_text_oracle() {
    let oracle = fixture();
    let cases = oracle["cases"].as_array().expect("cases");
    assert!(cases.len() >= 9, "夹具应至少 9 个用例");
    for case in cases {
        let name = case["name"].as_str().unwrap_or("?");
        let width = case["width"].as_u64().unwrap_or(80) as usize;
        let frame = serde_json::json!({
            "kind": "text",
            "text": case["text"],
            "paddingX": case["paddingX"],
            "paddingY": case["paddingY"],
        });
        let got = pi_cli::ext_tools::flatten_render_frame(&frame, width).expect("应可展开");
        let expected: Vec<String> = case["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .map(|line| line.as_str().unwrap_or_default().trim_end().to_string())
            .collect();
        let got: Vec<String> = got.iter().map(|line| line.trim_end().to_string()).collect();
        assert_eq!(got, expected, "用例 {name} @{width} 与 ref 不一致");
    }
}
