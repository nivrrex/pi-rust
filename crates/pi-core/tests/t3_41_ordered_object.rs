//! P4-5e 第 1 件：`ordered_object` —— 按给定键序出 JSON 字符串。
//!
//! 存在的理由：`serde_json` 的对象是 `BTreeMap`（按键名排序），而 rpc 响应的键序
//! 是**判据**（TS 会读回并逐字节比较）。golden/S28 实测 `get_tree.data` 是
//! `{"tree":…,"leafId":…}`（**不是**字母序）。
use serde_json::json;

#[test]
fn keeps_the_given_key_order() {
    let tree = json!([{"entry": {"id": "m1"}}]);
    let leaf = json!("m3b");
    let got = pi_core::session_io::ordered_object(&[("tree", &tree), ("leafId", &leaf)]);
    assert_eq!(got, r#"{"tree":[{"entry":{"id":"m1"}}],"leafId":"m3b"}"#);
    // 对照必须真的触发：字母序会给出 leafId 在前，这不是我们想要的。
    assert!(
        !got.starts_with(r#"{"leafId""#),
        "必须按给定键序输出（字母序是错的）"
    );
}

#[test]
fn escapes_keys_and_values_like_json() {
    let value = json!("引号 \" 反斜杠 \\ 换行 \n");
    let got = pi_core::session_io::ordered_object(&[("键\"x", &value)]);
    // 用真正的 JSON 解析器复核一遍：解析回来后键与值都要原样。
    let parsed: serde_json::Value = serde_json::from_str(&got).expect("必须是合法 JSON");
    assert_eq!(parsed["键\"x"], value);
    assert_eq!(got.matches('{').count(), 1);
}

#[test]
fn empty_object_is_valid_json() {
    let got = pi_core::session_io::ordered_object(&[]);
    assert_eq!(got, "{}");
    assert!(serde_json::from_str::<serde_json::Value>(&got).is_ok());
}
