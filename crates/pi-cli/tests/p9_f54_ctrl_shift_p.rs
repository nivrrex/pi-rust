//! F54/Z4：`shift+ctrl+p` 是**后退**（ref `core/keybindings.ts:112-115`
//! `app.model.cycleBackward`，非 Windows 默认 `shift+ctrl+p`；
//! `interactive-mode.ts:2998-2999` 两个键位共用 `cycleModel(direction)`）。
//!
//! 改前：`ctrl+shift+p` 与 `ctrl+p` 同义（都前进）⇒ 反向只有"绕一圈"的效果。

use pi_cli::tui::{next_switchable_model_directed, ModelChoice, NextModel};

fn choices() -> Vec<ModelChoice> {
    ["a", "b", "c"]
        .iter()
        .map(|id| ModelChoice {
            id: (*id).to_string(),
            provider: "p".to_string(),
        })
        .collect()
}

/// 前进：a → b、b → c、c → a。
#[test]
fn cycle_forward_walks_in_declaration_order() {
    let list = choices();
    let pick = |from: &str| match next_switchable_model_directed(&list, from, "p", true, |choice| {
        Ok(choice.id.clone())
    }) {
        NextModel::Chosen(choice, _) => choice.id,
        other => panic!("期望切换成功，得到 {other:?}"),
    };
    assert_eq!(pick("a"), "b");
    assert_eq!(pick("b"), "c");
    assert_eq!(pick("c"), "a");
}

/// 后退（改前失败）：a → c、c → b、b → a。
///
/// 改前 `next_switchable_model` 只有前进分支 ⇒ `a` 的后一个是 `b`，用例断言 `c` 必然失败。
#[test]
fn cycle_backward_walks_in_reverse_order() {
    let list = choices();
    let pick =
        |from: &str| match next_switchable_model_directed(&list, from, "p", false, |choice| {
            Ok(choice.id.clone())
        }) {
            NextModel::Chosen(choice, _) => choice.id,
            other => panic!("期望切换成功，得到 {other:?}"),
        };
    assert_eq!(pick("a"), "c");
    assert_eq!(pick("c"), "b");
    assert_eq!(pick("b"), "a");
}

/// 两种情况都跳过"构建失败"的模型，并保持方向。
#[test]
fn backward_skips_unbuildable_models() {
    let list = choices();
    let next = next_switchable_model_directed(&list, "a", "p", false, |choice| {
        if choice.id == "c" {
            Err("boom".to_string())
        } else {
            Ok(choice.id.clone())
        }
    });
    match next {
        NextModel::Chosen(choice, _) => assert_eq!(choice.id, "b"),
        other => panic!("期望选中 b，得到 {other:?}"),
    }
}
