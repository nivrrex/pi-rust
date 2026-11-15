//! `/scoped-models` 的**纯逻辑**（ref `components/scoped-models-selector.ts:24-90`）。
//!
//! 那个组件把"哪些模型能进 Ctrl+P 循环"表达成 `EnabledIds = string[] | null`：
//! `null` = **全部启用**（无过滤），数组 = 显式列表（**顺序参与 Ctrl+P 的循环顺序**）。
//!
//! 这里只实现纯函数部分（开关、全开/全关、移动、归一化），不碰 UI：
//! 每个函数都能用表驱动用例覆盖，面板只负责按键与渲染。
//!
//! 逐条对照 ref：
//! - `isEnabled`（`:33-35`）
//! - `normalizeEnabled`（`:38-40`）：显式列表覆盖了**全部**可用模型时折叠回 `null`
//! - `toggle`（`:42-47`）
//! - `enableAll`（`:49-57`）
//! - `clearAll`（`:59-66`）
//! - `move`（`:68-75`）：在显式列表里交换相邻两项（`null` 时是空操作）
//! - `getSortedIds`（`:77-81`）：已启用的按**列表顺序**在前，其余按原顺序在后

/// 启用集合：`None` = 全部启用（无过滤）；`Some(list)` = 显式有序列表。
pub type EnabledIds = Option<Vec<String>>;

/// 某个模型是否启用（`None` 时全部算启用）。
#[must_use]
pub fn is_enabled(enabled: &EnabledIds, id: &str) -> bool {
    match enabled {
        None => true,
        Some(list) => list.iter().any(|item| item == id),
    }
}

/// 显式列表覆盖**全部**可用模型时折叠成 `None`（ref `:38-40`）。
///
/// 折叠的意义：`None` 与"全选"在行为上等价，但 `None` 让**未来新增的模型**
/// 也自动进入循环（这正是没有配置过滤时的语义）。
#[must_use]
pub fn normalize_enabled(result: Vec<String>, all_ids: &[String]) -> EnabledIds {
    let covers_all = result.len() == all_ids.len()
        && result
            .iter()
            .all(|id| all_ids.iter().any(|other| other == id));
    if covers_all {
        None
    } else {
        Some(result)
    }
}

/// 切换单个模型（ref `:42-47`）。
///
/// - `None`（全启用）⇒ 变成"除它以外全启用"；
/// - 在列表里 ⇒ 移出列表；
/// - 不在列表里 ⇒ 追加。
///
/// 结果仍覆盖全部时折叠成 `None`（`normalizeEnabled`）。
#[must_use]
pub fn toggle(enabled: &EnabledIds, all_ids: &[String], id: &str) -> EnabledIds {
    match enabled {
        None => {
            let rest: Vec<String> = all_ids
                .iter()
                .filter(|model| model.as_str() != id)
                .cloned()
                .collect();
            normalize_enabled(rest, all_ids)
        }
        Some(list) => {
            if let Some(index) = list.iter().position(|item| item == id) {
                let mut next = list.clone();
                next.remove(index);
                normalize_enabled(next, all_ids)
            } else {
                let mut next = list.clone();
                next.push(id.to_string());
                normalize_enabled(next, all_ids)
            }
        }
    }
}

/// 启用（全部或指定子集，ref `:49-57`）。已全启用时是空操作。
#[must_use]
pub fn enable_all(
    enabled: &EnabledIds,
    all_ids: &[String],
    target_ids: Option<&[String]>,
) -> EnabledIds {
    let Some(list) = enabled else {
        return None;
    };
    let targets: Vec<&String> = match target_ids {
        Some(targets) => targets.iter().collect(),
        None => all_ids.iter().collect(),
    };
    let mut result = list.clone();
    for id in targets {
        if !result.iter().any(|item| item == id) {
            result.push(id.clone());
        }
    }
    normalize_enabled(result, all_ids)
}

/// 停用（全部或指定子集，ref `:59-66`）。
///
/// `None` 且未指定子集 ⇒ `Some(空列表)`（全关）；`None` 且指定子集 ⇒
/// "除该子集外全启用"。
#[must_use]
pub fn clear_all(
    enabled: &EnabledIds,
    all_ids: &[String],
    target_ids: Option<&[String]>,
) -> EnabledIds {
    match (enabled, target_ids) {
        (None, None) => Some(Vec::new()),
        (None, Some(targets)) => {
            let rest: Vec<String> = all_ids
                .iter()
                .filter(|id| !targets.iter().any(|target| target == *id))
                .cloned()
                .collect();
            Some(rest)
        }
        (Some(list), Some(targets)) => Some(
            list.iter()
                .filter(|id| !targets.iter().any(|target| target == *id))
                .cloned()
                .collect(),
        ),
        (Some(list), None) => Some(
            list.iter()
                .filter(|id| !all_ids.iter().any(|other| other == *id))
                .cloned()
                .collect(),
        ),
    }
}

/// 在显式列表里把某项移动 `delta` 位（ref `:68-75`）。
///
/// `None`（全启用）时是空操作——没有显式顺序可交换。
#[must_use]
pub fn move_item(enabled: &EnabledIds, id: &str, delta: isize) -> EnabledIds {
    let Some(list) = enabled else {
        return None;
    };
    let Some(index) = list.iter().position(|item| item == id) else {
        return Some(list.clone());
    };
    let new_index = index as isize + delta;
    if new_index < 0 || new_index as usize >= list.len() {
        return Some(list.clone());
    }
    let mut result = list.clone();
    result.swap(index, new_index as usize);
    Some(result)
}

/// 面板的显示顺序（ref `:77-81`）：已启用的按**列表顺序**在前，其余按原顺序在后。
///
/// 顺序有意义：Ctrl+P 按这个顺序循环（ref 的 `scopedModels` 就是有序的）。
#[must_use]
pub fn sorted_ids(enabled: &EnabledIds, all_ids: &[String]) -> Vec<String> {
    match enabled {
        None => all_ids.to_vec(),
        Some(list) => {
            let mut out = list.clone();
            for id in all_ids {
                if !list.iter().any(|item| item == id) {
                    out.push(id.clone());
                }
            }
            out
        }
    }
}

/// 底部计数文案的数据源（ref `:194-220` 的 `getFooterText` 前半段）：
/// `(enabled_count, unavailable_count, all_enabled)`。
#[must_use]
pub fn footer_counts(enabled: &EnabledIds, available_ids: &[String]) -> (usize, usize, bool) {
    match enabled {
        None => (available_ids.len(), 0, true),
        Some(list) => {
            let enabled_count = list
                .iter()
                .filter(|id| available_ids.iter().any(|other| other == *id))
                .count();
            let unavailable = list.len() - enabled_count;
            (enabled_count, unavailable, false)
        }
    }
}
