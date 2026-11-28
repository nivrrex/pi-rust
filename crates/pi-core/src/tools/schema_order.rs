//! 按**给定键序**把 JSON 渲染成文本（`serde_json` 的 `Map` 是 `BTreeMap`，序列化会排序）。
//!
//! 为什么需要它：TS 的 `declarationsEqual`（`ref/.../ai/src/utils/transcript.ts:140-141`）用
//! `JSON.stringify` 比对工具声明，而 `toToolDeclaration`（`:123-130`）对 `parameters`
//! 只做 JSON 往返、**保留原键序**——也就是说 **`parameters` 的键序参与判等**。
//! pi-rs 的 `serde_json::Map` 会把键排序，于是 TS 读我们的会话时会认为"工具声明变了"
//! （X02 的根因）。本模块把已经排好序的 `Value` **按 TS 的键序重新渲染**成文本。
//!
//! **单一来源**：渲染的输入就是各工具既有的 `schema()`（供校验器用），不另抄一份字符串常量。

use serde_json::Value;

/// 一条"某路径下的对象按这个键序输出"的规则。
///
/// - `path` 是从根开始的键序列（`&[]` 是根对象；`&["properties"]` 是 `properties` 里的每个属性对象）；
/// - `order` 是这些键的**输出顺序**；不在表里的键**按原顺序接在后面**（不丢字段）。
pub type OrderRule = (&'static [&'static str], &'static [&'static str]);

/// 通用键序：属性规范对象用得上（`type` 在 `description` 前）。
const GENERIC_ORDER: &[&str] = &["type", "description", "required", "properties", "items"];

/// 把 `value` 渲染成文本，对象按下述规则定序：
/// 先查 `rules` 里 `path` 完全匹配的规则；没有则用 [`GENERIC_ORDER`]（只对出现在其中的键）；
/// 仍未覆盖的键按**原有顺序**接在后面。
pub fn render_ordered(value: &Value, rules: &[OrderRule]) -> String {
    let mut out = String::new();
    render_into(value, &mut Vec::new(), rules, &mut out);
    out
}

fn render_into(value: &Value, path: &mut Vec<String>, rules: &[OrderRule], out: &mut String) {
    match value {
        Value::Object(map) => {
            let rule = rules.iter().find_map(|(keys, order)| {
                if keys.len() == path.len()
                    && keys
                        .iter()
                        .zip(path.iter())
                        .all(|(left, right)| left == right)
                {
                    Some(*order)
                } else {
                    None
                }
            });
            let order: Vec<&str> = match rule {
                Some(order) => order.to_vec(),
                None => GENERIC_ORDER.to_vec(),
            };
            out.push('{');
            let mut first = true;
            let mut emitted: Vec<&str> = Vec::new();
            for key in &order {
                if let Some(item) = map.get(*key) {
                    write_member(key, item, path, rules, out, &mut first);
                    emitted.push(key);
                }
            }
            // 规则没覆盖到的键：按原有顺序接上（保证不丢字段）。
            for (key, item) in map {
                if emitted.contains(&key.as_str()) {
                    continue;
                }
                write_member(key, item, path, rules, out, &mut first);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                render_into(item, path, rules, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_else(|_| "null".to_string())),
    }
}

fn write_member(
    key: &str,
    value: &Value,
    path: &mut Vec<String>,
    rules: &[OrderRule],
    out: &mut String,
    first: &mut bool,
) {
    if !*first {
        out.push(',');
    }
    *first = false;
    out.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()));
    out.push(':');
    path.push(key.to_string());
    render_into(value, path, rules, out);
    path.pop();
}

/// 各工具的键序表（照 `schema()` 的**源码顺序**抄键名；那正是 TS `toToolDeclaration` 的键序）。
///
/// 键序表按**工具名**索引，工具自己不用各自实现一遍渲染。
pub fn rules_for(tool_name: &str) -> &'static [OrderRule] {
    const READ: &[OrderRule] = &[
        (&[], &["type", "required", "properties"]),
        (&["properties"], &["path", "offset", "limit"]),
    ];
    const BASH: &[OrderRule] = &[
        (&[], &["type", "required", "properties"]),
        (&["properties"], &["command", "timeout"]),
    ];
    const EDIT: &[OrderRule] = &[
        (&[], &["type", "required", "properties"]),
        (&["properties"], &["path", "edits"]),
        // **`description` 在最后**：golden（S04 与 X02 都是）实测 `edits` 的键序是
        // `type, items, description`——我原先照源码字面量写成 `type, description, items`，
        // 是错的（源码字面量的顺序**不等于** TS `toToolDeclaration` 输出的顺序）。
        (&["properties", "edits"], &["type", "items", "description"]),
        (
            &["properties", "edits", "items"],
            &["type", "required", "properties"],
        ),
        (
            &["properties", "edits", "items", "properties"],
            &["oldText", "newText"],
        ),
    ];
    const WRITE: &[OrderRule] = &[
        (&[], &["type", "required", "properties"]),
        (&["properties"], &["path", "content"]),
    ];
    match tool_name {
        "read" => READ,
        "bash" => BASH,
        "edit" => EDIT,
        "write" => WRITE,
        // 未知工具：空表 → 退化成"通用键序 + 原有顺序"，不丢字段。
        // （不用 `_` 通配：`lint-banned.sh` 禁止 `_ =>`。）
        other => {
            let _ = other;
            &[]
        }
    }
}

/// 渲染一个工具的 `parameters` 文本（按该工具的键序表）。
pub fn render_schema_for(tool_name: &str, schema: &Value) -> String {
    render_ordered(schema, rules_for(tool_name))
}
