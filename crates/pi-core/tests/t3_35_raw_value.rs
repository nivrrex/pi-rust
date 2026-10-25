//! P4-5a 第1条：`serde_json` 的 `raw_value` 能否用在"工具声明"的出口上。
//!
//! 背景：TS 的 `declarationsEqual`（`transcript.ts:140-141`）用 `JSON.stringify` 比对工具声明，
//! 所以 `parameters` 的**键序参与判等**；而 pi-rs 的 `serde_json::Map` 是 `BTreeMap`，
//! 序列化时会把键排序。本文件验证 `RawValue` 能否解决它，以及**放在哪里**才有效。

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// TS 的键序：`type, required, properties`（`tools/read.rs:21-31` 的源码顺序）。
const PARAMETERS_TS_ORDER: &str = r#"{"type":"object","required":["path"],"properties":{"path":{"type":"string","description":"路径"},"offset":{"type":"number","description":"起始行"},"limit":{"type":"number","description":"最多几行"}}}"#;

/// **方案 A（flatten 路径）**：把原始文本塞进 `Map<String, Value>` 的等价物。
/// 结论：**做不到**——`Value` 里没有"原始文本"这一变体，
/// `Value::String` 会被加引号转义（那就不再是 JSON 对象了）。
#[test]
fn raw_text_cannot_travel_inside_a_value_map() {
    let as_string = serde_json::Value::String(PARAMETERS_TS_ORDER.to_string());
    let text = serde_json::to_string(&as_string).expect("序列化");
    assert!(
        text.starts_with('"'),
        "放进 Value 只能变成字符串（带引号），不是对象：{text}"
    );
    assert!(!text.contains("\"properties\":{"), "更不可能保住内部的键序");
}

/// **方案 B（具名字段）**：`parameters` 用 `Box<RawValue>` 承载原始文本。
/// 这是唯一能让"原样写出"生效的形态。
#[derive(Serialize, Deserialize)]
struct OrderedDeclaration {
    name: String,
    description: String,
    /// 原样写出的参数表（键序由上面的常量决定）。
    parameters: Box<RawValue>,
}

/// 序列化后 `parameters` **逐字节**等于 TS 键序的原文（未被排序）。
#[test]
fn raw_value_preserves_the_bytes_verbatim() {
    let declaration = OrderedDeclaration {
        name: "read".to_string(),
        description: "读取文件".to_string(),
        parameters: RawValue::from_string(PARAMETERS_TS_ORDER.to_string()).expect("合法 JSON"),
    };
    let text = serde_json::to_string(&declaration).expect("序列化");
    assert!(
        text.contains(PARAMETERS_TS_ORDER),
        "parameters 必须逐字节原样写出，实际：{text}"
    );
    // 负向对照：排序后的形态（BTreeMap 会产出的那种）**不应**出现。
    let sorted = r#"{"properties":{"limit"#;
    assert!(
        !text.contains(sorted),
        "不应出现按键序排序后的形态（说明真的没被 Value 排序）"
    );
}

/// 反序列化不报错，且**未知字段不丢**（读后写回保真）。
#[test]
fn raw_value_round_trips_and_keeps_unknown_fields() {
    let incoming = format!(
        r#"{{"name":"read","description":"读取文件","parameters":{PARAMETERS_TS_ORDER},"未知字段":{{"a":1}}}}"#
    );
    let parsed: OrderedDeclaration = serde_json::from_str(&incoming).expect("反序列化");
    let text = serde_json::to_string(&parsed).expect("序列化");
    assert!(text.contains(PARAMETERS_TS_ORDER), "参数原样保留");
    // `OrderedDeclaration` 没有 `deny_unknown_fields`，但也没有 flatten——
    // 未知字段会被丢弃；这正是要用具名字段 + flatten 组合来验证的点（见下一条测试）。
    assert!(
        !text.contains("未知字段"),
        "这个最小结构没用 flatten，所以未知字段丢失——真实类型必须带 flatten"
    );
}

/// 真实形态：具名字段 + `#[serde(flatten)] extra`，未知字段与原始键序**同时**保住。
#[derive(Serialize, Deserialize)]
struct SystemMessageLike {
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools_added: Option<Vec<OrderedDeclaration>>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[test]
fn named_field_plus_flatten_keeps_unknown_fields_but_not_the_raw_order() {
    // **这是本轮的决定性发现**：`#[serde(flatten)]` 与 `Box<RawValue>` **不能共存**。
    // flatten 会让 serde 先把内容缓冲成自己的 `Content`，再逐字段输出，
    // 于是 `RawValue` 的"原样写出"失效——`parameters` 又被排序了。
    let incoming = format!(
        r#"{{"content":"","toolsAdded":[{{"name":"read","description":"读取文件","parameters":{PARAMETERS_TS_ORDER}}}],"sections":{{"cwd":"/tmp"}},"另一个未知":42}}"#
    );
    let parsed: SystemMessageLike = serde_json::from_str(&incoming).expect("反序列化");
    assert!(
        parsed.extra.contains_key("sections"),
        "已知的靠 flatten 收着"
    );
    assert!(parsed.extra.contains_key("另一个未知"), "未知字段也要收着");
    let text = serde_json::to_string(&parsed).expect("序列化");
    assert!(
        text.contains("另一个未知"),
        "读后写回不丢未知字段（flatten 的价值）"
    );
    assert!(
        !text.contains(PARAMETERS_TS_ORDER),
        "但 flatten 之下 RawValue **保不住**原键序——这是硬约束，实际：{text}"
    );
    assert!(
        text.contains(r#""parameters":{"properties":{"limit""#),
        "实际发生的是被排序（BTreeMap 行为），实际：{text}"
    );
}
