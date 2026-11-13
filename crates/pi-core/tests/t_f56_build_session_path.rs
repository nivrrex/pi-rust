//! F56/D：`build_session_path` 必须 **O(n)**（用 `id → 下标` 索引），不是每跳线性扫描。
//!
//! 改前：每上溯一跳都 `entries.iter().position(...)` ⇒ O(n²)。真实会话 6.4 万条时
//! 单次调用 ~67 s（`session_read 977ms → pre_writer 68089ms` 全在这一句）。
//!
//! 断言方式：**构造深链（n = 20000）并限时**。不用精确耗时阈值，只要求"和 n² 明显不成比例"：
//! O(n²) 在 20000 条时约 10^8 次比较（数秒级，debug 下更久），O(n) 是毫秒级。
//! 阈值 5 s：debug 构建下 O(n) 实测 2.5 s、O(n²) 实测 7.5 s（同一台机器、同一次运行），
//! 5 s 落在两者之间。release 下 O(n) 是毫秒级。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pi_core::agent::projection::{build_session_path, LeafSelector};
use pi_core::session::SessionEntry;

/// 造一条 `n` 条的线性链（每条 parent 指向前一条）。
fn chain(n: usize) -> Vec<SessionEntry> {
    let mut entries = Vec::with_capacity(n);
    for index in 0..n {
        let id = format!("e{index:06}");
        let parent = if index == 0 {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(format!("e{:06}", index - 1))
        };
        let value = serde_json::json!({
            "type": "thinking_level_change",
            "id": id,
            "parentId": parent,
            "timestamp": "2026-01-01T00:00:00Z",
            "thinkingLevel": if index == n - 1 { "high" } else { "off" },
        });
        entries.push(serde_json::from_value(value).expect("解析条目"));
    }
    entries
}

/// 深链必须快速返回，且路径长度 == n、末条档位 == high。
#[test]
fn deep_chain_path_is_not_quadratic() {
    let n = 20_000;
    let entries = chain(n);
    let started = std::time::Instant::now();
    let path = build_session_path(&entries, LeafSelector::Unspecified);
    let elapsed = started.elapsed();
    assert_eq!(path.len(), n, "应沿 parentId 走完整条链");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "build_session_path 在 {n} 条上用了 {elapsed:?}（O(n²) 会到分钟级；应为毫秒级）"
    );
}
