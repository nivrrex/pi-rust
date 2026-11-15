//! F61/16：`/scoped-models` 的纯逻辑（ref `scoped-models-selector.ts:24-90`）。
//!
//! 那套组件把"哪些模型进 Ctrl+P 循环"表达成 `EnabledIds = string[] | null`：
//! `null` = 全部启用（无过滤，未来新增模型自动进入），数组 = 显式有序列表
//! （**顺序即 Ctrl+P 的循环顺序**）。
//!
//! 这些是纯函数（开关/全开/全关/移动/归一化/排序/计数），逐条与 ref 对照：
//! `isEnabled` `:33-35`、`normalizeEnabled` `:38-40`、`toggle` `:42-47`、
//! `enableAll` `:49-57`、`clearAll` `:59-66`、`move` `:68-75`、`getSortedIds` `:77-81`。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_tui::scoped_models::{
    clear_all, enable_all, footer_counts, is_enabled, move_item, normalize_enabled, sorted_ids,
    toggle, EnabledIds,
};

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

/// `isEnabled`：`None` 时全部启用；`Some` 时按列表。
#[test]
fn is_enabled_follows_the_representation() {
    assert!(is_enabled(&None, "a/m1"), "None = 全部启用");
    let some: EnabledIds = Some(ids(&["a/m1"]));
    assert!(is_enabled(&some, "a/m1"));
    assert!(!is_enabled(&some, "a/m2"));
}

/// `normalizeEnabled`：显式列表覆盖**全部**可用模型 ⇒ 折叠成 `None`。
#[test]
fn explicit_full_list_collapses_to_none() {
    let all = ids(&["a/m1", "a/m2"]);
    assert_eq!(
        normalize_enabled(all.clone(), &all),
        None,
        "全选应折叠成 None（未来新增模型也能进循环）"
    );
    // 顺序不同也仍算"覆盖全部"（ref 用 `every(...includes)`，不比较顺序）。
    assert_eq!(normalize_enabled(ids(&["a/m2", "a/m1"]), &all), None);
    // 少一个 ⇒ 保持显式列表。
    assert_eq!(
        normalize_enabled(ids(&["a/m1"]), &all),
        Some(ids(&["a/m1"]))
    );
    // 多一个不存在的 ⇒ 不算覆盖全部。
    assert_eq!(
        normalize_enabled(ids(&["a/m1", "a/m2", "x/y"]), &all),
        Some(ids(&["a/m1", "a/m2", "x/y"]))
    );
}

/// `toggle`：三条分支（None→除它以外全启用；在列表→移出；不在→追加）。
#[test]
fn toggle_covers_all_three_branches() {
    let all = ids(&["a/m1", "a/m2", "a/m3"]);
    // None ⇒ 关掉一个（变成显式两项）
    assert_eq!(
        toggle(&None, &all, "a/m2"),
        Some(ids(&["a/m1", "a/m3"])),
        "全启用时关掉一个 ⇒ 显式列出其余"
    );
    // 在列表里 ⇒ 移出
    let some: EnabledIds = Some(ids(&["a/m1", "a/m2"]));
    assert_eq!(toggle(&some, &all, "a/m1"), Some(ids(&["a/m2"])));
    // 不在列表里 ⇒ 追加；若因此覆盖全部则折叠成 None
    let partial: EnabledIds = Some(ids(&["a/m1"]));
    assert_eq!(
        toggle(&partial, &all, "a/m2"),
        Some(ids(&["a/m1", "a/m2"])),
        "追加后仍未覆盖全部 ⇒ 保持显式列表"
    );
    let almost: EnabledIds = Some(ids(&["a/m1", "a/m2"]));
    assert_eq!(
        toggle(&almost, &all, "a/m3"),
        None,
        "追加后覆盖全部 ⇒ 折叠成 None"
    );
}

/// `enableAll`：全启用时是空操作；显式列表时补全（并归一化）。
#[test]
fn enable_all_is_a_noop_when_everything_is_enabled() {
    let all = ids(&["a/m1", "a/m2"]);
    assert_eq!(enable_all(&None, &all, None), None);
    let partial: EnabledIds = Some(ids(&["a/m1"]));
    assert_eq!(
        enable_all(&partial, &all, None),
        None,
        "补齐到全部 ⇒ 折叠成 None"
    );
    // 只启用指定子集
    let empty: EnabledIds = Some(Vec::new());
    assert_eq!(
        enable_all(&empty, &all, Some(&ids(&["a/m2"]))),
        Some(ids(&["a/m2"]))
    );
}

/// `clearAll`：`None` 且不带子集 ⇒ 全关；带子集 ⇒ "除该子集外全启用"。
#[test]
fn clear_all_distinguishes_subset_from_whole() {
    let all = ids(&["a/m1", "a/m2", "a/m3"]);
    assert_eq!(
        clear_all(&None, &all, None),
        Some(Vec::new()),
        "全关 ⇒ 空列表（不是 None）"
    );
    assert_eq!(
        clear_all(&None, &all, Some(&ids(&["a/m2"]))),
        Some(ids(&["a/m1", "a/m3"])),
        "全启用时只关 m2 ⇒ 其余显式列出"
    );
    let some: EnabledIds = Some(ids(&["a/m1", "a/m2"]));
    assert_eq!(
        clear_all(&some, &all, Some(&ids(&["a/m1"]))),
        Some(ids(&["a/m2"]))
    );
    // 不带子集：把列表里"可用"的项全部清掉（ref 用 `targets = enabledIds`）。
    assert_eq!(
        clear_all(&some, &all, None),
        Some(Vec::new()),
        "显式列表且不带子集 ⇒ 清空"
    );
}

/// `move`：交换相邻两项；`None` 时是空操作；越界不动。
#[test]
fn move_swaps_adjacent_items_only() {
    let some: EnabledIds = Some(ids(&["a/m1", "a/m2", "a/m3"]));
    assert_eq!(
        move_item(&some, "a/m2", -1),
        Some(ids(&["a/m2", "a/m1", "a/m3"])),
        "上移一位"
    );
    assert_eq!(
        move_item(&some, "a/m2", 1),
        Some(ids(&["a/m1", "a/m3", "a/m2"])),
        "下移一位"
    );
    assert_eq!(move_item(&some, "a/m1", -1), some, "已在首位 ⇒ 不动");
    assert_eq!(move_item(&some, "a/m3", 1), some, "已在末位 ⇒ 不动");
    assert_eq!(move_item(&some, "x/y", 1), some, "不存在的 id ⇒ 不动");
    assert_eq!(move_item(&None, "a/m1", 1), None, "None 没有显式顺序");
}

/// `getSortedIds`：已启用的按列表顺序在前，其余按原顺序在后。
#[test]
fn sorted_ids_put_enabled_first_in_list_order() {
    let all = ids(&["a/m1", "a/m2", "a/m3"]);
    assert_eq!(sorted_ids(&None, &all), all, "None ⇒ 原顺序");
    let some: EnabledIds = Some(ids(&["a/m3", "a/m1"]));
    assert_eq!(
        sorted_ids(&some, &all),
        ids(&["a/m3", "a/m1", "a/m2"]),
        "已启用的按列表顺序，未启用的按原顺序追加"
    );
}

/// 底部计数：`None` ⇒ 全部可用都是启用、无不可用；`Some` ⇒ 按可用集统计。
#[test]
fn footer_counts_report_enabled_and_unavailable() {
    let available = ids(&["a/m1", "a/m2"]);
    assert_eq!(footer_counts(&None, &available), (2, 0, true));
    let some: EnabledIds = Some(ids(&["a/m1", "gone/model"]));
    assert_eq!(
        footer_counts(&some, &available),
        (1, 1, false),
        "不在可用集里的启用项算 unavailable"
    );
}

/// **组合语义**（ref 组件的实际用法）：先全关、再逐个开，顺序即 Ctrl+P 的循环顺序。
#[test]
fn sequential_enable_builds_the_cycle_order() {
    let all = ids(&["a/m1", "a/m2", "a/m3"]);
    let mut enabled = clear_all(&None, &all, None);
    assert!(enabled.is_some(), "全关后应是显式列表（不是 None）");
    for id in ["a/m3", "a/m1"] {
        enabled = toggle(&enabled, &all, id);
    }
    assert_eq!(enabled, Some(ids(&["a/m3", "a/m1"])));
    assert_eq!(
        sorted_ids(&enabled, &all),
        ids(&["a/m3", "a/m1", "a/m2"]),
        "循环顺序 = 启用顺序，未启用的排在后面"
    );
}

/// 面板交互的**状态机**（不涉及渲染）：`ScopedModelsSelector` 的按键语义。
///
/// 放在这里是因为它不依赖 pi-cli 的运行时状态，只验证"按键 → 集合变化"与 ref
/// `scoped-models-selector.ts:299-365` 的对应关系。
#[test]
fn selector_key_semantics_match_ref() {
    use pi_tui::scoped_models::EnabledIds;
    // 用 pi-cli 的面板结构需要 pi-cli 依赖，这里直接用纯函数复现它的按键映射，
    // 确认"空格=toggle、ctrl+a=enableAll、ctrl+x=clearAll、alt+↑↓=move"的组合正确。
    let all = ids(&["a/m1", "a/m2", "a/m3"]);
    let mut enabled: EnabledIds = None;

    // 空格在 None 上 ⇒ 关掉当前项（第一项）
    enabled = toggle(&enabled, &all, "a/m1");
    assert_eq!(enabled, Some(ids(&["a/m2", "a/m3"])));
    // ctrl+x 全关
    enabled = clear_all(&enabled, &all, None);
    assert_eq!(enabled, Some(Vec::new()));
    // ctrl+a 全开 ⇒ 归一化成 None
    enabled = enable_all(&enabled, &all, None);
    assert_eq!(enabled, None);
    // 再关两项后用 alt+↓ 重排：顺序改变但集合不变
    enabled = toggle(&None, &all, "a/m1");
    enabled = toggle(&enabled, &all, "a/m3");
    assert_eq!(enabled, Some(ids(&["a/m2"])));
    let reordered = move_item(&enabled, "a/m2", 1);
    assert_eq!(reordered, Some(ids(&["a/m2"])), "只有一项时移动无效果");
}
