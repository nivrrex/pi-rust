//! P4-5d 第 5 件第一步（读树）：`build_tree` 的形状与 TS `getTree()` 对齐。
//!
//! 形状依据 golden/S28 的实测：节点 `{entry, children}`，`parentId` 为 null/缺省的是根，
//! `type == "session"` 的文件头**不是**树节点，`leafId` 是**最后一条**条目的 id。
use serde_json::json;

fn entries() -> Vec<serde_json::Value> {
    vec![
        json!({"type":"session","version":3,"id":"seed-1"}),
        json!({"type":"message","id":"m1","parentId":null,"timestamp":"t1","message":{"role":"user"}}),
        json!({"type":"message","id":"m2","parentId":"m1","timestamp":"t2","message":{"role":"assistant"}}),
        json!({"type":"message","id":"m3a","parentId":"m2","timestamp":"t3","message":{"role":"user"}}),
        json!({"type":"message","id":"m3b","parentId":"m2","timestamp":"t4","message":{"role":"user"}}),
        json!({"type":"thinking_level_change","id":"tl1","parentId":"m3b","timestamp":"t5"}),
    ]
}

#[test]
fn tree_matches_ts_shape_for_a_branching_session() {
    let (tree, leaf) = pi_core::session_io::build_tree(&entries());
    // 对照必须真的触发：种子里确实有两条同一 parent 的子条目（真分叉）。
    // 递归收 id（分叉在**第二层**，只收一层会漏）。
    fn ids_in(nodes: &[serde_json::Value]) -> Vec<String> {
        let mut out = Vec::new();
        for node in nodes {
            out.push(node["entry"]["id"].as_str().unwrap_or_default().to_string());
            let children: Vec<serde_json::Value> =
                node["children"].as_array().cloned().unwrap_or_default();
            out.extend(ids_in(&children));
        }
        out
    }
    let all_ids = ids_in(&tree);
    assert!(all_ids.contains(&"m3a".to_string()) && all_ids.contains(&"m3b".to_string()));

    // 根只有一个（m1）；文件头不出现。
    assert_eq!(tree.len(), 1, "树应有唯一根：{tree:#?}");
    assert_eq!(tree[0]["entry"]["id"], "m1");
    assert!(
        !serde_json::to_string(&tree)
            .unwrap_or_default()
            .contains("session"),
        "文件头不应成为树节点"
    );
    // m2 有两个孩子（真分叉）。
    let m2 = &tree[0]["children"][0];
    assert_eq!(m2["entry"]["id"], "m2");
    assert_eq!(m2["children"].as_array().map(Vec::len), Some(2), "{m2:#?}");
    // m3b 下挂着 thinking_level_change。
    let ids: Vec<&str> = m2["children"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| c["entry"]["id"].as_str().unwrap_or_default())
        .collect();
    assert!(ids.contains(&"m3b"));
    // leafId 是最后一条条目（不是最后一条 message）。
    assert_eq!(leaf.as_deref(), Some("tl1"), "leafId 应为最后一条条目");
}

#[test]
fn dangling_parent_becomes_a_root() {
    // 手工编辑过的文件：parent 指向不存在的条目 → 该条目当根，**不丢**。
    let (tree, leaf) = pi_core::session_io::build_tree(&[
        json!({"type":"message","id":"x","parentId":"missing"}),
        json!({"type":"message","id":"y","parentId":"x"}),
    ]);
    assert_eq!(leaf.as_deref(), Some("y"));
    assert_eq!(tree.len(), 1, "悬空父条目应成为根：{tree:#?}");
    assert_eq!(tree[0]["entry"]["id"], "x");
    assert_eq!(tree[0]["children"][0]["entry"]["id"], "y");
}

/// `get_fork_messages` 的规则（`agent-session.ts:3782-3796`）：
/// 只取 `message.role=="user"`、`contentText(content, "")` 非空的条目，**文件顺序**，
/// 且**不限分叉路径**（m3a/m3b 都出现）。
#[test]
fn fork_messages_follows_the_ts_rule() {
    let entries = vec![
        json!({"type":"session","id":"h"}),
        json!({"type":"message","id":"m1","message":{"role":"user","content":[{"type":"text","text":"第一个问题"}]}}),
        json!({"type":"message","id":"m2","message":{"role":"assistant","content":[{"type":"text","text":"回答"}]}}),
        json!({"type":"message","id":"m3a","message":{"role":"user","content":[{"type":"text","text":"分叉 A"}]}}),
        json!({"type":"message","id":"m3b","message":{"role":"user","content":[{"type":"text","text":"分叉 B"}]}}),
        // 空文本的 user 消息要**跳过**；字符串形式的内容要原样。
        json!({"type":"message","id":"m4","message":{"role":"user","content":[]}}),
        json!({"type":"message","id":"m5","message":{"role":"user","content":"字符串内容"}}),
    ];
    let got = pi_core::session_io::fork_messages(&entries);
    let pairs: Vec<(String, String)> = got
        .iter()
        .map(|m| {
            (
                m["entryId"].as_str().unwrap_or_default().to_string(),
                m["text"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("m1".to_string(), "第一个问题".to_string()),
            ("m3a".to_string(), "分叉 A".to_string()),
            ("m3b".to_string(), "分叉 B".to_string()),
            ("m5".to_string(), "字符串内容".to_string()),
        ],
        "assistant 与空文本都要排除：{got:#?}"
    );
}
