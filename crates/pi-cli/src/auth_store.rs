//! `auth.json` 的读写（给 `/login`、`/logout` 用）。
//!
//! 格式与 ref 完全一致（`core/auth-storage.ts:25`、`:106`、`:358`）：
//! ```json
//! { "<provider>": { "type": "api_key", "key": "…" } }
//! ```
//! - 序列化用 `JSON.stringify(data, null, 2)` 等价的两空格缩进 + 末尾无换行差异由 ref 决定
//!   （ref 直接写字符串，故这里不追加换行）。
//! - 写入用 `mode: 0o600`（ref 同），Unix 下临时文件即设为 0600，再 `rename` 原子替换。
//! - **保留未知键**：既有 provider 条目里 ref 未定义的字段、以及其他 provider 条目都原样写回。

use std::path::Path;

/// 写 `auth.json` 时的错误。**不携带 key 明文**（只带 provider 名与 I/O 描述）。
#[derive(Debug, thiserror::Error)]
pub enum AuthStoreError {
    #[error("provider name must not be empty")]
    EmptyProvider,
    #[error("API key must not be empty")]
    EmptyKey,
    #[error("auth.json is not a JSON object")]
    NotAnObject,
    #[error("cannot read auth.json: {0}")]
    Read(String),
    #[error("cannot write auth.json: {0}")]
    Write(String),
    #[error("cannot lock auth.json: {0}")]
    Lock(String),
}

fn read_object(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>, AuthStoreError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(AuthStoreError::Read(error.to_string())),
    };
    if text.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| AuthStoreError::Read(error.to_string()))?;
    if let serde_json::Value::Object(map) = parsed {
        Ok(map)
    } else {
        Err(AuthStoreError::NotAnObject)
    }
}

/// 原子写入：同目录临时文件 + Unix 0600 + rename。跨进程锁由调用方持有。
fn write_atomic(
    path: &Path,
    data: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), AuthStoreError> {
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(data.clone()))
        .map_err(|error| AuthStoreError::Write(error.to_string()))?;
    crate::config_store::write_atomic(path, &text, true)
        .map_err(|error| AuthStoreError::Write(error.to_string()))
}

/// 写入 `type: "api_key"` 条目；已有同 provider 条目里的其它字段（未知键）保留。
pub fn set_api_key(path: &Path, provider: &str, key: &str) -> Result<(), AuthStoreError> {
    if provider.is_empty() {
        return Err(AuthStoreError::EmptyProvider);
    }
    if key.trim().is_empty() {
        return Err(AuthStoreError::EmptyKey);
    }
    let guard = crate::config_store::ConfigLockGuard::acquire(
        path,
        std::time::Duration::from_secs(30),
        std::time::Duration::from_secs(30),
    )
    .map_err(|error| AuthStoreError::Lock(error.to_string()))?;
    let result = (|| {
        let mut data = read_object(path)?;
        let mut entry = match data.remove(provider) {
            Some(serde_json::Value::Object(map)) => map,
            Some(_) | None => serde_json::Map::new(),
        };
        entry.insert(
            "type".to_string(),
            serde_json::Value::String("api_key".to_string()),
        );
        entry.insert(
            "key".to_string(),
            serde_json::Value::String(key.to_string()),
        );
        data.insert(provider.to_string(), serde_json::Value::Object(entry));
        write_atomic(path, &data)
    })();
    guard
        .check()
        .map_err(|error| AuthStoreError::Lock(error.to_string()))?;
    guard
        .release()
        .map_err(|error| AuthStoreError::Lock(error.to_string()))?;
    result
}

/// 删除某 provider 的凭据（`/logout`）；其它条目与未知键保持不变。
///
/// 文件不存在或没有该条目时返回 `Ok(false)`。
pub fn remove_provider(path: &Path, provider: &str) -> Result<bool, AuthStoreError> {
    if provider.is_empty() {
        return Err(AuthStoreError::EmptyProvider);
    }
    let guard = crate::config_store::ConfigLockGuard::acquire(
        path,
        std::time::Duration::from_secs(30),
        std::time::Duration::from_secs(30),
    )
    .map_err(|error| AuthStoreError::Lock(error.to_string()))?;
    let result = (|| {
        let mut data = read_object(path)?;
        if data.remove(provider).is_none() {
            return Ok(false);
        }
        write_atomic(path, &data)?;
        Ok(true)
    })();
    guard
        .check()
        .map_err(|error| AuthStoreError::Lock(error.to_string()))?;
    guard
        .release()
        .map_err(|error| AuthStoreError::Lock(error.to_string()))?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tmpdir");
        let path = dir.path().join("auth.json");
        (dir, path)
    }

    #[test]
    fn writes_ref_format_and_preserves_unknown_keys() {
        let (_dir, path) = temp_path();
        std::fs::write(
            &path,
            r#"{"other":{"type":"api_key","key":"k1"},"anthropic":{"type":"api_key","key":"old","extraField":{"a":1}},"top":"keep"}"#,
        )
        .expect("seed");
        set_api_key(&path, "anthropic", "new-key").expect("set");
        let text = std::fs::read_to_string(&path).expect("read");
        // 期望值取 ref 的 JSON.stringify(data, null, 2) 形状，不由被测函数生成
        assert!(text.contains("\"type\": \"api_key\""), "{text}");
        assert!(text.contains("\"key\": \"new-key\""), "{text}");
        assert!(text.contains("\"key\": \"k1\""), "其它 provider 保留");
        assert!(text.contains("\"extraField\""), "未知键保留");
        assert!(text.contains("\"top\": \"keep\""), "未知顶层键保留");
        assert!(!text.contains("old"), "旧 key 被覆盖");
    }

    #[test]
    fn rejects_empty_key_and_provider() {
        let (_dir, path) = temp_path();
        assert!(matches!(
            set_api_key(&path, "p", "  "),
            Err(AuthStoreError::EmptyKey)
        ));
        assert!(matches!(
            set_api_key(&path, "", "k"),
            Err(AuthStoreError::EmptyProvider)
        ));
        assert!(!path.exists(), "拒绝时不得落盘");
    }

    #[test]
    fn unix_permissions_are_600() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let (_dir, path) = temp_path();
            set_api_key(&path, "p", "k").expect("set");
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "ref AUTH_FILE_WRITE_OPTIONS.mode=0o600");
        }
    }

    #[test]
    fn concurrent_writes_do_not_corrupt() {
        let (_dir, path) = temp_path();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|index| {
                let path = path.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    set_api_key(&path, &format!("p{index}"), &format!("k{index}"))
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("join").expect("write");
        }
        // 原子 rename 只保证“不会读到半截 JSON”，不保证读改写期间不互相覆盖。
        // 所有 provider 都必须保留，否则并发登录会静默丢凭据。
        let text = std::fs::read_to_string(&path).expect("read");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        for index in 0..16 {
            assert_eq!(
                parsed.get(format!("p{index}")),
                Some(&serde_json::json!({ "type": "api_key", "key": format!("k{index}") })),
                "并发写入不得丢失 p{index}"
            );
        }
        // 不残留临时文件
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("readdir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "临时文件未清理");
    }

    #[test]
    fn remove_provider_keeps_others() {
        let (_dir, path) = temp_path();
        set_api_key(&path, "a", "ka").expect("a");
        set_api_key(&path, "b", "kb").expect("b");
        assert!(remove_provider(&path, "a").expect("remove"));
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(!text.contains("\"ka\""), "被删条目应消失");
        assert!(text.contains("\"kb\""), "其它条目保留");
        assert!(!remove_provider(&path, "missing").expect("remove-missing"));
    }

    #[test]
    fn errors_never_leak_key_material() {
        let dir = tempfile::tempdir().expect("tmpdir");
        // 目录不可写（用文件占位父路径制造写入失败）
        let blocked = dir.path().join("file-not-dir");
        std::fs::write(&blocked, "x").expect("seed");
        let path = blocked.join("auth.json");
        let error = set_api_key(&path, "p", "SECRET-MATERIAL-123").expect_err("must fail");
        let text = error.to_string();
        assert!(
            !text.contains("SECRET-MATERIAL-123"),
            "错误文案泄漏 key: {text}"
        );
    }
}
