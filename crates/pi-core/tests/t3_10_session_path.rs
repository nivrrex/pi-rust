//! P3-12：会话路径规则——id 校验、目录编码、文件名。
//!
//! 判据：`session-manager.ts:264-278、589-594、1079`（见 `spec/c-session.md` 第 1 节）。

use pi_core::session_path::{
    assert_valid_session_id, create_session_id, default_session_dir, session_file_name,
    session_file_path, InvalidSessionId,
};

#[test]
fn valid_session_ids_are_accepted() {
    for id in [
        "a",
        "abc",
        "A1",
        "a-b",
        "a_b",
        "a.b",
        "0f8e2c1a-1b2c-3d4e-5f60-718293a4b5c6",
    ] {
        assert!(assert_valid_session_id(id).is_ok(), "{id} 应当合法");
    }
}

#[test]
fn invalid_session_ids_are_rejected_with_the_typescript_message() {
    for id in ["", "-abc", "abc-", ".abc", "_abc", "a b", "a/b", "中文"] {
        let error = assert_valid_session_id(id).expect_err(&format!("{id} 应当非法"));
        assert_eq!(error, InvalidSessionId);
    }
    assert_eq!(
        InvalidSessionId.to_string(),
        "Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character"
    );
}

#[test]
fn session_id_is_a_uuid_v7() {
    let id = create_session_id();
    assert_valid_session_id(&id).expect("生成的 id 合法");
    // uuid v7 的版本位是 7。
    let version = id.chars().nth(14).expect("有版本位");
    assert_eq!(version, '7', "createSessionId 用的是 uuidv7：{id}");
}

#[test]
fn default_session_dir_encodes_the_cwd() {
    // 去掉开头的 `/`，其余 `/`、`\`、`:` 换成 `-`，前后各两个横线。
    assert_eq!(
        default_session_dir("/home/me/proj", "/cfg/agent"),
        std::path::PathBuf::from("/cfg/agent/sessions/--home-me-proj--")
    );
    assert_eq!(
        default_session_dir("/tmp/a:b", "/cfg"),
        std::path::PathBuf::from("/cfg/sessions/--tmp-a-b--")
    );
}

#[test]
fn relative_cwd_is_encoded_without_leading_separator() {
    assert_eq!(
        default_session_dir("proj/sub", "/cfg"),
        std::path::PathBuf::from("/cfg/sessions/--proj-sub--")
    );
}

#[test]
fn session_file_name_replaces_colons_and_dots_in_the_timestamp() {
    // ISO 时间戳里的 `:` 与 `.` 换成 `-`。
    assert_eq!(
        session_file_name("2026-01-02T03:04:05.678Z", "abc"),
        "2026-01-02T03-04-05-678Z_abc.jsonl"
    );
}

#[test]
fn session_file_path_joins_the_rules() {
    let path = session_file_path("/home/me/proj", "/cfg", "2026-01-02T03:04:05.678Z", "abc");
    assert_eq!(
        path,
        std::path::PathBuf::from(
            "/cfg/sessions/--home-me-proj--/2026-01-02T03-04-05-678Z_abc.jsonl"
        )
    );
}
