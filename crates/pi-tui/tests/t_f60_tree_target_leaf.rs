//! F60/14：`/tree` 的**新叶子选择规则**（ref `agent-session.ts:3713-3724`）。
//!
//! ```text
//! if (targetEntry.type === "message" && targetEntry.message.role === "user") {
//!   newLeafId = targetEntry.parentId;   // 回到这条消息**之前**
//!   editorText = contentText(...);      // 文本放回编辑器
//! } else {
//!   newLeafId = targetId;               // 其它条目：停在这一条
//! }
//! ```
//!
//! 我们此前一律用选中行自己的 id ⇒ 选中用户消息时叶子落在**该消息**上，
//! 于是它既留在上下文里、又被同时放回编辑器，重新发送就重复了一遍。
//!
//! 本用例把规则固化成纯函数（`pi_tui::tree_target_leaf`），供 pi-cli 调用，
//! 避免把这段判断散落在 UI 代码里。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::tree_target_leaf;

/// 选中**用户消息** ⇒ 新叶子是它的父节点（回到该消息之前）。
#[test]
fn user_message_targets_its_parent() {
    let (leaf, restore_text) = tree_target_leaf(true, "id-user", Some("id-parent"), "重来一次");
    assert_eq!(leaf.as_deref(), Some("id-parent"));
    assert_eq!(restore_text.as_deref(), Some("重来一次"));
}

/// 根上的用户消息 ⇒ 新叶子为 `None`（ref 允许 `newLeafId === null`）。
#[test]
fn root_user_message_targets_null() {
    let (leaf, restore_text) = tree_target_leaf(true, "id-user", None, "第一句");
    assert_eq!(leaf, None, "根上的用户消息应导航到根（None）");
    assert_eq!(restore_text.as_deref(), Some("第一句"));
}

/// 选中 assistant / tool 等非用户条目 ⇒ 新叶子是**目标本身**，不往编辑器塞文本。
#[test]
fn non_user_targets_itself_without_editor_text() {
    let (leaf, restore_text) = tree_target_leaf(false, "id-assistant", Some("id-user"), "回答");
    assert_eq!(leaf.as_deref(), Some("id-assistant"));
    assert_eq!(
        restore_text, None,
        "非用户条目不应把正文放回编辑器（ref 只给 user / custom_message 设 editorText）"
    );
}

/// **无摘要导航也必须切叶子**（ref `agent-session.ts:3739-3746`）：
/// `newLeafId === null` ⇒ `resetLeaf()`（空串表达），否则 `branch(newLeafId)`。
///
/// 我们此前"No summary"路径只清屏 + 回填文本，**会话链仍指向旧分支** ⇒
/// 下一条消息挂回旧分支，屏幕与实际会话不一致。
#[test]
fn leaf_moves_to_the_target_without_summary() {
    assert_eq!(
        pi_tui::tree_leaf_after_navigation("id-target").as_deref(),
        Some("id-target"),
        "无摘要导航应把叶子切到目标"
    );
    assert_eq!(
        pi_tui::tree_leaf_after_navigation(""),
        None,
        "空串（导航到根）应把叶子置为 None（ref 的 resetLeaf）"
    );
}
