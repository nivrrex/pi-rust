//! F53d/D：行内词级比对与 jsdiff `diffWords` 的 oracle 逐字对拍。
//!
//! oracle 由 `tools/difftest/jsdiff-oracle/gen-intraline-oracle.mjs` 生成
//! （直接调用 TS 包依赖的 `diff@8.0.4`，并按 ref `components/diff.ts:33-77`
//! 的 `renderIntraLineDiff` 复刻），期望值存在 `expected.json`。
use pi_tui::intraline::render_intra_line_diff;

fn oracle() -> Vec<(String, String, String)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/difftest/jsdiff-oracle/expected.json");
    let text = std::fs::read_to_string(&path).expect("读 oracle");
    let value: serde_json::Value = serde_json::from_str(&text).expect("解析 oracle");
    value
        .as_array()
        .expect("数组")
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap_or_default().to_string(),
                entry["removed"].as_str().unwrap_or_default().to_string(),
                entry["added"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

fn cases() -> Vec<(String, String)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/difftest/jsdiff-oracle/cases.json");
    let text = std::fs::read_to_string(&path).expect("读 cases");
    let value: serde_json::Value = serde_json::from_str(&text).expect("解析 cases");
    value
        .as_array()
        .expect("数组")
        .iter()
        .map(|entry| {
            (
                entry["old"].as_str().unwrap_or_default().to_string(),
                entry["new"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

#[test]
fn matches_jsdiff_diffwords_oracle() {
    let expected = oracle();
    let inputs = cases();
    assert_eq!(expected.len(), inputs.len(), "oracle 与样例数量不一致");
    let mut failures = Vec::new();
    for ((name, want_removed, want_added), (old, new)) in expected.iter().zip(inputs.iter()) {
        let (removed, added) = render_intra_line_diff(old, new);
        if &removed != want_removed || &added != want_added {
            failures.push(format!(
                "{name}\n  old={old:?}\n  new={new:?}\n  期望 removed={want_removed:?} added={want_added:?}\n  实际 removed={removed:?} added={added:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} 个样例与 oracle 不一致：\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `render_diff_lines` 与 ref `renderDiff` 的 oracle 逐行比较。
#[test]
fn diff_lines_match_ref_rendering() {
    let here =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/difftest/jsdiff-oracle");
    let cases: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(here.join("diff-cases.json")).expect("读 diff-cases"),
    )
    .expect("解析 diff-cases");
    let expected: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(here.join("diff-expected.json")).expect("读 diff-expected"),
    )
    .expect("解析 diff-expected");
    let mut failures = Vec::new();
    for (case, want) in cases
        .as_array()
        .expect("数组")
        .iter()
        .zip(expected.as_array().expect("数组").iter())
    {
        let name = case["name"].as_str().unwrap_or_default();
        let diff = case["diff"].as_str().unwrap_or_default();
        let want_lines: Vec<String> = want["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect();
        let got = pi_tui::intraline::render_diff_lines(diff);
        if got != want_lines {
            failures.push(format!("{name}\n  期望 {want_lines:?}\n  实际 {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} 条 diff 渲染不一致：\n{}",
        failures.len(),
        failures.join("\n")
    );
}
