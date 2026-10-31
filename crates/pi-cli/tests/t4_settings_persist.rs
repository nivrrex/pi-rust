//! P7-7c2b：`/settings` 改值写回 `settings.json`（读-改-写，保留未知键）。

use pi_cli::settings::{
    json_key_for, json_value_for, persist_change, persist_nested_setting, persist_setting,
    SettingKind,
};

fn tempdir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "pi-rs-settings-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("建临时目录");
    base
}

/// 未知键必须原样保留（ref 的读-改-写语义；用户配置里有很多本实现不认识的项）。
#[test]
fn persist_preserves_unknown_keys() {
    let dir = tempdir();
    let path = dir.join("settings.json");
    std::fs::write(
        &path,
        r#"{"theme":"dark","nested":{"a":1},"autoCompact":true}"#,
    )
    .expect("写初始 settings");

    persist_setting(&dir, "autoCompact", serde_json::Value::Bool(false)).expect("落盘成功");

    let text = std::fs::read_to_string(&path).expect("读回");
    let value: serde_json::Value = serde_json::from_str(&text).expect("合法 JSON");
    assert_eq!(
        value["autoCompact"],
        serde_json::Value::Bool(false),
        "目标键应被覆盖"
    );
    assert_eq!(
        value["theme"],
        serde_json::Value::String("dark".into()),
        "未知键不得丢失"
    );
    assert_eq!(
        value["nested"]["a"],
        serde_json::json!(1),
        "嵌套未知键不得丢失"
    );
    assert!(text.ends_with('\n'), "写回应带尾部换行");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 文件不存在时创建；顶层不是对象时返回错误（不静默覆盖）。
#[test]
fn persist_creates_file_and_rejects_non_object() {
    let dir = tempdir();
    persist_setting(&dir, "quietStartup", serde_json::Value::Bool(true)).expect("新建成功");
    let text = std::fs::read_to_string(dir.join("settings.json")).expect("读回");
    assert!(
        text.contains("\"quietStartup\": true"),
        "应写入新键：{text}"
    );

    std::fs::write(dir.join("settings.json"), "[1,2]").expect("写数组");
    let err = persist_setting(&dir, "quietStartup", serde_json::Value::Bool(false))
        .expect_err("顶层数组应报错");
    assert!(err.contains("顶层不是 JSON 对象"), "错误应说明原因：{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// id → JSON 键 + 值类型的映射：抽查布尔/数字/字符串三类，并确认子菜单条目返回 `None`。
#[test]
fn json_key_and_value_mapping() {
    assert_eq!(
        json_key_for("autocompact"),
        Some(("autoCompact", SettingKind::Bool))
    );
    assert_eq!(
        json_key_for("editor-padding"),
        Some(("editorPaddingX", SettingKind::Number))
    );
    assert_eq!(
        json_key_for("transport"),
        Some(("transport", SettingKind::Text))
    );
    assert_eq!(json_key_for("theme"), None, "主题是子菜单，不走这里");

    assert_eq!(
        json_value_for(SettingKind::Bool, "false"),
        Some(serde_json::Value::Bool(false))
    );
    assert_eq!(
        json_value_for(SettingKind::Bool, "yes"),
        None,
        "非法布尔不得落盘"
    );
    assert_eq!(
        json_value_for(SettingKind::Number, "2"),
        Some(serde_json::json!(2))
    );
    assert_eq!(json_value_for(SettingKind::Number, "two"), None);
    assert_eq!(
        json_value_for(SettingKind::Text, "auto"),
        Some(serde_json::Value::String("auto".into()))
    );
}

/// 生产路径：`persist_change` 把界面的 `("autocompact","false")` 写成 `"autoCompact": false`。
/// 同时验证"标签值不落盘"的分支（`http-idle-timeout` 的 `5 min` ⇒ `Ok(false)`，文件不变）。
#[test]
fn persist_change_writes_expected_json() {
    let dir = tempdir();
    assert_eq!(persist_change(&dir, "autocompact", "false"), Ok(true));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).expect("读回"))
            .expect("合法 JSON");
    assert_eq!(value["autoCompact"], serde_json::Value::Bool(false));

    assert_eq!(persist_change(&dir, "editor-padding", "2"), Ok(true));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).expect("读回"))
            .expect("合法 JSON");
    assert_eq!(value["editorPaddingX"], serde_json::json!(2));

    // 标签值：`5 min` 走标签→毫秒表 ⇒ 写 300000（ref `http-dispatcher.ts:8-14`）
    assert_eq!(persist_change(&dir, "http-idle-timeout", "5 min"), Ok(true));
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).expect("读回"))
            .expect("合法 JSON");
    assert_eq!(value["httpIdleTimeoutMs"], serde_json::json!(300000));
    assert_eq!(
        persist_change(&dir, "http-idle-timeout", "disabled"),
        Ok(true)
    );
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).expect("读回"))
            .expect("合法 JSON");
    assert_eq!(
        value["httpIdleTimeoutMs"],
        serde_json::json!(0),
        "disabled 应写 0"
    );
    // 未知标签 ⇒ 不写、不报错
    let before = std::fs::read_to_string(dir.join("settings.json")).expect("读回");
    assert_eq!(
        persist_change(&dir, "http-idle-timeout", "9 min"),
        Ok(false)
    );
    let after = std::fs::read_to_string(dir.join("settings.json")).expect("读回");
    assert_eq!(before, after, "无效标签不得改动文件");

    // 子菜单条目：不落盘
    assert_eq!(persist_change(&dir, "theme", "light"), Ok(false));
    let _ = std::fs::remove_dir_all(&dir);
}

/// 嵌套段落盘（`warnings.anthropicExtraUsage`）：段不在时创建、段被保留、段不是对象时报错。
#[test]
fn persist_nested_section() {
    let dir = tempdir();
    std::fs::write(
        dir.join("settings.json"),
        r#"{"theme":"dark","warnings":{"other":1}}"#,
    )
    .expect("写初始 settings");
    persist_nested_setting(
        &dir,
        "warnings",
        "anthropicExtraUsage",
        serde_json::Value::Bool(false),
    )
    .expect("写入成功");
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).expect("读回"))
            .expect("合法 JSON");
    assert_eq!(
        value["warnings"]["anthropicExtraUsage"],
        serde_json::json!(false)
    );
    assert_eq!(
        value["warnings"]["other"],
        serde_json::json!(1),
        "段内其它键不得丢失"
    );
    assert_eq!(
        value["theme"],
        serde_json::json!("dark"),
        "顶层其它键不得丢失"
    );

    // 段存在但不是对象 ⇒ 报错
    std::fs::write(dir.join("settings.json"), r#"{"warnings":true}"#).expect("写坏段");
    let err = persist_nested_setting(
        &dir,
        "warnings",
        "anthropicExtraUsage",
        serde_json::Value::Bool(true),
    )
    .expect_err("段不是对象应报错");
    assert!(err.contains("不是 JSON 对象"), "错误应说明原因：{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
