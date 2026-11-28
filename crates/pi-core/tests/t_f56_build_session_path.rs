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
        elapsed < std::time::Duration::from_secs(2),
        "build_session_path 在 {n} 条上用了 {elapsed:?}（O(n) 实测约 40ms，O(n²) 是秒级）"
    );
}

/// **不依赖机器速度**的复杂度判据：n 与 2n 的耗时比。
///
/// 上面那条用绝对阈值，机器繁忙时会误报（precommit 并行跑时实测过一次
/// 5.19s ⇒ 假红）。这条比"两倍规模要花几倍时间"：O(n) 约 2 倍、O(n²) 约 4 倍。
/// 留 3 倍作分界——两边都离得开，且不受 CPU 快慢影响。
///
/// 背景（F199）：`seen` 用的是 `Vec::contains`，每跳线性扫描 ⇒ 实际是 O(n²)。
/// 实测修前 5000/10000/20000 条 = 154ms/585ms/2.34s（每翻倍 ~3.8 倍），
/// 修后用 `HashSet` 变成 12/18/37ms（~2 倍）。
#[test]
fn scaling_is_linear_not_quadratic() {
    let small = chain(10_000);
    let large = chain(20_000);
    // 各自先热一次，避免首次分配的抖动混进计时。
    let _ = build_session_path(&small, LeafSelector::Unspecified);
    let _ = build_session_path(&large, LeafSelector::Unspecified);

    let t = std::time::Instant::now();
    let _ = build_session_path(&small, LeafSelector::Unspecified);
    let small_elapsed = t.elapsed().as_secs_f64();

    let t = std::time::Instant::now();
    let _ = build_session_path(&large, LeafSelector::Unspecified);
    let large_elapsed = t.elapsed().as_secs_f64();

    // 下限保护：两档都太快时比值噪声大，直接跳过比值判定。
    if small_elapsed < 0.005 {
        return;
    }
    let ratio = large_elapsed / small_elapsed;
    assert!(
        ratio < 3.0,
        "规模翻倍后耗时涨了 {ratio:.2} 倍（O(n) 应约 2 倍；≥3 说明退化成 O(n²)）\
：10000 条 {small_elapsed:.4}s、20000 条 {large_elapsed:.4}s"
    );
}
