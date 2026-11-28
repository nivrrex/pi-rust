//! F47/W2：页脚 `(provider)` 前缀的判据是**可用 provider 数**（ref `footer.ts:193-200`），
//! 而 ref 的可用集合来自模型运行时快照（`interactive-mode.ts:5091-5099`
//! `uniqueProviders.size`），**没有凭据的 provider 不进快照**。
//! 此前 pi-rs 直接数 `models.json` 的 provider 条目 ⇒ 多算 ⇒ 本应 `auto` 时显示
//! `(mw-auto) auto`（真机 Linux 对拍：TS `auto`、pi-rs `(mw-auto) auto`）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

fn config(json: &str) -> pi_ai::config::ModelsConfig {
    serde_json::from_str(json).expect("models.json 应可解析")
}

#[test]
fn providers_without_credentials_are_not_counted() {
    let dir = tempfile::tempdir().expect("临时目录");
    let cfg = config(
        r#"{"providers":{
            "with-key":{"apiKey":"k","models":[{"id":"a"}]},
            "no-key":{"models":[{"id":"b"}]},
            "blank-key":{"apiKey":"  ","models":[{"id":"c"}]}
        }}"#,
    );
    assert_eq!(
        pi_cli::runtime::available_provider_count(&cfg, dir.path()),
        1,
        "只有带非空 apiKey 的 provider 算可用"
    );
}

#[test]
fn providers_with_auth_json_entry_are_counted() {
    let dir = tempfile::tempdir().expect("临时目录");
    std::fs::write(
        dir.path().join("auth.json"),
        r#"{"from-auth":{"type":"api_key","key":"k"}}"#,
    )
    .expect("写 auth.json");
    let cfg = config(
        r#"{"providers":{
            "from-auth":{"models":[{"id":"a"}]},
            "nothing":{"models":[{"id":"b"}]}
        }}"#,
    );
    assert_eq!(
        pi_cli::runtime::available_provider_count(&cfg, dir.path()),
        1,
        "auth.json 里有条目的 provider 算可用"
    );
    assert!(
        dir.path().join("auth.json").exists(),
        "{} 应存在",
        Path::new("auth.json").display()
    );
}
