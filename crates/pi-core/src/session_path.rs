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

/// 给定目录是否就是"当前 cwd 的默认会话分区"。
///
/// ref `session-manager.ts:1855-1860`（`list`）与 `:1791-1796`（`continueRecent`）用同一条规则：
/// `filterCwd = sessionDir !== undefined && dir !== getDefaultSessionDirPath(cwd)`。
/// **只在"用户显式指定了自定义会话目录"时才按 header 里的 cwd 过滤**；
/// 默认分区目录（`sessions/--<cwd>--/`）里反过来**不**过滤 —— 这正是
/// "目录被改名（大小写/重音/整名）后旧会话仍能 `/resume` 与 `-c`"的原因。
#[must_use]
pub fn is_default_session_dir(session_dir: &Path, cwd: &str, agent_dir: &str) -> bool {
    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }
    canonical(session_dir) == canonical(&default_session_dir(cwd, agent_dir))
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

#[cfg(test)]
mod default_dir_tests {
    use super::{default_session_dir, is_default_session_dir};

    #[test]
    fn recognizes_default_partition_dir() {
        let dir = default_session_dir("/root/Desktop/colibri", "/agent");
        assert!(is_default_session_dir(
            &dir,
            "/root/Desktop/colibri",
            "/agent"
        ));
        // 尾部斜杠不影响判定（ref `resolvePath` 会规范化）
        let with_slash = format!("{}/", dir.display());
        assert!(is_default_session_dir(
            std::path::Path::new(&with_slash),
            "/root/Desktop/colibri",
            "/agent"
        ));
    }

    /// 自定义目录（不是当前 cwd 的默认分区）⇒ 需要按 header cwd 过滤。
    #[test]
    fn custom_dir_is_not_default() {
        let custom = std::path::Path::new("/agent/sessions/custom");
        assert!(!is_default_session_dir(
            custom,
            "/root/Desktop/colibri",
            "/agent"
        ));
    }

    /// 大小写/重音不同的目录名是**不同**的默认分区（Linux 文件系统语义），
    /// 所以"改名后旧会话仍可见"必须靠"默认目录不过滤"，而不是靠模糊匹配路径。
    #[test]
    fn differently_spelled_cwd_is_a_different_partition() {
        let old = default_session_dir("/root/Desktop/Colibrì", "/agent");
        assert!(!is_default_session_dir(
            &old,
            "/root/Desktop/colibri",
            "/agent"
        ));
    }
}
