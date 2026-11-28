//! 深链树的构造**不得依赖递归**（真实会话上万条，会爆栈）。
//!
//! 实测：`build_tree` 递归实现在 2000 条链时正常（173ms），**3000 条直接
//! `has overflowed its stack` 并 abort 整个进程**。真实会话（几万条）走
//! `/tree`、`/fork`、rpc 的 `get_tree` 都会踩到。
//!
//! 本用例构造 2 万条线性链，要求：不崩、输出合法、节点数正确。

use pi_core::session_io::build_tree;

fn chain(n: usize) -> Vec<serde_json::Value> {
    let mut entries = vec![serde_json::json!({"type":"session","version":3,"id":"h"})];
    let mut prev = serde_json::Value::Null;
    for i in 0..n {
        entries.push(serde_json::json!({
            "type":"message",
            "id": format!("e{i:06}"),
            "parentId": prev,
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {"role":"user","content":[]}
        }));
        prev = serde_json::Value::String(format!("e{i:06}"));
    }
    entries
}

#[test]
fn deep_chain_does_not_overflow_the_stack() {
    let n = 20_000;
    let entries = chain(n);
    let started = std::time::Instant::now();
    let (tree, leaf) = build_tree(&entries);
    let elapsed = started.elapsed();
    let expected_leaf = format!("e{:06}", n - 1);
    assert_eq!(leaf.as_deref(), Some(expected_leaf.as_str()));
    // **不把整棵树喂给 `serde_json::from_str`**：解析 2 万层嵌套会撞上解析器
    // 自己的递归上限（实测 `recursion limit exceeded`），那是消费方约束、
    // 不是 `build_tree` 的问题。这里按文本结构核对。
    assert_eq!(tree.matches("{\"entry\":").count(), n, "节点数应等于条目数");
    assert_eq!(
        tree.matches("\"children\":[").count(),
        n,
        "每层应各有一个 children 数组"
    );
    // 前序写缓冲的实现是 O(输出大小)；递归版在 3000 层就 abort，
    // 自底向上拼串版 10000 层要 4s。这里给一个宽松上限：2s。
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "2 万层用了 {elapsed:?}（O(n²) 会到十秒级）"
    );
}
