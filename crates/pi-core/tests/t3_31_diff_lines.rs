//! `diffLines` 与 jsdiff 的逐项比对（期望来自真实 `diff@8.0.4`）。
//!
//! 移植中最关键的一处修复：主循环对角线步长是 **2**（`base.js:73`）——见 `diff.rs` 的模块注释。

use pi_core::tools::diff::{create_two_files_patch, diff_lines, generate_diff_string, Change};
use serde_json::Value;

fn workspace() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("工作区根")
        .to_path_buf()
}

fn expected_change(value: &Value) -> Change {
    Change {
        value: value["value"].as_str().unwrap_or_default().to_string(),
        count: value["count"].as_u64().unwrap_or_default() as usize,
        added: value["added"].as_bool().unwrap_or(false),
        removed: value["removed"].as_bool().unwrap_or(false),
    }
}

#[test]
fn diff_lines_matches_jsdiff_on_every_oracle_case() {
    let raw = std::fs::read_to_string(workspace().join("fixtures/diff/cases.json"))
        .expect("先跑 tools/oracle/diff-oracle.mjs");
    let parsed: Value = serde_json::from_str(&raw).expect("cases.json 是 JSON");
    let records = parsed["records"].as_array().expect("有 records");

    let mut problems: Vec<String> = Vec::new();
    let mut checked = 0;
    for record in records {
        if record["kind"] != "diffLines" {
            continue;
        }
        let name = record["name"].as_str().unwrap_or("?");
        let old = record["oldText"].as_str().unwrap_or_default();
        let new = record["newText"].as_str().unwrap_or_default();
        let want: Vec<Change> = record["changes"]
            .as_array()
            .expect("有 changes")
            .iter()
            .map(expected_change)
            .collect();
        let got = diff_lines(old, new);
        checked += 1;
        if got != want {
            problems.push(format!(
                "{name}:\n   jsdiff: {:?}\n   rust  : {:?}",
                want.iter()
                    .map(|c| (&c.value, c.count, c.added, c.removed))
                    .collect::<Vec<_>>(),
                got.iter()
                    .map(|c| (&c.value, c.count, c.added, c.removed))
                    .collect::<Vec<_>>()
            ));
        }
    }
    assert!(
        checked >= 30,
        "oracle 的 diffLines 用例应不少于 30 组，实际 {checked}"
    );
    assert!(
        problems.is_empty(),
        "与 jsdiff 不一致 {} / {checked} 组：\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn create_two_files_patch_matches_jsdiff_on_every_oracle_case() {
    let raw = std::fs::read_to_string(workspace().join("fixtures/diff/cases.json"))
        .expect("先跑 tools/oracle/diff-oracle.mjs");
    let parsed: Value = serde_json::from_str(&raw).expect("cases.json 是 JSON");
    let records = parsed["records"].as_array().expect("有 records");

    let mut problems: Vec<String> = Vec::new();
    let mut checked = 0;
    for record in records {
        if record["kind"] != "createTwoFilesPatch" {
            continue;
        }
        let name = record["name"].as_str().unwrap_or("?");
        let old = record["oldText"].as_str().unwrap_or_default();
        let new = record["newText"].as_str().unwrap_or_default();
        let want = record["patch"].as_str().unwrap_or_default();
        // `edit-diff.ts:368` 用的是 `context: 4` + `FILE_HEADERS_ONLY`。
        let got = create_two_files_patch("f.txt", old, new, Some(4));
        checked += 1;
        if got != want {
            problems.push(format!("{name}:\n   jsdiff: {want:?}\n   rust  : {got:?}"));
        }
    }
    assert!(
        checked >= 10,
        "oracle 的 patch 用例应不少于 10 组，实际 {checked}"
    );
    assert!(
        problems.is_empty(),
        "与 jsdiff 不一致 {} / {checked} 组：\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn generate_diff_string_matches_edit_diff_oracle() {
    let raw = std::fs::read_to_string(workspace().join("fixtures/diff/display-cases.json"))
        .expect("先跑 tools/oracle/diff-display-oracle.mjs");
    let parsed: Value = serde_json::from_str(&raw).expect("display-cases.json 是 JSON");
    let records = parsed["records"].as_array().expect("有 records");
    assert!(records.len() >= 15, "显示 diff 的用例应不少于 15 组");

    let mut problems: Vec<String> = Vec::new();
    for record in records {
        let name = record["name"].as_str().unwrap_or("?");
        let old = record["oldText"].as_str().unwrap_or_default();
        let new = record["newText"].as_str().unwrap_or_default();
        let want_diff = record["diff"].as_str().unwrap_or_default();
        let want_first = record["firstChangedLine"].as_i64();
        let got = generate_diff_string(old, new, 4);
        if got.diff != want_diff {
            problems.push(format!(
                "{name}: diff 不同\n   ts  : {want_diff:?}\n   rust: {:?}",
                got.diff
            ));
        }
        if got.first_changed_line != want_first {
            problems.push(format!(
                "{name}: firstChangedLine 期望 {want_first:?} 实际 {:?}",
                got.first_changed_line
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "与 generateDiffString 不一致 {} 处：\n{}",
        problems.len(),
        problems.join("\n")
    );
}
