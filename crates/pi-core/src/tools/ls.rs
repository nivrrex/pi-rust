//! `ls` 工具：列目录（只读，默认不启用）。
//!
//! 对应上游 `packages/coding-agent/src/core/tools/ls.ts`（175 行）：
//! - schema：`path`（可选，默认当前目录）、`limit`（可选，默认 500）；
//! - 条目**按名字小写排序**（`:120-121` 的 `localeCompare`），目录加 `/` 后缀；
//! - 无法 `stat` 的条目**跳过**（`:139-144`）；
//! - 空目录 → `(empty directory)`（`:148-150`）；
//! - 超 `limit` 或超 50KB 时在末尾追加 `[<notices 用 ". " 连接>]`（`:156-160`）。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::read_text::{truncate_head, DEFAULT_MAX_BYTES};
use super::tool::{Tool, ToolContext, ToolResult};

/// 默认条目上限（`ls.ts:24`）。
const DEFAULT_LIMIT: usize = 500;

/// `ls` 工具。
#[derive(Debug, Default)]
pub struct LsTool;

/// 参数 schema（`ls.ts:14-17`）。
#[must_use]
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": {"type": "string", "description": "Directory to list (default: current directory)"},
            "limit": {"type": "number", "description": "Maximum number of entries to return (default: 500)"}
        }
    })
}

impl Tool for LsTool {
    fn name(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        r#"List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Output is truncated to 500 entries or 50KB (whichever is hit first)."#
    }

    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(schema)
    }

    fn execute<'a>(
        &'a self,
        arguments: Value,
        context: &'a ToolContext,
        _cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, ToolResult> {
        Box::pin(async move { list(&arguments, &context.cwd) })
    }
}

/// 纯逻辑 + I/O 的执行体（把 `dir` 与参数分开，便于单测）。
fn list(arguments: &Value, cwd: &Path) -> ToolResult {
    let requested = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(".");
    let dir: PathBuf = resolve(cwd, requested);
    let limit = arguments
        .get("limit")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map_or(DEFAULT_LIMIT, |value| value as usize);

    if !dir.exists() {
        return ToolResult::error(format!("Path not found: {}", dir.display()));
    }
    let Ok(metadata) = std::fs::metadata(&dir) else {
        return ToolResult::error(format!("Path not found: {}", dir.display()));
    };
    if !metadata.is_dir() {
        return ToolResult::error(format!("Not a directory: {}", dir.display()));
    }
    let Ok(reader) = std::fs::read_dir(&dir) else {
        return ToolResult::error(format!("Cannot read directory: {}", dir.display()));
    };
    let mut names: Vec<String> = reader
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    // 大小写不敏感的字典序（`ls.ts:120-121` 的 `localeCompare` 近似）。
    names.sort_by_key(|name| name.to_lowercase());

    let mut results: Vec<String> = Vec::new();
    let mut entry_limit_reached = false;
    for name in names {
        if results.len() >= limit {
            entry_limit_reached = true;
            break;
        }
        let full = dir.join(&name);
        // 无法 stat 的条目跳过（`ls.ts:139-144`）。
        let Ok(entry_metadata) = std::fs::symlink_metadata(&full) else {
            continue;
        };
        let is_dir =
            entry_metadata.is_dir() || (entry_metadata.file_type().is_symlink() && full.is_dir());
        results.push(if is_dir { format!("{name}/") } else { name });
    }

    if results.is_empty() {
        return ToolResult::text("(empty directory)");
    }

    let raw = results.join("\n");
    let truncation = truncate_head(&raw, usize::MAX, DEFAULT_MAX_BYTES);
    let mut output = truncation.content;
    let mut notices: Vec<String> = Vec::new();
    if entry_limit_reached {
        notices.push(format!(
            "{limit} entries limit reached. Use limit={} for more",
            limit.saturating_mul(2)
        ));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join(". ")));
    }
    ToolResult::text(output)
}

/// 相对路径按 cwd 归一（上游 `resolveToCwd`）；绝对路径原样用。
fn resolve(cwd: &Path, requested: &str) -> PathBuf {
    let path = Path::new(requested);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// 字节数的人类可读形态（上游 `truncate.ts` 的 `formatSize`：`50KB`）。
fn format_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{}MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tool::ToolContext;

    fn context(dir: &Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_path_buf(),
        }
    }

    fn run(dir: &Path, arguments: Value) -> String {
        let tool = LsTool;
        let cancel = CancellationToken::new();
        let result = futures::executor::block_on(tool.execute(arguments, &context(dir), &cancel));
        match result.content.first() {
            Some(crate::tools::tool::ToolResultBlock::Text { text }) => text.clone(),
            _other => String::new(),
        }
    }

    fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pi-ls-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).expect("建目录");
        std::fs::write(dir.join("b.txt"), "b").expect("写文件");
        std::fs::write(dir.join("A.txt"), "a").expect("写文件");
        std::fs::write(dir.join(".hidden"), "h").expect("写文件");
        dir
    }

    /// 排序（大小写不敏感）、目录 `/` 后缀、含点文件（`ls.ts:110-146`）。
    #[test]
    fn lists_sorted_with_directory_suffix_and_dotfiles() {
        let dir = fixture("sorted");
        let text = run(&dir, json!({}));
        assert_eq!(text, ".hidden\nA.txt\nb.txt\nsub/");
    }

    /// 空目录 → `(empty directory)`（`ls.ts:148-150`）。
    #[test]
    fn empty_directory_message() {
        let dir = std::env::temp_dir().join(format!("pi-ls-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建目录");
        assert_eq!(run(&dir, json!({})), "(empty directory)");
    }

    /// `limit` 到顶时追加可操作提示（`ls.ts:152-158`）。
    #[test]
    fn limit_notice_is_appended() {
        let dir = fixture("limit");
        let text = run(&dir, json!({"limit": 2}));
        assert!(text.starts_with(".hidden\nA.txt"), "取前两个：{text}");
        assert!(
            text.contains("[2 entries limit reached. Use limit=4 for more]"),
            "{text}"
        );
    }

    /// 路径不存在 / 不是目录的错误文案（`ls.ts:89-100`）。
    #[test]
    fn errors_match_upstream_text() {
        let dir = fixture("errors");
        let missing = dir.join("nope");
        assert_eq!(
            run(&dir, json!({"path": "nope"})),
            format!("Path not found: {}", missing.display())
        );
        let file = dir.join("b.txt");
        assert_eq!(
            run(&dir, json!({"path": "b.txt"})),
            format!("Not a directory: {}", file.display())
        );
    }
}
