//! 会话文件的路径规则，对应 `packages/coding-agent/src/core/session-manager.ts:264-278、589-594`
//! 与 `spec/c-session.md` 第 1 节。纯函数，无 I/O。

use std::path::{Path, PathBuf};

/// 会话 id 的校验失败。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character")]
pub struct InvalidSessionId;

/// 校验会话 id：非空、只含字母数字与 `-_.`、且首尾必须是字母数字。
///
/// 逐字对应 `session-manager.ts:269-274` 的正则与文案。
pub fn assert_valid_session_id(id: &str) -> Result<(), InvalidSessionId> {
    let bytes = id.as_bytes();
    let Some(first) = bytes.first() else {
        return Err(InvalidSessionId);
    };
    let Some(last) = bytes.last() else {
        return Err(InvalidSessionId);
    };
    let alnum = |byte: u8| byte.is_ascii_alphanumeric();
    if !alnum(*first) || !alnum(*last) {
        return Err(InvalidSessionId);
    }
    if id.len() == 1 {
        return Ok(());
    }
    if bytes[1..bytes.len() - 1]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Ok(())
    } else {
        Err(InvalidSessionId)
    }
}

/// 默认会话目录：`<agentDir>/sessions/--<编码后的 cwd>--`。
///
/// 编码规则（`session-manager.ts:592`）：去掉开头的 `/` 或 `\`，再把其余的 `/`、`\`、`:`
/// 全部换成 `-`；最后用**前后各两个横线**包起来。
pub fn default_session_dir(cwd: &str, agent_dir: &str) -> PathBuf {
    let trimmed = cwd
        .strip_prefix('/')
        .or_else(|| cwd.strip_prefix('\\'))
        .unwrap_or(cwd);
    let encoded: String = trimmed
        .chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':') {
                '-'
            } else {
                ch
            }
        })
        .collect();
    Path::new(agent_dir)
        .join("sessions")
        .join(format!("--{encoded}--"))
}

/// 会话文件名：`<fileTimestamp>_<sessionId>.jsonl`。
///
/// `fileTimestamp` 是 ISO 时间戳把 `:` 与 `.` 换成 `-`（`session-manager.ts:1079`）。
pub fn session_file_name(iso_timestamp: &str, session_id: &str) -> String {
    let file_timestamp: String = iso_timestamp
        .chars()
        .map(|ch| if matches!(ch, ':' | '.') { '-' } else { ch })
        .collect();
    format!("{file_timestamp}_{session_id}.jsonl")
}

/// 会话文件的完整路径。
pub fn session_file_path(
    cwd: &str,
    agent_dir: &str,
    iso_timestamp: &str,
    session_id: &str,
) -> PathBuf {
    default_session_dir(cwd, agent_dir).join(session_file_name(iso_timestamp, session_id))
}

/// 生成会话 id（uuid v7，与 TS 的 `createSessionId()` 一致）。
pub fn create_session_id() -> String {
    uuid::Uuid::now_v7().to_string()
}
