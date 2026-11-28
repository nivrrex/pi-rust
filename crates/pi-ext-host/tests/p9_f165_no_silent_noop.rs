//! `runner.mjs` 里**未实现**的 API 必须是"警告后无效"，不能是静默空实现。
//!
//! 依据：`LIMITATIONS.md` 明确承诺未实现成员经 `noteNotImplemented` 打
//! `[ext-host] … 未实现` 警告（不静默）。此前 `unregisterProvider() {}` 是唯一
//! 的空实现——扩展调用后既没生效、也没有任何提示。
//!
//! 本用例只读 `runner.mjs` 源码，不需要 node：钉住"哪些名字必须出现在
//! `noteNotImplemented(...)` 里"。

fn runner_source() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tools/ext-host/runner.mjs");
    std::fs::read_to_string(&path).expect("读 runner.mjs")
}

#[test]
fn every_unimplemented_api_warns_instead_of_no_op() {
    let source = runner_source();
    // 未实现的 API 清单（与 LIMITATIONS 的 `ExtensionAPI` / `ExtensionUIContext` 一致）。
    let expected = [
        "pi.registerShortcut",
        "pi.registerMessageRenderer",
        "pi.registerEntryRenderer",
        "pi.registerProvider",
        "pi.unregisterProvider",
        "pi.sendMessage",
        "pi.exec",
        "pi.setModel",
    ];
    let missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|name| !source.contains(&format!("noteNotImplemented(\"{name}\")")))
        .collect();
    assert!(
        missing.is_empty(),
        "这些未实现的 API 没有打警告（会静默失败）：{missing:?}"
    );
}

/// 已实现的 API **不应**出现 `noteNotImplemented` 调用（否则会把可用功能报成未实现）。
#[test]
fn implemented_apis_do_not_claim_to_be_missing() {
    let source = runner_source();
    for name in [
        "pi.setLabel",
        "pi.setActiveTools",
        "pi.setSessionName",
        "pi.getSessionName",
        "pi.setThinkingLevel",
        "pi.getThinkingLevel",
    ] {
        assert!(
            !source.contains(&format!("noteNotImplemented(\"{name}\")")),
            "{name} 已实现，不应再打未实现警告"
        );
    }
}
