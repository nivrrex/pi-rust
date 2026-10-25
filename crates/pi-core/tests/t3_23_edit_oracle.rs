//! edit 工具的 oracle 比对：输入与期望全部来自 TS 包**真实执行**的
//! `createEditToolDefinition`（`tools/oracle/edit-oracle.mjs` → `fixtures/edit/cases.json`）。
//!
//! 比 `t3_21_edit_golden` 强的地方：那三条是"TS 在 golden 场景里用过的参数"，覆盖的是
//! 正常路径；这里 40 组是**专门构造**的边界（重复、部分行覆盖、CRLF、NFKC、空文件…），
//! 且每组都断言改后文件内容。

use pi_core::tools::edit::EditTool;
use pi_core::tools::tool::{Tool, ToolContext};
use serde_json::{json, Value};
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("工作区根")
        .to_path_buf()
}

fn cases() -> Vec<Value> {
    let raw = std::fs::read_to_string(workspace_root().join("fixtures/edit/cases.json"))
        .expect("读 fixtures/edit/cases.json（先跑 tools/oracle/run-edit.sh）");
    serde_json::from_str(&raw).expect("cases.json 应是数组")
}

/// 跑一组用例，返回 `(isError, 文本, 改后文件内容)`。
async fn run_case(content: &str, edits: &Value) -> (bool, String, String) {
    let dir = tempfile::tempdir().expect("临时目录");
    let file = dir.path().join("target.txt");
    std::fs::write(&file, content).expect("写初始文件");
    let context = ToolContext {
        cwd: dir.path().to_path_buf(),
    };
    let result = EditTool
        .invoke(
            json!({"path": "target.txt", "edits": edits}),
            &context,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await;
    let after = std::fs::read_to_string(&file).unwrap_or_default();
    (result.is_error, result.text_content(), after)
}

/// 与 TS **已知**不一致的用例：`name` + 原因。
///
/// 列表是白名单也是**断言**：里面每一条都必须真的不一致（若哪天实现了 NFKC，这些
/// 用例会变成一致，测试就会失败并提示把它从表里删掉），表外的用例必须逐字节一致。
fn known_differences() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "nfkc_fullwidth_vs_halfwidth",
            "TS 第一步是 `text.normalize(\"NFKC\")`，全角 Ａ 会折成半角 A；我未实现 NFKC（需 unicode-normalization 依赖，待决策），因此找不到匹配而报错",
        ),
        (
            "nfkc_fullwidth_in_old_text",
            "同上：`oldText` 里的全角字符在 TS 侧也会先被 NFKC 折叠",
        ),
    ]
}

#[tokio::test]
async fn every_oracle_case_matches_ts() {
    let all = cases();
    assert!(
        all.len() >= 30,
        "oracle 用例应有 30 组以上，实际 {}",
        all.len()
    );
    let known = known_differences();
    let mut differences: Vec<String> = Vec::new();
    let mut unexpectedly_matching: Vec<&str> = Vec::new();
    for case in &all {
        let name = case["name"].as_str().unwrap_or("?");
        let is_known = known.iter().any(|(n, _)| *n == name);
        let content = case["content"].as_str().unwrap_or("");
        let edits = &case["edits"];
        let expect_error = case["isError"].as_bool().unwrap_or(true);
        let expect_after = case["fileAfter"].as_str();

        let (is_error, text, after) = run_case(content, edits).await;

        let mut differs = false;
        if is_error != expect_error {
            differs = true;
            if !is_known {
                differences.push(format!(
                    "{name}: isError 期望 {expect_error} 实际 {is_error}（我的文本：{text:?}）"
                ));
            }
        } else if let Some(expected) = expect_after {
            if after != expected {
                differs = true;
                if !is_known {
                    differences.push(format!("{name}: 改后内容期望 {expected:?} 实际 {after:?}"));
                }
            }
        }
        if is_known && !differs {
            unexpectedly_matching.push(name);
        }
    }
    assert!(
        unexpectedly_matching.is_empty(),
        "这些用例已在 known_differences 里，但实际与 TS 一致了，请从表里删掉：{unexpectedly_matching:?}"
    );
    assert!(
        differences.is_empty(),
        "与 TS 不一致的用例 {} 组（表外，必须全部一致）：\n{}",
        differences.len(),
        differences.join("\n")
    );
}
